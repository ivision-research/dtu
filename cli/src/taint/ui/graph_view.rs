use std::{
    cell::Cell,
    collections::{HashMap, HashSet},
    ops::{Deref, DerefMut},
    str::FromStr,
};

use anyhow::{bail, Context as AnyContext};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use dtu::{
    analysis::taint::{
        db::{
            Filter, ResolvableIds, ResolvedNode, ResolvedTaintGraph, ResolvedTaintGraphs,
            ResolvedTaintSink,
        },
        models::{AnalyzedMethodId, NodeId, SubgraphId},
    },
    db::graph::{models::MethodId, GraphDatabase, MethodSpec},
    utils::rccow::RcStr,
};
use itertools::Itertools;
use ratatui::{
    layout::{Alignment, Constraint, Layout},
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState, Wrap},
    Frame,
};

use crate::{
    taint::ui::{
        config::{Config, Detail},
        Command, CommandHandler, Tools, Window, WindowAction,
    },
    ui::fit,
    utils::dtu_open_method_spec,
};

static KEYS_HELP: &str = "\
j / k          move the cursor, arrows work too
^d / ^u        move half a screen
PgDn / PgUp    move a whole screen
g / G          first / last row
h / l          pan sideways, arrows work too
d              change detail level
<space>        collapse or expand, follow a `continued at` row
o              open what the row names
O              open the method the row sits in
f              focus the selected node: draw only the routes reaching it, again to leave
^o             back to where the last `continued at` jump started
s              toggle phi, array and instruction rows
u              toggle unhide all prefixes
r              toggle the rows naming edges the tree cannot draw
H              hide or unhide this graph
.              show hidden graphs
/              search, Enter to go to the first match, Esc to cancel
               collapsed subtrees are searched too and opened to reach a match
n / N          next / previous match
?              this list
:              command, try `help`
q / Esc        back

drag           pan with the mouse
wheel          scroll, shift for sideways";

static COMMANDS: &[(&'static str, Command<State>)] = &[
    (
        "filter",
        Command {
            help: Some("filter nodes, keeping the path back to the entry point"),
            long_help: Some(
                r#"filter class=X method=Y field=Z sig=S ret=R

Matching nodes are highlighted and everything that isn't a match or an
ancestor of one is hidden. Run with no arguments to clear."#,
            ),
            func: do_filter,
        },
    ),
    (
        "clear",
        Command {
            help: Some("clear the active filter"),
            long_help: None,
            func: clear_filter,
        },
    ),
    (
        "expand",
        Command {
            help: Some("expand every subtree, ignoring the configured cutoff"),
            long_help: None,
            func: expand_all,
        },
    ),
    (
        "collapse",
        Command {
            help: Some("collapse every subtree"),
            long_help: None,
            func: collapse_all,
        },
    ),
    (
        "hidden-pkgs",
        Command {
            help: Some("show all hidden packages"),
            long_help: None,
            func: show_hidden_pkgs,
        },
    ),
    (
        "hide-pkg",
        Command {
            help: Some("hide nodes matching the package prefix [additive]"),
            long_help: None,
            func: hide_package_prefix,
        },
    ),
    (
        "unhide-pkg",
        Command {
            help: Some("unhide a provided package prefix if hidden"),
            long_help: None,
            func: unhide_package_prefix,
        },
    ),
    (
        "unhide-all-pkg",
        Command {
            help: Some("unhide all hidden package prefixes"),
            long_help: None,
            func: unhide_package_all,
        },
    ),
];

fn show_hidden_pkgs(
    _args: Vec<String>,
    _tools: &Tools,
    _cfg: &mut Config,
    state: &mut State,
) -> anyhow::Result<()> {
    if state.hidden_prefixes.len() == 0 {
        state.message.set(Some(String::from("no hidden prefixes")));
        return Ok(());
    }

    let msg = state.hidden_prefixes.iter().join("\n");
    state.message.set(Some(msg));

    Ok(())
}

fn unhide_package_all(
    _args: Vec<String>,
    _tools: &Tools,
    _cfg: &mut Config,
    state: &mut State,
) -> anyhow::Result<()> {
    state.hidden_prefixes.clear();
    state.rebuild_visible();
    Ok(())
}

fn hide_package_prefix(
    args: Vec<String>,
    _tools: &Tools,
    _cfg: &mut Config,
    state: &mut State,
) -> anyhow::Result<()> {
    if args.len() == 0 {
        bail!("require at least one package prefix: Lfoo/bar");
    }
    state.hidden_prefixes.reserve(args.len());

    for arg in args {
        if !arg.starts_with('L') {
            state
                .hidden_prefixes
                .push(format!("L{}", arg.replace('.', "/")));
        } else {
            state.hidden_prefixes.push(arg);
        }
    }
    state.rebuild_visible();

    Ok(())
}

fn unhide_package_prefix(
    args: Vec<String>,
    _tools: &Tools,
    _cfg: &mut Config,
    state: &mut State,
) -> anyhow::Result<()> {
    if args.len() == 0 {
        bail!("require at least one package prefix: Lfoo/bar");
    }

    let search_args = args
        .into_iter()
        .map(|it| {
            if !it.starts_with('L') {
                format!("L{}", it.replace('.', "/"))
            } else {
                it
            }
        })
        .collect::<Vec<_>>();

    let prefixes = std::mem::take(&mut state.hidden_prefixes)
        .into_iter()
        .filter(|it| !search_args.iter().any(|remove| it == remove));

    state.hidden_prefixes = prefixes.collect::<Vec<_>>();
    state.rebuild_visible();
    Ok(())
}

fn expand_all(
    _args: Vec<String>,
    _tools: &Tools,
    _cfg: &mut Config,
    state: &mut State,
) -> anyhow::Result<()> {
    state.set_all_collapsed(false);
    Ok(())
}

fn collapse_all(
    _args: Vec<String>,
    _tools: &Tools,
    _cfg: &mut Config,
    state: &mut State,
) -> anyhow::Result<()> {
    state.set_all_collapsed(true);
    Ok(())
}

fn clear_filter(
    _args: Vec<String>,
    _tools: &Tools,
    _cfg: &mut Config,
    state: &mut State,
) -> anyhow::Result<()> {
    state.set_filters(Vec::new());
    Ok(())
}

fn do_filter(
    args: Vec<String>,
    _tools: &Tools,
    _cfg: &mut Config,
    state: &mut State,
) -> anyhow::Result<()> {
    if args.is_empty() {
        state.set_filters(Vec::new());
        return Ok(());
    }

    let mut filters = Vec::with_capacity(args.len());
    for arg in args {
        filters.push(Filter::from_str(&arg)?);
    }

    if filters.iter().any(|it| node_level_filter(it).is_none()) {
        log::info!("graph-level filters are ignored in the graph view");
    }

    state.set_filters(filters);
    Ok(())
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum NodeKind {
    GraphRoot,
    /// An incoming edge that wasn't in the reachability graph (which is deliberately incomplete
    /// with respect to edges)
    Reference,
    Phi,
    Array,
    Call,
    Field,
    /// A bare instruction the tainted value flowed through
    Instruction,
    /// A call to a method the graph database does not know
    External,
}

impl NodeKind {
    fn is_structural(self) -> bool {
        matches!(self, Self::Phi | Self::Array | Self::Instruction)
    }
}

impl NodeKind {
    fn color(self) -> Color {
        match self {
            Self::GraphRoot => Color::Cyan,
            Self::Reference => Color::DarkGray,
            Self::Phi => Color::Magenta,
            Self::Array => Color::Blue,
            Self::Call => Color::Reset,
            Self::Field => Color::Green,
            Self::Instruction => Color::DarkGray,
            Self::External => Color::Yellow,
        }
    }
}

/// The pieces a [Filter] matches against, pulled off the resolved sink once so that filtering
/// never has to re-parse a display string.
///
/// Everything is stored lowercased because the database side of the same filters matches through
/// FTS5, which is case insensitive. Matching case sensitively here would hide nodes that the
/// method list said were there.
#[derive(Default)]
struct MatchFields {
    class: String,
    method: String,
    field: String,
    signature: String,
    ret: String,
}

impl MatchFields {
    /// `filter` must already hold a lowercased needle, see [lowercased]
    fn matches(&self, filter: &Filter) -> bool {
        match filter {
            Filter::ClassContains(needle) => self.class.contains(needle.as_str()),
            Filter::MethodContains(needle) => self.method.contains(needle.as_str()),
            Filter::FieldContains(needle) => self.field.contains(needle.as_str()),
            Filter::SigContains(needle) => self.signature.contains(needle.as_str()),
            Filter::RetContains(needle) => self.ret.contains(needle.as_str()),
            // Whole-graph filters never describe a single node
            Filter::NoPhi | Filter::FTS5(_) | Filter::MinLength(_) | Filter::MaxLength(_) => false,
        }
    }
}

/// One drawn row
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct VisibleRow {
    /// Index into [State::flat]
    idx: u32,
    /// Depth among the rows actually drawn, which differs from the node's own depth whenever
    /// something between it and the root is hidden
    depth: u16,
}

/// One node of one graph, flattened into pre-order.
///
/// `parent` always points at a lower index, which is what lets the filter closure and the tree
/// connectors be computed with simple index walks.
struct FlatNode {
    parent: Option<u32>,
    depth: u16,
    /// Nodes in this subtree, including this one, so a collapsed subtree is skipped by adding it
    subtree: u32,
    is_last_child: bool,
    kind: NodeKind,
    /// The graph this row belongs to, `None` on a source header since one spans several
    graph: Option<SubgraphId>,
    /// This row's own node, `None` for a source header
    node: Option<NodeId>,
    /// Set on a [NodeKind::Reference] row: the node it points at
    jump_to: Option<NodeId>,
    /// How many edges arrive at this node within its graph. More than one means the tree is
    /// drawing only one of the routes here.
    ways_in: usize,
    /// `Z.m`
    tiny: RcStr<'static>,
    /// `c/Z.m`
    mid: RcStr<'static>,
    full: RcStr<'static>,
    /// The callee's definition, present only for call nodes
    callee: Option<MethodId>,
    /// The method this node sits inside. Absent only on a source header, so every real row can
    /// be opened at the place the taint actually flows through.
    in_method: Option<MethodId>,
    /// [Self::in_method] as it is shown on the detail line, empty for a source header
    in_method_display: String,
    fields: MatchFields,
    matched: bool,
}

/// `Lcom/foo/Bar;` -> `Bar`
fn simple_class(class: &str) -> &str {
    let class = class.strip_prefix('L').unwrap_or(class);
    let class = class.strip_suffix(';').unwrap_or(class);
    match class.rfind('/') {
        Some(idx) => &class[idx + 1..],
        None => class,
    }
}

/// `Lcom/foo/Bar;` -> `c/f/Bar`
fn compressed_class(class: &str) -> String {
    let class = class.strip_prefix('L').unwrap_or(class);
    let class = class.strip_suffix(';').unwrap_or(class);
    let Some(last) = class.rfind('/') else {
        return class.into();
    };

    let mut out = String::with_capacity(class.len());
    for segment in class[..last].split('/') {
        if let Some(first) = segment.chars().next() {
            out.push(first);
            out.push('/');
        }
    }
    out.push_str(&class[last + 1..]);
    out
}

/// Pull `Bar.baz` out of a display string like `Lcom/foo/Bar;->baz(I)V`
fn tiny_from_display(display: &str) -> String {
    let Some((class, rest)) = display.split_once("->") else {
        return display.into();
    };
    let member = rest
        .split_once('(')
        .map(|(name, _)| name)
        .unwrap_or_else(|| rest.split_once(':').map(|(name, _)| name).unwrap_or(rest));
    format!("{}.{}", simple_class(class), member)
}

/// A copy of `filter` with its needle lowercased, to match against [MatchFields]
fn lowercased(filter: &Filter) -> Filter {
    match filter {
        Filter::ClassContains(s) => Filter::ClassContains(s.to_lowercase()),
        Filter::MethodContains(s) => Filter::MethodContains(s.to_lowercase()),
        Filter::FieldContains(s) => Filter::FieldContains(s.to_lowercase()),
        Filter::SigContains(s) => Filter::SigContains(s.to_lowercase()),
        Filter::RetContains(s) => Filter::RetContains(s.to_lowercase()),
        other => other.clone(),
    }
}

/// The node-level predicate for a [Filter], or `None` for filters that only describe a whole graph
fn node_level_filter(filter: &Filter) -> Option<&Filter> {
    match filter {
        Filter::ClassContains(_)
        | Filter::MethodContains(_)
        | Filter::FieldContains(_)
        | Filter::RetContains(_)
        | Filter::SigContains(_) => Some(filter),
        Filter::NoPhi | Filter::FTS5(_) | Filter::MinLength(_) | Filter::MaxLength(_) => None,
    }
}

/// Split `Lcom/foo/Bar;->baz` into its class and member halves
fn split_external(display: &str) -> (&str, &str) {
    let Some((class, rest)) = display.split_once("->") else {
        return (display, "");
    };
    let member = match rest.find(['(', ':']) {
        Some(at) => &rest[..at],
        None => rest,
    };
    (class, member)
}

/// How many rows a focused view will draw before giving up
///
/// The paths into one node are usually few, but a node deep in a re-convergent region can have
/// a great many, so the expansion is capped rather than trusted.
const MAX_FOCUS_ROWS: usize = 5_000;

pub struct State {
    method: String,
    flat: Vec<FlatNode>,
    /// Every edge of every graph, kept so a focused view can expand the paths into a node.
    /// The spanning tree in [Self::flat] cannot answer that on its own.
    edges: HashMap<SubgraphId, Vec<(NodeId, NodeId)>>,
    /// The rows of the whole analysis, parked while [Self::flat] holds a focused view
    parked: Option<Vec<FlatNode>>,
    /// Set when the focused view hit [MAX_FOCUS_ROWS]
    focus_truncated: bool,
    /// The committed search, lowercased. `n` and `N` step through its matches.
    search: Option<String>,
    /// The search being typed, which only becomes [Self::search] on Enter
    search_input: Option<String>,
    /// Rows jumped away from, newest last, as indices into [Self::flat]
    ///
    /// Following a `continued at` row moves the cursor somewhere else in the tree, and this is
    /// how it gets back. Cleared when focusing, since those indices name different rows.
    jumps: Vec<u32>,
    /// The rows in display order, honoring the filter, collapsed subtrees and hidden kinds
    visible: Vec<VisibleRow>,
    /// Manual expand (`true`) / collapse (`false`)
    overrides: HashMap<u32, bool>,
    filters: Vec<Filter>,
    /// Graphs marked hidden in the database, kept out of the view unless [Self::show_hidden]
    hidden: HashSet<SubgraphId>,
    show_hidden: bool,
    /// Whether [NodeKind::Reference] rows are drawn
    show_references: bool,
    /// Whether phi, array and instruction rows are drawn. With these off the view is just the
    /// method and field sinks, with everything underneath a dropped row pulled up onto its
    /// nearest drawn ancestor.
    show_structural: bool,
    /// Index into [Self::visible]
    sel: usize,
    top: usize,
    detail: Detail,
    x_offset: usize,
    drag: Option<DragAnchor>,
    /// Content height of the last draw, so scrolling can clamp without guessing
    viewport: Cell<u16>,
    message: Cell<Option<String>>,

    hidden_prefixes: Vec<String>,
    saved_hidden_prefixes: Option<Vec<String>>,
}

#[derive(Clone, Copy)]
struct DragAnchor {
    col: u16,
    row: u16,
    top: usize,
    x_offset: usize,
}

pub struct GraphViewWindow {
    command: CommandHandler<State>,
    state: State,
}

impl Deref for GraphViewWindow {
    type Target = State;
    fn deref(&self) -> &Self::Target {
        &self.state
    }
}

impl DerefMut for GraphViewWindow {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.state
    }
}

impl GraphViewWindow {
    pub fn new(
        tools: &Tools,
        cfg: &Config,
        ana_id: AnalyzedMethodId,
        filters: Option<Vec<Filter>>,
    ) -> anyhow::Result<Self> {
        let db = tools.db;
        let ana = db.get_analysis_info(ana_id)?;
        ana.ensure_valid()?;

        let graphs = db.get_graphs_for_analysis(&ana)?;
        let report_map = db.get_report_id_map(&ana, ResolvableIds::All)?;
        let Ok(analyzed) = report_map.get_method(ana.method) else {
            bail!("BUG! the analyzed method itself wasn't included in all referenced methods");
        };
        let method = analyzed.as_smali();

        let resolved = ResolvedTaintGraphs::resolve(graphs, &report_map)?;
        let flat = flatten(&analyzed.spec, &resolved);
        if flat.is_empty() {
            bail!("no graphs for analysis entry: {ana_id}");
        }

        let edges = resolved
            .graphs
            .into_iter()
            .map(|it| (it.id, it.edges))
            .collect();

        let mut state = State {
            method,
            flat,
            message: Cell::new(None),
            edges,
            parked: None,
            focus_truncated: false,
            search: None,
            search_input: None,
            hidden_prefixes: cfg.graph.hidden.clone(),
            saved_hidden_prefixes: None,
            jumps: Vec::new(),
            visible: Vec::new(),
            overrides: HashMap::new(),
            filters: Vec::new(),
            hidden: HashSet::from_iter(db.get_hidden_graphs()?),
            show_hidden: false,
            detail: cfg.graph.detail,
            show_references: cfg.graph.show_refs,
            show_structural: cfg.graph.show_all,
            sel: 0,
            top: 0,
            x_offset: 0,
            drag: None,
            viewport: Cell::new(1),
        };

        state.set_filters(filters.unwrap_or_default());

        Ok(Self {
            command: CommandHandler::new(HashMap::from_iter(COMMANDS.iter().copied()), KEYS_HELP),
            state,
        })
    }

    fn draw_msg(&self, msg: String, frame: &mut Frame) {
        let layout = Layout::vertical(&[
            Constraint::Length(1),
            Constraint::Fill(1),
            Constraint::Length(1),
        ]);
        let [_, body_area, _] = frame.area().layout(&layout);
        let body = Paragraph::new(msg)
            .wrap(Wrap::default())
            .alignment(Alignment::Left)
            .block(Block::default().borders(Borders::ALL));
        frame.render_widget(body, body_area);
    }
}

/// Walk every graph's tree into one pre-order list, one entry per node.
///
/// Graphs are grouped under one header per taint source rather than one per graph: the source is
/// what distinguishes them, and the graph id says nothing a reader can act on.
fn flatten(spec: &MethodSpec, graphs: &ResolvedTaintGraphs<'_>) -> Vec<FlatNode> {
    let mut out = Vec::new();

    for group in group_by_source(graphs) {
        let header = out.len();
        let label = match group.graphs.len() {
            1 => group.source.to_string(),
            count => format!("{} ({count} graphs)", group.source),
        };
        out.push(FlatNode {
            parent: None,
            depth: 0,
            subtree: 1,
            is_last_child: true,
            kind: NodeKind::GraphRoot,
            tiny: RcStr::own_str(&label),
            mid: RcStr::own_str(&label),
            full: RcStr::from(label),
            graph: None,
            node: None,
            jump_to: None,
            ways_in: 0,
            callee: None,
            in_method: Some(spec.id),
            in_method_display: "".into(),
            fields: MatchFields::default(),
            matched: false,
        });

        let last = group.graphs.len() - 1;
        for (at, graph) in group.graphs.iter().enumerate() {
            push_graph(&mut out, graph, header as u32, at == last);
        }

        out[header].subtree = (out.len() - header) as u32;
    }

    out
}

struct SourceGroup<'a> {
    source: &'a str,
    graphs: Vec<&'a ResolvedTaintGraph<'a>>,
}

/// Bucket every graph's root by its taint source, keeping first-seen order so the view is stable
fn group_by_source<'a>(graphs: &'a ResolvedTaintGraphs<'a>) -> Vec<SourceGroup<'a>> {
    let mut groups: Vec<SourceGroup<'a>> = Vec::new();

    for graph in graphs.iter() {
        let source = graph.source.display.as_str();
        match groups.iter_mut().find(|it| it.source == source) {
            Some(group) => group.graphs.push(graph),
            None => groups.push(SourceGroup {
                source,
                graphs: vec![graph],
            }),
        }
    }

    groups
}

/// Lay one graph out under `parent`
///
/// The discovery parents give the tree, and every edge that tree cannot express becomes a leaf
/// reference row under its destination: a node reached three ways shows the two routes the tree
/// dropped rather than silently keeping one.
fn push_graph(out: &mut Vec<FlatNode>, graph: &ResolvedTaintGraph<'_>, parent: u32, is_last: bool) {
    let by_id: HashMap<NodeId, &ResolvedNode<'_>> =
        graph.nodes.iter().map(|it| (it.id, it)).collect();

    let mut children_of: HashMap<NodeId, Vec<&ResolvedNode<'_>>> = HashMap::new();
    for node in &graph.nodes {
        if let Some(parent) = node.parent {
            children_of.entry(parent).or_default().push(node);
        }
    }

    // Edges the tree could not draw, keyed by source. Reading is top-down, so an edge leaving a
    // node belongs under that node: its real children plus these are every edge out of it.
    let mut extra_out: HashMap<NodeId, Vec<NodeId>> = HashMap::new();
    // How many ways each node is reached, so a node with more than one can say so without
    // putting the same edge on screen twice
    let mut ways_in: HashMap<NodeId, usize> = HashMap::new();

    for (src, dst) in &graph.edges {
        *ways_in.entry(*dst).or_default() += 1;
        let is_tree_edge = by_id.get(dst).is_some_and(|it| it.parent == Some(*src));
        if !is_tree_edge {
            extra_out.entry(*src).or_default().push(*dst);
        }
    }

    let Some(entry) = by_id.get(&graph.entry) else {
        log::warn!("graph {} has no entry node, skipping it", graph.id);
        return;
    };

    push_node(
        out,
        entry,
        graph.id,
        parent,
        1,
        is_last,
        &by_id,
        &children_of,
        &extra_out,
        &ways_in,
    );
}

fn push_node(
    out: &mut Vec<FlatNode>,
    node: &ResolvedNode<'_>,
    graph: SubgraphId,
    parent: u32,
    depth: u16,
    is_last_child: bool,
    by_id: &HashMap<NodeId, &ResolvedNode<'_>>,
    children_of: &HashMap<NodeId, Vec<&ResolvedNode<'_>>>,
    extra_out: &HashMap<NodeId, Vec<NodeId>>,
    ways_in: &HashMap<NodeId, usize>,
) {
    let idx = out.len();
    out.push(flat_node(node, graph, parent, depth, is_last_child));
    out[idx].ways_in = ways_in.get(&node.id).copied().unwrap_or(0);

    let children = children_of.get(&node.id).map(Vec::as_slice).unwrap_or(&[]);
    let continues = extra_out.get(&node.id).map(Vec::as_slice).unwrap_or(&[]);
    let last = children.len() + continues.len();

    for (at, child) in children.iter().enumerate() {
        push_node(
            out,
            child,
            graph,
            idx as u32,
            depth + 1,
            at + 1 == last,
            by_id,
            children_of,
            extra_out,
            ways_in,
        );
    }

    for (at, target) in continues.iter().enumerate() {
        // Edges are within the graph, so the destination is always one of its nodes
        let Some(target_node) = by_id.get(target) else {
            continue;
        };
        out.push(reference_node(
            *target,
            node_info(&target_node.sink),
            graph,
            idx as u32,
            depth + 1,
            children.len() + at + 1 == last,
        ));
    }

    out[idx].subtree = (out.len() - idx) as u32;
}

/// A bare header row, used to title a focused view
fn header_node(label: &str) -> FlatNode {
    FlatNode {
        parent: None,
        depth: 0,
        subtree: 1,
        is_last_child: true,
        kind: NodeKind::GraphRoot,
        tiny: RcStr::own_str(label),
        mid: RcStr::own_str(label),
        full: RcStr::own_str(label),
        graph: None,
        node: None,
        jump_to: None,
        ways_in: 0,
        callee: None,
        in_method: None,
        in_method_display: "".into(),
        fields: MatchFields::default(),
        matched: false,
    }
}

/// Copy a node's canonical row into a focused view at a new position in the tree
fn copied_row(from: &FlatNode, parent: u32, depth: u16, is_last_child: bool) -> FlatNode {
    FlatNode {
        parent: Some(parent),
        depth,
        subtree: 1,
        is_last_child,
        kind: from.kind,
        tiny: from.tiny.clone(),
        mid: from.mid.clone(),
        full: from.full.clone(),
        graph: from.graph,
        node: from.node,
        jump_to: None,
        // Every route is drawn, so the count would be repeating what is on screen
        ways_in: 0,
        callee: from.callee.clone(),
        in_method: from.in_method.clone(),
        in_method_display: from.in_method_display.clone(),
        fields: MatchFields {
            class: from.fields.class.clone(),
            method: from.fields.method.clone(),
            field: from.fields.field.clone(),
            signature: from.fields.signature.clone(),
            ret: from.fields.ret.clone(),
        },
        matched: false,
    }
}

/// Everything reachable from `start` by following `edges`, not including `start` itself
fn closure(start: NodeId, edges: &HashMap<NodeId, Vec<NodeId>>) -> HashSet<NodeId> {
    let mut seen = HashSet::new();
    let mut queue = vec![start];
    while let Some(node) = queue.pop() {
        for next in edges.get(&node).map(Vec::as_slice).unwrap_or(&[]) {
            if seen.insert(*next) {
                queue.push(*next);
            }
        }
    }
    seen.remove(&start);
    seen
}

/// Draw every route through `target`, one row per hop per route
///
/// `past` says the walk has already gone through the target, which decides whether the next
/// hops come from the upstream or the downstream edges.
fn expand_paths(
    out: &mut Vec<FlatNode>,
    at: NodeId,
    target: NodeId,
    graph: SubgraphId,
    parent: u32,
    depth: u16,
    is_last_child: bool,
    past: bool,
    up_of: &HashMap<NodeId, Vec<NodeId>>,
    down_of: &HashMap<NodeId, Vec<NodeId>>,
    canonical: &HashMap<NodeId, usize>,
    source_rows: &[FlatNode],
    on_path: &mut HashSet<NodeId>,
    truncated: &mut bool,
) {
    if out.len() >= MAX_FOCUS_ROWS {
        *truncated = true;
        return;
    }

    let Some(canon) = canonical.get(&at) else {
        return;
    };

    let idx = out.len();
    out.push(copied_row(
        &source_rows[*canon],
        parent,
        depth,
        is_last_child,
    ));

    let past = past || at == target;
    let next_of = if past { down_of } else { up_of };

    // A node already on this path would loop
    if on_path.insert(at) {
        let next = next_of.get(&at).map(Vec::as_slice).unwrap_or(&[]);
        let last = next.len().saturating_sub(1);
        for (order, step) in next.iter().enumerate() {
            expand_paths(
                out,
                *step,
                target,
                graph,
                idx as u32,
                depth + 1,
                order == last,
                past,
                up_of,
                down_of,
                canonical,
                source_rows,
                on_path,
                truncated,
            );
        }
        on_path.remove(&at);
    }

    out[idx].subtree = (out.len() - idx) as u32;
}

/// A leaf row naming an outgoing edge the tree could not draw
///
/// It carries the destination node's match fields so a filter finds it: searching for a class is
/// how you ask which nodes continue into it.
fn reference_node(
    target: NodeId,
    info: NodeInfo,
    graph: SubgraphId,
    parent: u32,
    depth: u16,
    is_last_child: bool,
) -> FlatNode {
    FlatNode {
        parent: Some(parent),
        depth,
        subtree: 1,
        is_last_child,
        kind: NodeKind::Reference,
        tiny: RcStr::from(format!("-> {}", info.tiny)),
        mid: RcStr::from(format!("continued at {}", info.mid)),
        full: RcStr::from(format!("continued at {}", info.full)),
        graph: Some(graph),
        node: None,
        jump_to: Some(target),
        ways_in: 0,
        callee: None,
        in_method: None,
        in_method_display: "".into(),
        fields: info.fields,
        matched: false,
    }
}

/// Everything a row needs, derived from one resolved sink
struct NodeInfo {
    kind: NodeKind,
    tiny: RcStr<'static>,
    mid: RcStr<'static>,
    full: RcStr<'static>,
    callee: Option<MethodId>,
    fields: MatchFields,
}

fn node_info(sink: &ResolvedTaintSink<'_>) -> NodeInfo {
    match sink {
        ResolvedTaintSink::Phi => NodeInfo {
            kind: NodeKind::Phi,
            tiny: RcStr::from("phi"),
            mid: RcStr::from("<Phi>"),
            full: RcStr::from("<Phi(...)>"),
            callee: None,
            fields: MatchFields::default(),
        },
        ResolvedTaintSink::Array => NodeInfo {
            kind: NodeKind::Array,
            tiny: RcStr::from("[]"),
            mid: RcStr::from("<Array>"),
            full: RcStr::from("<Array>"),
            callee: None,
            fields: MatchFields::default(),
        },
        ResolvedTaintSink::Call(m) => NodeInfo {
            kind: NodeKind::Call,
            tiny: RcStr::from(format!(
                "{}.{}",
                simple_class(m.spec.class.as_str()),
                m.spec.name
            )),
            mid: RcStr::from(format!(
                "{}.{}",
                compressed_class(m.spec.class.as_str()),
                m.spec.name
            )),
            full: RcStr::from(m.display.clone()),
            callee: Some(m.spec.id),
            fields: MatchFields {
                class: m.spec.class.as_str().to_lowercase().into(),
                method: m.spec.name.as_str().to_lowercase().into(),
                signature: m.spec.signature.as_str().to_lowercase().into(),
                ret: m.spec.ret.as_str().to_lowercase().into(),
                ..MatchFields::default()
            },
        },
        ResolvedTaintSink::Field(f) => NodeInfo {
            kind: NodeKind::Field,
            tiny: RcStr::from(format!(
                "{}.{}",
                simple_class(f.spec.class.as_str()),
                f.spec.name
            )),
            mid: RcStr::from(format!(
                "{}.{}",
                compressed_class(f.spec.class.as_str()),
                f.spec.name
            )),
            full: RcStr::own_str(&f.display),
            callee: None,
            fields: MatchFields {
                class: f.spec.class.as_str().to_lowercase().into(),
                field: f.spec.name.as_str().to_lowercase().into(),
                ret: f.spec.ty.as_str().to_lowercase().into(),
                ..MatchFields::default()
            },
        },
        ResolvedTaintSink::ExternalCall(t) => {
            let (class, method) = split_external(t);
            NodeInfo {
                kind: NodeKind::External,
                tiny: RcStr::from(tiny_from_display(t)),
                mid: RcStr::from(tiny_from_display(t)),
                full: RcStr::own_str(*t),
                callee: None,
                fields: MatchFields {
                    class: class.to_lowercase().into(),
                    method: method.to_lowercase().into(),
                    ..MatchFields::default()
                },
            }
        }
        ResolvedTaintSink::ExternalField(t) => {
            let (class, field) = split_external(t);
            NodeInfo {
                kind: NodeKind::Field,
                tiny: RcStr::from(tiny_from_display(t)),
                mid: RcStr::from(tiny_from_display(t)),
                full: RcStr::own_str(*t),
                callee: None,
                fields: MatchFields {
                    class: class.to_lowercase().into(),
                    field: field.to_lowercase().into(),
                    ..MatchFields::default()
                },
            }
        }
        ResolvedTaintSink::Instruction(t) => NodeInfo {
            kind: NodeKind::Instruction,
            tiny: RcStr::from(tiny_from_display(t)),
            mid: RcStr::own_str(*t),
            full: RcStr::own_str(*t),
            callee: None,
            fields: MatchFields::default(),
        },
    }
}

fn flat_node(
    node: &ResolvedNode<'_>,
    graph: SubgraphId,
    parent: u32,
    depth: u16,
    is_last_child: bool,
) -> FlatNode {
    let info = node_info(&node.sink);

    FlatNode {
        parent: Some(parent),
        depth,
        subtree: 1,
        is_last_child,
        kind: info.kind,
        graph: Some(graph),
        node: Some(node.id),
        jump_to: None,
        ways_in: 0,
        tiny: info.tiny.clone(),
        mid: info.mid.clone(),
        full: info.full.clone(),
        callee: info.callee,
        in_method: Some(node.in_method.spec.id),
        in_method_display: node.in_method.display.as_str().into(),
        fields: info.fields,
        matched: false,
    }
}

impl State {
    fn set_filters(&mut self, filters: Vec<Filter>) {
        let node_filters = filters
            .iter()
            .filter_map(node_level_filter)
            .map(lowercased)
            .collect::<Vec<_>>();

        // Matching is recomputed from scratch: a node matches only when every node-level filter
        // matches it, which is how the filters combine everywhere else.
        for node in self.flat.iter_mut() {
            node.matched = !node_filters.is_empty()
                && node.kind != NodeKind::GraphRoot
                && node_filters.iter().all(|it| node.fields.matches(it));
        }

        self.filters = filters;
        self.rebuild_visible();
    }

    fn filtering(&self) -> bool {
        self.filters
            .iter()
            .any(|it| node_level_filter(it).is_some())
    }

    /// Nodes to draw: every match, plus the ancestors that connect it to its entry point
    fn compute_keep(&self) -> Option<Vec<bool>> {
        if !self.filtering() {
            return None;
        }

        let mut keep = vec![false; self.flat.len()];
        // A parent always sits at a lower index than its children, so one reverse pass closes
        // the match set upwards.
        for idx in (0..self.flat.len()).rev() {
            if self.flat[idx].matched {
                keep[idx] = true;
            }
            if !keep[idx] {
                continue;
            }
            if let Some(parent) = self.flat[idx].parent {
                keep[parent as usize] = true;
            }
        }

        // Then forwards, so a kept node keeps the rows naming its other routes. Dropping those
        // would leave the view claiming a node is reached several ways without saying how.
        for idx in 0..self.flat.len() {
            if self.flat[idx].kind != NodeKind::Reference {
                continue;
            }
            if self.flat[idx]
                .parent
                .is_some_and(|parent| keep[parent as usize])
            {
                keep[idx] = true;
            }
        }

        Some(keep)
    }

    /// Whether a row is hidden because its graph is. A source header spans several graphs, so it
    /// only disappears once every graph beneath it is hidden.
    fn graph_hidden(&self, idx: usize) -> bool {
        if let Some(graph) = self.flat[idx].graph {
            return self.hidden.contains(&graph);
        }

        let end = idx + self.flat[idx].subtree as usize;
        !self.flat[idx + 1..end]
            .iter()
            .any(|it| it.graph.is_some_and(|graph| !self.hidden.contains(&graph)))
    }

    fn set_show_structural(&mut self, show: bool) {
        if self.show_structural == show {
            return;
        }
        self.show_structural = show;
        self.rebuild_visible();
    }

    fn set_show_references(&mut self, show: bool) {
        if self.show_references == show {
            return;
        }
        self.show_references = show;
        self.rebuild_visible();
    }

    fn toggle_show_hidden(&mut self) {
        self.show_hidden = !self.show_hidden;
        self.rebuild_visible();
    }

    /// Hide or unhide the graph the cursor is in, persisting it so triage survives a restart
    fn toggle_graph_hidden(&mut self, tools: &Tools) -> anyhow::Result<()> {
        let Some(node) = self.selected_node() else {
            return Ok(());
        };
        let Some(graph) = self.flat[node as usize].graph else {
            bail!("select a node inside a graph to hide it");
        };

        if self.hidden.insert(graph) {
            tools.db.hide_graph(graph)?;
        } else {
            self.hidden.remove(&graph);
            tools.db.unhide_graph(graph)?;
        }

        self.rebuild_visible();
        Ok(())
    }

    fn is_collapsed(&self, idx: usize) -> bool {
        let node = &self.flat[idx];
        if node.subtree == 1 {
            return false;
        }
        match self.overrides.get(&(idx as u32)) {
            Some(expanded) => !*expanded,
            None => false,
        }
    }

    fn rebuild_visible(&mut self) {
        let selected = self.selected_node();
        let keep = self.compute_keep();

        // Depth counting only the rows actually drawn. A row that is dropped but still descended
        // into passes its own depth down, which is what pulls its children up a level.
        let mut drawn_depth = vec![0u16; self.flat.len()];
        let mut visible = Vec::new();
        let mut idx = 0;

        while idx < self.flat.len() {
            let node = &self.flat[idx];
            let subtree = node.subtree as usize;
            let parent_depth = node.parent.map(|it| drawn_depth[it as usize]);

            // A focused view draws every route, so a row pointing at one elsewhere is noise
            if (!self.show_references || self.focused()) && node.kind == NodeKind::Reference {
                idx += subtree;
                continue;
            }
            if !self.show_hidden && self.graph_hidden(idx) {
                idx += subtree;
                continue;
            }
            if keep.as_ref().is_some_and(|keep| !keep[idx]) {
                idx += subtree;
                continue;
            }

            // Don't show "structural" rows
            if !self.show_structural && node.kind.is_structural() {
                drawn_depth[idx] = parent_depth.unwrap_or(0);
                idx += 1;
                continue;
            }

            // Don't show hidden prefixes

            if self.hidden_prefixes.len() > 0 {
                let full = match node.kind {
                    NodeKind::Instruction
                    | NodeKind::Phi
                    | NodeKind::Array
                    | NodeKind::GraphRoot => None,
                    NodeKind::Reference => Some(&node.full["continued at ".len()..]),
                    _ => Some(&node.full[..]),
                };

                if let Some(full) = full {
                    if self.hidden_prefixes.iter().any(|it| full.starts_with(it)) {
                        drawn_depth[idx] = parent_depth.unwrap_or(0);
                        idx += 1;
                        continue;
                    }
                }
            }

            let depth = parent_depth.map(|it| it + 1).unwrap_or(0);
            drawn_depth[idx] = depth;
            visible.push(VisibleRow {
                idx: idx as u32,
                depth,
            });

            idx += if self.is_collapsed(idx) { subtree } else { 1 };
        }

        self.visible = visible;
        self.restore_selection(selected);
        // The row count just changed, so the old scroll offset may now be past the end
        self.top = self.top.min(self.max_top());
        self.scroll_to_sel();
    }

    fn selected_node(&self) -> Option<u32> {
        self.visible.get(self.sel).map(|it| it.idx)
    }

    /// Keep the cursor on the same node across a rebuild, falling back to the nearest row
    fn restore_selection(&mut self, node: Option<u32>) {
        let Some(node) = node else {
            self.sel = 0;
            return;
        };

        self.sel = match self.visible.binary_search_by_key(&node, |it| it.idx) {
            Ok(at) => at,
            Err(at) => at.min(self.visible.len().saturating_sub(1)),
        };
    }

    fn set_detail(&mut self, detail: Detail) {
        if self.detail == detail {
            return;
        }
        self.detail = detail;
        self.rebuild_visible();
    }

    fn start_search(&mut self) {
        self.search_input = Some(String::new());
    }

    fn cancel_search(&mut self) {
        self.search_input = None;
    }

    /// Take what was typed as the search and go to the first match
    fn commit_search(&mut self) {
        let Some(query) = self.search_input.take() else {
            return;
        };
        if query.is_empty() {
            return;
        }
        self.search = Some(query.to_lowercase());
        self.step_match(true);
    }

    /// Whether a row could be drawn at all, ignoring whether it happens to be collapsed away
    ///
    /// Collapse is the one thing a search will undo, so it is not a reason to skip a row. The
    /// filter and the kind toggles are deliberate exclusions and are honoured.
    fn searchable(&self, idx: usize, keep: &Option<Vec<bool>>) -> bool {
        let node = &self.flat[idx];
        if keep.as_ref().is_some_and(|keep| !keep[idx]) {
            return false;
        }
        if (!self.show_references || self.focused()) && node.kind == NodeKind::Reference {
            return false;
        }
        if !self.show_structural && node.kind.is_structural() {
            return false;
        }
        if !self.show_hidden && self.graph_hidden(idx) {
            return false;
        }
        true
    }

    /// Move to the next or previous row matching the search, wrapping around
    ///
    /// Collapsed subtrees are searched too, and reaching a match opens the way to it.
    fn step_match(&mut self, forward: bool) -> bool {
        let Some(query) = self.search.clone() else {
            return false;
        };

        let keep = self.compute_keep();
        // Ascending, because `flat` is in pre-order and so already in display order
        let candidates = (0..self.flat.len())
            .filter(|idx| self.searchable(*idx, &keep))
            .map(|idx| idx as u32)
            .collect::<Vec<u32>>();

        if candidates.is_empty() {
            return false;
        }

        let current = self.selected_node().unwrap_or(0);
        let at = candidates
            .binary_search(&current)
            .unwrap_or_else(|insert| insert);

        let count = candidates.len();
        for step in 1..=count {
            let pos = if forward {
                (at + step) % count
            } else {
                (at + count - (step % count)) % count
            };
            let idx = candidates[pos];
            if self.flat[idx as usize].full.to_lowercase().contains(&query) {
                self.reveal(idx);
                return true;
            }
        }
        false
    }

    /// Open the way to a row and put the cursor on it
    fn reveal(&mut self, idx: u32) {
        let mut ancestor = self.flat[idx as usize].parent;
        while let Some(at) = ancestor {
            self.overrides.insert(at, true);
            ancestor = self.flat[at as usize].parent;
        }

        self.rebuild_visible();
        if let Ok(found) = self.visible.binary_search_by_key(&idx, |it| it.idx) {
            self.sel = found;
            self.scroll_to_sel();
        }
    }

    /// Return to the row the last jump started from
    fn jump_back(&mut self) -> bool {
        let Some(row) = self.jumps.pop() else {
            return false;
        };

        // It may have been collapsed away since, so open the way to it again
        self.reveal(row);
        true
    }

    fn focused(&self) -> bool {
        self.parked.is_some()
    }

    /// Draw only the routes that reach the selected node
    ///
    /// Unlike the normal view this expands the paths rather than drawing a spanning tree, so a
    /// node reached three ways appears three times, once per route. That is affordable here and
    /// not in general: it is the ancestry of one node, not of every node.
    fn focus_selected(&mut self) -> anyhow::Result<()> {
        let Some(row) = self.selected_node() else {
            return Ok(());
        };
        let row = &self.flat[row as usize];
        let (Some(graph), Some(target)) = (row.graph, row.node.or(row.jump_to)) else {
            bail!("select a node inside a graph to focus it");
        };

        let edges = self.edges.get(&graph).cloned().unwrap_or_default();
        let source = self
            .flat
            .iter()
            .position(|it| it.kind == NodeKind::GraphRoot && it.parent.is_none());

        // Canonical row per node, to copy labels from rather than rebuild them
        let canonical: HashMap<NodeId, usize> = self
            .flat
            .iter()
            .enumerate()
            .filter(|(_, it)| it.graph == Some(graph))
            .filter_map(|(at, it)| it.node.map(|node| (node, at)))
            .collect();

        let Some(entry) = self
            .flat
            .iter()
            .find(|it| it.graph == Some(graph) && it.depth == 1)
            .and_then(|it| it.node)
        else {
            bail!("the graph has no entry node to walk from");
        };

        // A route through the node is what came before it and what comes after, so both
        // directions are closed over: above the node is drawn out of its ancestors, below out
        // of its descendants.
        let mut into: HashMap<NodeId, Vec<NodeId>> = HashMap::new();
        let mut from: HashMap<NodeId, Vec<NodeId>> = HashMap::new();
        for (src, dst) in &edges {
            into.entry(*dst).or_default().push(*src);
            from.entry(*src).or_default().push(*dst);
        }

        let ancestors = closure(target, &into);
        let descendants = closure(target, &from);

        // Keeping the two directions in separate maps is what stops the walk wandering into a
        // branch off an ancestor that never passes through the node at all.
        let mut up_of: HashMap<NodeId, Vec<NodeId>> = HashMap::new();
        let mut down_of: HashMap<NodeId, Vec<NodeId>> = HashMap::new();
        for (src, dst) in &edges {
            if ancestors.contains(src) && (ancestors.contains(dst) || *dst == target) {
                up_of.entry(*src).or_default().push(*dst);
            }
            if (descendants.contains(src) || *src == target) && descendants.contains(dst) {
                down_of.entry(*src).or_default().push(*dst);
            }
        }

        let mut rows: Vec<FlatNode> = Vec::new();
        let label = format!(
            "routes through {}",
            self.label(&self.flat[canonical[&target]])
        );
        rows.push(header_node(&label));

        let mut truncated = false;
        let mut on_path = HashSet::new();
        expand_paths(
            &mut rows,
            entry,
            target,
            graph,
            0,
            1,
            true,
            false,
            &up_of,
            &down_of,
            &canonical,
            &self.flat,
            &mut on_path,
            &mut truncated,
        );
        rows[0].subtree = rows.len() as u32;

        if rows.len() == 1 {
            bail!("nothing reaches that node");
        }

        _ = source;
        self.parked = Some(std::mem::replace(&mut self.flat, rows));
        self.jumps.clear();
        self.focus_truncated = truncated;
        self.overrides.clear();
        self.sel = 0;
        self.top = 0;
        self.rebuild_visible();
        Ok(())
    }

    /// Go back to the whole analysis
    fn leave_focus(&mut self) {
        let Some(parked) = self.parked.take() else {
            return;
        };
        self.flat = parked;
        self.focus_truncated = false;
        self.jumps.clear();
        self.overrides.clear();
        self.sel = 0;
        self.top = 0;
        self.rebuild_visible();
    }

    /// Move the cursor to the node a [NodeKind::Reference] row points at
    ///
    /// Returns false when the row is not a reference, so the caller can fall back to collapsing.
    fn follow_reference(&mut self) -> bool {
        let Some(row) = self.selected_node() else {
            return false;
        };
        let Some(target) = self.flat[row as usize].jump_to else {
            return false;
        };

        // The target is drawn somewhere in the same graph, but it may be inside a collapsed
        // subtree, so expand every ancestor on the way before selecting it.
        let Some(at) = self
            .flat
            .iter()
            .position(|it| it.node == Some(target) && it.graph == self.flat[row as usize].graph)
        else {
            return false;
        };

        self.jumps.push(row);
        self.reveal(at as u32);
        true
    }

    /// Force every subtree open or shut
    fn set_all_collapsed(&mut self, collapsed: bool) {
        self.overrides.clear();
        for idx in 0..self.flat.len() {
            // A leaf has nothing to open, and an override on one would never be read
            if self.flat[idx].subtree > 1 {
                self.overrides.insert(idx as u32, !collapsed);
            }
        }
        self.rebuild_visible();
    }

    fn toggle_collapse(&mut self) {
        let Some(node) = self.selected_node() else {
            return;
        };
        if self.flat[node as usize].subtree == 1 {
            return;
        }

        let collapsed = self.is_collapsed(node as usize);
        self.overrides.insert(node, collapsed);
        self.rebuild_visible();
    }

    fn label<'a>(&self, node: &'a FlatNode) -> &'a str {
        match self.detail {
            Detail::Minimal => &node.tiny,
            Detail::Moderate => &node.mid,
            Detail::Full => &node.full,
        }
    }

    /// Box-drawing connectors for a row, derived from the ancestors' last-child flags
    /// Connectors for the row at visible position `at`
    fn prefix(&self, at: usize) -> String {
        let row = self.visible[at];
        let node = &self.flat[row.idx as usize];
        if row.depth == 0 {
            return String::new();
        }

        let mut segments = Vec::with_capacity(node.depth as usize);
        let mut at = node.parent;
        while let Some(parent) = at {
            let parent = &self.flat[parent as usize];
            if parent.depth == 0 {
                break;
            }
            segments.push(if parent.is_last_child { "   " } else { "│  " });
            at = parent.parent;
        }
        segments.reverse();

        let mut out = segments.concat();
        out.push_str(if node.is_last_child {
            "└─ "
        } else {
            "├─ "
        });
        out
    }

    fn row_text(&self, at: usize) -> String {
        let idx = self.visible[at].idx as usize;
        let node = &self.flat[idx];
        let mut text = self.prefix(at);
        text.push_str(self.label(node));

        if node.ways_in > 1 {
            // A count rather than rows: the routes in are already drawn as `continued at`
            // under each of their sources
            text.push_str(&format!("  [{} routes in]", node.ways_in));
        }

        if self.is_collapsed(idx) {
            // subtree counts this node, the summary is about what it hides
            text.push_str(&format!("  [+{} hidden]", node.subtree - 1));
        }

        text
    }

    /// Half a screen, the usual Ctrl-D/Ctrl-U step
    fn half_page(&self) -> isize {
        ((self.viewport.get() / 2).max(1)) as isize
    }

    fn move_sel(&mut self, delta: isize) {
        if self.visible.is_empty() {
            return;
        }
        let last = self.visible.len() - 1;
        self.sel = self.sel.saturating_add_signed(delta).min(last);
        self.scroll_to_sel();
    }

    fn scroll_to_sel(&mut self) {
        let height = self.viewport.get() as usize;
        if self.sel < self.top {
            self.top = self.sel;
        } else if height > 0 && self.sel >= self.top + height {
            self.top = self.sel + 1 - height;
        }
    }

    fn max_top(&self) -> usize {
        let height = self.viewport.get() as usize;
        self.visible.len().saturating_sub(height.max(1))
    }

    /// Drag the cursor along with the viewport so it never leaves the screen.
    ///
    /// The mouse moves the camera and the keys move the cursor, but letting the two drift apart
    /// means `j` jumps somewhere unrelated and `o` acts on a row nobody can see.
    fn clamp_sel_to_view(&mut self) {
        if self.visible.is_empty() {
            self.sel = 0;
            return;
        }

        let top = self.top.min(self.max_top());
        let height = self.viewport.get() as usize;
        let bottom = (top + height.saturating_sub(1)).min(self.visible.len() - 1);
        self.sel = self.sel.clamp(top, bottom);
    }

    fn scroll_horizontal(&mut self, delta: isize) {
        self.x_offset = self.x_offset.saturating_add_signed(delta);
    }

    fn open_method(tools: &Tools, id: MethodId) -> anyhow::Result<()> {
        let gdb = tools.db.graph();
        let spec = gdb
            .get_method_by_id(id)
            .with_context(|| "getting method from the graph database")?;
        dtu_open_method_spec(tools.ctx, &spec)
    }

    /// Open whatever the selected row names.
    ///
    /// A call row shows its callee, so that is what opens. Rows that name nothing openable (phi,
    /// array, a field access) fall back to the method they sit in, which is the only code there
    /// is to look at for them.
    fn open_selected(&self, tools: &Tools) -> anyhow::Result<()> {
        let Some(node) = self.selected_node() else {
            return Ok(());
        };
        let node = &self.flat[node as usize];
        let Some(id) = node.callee.as_ref().or(node.in_method.as_ref()) else {
            bail!("a source header has no method to open");
        };
        Self::open_method(tools, *id)
    }

    /// Open the method the selected node sits inside, rather than what the row names
    fn open_location(&self, tools: &Tools) -> anyhow::Result<()> {
        let Some(node) = self.selected_node() else {
            return Ok(());
        };
        let Some(id) = &self.flat[node as usize].in_method else {
            bail!("a source header has no method to open");
        };
        Self::open_method(tools, *id)
    }

    /// The method the selected node sits in, for the detail line
    fn selected_location(&self) -> Option<&str> {
        let node = self.selected_node()?;
        let location = &self.flat[node as usize].in_method_display;
        (!location.is_empty()).then_some(location)
    }
}

/// Take `width` characters starting `offset` in, which is how horizontal scrolling is applied
fn slice_line(text: &str, offset: usize, width: usize) -> String {
    text.chars().skip(offset).take(width).collect()
}

impl Window for GraphViewWindow {
    fn draw(&self, _tools: &Tools, _cfg: &mut Config, frame: &mut Frame) {
        if self.command.draw(frame) {
            return;
        }

        if let Some(msg) = self.message.take() {
            self.draw_msg(msg, frame);
            return;
        }

        let layout = Layout::vertical(&[
            Constraint::Length(1),
            Constraint::Fill(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ]);
        let [title_area, body_area, detail_area, status_area] = frame.area().layout(&layout);

        self.viewport.set(body_area.height);

        let height = body_area.height as usize;
        let top = self.top.min(self.max_top());
        let width = body_area.width.saturating_sub(1) as usize;

        let mut lines = Vec::with_capacity(height);
        for (offset, row) in self
            .visible
            .iter()
            .skip(top)
            .take(height)
            .copied()
            .enumerate()
        {
            let node = &self.flat[row.idx as usize];
            let text = slice_line(&self.row_text(top + offset), self.x_offset, width);

            let mut style = Style::new().fg(node.kind.color());
            if node.matched {
                style = style.add_modifier(Modifier::BOLD).fg(Color::Yellow);
            }
            if node.kind == NodeKind::GraphRoot {
                style = style.add_modifier(Modifier::BOLD);
            }
            if self.show_hidden && self.graph_hidden(row.idx as usize) {
                style = Style::new().fg(Color::Gray);
            }
            if top + offset == self.sel {
                style = style.add_modifier(Modifier::REVERSED);
            }

            lines.push(Line::from(Span::styled(text, style)));
        }

        // The counts are short and always useful, so the method name is what gives way when the
        // terminal is too narrow for both.
        let counts = format!(
            "  [{} nodes{}, detail {}]",
            self.visible.len(),
            if self.hidden_prefixes.len() > 0 {
                "*"
            } else {
                ""
            },
            self.detail.as_str()
        );
        let name_width = (title_area.width as usize).saturating_sub(counts.chars().count());
        let title = Line::from(vec![
            Span::styled(fit(&self.method, name_width), Style::new().bold()),
            Span::raw(counts),
        ])
        .centered();
        frame.render_widget(title, title_area);

        if lines.is_empty() {
            // FTS5 picked this method for the list but nothing here matched, so say so rather
            // than leaving an empty pane that looks like a broken view
            let empty = if self.filtering() {
                "no nodes match the filter"
            } else {
                "no nodes"
            };
            frame.render_widget(Line::raw(empty).centered().dim(), body_area);
        }

        for (at, line) in lines.into_iter().enumerate() {
            let mut area = body_area;
            area.y += at as u16;
            area.height = 1;
            frame.render_widget(line, area);
        }

        let mut scrollbar_state = ScrollbarState::new(self.visible.len()).position(top);
        frame.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight),
            body_area,
            &mut scrollbar_state,
        );

        let detail = match (&self.search_input, self.selected_location()) {
            (Some(input), _) => format!("/{input}"),
            (None, Some(location)) => format!("in {location}"),
            (None, None) => String::new(),
        };
        frame.render_widget(
            Line::raw(fit(&detail, detail_area.width as usize)).dim(),
            detail_area,
        );

        let mut status = String::from(
            "j/k move | <space> collapse/follow | o open | O in-method | s sinks | r refs | f focus | : command | q back | ? help",
        );
        if self.filtering() {
            status.push_str("  [filtered]");
        }
        if self.show_hidden {
            status.push_str("  [showing hidden]");
        }
        if !self.show_references {
            status.push_str("  [refs hidden]");
        }
        if !self.show_structural {
            status.push_str("  [sinks only]");
        }
        if self.focused() {
            status.push_str(if self.focus_truncated {
                "  [focused, truncated]"
            } else {
                "  [focused]"
            });
        }
        frame.render_widget(
            Line::raw(fit(&status, status_area.width as usize)).dim(),
            status_area,
        );
    }

    fn on_key_event(
        &mut self,
        tools: &Tools,
        cfg: &mut Config,
        evt: KeyEvent,
    ) -> anyhow::Result<WindowAction> {
        let Self { command, state } = self;
        if command.is_active() {
            return command.on_key_event(evt, tools, cfg, state);
        }

        let cfg = &mut cfg.graph;

        // A search in progress takes every printable key, so this runs before the bindings
        if self.search_input.is_some() {
            match evt.code {
                KeyCode::Esc => self.cancel_search(),
                KeyCode::Enter => self.commit_search(),
                KeyCode::Backspace => {
                    if let Some(input) = self.search_input.as_mut() {
                        input.pop();
                    }
                }
                KeyCode::Char(c) => {
                    if let Some(input) = self.search_input.as_mut() {
                        input.push(c);
                    }
                }
                _ => return Ok(WindowAction::Nothing),
            }
            return Ok(WindowAction::Redraw);
        }

        if evt.modifiers == KeyModifiers::CONTROL {
            match evt.code {
                KeyCode::Char('d') => {
                    let page = self.half_page();
                    self.move_sel(page);
                }
                KeyCode::Char('u') => {
                    let page = self.half_page();
                    self.move_sel(-page);
                }
                KeyCode::Char('o') => {
                    if !self.jump_back() {
                        return Ok(WindowAction::Nothing);
                    }
                }
                _ => return Ok(WindowAction::Nothing),
            }
            return Ok(WindowAction::Redraw);
        }

        if evt.modifiers != KeyModifiers::NONE && evt.modifiers != KeyModifiers::SHIFT {
            return Ok(WindowAction::Nothing);
        }

        match evt.code {
            KeyCode::Char('q') | KeyCode::Esc => return Ok(WindowAction::PopWindow),
            KeyCode::Char('/') => self.start_search(),
            KeyCode::Char('n') => {
                if !self.step_match(true) {
                    return Ok(WindowAction::Nothing);
                }
            }
            KeyCode::Char('N') => {
                if !self.step_match(false) {
                    return Ok(WindowAction::Nothing);
                }
            }
            KeyCode::Char('f') => {
                if self.focused() {
                    self.leave_focus();
                } else {
                    self.focus_selected()?;
                }
            }
            KeyCode::Char('?') => self.command.show_keys(),
            KeyCode::Char(':') => self.command.activate(),
            KeyCode::Char('j') | KeyCode::Down => self.move_sel(1),
            KeyCode::Char('k') | KeyCode::Up => self.move_sel(-1),
            KeyCode::Char('g') | KeyCode::Home => {
                self.sel = 0;
                self.scroll_to_sel();
            }
            KeyCode::Char('G') | KeyCode::End => {
                self.sel = self.visible.len().saturating_sub(1);
                self.scroll_to_sel();
            }
            KeyCode::PageDown => {
                let page = self.viewport.get() as isize;
                self.move_sel(page);
            }
            KeyCode::PageUp => {
                let page = self.viewport.get() as isize;
                self.move_sel(-page);
            }
            KeyCode::Char('h') | KeyCode::Left => self.scroll_horizontal(-4),
            KeyCode::Char('l') | KeyCode::Right => self.scroll_horizontal(4),
            KeyCode::Char('d') => {
                cfg.detail.next();
                self.set_detail(cfg.detail);
            }
            KeyCode::Char(' ') | KeyCode::Enter => {
                if !self.follow_reference() {
                    self.toggle_collapse();
                }
            }
            KeyCode::Char('o') => self.open_selected(tools)?,
            KeyCode::Char('O') => self.open_location(tools)?,
            KeyCode::Char('u') => {
                match self.saved_hidden_prefixes.take() {
                    Some(v) => {
                        self.hidden_prefixes.reserve(v.len());
                        self.hidden_prefixes.extend(v.into_iter());
                    }
                    None => {
                        self.saved_hidden_prefixes =
                            Some(std::mem::take(&mut self.hidden_prefixes));
                    }
                }
                self.rebuild_visible();
            }
            KeyCode::Char('s') => {
                cfg.show_all = !cfg.show_all;
                self.set_show_structural(cfg.show_all);
            }
            KeyCode::Char('r') => {
                cfg.show_refs = !cfg.show_refs;
                self.set_show_references(cfg.show_refs);
            }
            KeyCode::Char('H') => self.toggle_graph_hidden(tools)?,
            KeyCode::Char('.') => self.toggle_show_hidden(),
            _ => return Ok(WindowAction::Nothing),
        }

        Ok(WindowAction::Redraw)
    }

    fn on_mouse_event(
        &mut self,
        _tools: &Tools,
        _cfg: &mut Config,
        evt: MouseEvent,
    ) -> anyhow::Result<WindowAction> {
        match evt.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                self.drag = Some(DragAnchor {
                    col: evt.column,
                    row: evt.row,
                    top: self.top,
                    x_offset: self.x_offset,
                });
                return Ok(WindowAction::Nothing);
            }
            MouseEventKind::Up(MouseButton::Left) => {
                self.drag = None;
                return Ok(WindowAction::Nothing);
            }
            // Dragging moves the content with the pointer, so the view follows the opposite way
            MouseEventKind::Drag(MouseButton::Left) => {
                let Some(anchor) = self.drag else {
                    return Ok(WindowAction::Nothing);
                };
                let rows = i32::from(evt.row) - i32::from(anchor.row);
                let cols = i32::from(evt.column) - i32::from(anchor.col);
                self.top = anchor
                    .top
                    .saturating_add_signed(-rows as isize)
                    .min(self.max_top());
                self.x_offset = anchor.x_offset.saturating_add_signed(-cols as isize);
                self.clamp_sel_to_view();
            }
            MouseEventKind::ScrollDown => self.move_sel(1),
            MouseEventKind::ScrollUp => self.move_sel(-1),
            MouseEventKind::ScrollLeft => self.scroll_horizontal(-4),
            MouseEventKind::ScrollRight => self.scroll_horizontal(4),
            _ => return Ok(WindowAction::Nothing),
        }

        Ok(WindowAction::Redraw)
    }
}

#[cfg(test)]
mod test {
    use super::*;

    impl State {
        /// The drawn rows as flat indices, which is what the assertions here care about
        fn visible_idx(&self) -> Vec<u32> {
            self.visible.iter().map(|it| it.idx).collect()
        }
    }

    #[test]
    fn simple_class_strips_the_package_and_wrapper() {
        assert_eq!(simple_class("Lcom/foo/Bar;"), "Bar");
        assert_eq!(simple_class("Lcom/foo/Bar$Inner;"), "Bar$Inner");
        assert_eq!(simple_class("LBar;"), "Bar");
        assert_eq!(simple_class("Bar"), "Bar");
        assert_eq!(simple_class(""), "");
    }

    #[test]
    fn compressed_class_keeps_only_the_final_segment_whole() {
        assert_eq!(compressed_class("Lcom/foo/Bar;"), "c/f/Bar");
        assert_eq!(compressed_class("Landroidx/compose/ui/a;"), "a/c/u/a");
        assert_eq!(compressed_class("LBar;"), "Bar");
        assert_eq!(compressed_class(""), "");
    }

    #[test]
    fn tiny_from_display_handles_methods_and_fields() {
        assert_eq!(tiny_from_display("Lcom/foo/Bar;->baz(I)V"), "Bar.baz");
        assert_eq!(tiny_from_display("Lcom/foo/Bar;->baz:I"), "Bar.baz");
        assert_eq!(tiny_from_display("no-arrow-here"), "no-arrow-here");
    }

    #[test]
    fn split_external_separates_class_from_member() {
        assert_eq!(
            split_external("Lcom/foo/Bar;->baz(I)V"),
            ("Lcom/foo/Bar;", "baz")
        );
        assert_eq!(
            split_external("Lcom/foo/Bar;->baz:I"),
            ("Lcom/foo/Bar;", "baz")
        );
        assert_eq!(split_external("junk"), ("junk", ""));
    }

    fn node(parent: Option<u32>, depth: u16, class: &str) -> FlatNode {
        FlatNode {
            parent,
            depth,
            subtree: 1,
            is_last_child: true,
            kind: if parent.is_none() {
                NodeKind::GraphRoot
            } else {
                NodeKind::Call
            },
            graph: parent.map(|_| SubgraphId::from(1)),
            node: None,
            jump_to: None,
            ways_in: 0,
            tiny: RcStr::own_str(class),
            mid: RcStr::own_str(class),
            full: RcStr::own_str(class),
            callee: None,
            in_method: None,
            in_method_display: "".into(),
            fields: MatchFields {
                // Mirrors node_info: match fields are stored lowercased
                class: class.to_lowercase().into(),
                ..MatchFields::default()
            },
            matched: false,
        }
    }

    /// header -> A -> {B, C -> D}
    fn state() -> State {
        let mut flat = vec![
            node(None, 0, "header"),
            node(Some(0), 1, "LA;"),
            node(Some(1), 2, "LB;"),
            node(Some(1), 2, "LC;"),
            node(Some(3), 3, "LD;"),
        ];
        flat[0].subtree = 5;
        flat[1].subtree = 4;
        flat[3].subtree = 2;
        flat[2].is_last_child = false;

        let mut state = State {
            report_map: ReportIdMap::new_empty(),
            message: Cell::new(None),
            hidden_prefixes: vec![],
            saved_hidden_prefixes: None,
            method: "test".into(),
            flat,
            visible: Vec::new(),
            overrides: HashMap::new(),
            filters: Vec::new(),
            edges: HashMap::new(),
            parked: None,
            focus_truncated: false,
            search: None,
            search_input: None,
            jumps: Vec::new(),
            hidden: HashSet::new(),
            show_hidden: false,
            show_references: true,
            show_structural: true,
            detail: Detail::Full,
            sel: 0,
            top: 0,
            x_offset: 0,
            drag: None,
            viewport: Cell::new(10),
        };
        state.rebuild_visible();
        state
    }

    #[test]
    fn everything_is_visible_without_a_filter_or_cutoff() {
        assert_eq!(state().visible_idx(), vec![0, 1, 2, 3, 4]);
    }

    #[test]
    fn a_filter_keeps_matches_and_their_ancestors() {
        let mut state = state();
        state.set_filters(vec![Filter::ClassContains("LD;".into())]);
        // B is dropped, but C and A survive as the path back to the entry point
        assert_eq!(state.visible_idx(), vec![0, 1, 3, 4]);
        assert!(state.flat[4].matched);
        assert!(!state.flat[3].matched);
    }

    #[test]
    fn a_filter_matching_nothing_hides_everything() {
        let mut state = state();
        state.set_filters(vec![Filter::ClassContains("nope".into())]);
        assert!(state.visible_idx().is_empty());
    }

    #[test]
    fn filtering_ignores_case_like_the_database_does() {
        let mut state = state();
        // The list screen matches through FTS5, which would have offered this method up for
        // `class=Runtime` even though the class is spelled in lowercase
        state.flat[2].fields.class = "landroidx/compose/runtime/u1;".into();
        state.set_filters(vec![Filter::ClassContains("Runtime".into())]);
        assert!(state.flat[2].matched);
        assert_eq!(state.visible_idx(), vec![0, 1, 2]);
    }

    #[test]
    fn filtering_is_case_insensitive_in_both_directions() {
        let mut state = state();
        state.flat[2].fields.class = "lcom/foo/runtimebar;".into();
        state.set_filters(vec![Filter::ClassContains("RUNTIMEBAR".into())]);
        assert!(state.flat[2].matched);
    }

    #[test]
    fn filters_combine_with_and() {
        let mut state = state();
        state.set_filters(vec![
            Filter::ClassContains("LD;".into()),
            Filter::ClassContains("LB;".into()),
        ]);
        assert!(state.visible_idx().is_empty());
    }

    #[test]
    fn graph_level_filters_leave_the_view_unfiltered() {
        let mut state = state();
        state.set_filters(vec![Filter::NoPhi]);
        assert!(!state.filtering());
        assert_eq!(state.visible_idx(), vec![0, 1, 2, 3, 4]);
    }

    #[test]
    fn collapsing_hides_a_whole_subtree() {
        let mut state = state();
        state.sel = 3;
        state.toggle_collapse();
        assert_eq!(state.visible_idx(), vec![0, 1, 2, 3]);

        state.toggle_collapse();
        assert_eq!(state.visible_idx(), vec![0, 1, 2, 3, 4]);
    }

    #[test]
    fn collapsing_a_leaf_does_nothing() {
        let mut state = state();
        state.sel = 2;
        state.toggle_collapse();
        assert_eq!(state.visible_idx(), vec![0, 1, 2, 3, 4]);
    }

    #[test]
    fn selection_follows_its_node_across_a_rebuild() {
        let mut state = state();
        state.sel = 4;
        state.set_filters(vec![Filter::ClassContains("LD;".into())]);
        assert_eq!(state.selected_node(), Some(4));
    }

    #[test]
    fn connectors_reflect_the_ancestor_chain() {
        let state = state();
        // Each level indents past the 3-column connector its parent was drawn with
        assert_eq!(state.prefix(0), "");
        assert_eq!(state.prefix(1), "└─ ");
        assert_eq!(state.prefix(2), "   ├─ ");
        assert_eq!(state.prefix(4), "      └─ ");
    }

    /// A flat list of `count` sibling rows under one header, with a `height`-row viewport
    fn scrollable(count: usize, height: u16) -> State {
        let mut flat = vec![node(None, 0, "header")];
        for at in 0..count {
            flat.push(node(Some(0), 1, &format!("L{at};")));
        }
        flat[0].subtree = (count + 1) as u32;

        let mut state = State {
            report_map: ReportIdMap::new_empty(),
            message: Cell::new(None),
            hidden_prefixes: vec![],
            saved_hidden_prefixes: None,

            method: "test".into(),
            flat,
            visible: Vec::new(),
            overrides: HashMap::new(),
            filters: Vec::new(),
            edges: HashMap::new(),
            parked: None,
            focus_truncated: false,
            search: None,
            search_input: None,
            jumps: Vec::new(),
            hidden: HashSet::new(),
            show_hidden: false,
            show_references: true,
            show_structural: true,
            detail: Detail::Full,
            sel: 0,
            top: 0,
            x_offset: 0,
            drag: None,
            viewport: Cell::new(height),
        };
        state.rebuild_visible();
        state
    }

    #[test]
    fn moving_past_the_edge_scrolls_the_viewport() {
        let mut state = scrollable(50, 10);
        state.sel = 9;
        state.move_sel(1);
        assert_eq!(state.sel, 10);
        assert_eq!(state.top, 1);
    }

    /// Two graphs under one source header
    fn two_graphs() -> State {
        let mut flat = vec![
            node(None, 0, "header"),
            node(Some(0), 1, "LA;"),
            node(Some(1), 2, "LB;"),
            node(Some(0), 1, "LC;"),
        ];
        flat[0].subtree = 4;
        flat[1].subtree = 2;
        flat[1].graph = Some(SubgraphId::from(1));
        flat[2].graph = Some(SubgraphId::from(1));
        flat[3].graph = Some(SubgraphId::from(2));

        let mut state = State {
            report_map: ReportIdMap::new_empty(),
            message: Cell::new(None),
            hidden_prefixes: vec![],
            saved_hidden_prefixes: None,

            method: "test".into(),
            flat,
            visible: Vec::new(),
            overrides: HashMap::new(),
            filters: Vec::new(),
            edges: HashMap::new(),
            parked: None,
            focus_truncated: false,
            search: None,
            search_input: None,
            jumps: Vec::new(),
            hidden: HashSet::new(),
            show_hidden: false,
            show_references: true,
            show_structural: true,
            detail: Detail::Full,
            sel: 0,
            top: 0,
            x_offset: 0,
            drag: None,
            viewport: Cell::new(10),
        };
        state.rebuild_visible();
        state
    }

    #[test]
    fn hiding_a_graph_drops_it_and_its_subtree() {
        let mut state = two_graphs();
        assert_eq!(state.visible_idx(), vec![0, 1, 2, 3]);

        state.hidden.insert(SubgraphId::from(1));
        state.rebuild_visible();
        // The header survives because graph 2 is still showing
        assert_eq!(state.visible_idx(), vec![0, 3]);
    }

    #[test]
    fn a_header_goes_away_once_every_graph_under_it_is_hidden() {
        let mut state = two_graphs();
        state.hidden.insert(SubgraphId::from(1));
        state.hidden.insert(SubgraphId::from(2));
        state.rebuild_visible();
        assert!(state.visible_idx().is_empty());
    }

    #[test]
    fn show_hidden_brings_everything_back() {
        let mut state = two_graphs();
        state.hidden.insert(SubgraphId::from(1));
        state.rebuild_visible();
        assert_eq!(state.visible_idx(), vec![0, 3]);

        state.toggle_show_hidden();
        assert_eq!(state.visible_idx(), vec![0, 1, 2, 3]);

        state.toggle_show_hidden();
        assert_eq!(state.visible_idx(), vec![0, 3]);
    }

    #[test]
    fn hiding_combines_with_filtering() {
        let mut state = two_graphs();
        state.hidden.insert(SubgraphId::from(2));
        state.set_filters(vec![Filter::ClassContains("LB;".into())]);
        // Graph 2 is hidden and only LB matches in graph 1, so LA survives as its ancestor
        assert_eq!(state.visible_idx(), vec![0, 1, 2]);
    }

    #[test]
    fn half_page_is_at_least_one_row() {
        let state = two_graphs();
        assert_eq!(state.half_page(), 5);
        state.viewport.set(1);
        assert_eq!(state.half_page(), 1);
        state.viewport.set(0);
        assert_eq!(state.half_page(), 1);
    }

    fn reference_row(parent: u32, target: u32) -> FlatNode {
        let info = NodeInfo {
            kind: NodeKind::Call,
            tiny: RcStr::from("LB;"),
            mid: RcStr::from("LB;"),
            full: RcStr::from("LB;"),
            callee: None,
            fields: MatchFields {
                class: "lb;".into(),
                ..MatchFields::default()
            },
        };
        reference_node(
            NodeId::from(target as i32),
            info,
            SubgraphId::from(1),
            parent,
            2,
            true,
        )
    }

    #[test]
    fn a_reference_row_is_a_leaf() {
        let row = reference_row(1, 9);
        assert_eq!(row.subtree, 1);
        assert_eq!(row.kind, NodeKind::Reference);
        assert_eq!(row.jump_to, Some(NodeId::from(9)));
        assert!(row.node.is_none());
    }

    /// header -> A -> B, where B continues into a node drawn elsewhere
    fn with_reference() -> State {
        let mut flat = vec![
            node(None, 0, "header"),
            node(Some(0), 1, "LA;"),
            node(Some(1), 2, "LB;"),
            reference_row(2, 99),
        ];
        flat[0].subtree = 4;
        flat[1].subtree = 3;
        flat[2].subtree = 2;

        let mut state = State {
            report_map: ReportIdMap::new_empty(),
            message: Cell::new(None),
            hidden_prefixes: vec![],
            saved_hidden_prefixes: None,

            method: "test".into(),
            flat,
            visible: Vec::new(),
            overrides: HashMap::new(),
            filters: Vec::new(),
            edges: HashMap::new(),
            parked: None,
            focus_truncated: false,
            search: None,
            search_input: None,
            jumps: Vec::new(),
            hidden: HashSet::new(),
            show_hidden: false,
            show_references: true,
            show_structural: true,
            detail: Detail::Full,
            sel: 0,
            top: 0,
            x_offset: 0,
            drag: None,
            viewport: Cell::new(10),
        };
        state.rebuild_visible();
        state
    }

    #[test]
    fn a_kept_node_keeps_the_rows_naming_its_other_routes() {
        let mut state = with_reference();
        assert_eq!(state.visible_idx(), vec![0, 1, 2, 3]);

        state.set_filters(vec![Filter::ClassContains("LB;".into())]);
        // The `continued at` row survives because its parent did, even though it is not a match
        assert_eq!(state.visible_idx(), vec![0, 1, 2, 3]);
        // Both match: the row naming the alternate route is findable by the same search
        assert!(state.flat[2].matched);
        assert!(state.flat[3].matched);
    }

    #[test]
    fn a_reference_row_is_dropped_when_its_parent_is() {
        let mut state = with_reference();
        state.set_filters(vec![Filter::ClassContains("nope".into())]);
        assert!(state.visible_idx().is_empty());
    }

    #[test]
    fn collapse_all_leaves_only_the_headers() {
        let mut state = state();
        state.set_all_collapsed(true);
        assert_eq!(state.visible_idx(), vec![0]);

        state.set_all_collapsed(false);
        assert_eq!(state.visible_idx(), vec![0, 1, 2, 3, 4]);
    }

    #[test]
    fn expanding_does_not_record_overrides_for_leaves() {
        let mut state = state();
        state.set_all_collapsed(false);
        // Only the header, A and C have subtrees; B and D are leaves
        assert_eq!(state.overrides.len(), 3);
    }

    #[test]
    fn hiding_references_leaves_a_plain_tree() {
        let mut state = with_reference();
        assert_eq!(state.visible_idx(), vec![0, 1, 2, 3]);

        state.set_show_references(false);
        assert_eq!(state.visible_idx(), vec![0, 1, 2]);

        state.set_show_references(true);
        assert_eq!(state.visible_idx(), vec![0, 1, 2, 3]);
    }

    #[test]
    fn hidden_references_stay_hidden_under_a_filter() {
        let mut state = with_reference();
        state.set_show_references(false);
        state.set_filters(vec![Filter::ClassContains("LB;".into())]);
        // LB matches and the reference row would too, but it is switched off
        assert_eq!(state.visible_idx(), vec![0, 1, 2]);
    }

    /// header -> A(call) -> phi -> D(call), so the sink sits under a structural node
    fn with_structural() -> State {
        let mut flat = vec![
            node(None, 0, "header"),
            node(Some(0), 1, "LA;"),
            node(Some(1), 2, "phi"),
            node(Some(2), 3, "LD;"),
        ];
        flat[0].subtree = 4;
        flat[1].subtree = 3;
        flat[2].subtree = 2;
        flat[2].kind = NodeKind::Phi;

        let mut state = State {
            report_map: ReportIdMap::new_empty(),
            message: Cell::new(None),
            hidden_prefixes: Vec::new(),
            saved_hidden_prefixes: None,
            method: "test".into(),
            flat,
            visible: Vec::new(),
            overrides: HashMap::new(),
            filters: Vec::new(),
            edges: HashMap::new(),
            parked: None,
            focus_truncated: false,
            search: None,
            search_input: None,
            jumps: Vec::new(),
            hidden: HashSet::new(),
            show_hidden: false,
            show_references: true,
            show_structural: true,
            detail: Detail::Full,
            sel: 0,
            top: 0,
            x_offset: 0,
            drag: None,
            viewport: Cell::new(10),
        };
        state.rebuild_visible();
        state
    }

    #[test]
    fn hiding_structural_rows_keeps_the_sinks_under_them() {
        let mut state = with_structural();
        assert_eq!(state.visible_idx(), vec![0, 1, 2, 3]);

        state.set_show_structural(false);
        // The phi is gone but the call beneath it survives
        assert_eq!(state.visible_idx(), vec![0, 1, 3]);
    }

    #[test]
    fn a_dropped_row_pulls_its_children_up_a_level() {
        let mut state = with_structural();
        assert_eq!(state.visible[3].depth, 3);

        state.set_show_structural(false);
        // LD was three levels down, the phi's removal makes it two
        assert_eq!(state.visible[2].idx, 3);
        assert_eq!(state.visible[2].depth, 2);
    }

    #[test]
    fn hiding_structural_rows_is_reversible() {
        let mut state = with_structural();
        state.set_show_structural(false);
        state.set_show_structural(true);
        assert_eq!(state.visible_idx(), vec![0, 1, 2, 3]);
        assert_eq!(state.visible[3].depth, 3);
    }

    #[test]
    fn external_calls_are_not_structural() {
        assert!(NodeKind::Phi.is_structural());
        assert!(NodeKind::Array.is_structural());
        assert!(NodeKind::Instruction.is_structural());
        assert!(!NodeKind::External.is_structural());
        assert!(!NodeKind::Call.is_structural());
        assert!(!NodeKind::Field.is_structural());
    }

    #[test]
    fn a_node_reached_more_than_once_says_so() {
        let mut state = state();
        state.flat[2].ways_in = 3;
        state.rebuild_visible();

        let at = state.visible_idx().iter().position(|it| *it == 2).unwrap();
        assert!(state.row_text(at).contains("[3 routes in]"));

        // One way in is the normal case and earns no marker
        state.flat[2].ways_in = 1;
        assert!(!state.row_text(at).contains("routes in"));
    }

    #[test]
    fn following_a_reference_can_be_undone() {
        let mut state = with_reference();
        // Point the reference at LA, which is drawn at index 1
        state.flat[1].node = Some(NodeId::from(1));
        state.flat[3].jump_to = Some(NodeId::from(1));
        state.sel = 3;

        assert!(state.follow_reference());
        assert_eq!(state.selected_node(), Some(1));
        assert_eq!(state.jumps, vec![3]);

        assert!(state.jump_back());
        assert_eq!(state.selected_node(), Some(3));
        assert!(state.jumps.is_empty());
    }

    #[test]
    fn jumping_back_with_nothing_to_undo_does_nothing() {
        let mut state = with_reference();
        state.sel = 2;
        assert!(!state.jump_back());
        assert_eq!(state.sel, 2);
    }

    #[test]
    fn jumps_nest() {
        let mut state = with_reference();
        state.flat[1].node = Some(NodeId::from(1));
        state.flat[3].jump_to = Some(NodeId::from(1));

        state.sel = 3;
        assert!(state.follow_reference());
        state.sel = 3;
        assert!(state.follow_reference());
        assert_eq!(state.jumps.len(), 2);

        assert!(state.jump_back());
        assert!(state.jump_back());
        assert!(!state.jump_back());
    }

    #[test]
    fn a_focused_view_hides_reference_rows() {
        let mut state = with_reference();
        assert_eq!(state.visible_idx(), vec![0, 1, 2, 3]);

        // Stand in for a focused view: the rows are replaced and `parked` holds the originals
        state.parked = Some(Vec::new());
        state.rebuild_visible();
        assert!(state.focused());
        assert_eq!(state.visible_idx(), vec![0, 1, 2]);
    }

    #[test]
    fn search_jumps_to_the_first_match_and_n_steps_on() {
        let mut state = state();
        state.flat[2].full = RcStr::from("Lcom/foo/Target;->parseUri");
        state.flat[4].full = RcStr::from("Lcom/bar/Other;->parseUri");

        state.start_search();
        for c in "parseUri".chars() {
            state.search_input.as_mut().unwrap().push(c);
        }
        state.commit_search();

        assert!(state.search_input.is_none());
        assert_eq!(state.selected_node(), Some(2));

        assert!(state.step_match(true));
        assert_eq!(state.selected_node(), Some(4));
    }

    #[test]
    fn search_opens_a_collapsed_subtree_to_reach_a_match() {
        let mut state = state();
        // LD sits under LC; collapse LC so it is not drawn
        state.flat[4].full = RcStr::from("buried target");
        state.sel = 3;
        state.toggle_collapse();
        assert_eq!(state.visible_idx(), vec![0, 1, 2, 3]);

        state.search = Some("buried target".into());
        state.sel = 0;
        assert!(state.step_match(true));

        assert_eq!(state.selected_node(), Some(4));
        assert!(state.visible_idx().contains(&4));
    }

    #[test]
    fn search_still_respects_a_filter() {
        let mut state = state();
        state.flat[4].full = RcStr::from("LD; target");
        state.set_filters(vec![Filter::ClassContains("LB;".into())]);
        // LD is filtered out, so its text is not searchable
        state.search = Some("target".into());
        assert!(!state.step_match(true));
    }

    #[test]
    fn search_skips_structural_rows_when_they_are_off() {
        let mut state = state();
        state.flat[2].kind = NodeKind::Phi;
        state.flat[2].full = RcStr::from("phi target");
        state.search = Some("phi target".into());

        assert!(state.step_match(true));
        assert_eq!(state.selected_node(), Some(2));

        state.set_show_structural(false);
        state.sel = 0;
        assert!(!state.step_match(true));
    }

    #[test]
    fn search_wraps_around() {
        let mut state = state();
        state.flat[2].full = RcStr::from("only match");
        state.search = Some("only match".into());

        state.sel = 3;
        assert!(state.step_match(true));
        assert_eq!(state.selected_node(), Some(2));
    }

    #[test]
    fn search_steps_backwards_too() {
        let mut state = state();
        state.flat[1].full = RcStr::from("hit");
        state.flat[4].full = RcStr::from("hit");
        state.search = Some("hit".into());

        state.sel = 0;
        assert!(state.step_match(false));
        assert_eq!(state.selected_node(), Some(4));
        assert!(state.step_match(false));
        assert_eq!(state.selected_node(), Some(1));
    }

    #[test]
    fn search_is_case_insensitive() {
        let mut state = state();
        state.flat[2].full = RcStr::from("Landroidx/compose/runtime/Foo;");
        state.start_search();
        for c in "RUNTIME".chars() {
            state.search_input.as_mut().unwrap().push(c);
        }
        state.commit_search();
        assert_eq!(state.selected_node(), Some(2));
    }

    #[test]
    fn a_search_that_matches_nothing_leaves_the_cursor_alone() {
        let mut state = state();
        state.sel = 2;
        state.search = Some("nothing here".into());
        assert!(!state.step_match(true));
        assert_eq!(state.sel, 2);
    }

    #[test]
    fn cancelling_a_search_keeps_the_previous_one() {
        let mut state = state();
        state.search = Some("old".into());
        state.start_search();
        state.search_input.as_mut().unwrap().push('x');
        state.cancel_search();
        assert!(state.search_input.is_none());
        assert_eq!(state.search.as_deref(), Some("old"));
    }

    #[test]
    fn slice_line_applies_the_horizontal_offset() {
        assert_eq!(slice_line("abcdef", 0, 3), "abc");
        assert_eq!(slice_line("abcdef", 2, 3), "cde");
        assert_eq!(slice_line("abcdef", 10, 3), "");
        assert_eq!(slice_line("abc", 0, 10), "abc");
    }
}
