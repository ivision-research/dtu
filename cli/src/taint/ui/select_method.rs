use std::{
    borrow::Cow,
    cell::Cell,
    collections::{HashMap, HashSet},
    ops::{Deref, DerefMut},
};

use anyhow::bail;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};
use dtu::{
    analysis::taint::{
        db::{AnalyzedMethodSpec, Filter, GraphTaintAnalysisDb},
        models::AnalyzedMethodId,
    },
    db::graph::MethodSpec,
    smalisa::{ClassName as SClassName, JavaClassName, OwnedJavaClassName},
    utils::{FilterContainer, FilterVec, SmaliMethodSignatureIterator},
};
use ratatui::{
    layout::{Constraint, Layout},
    style::{Color, Style, Stylize},
    text::{Line, Span, Text},
    widgets::ListState,
    Frame,
};
use unicode_width::UnicodeWidthStr;

use crate::{
    taint::ui::{
        config::{Config, Detail},
        window::WindowAction,
        Command, CommandHandler, GraphViewWindow, Tools,
    },
    ui::{fit, widgets::list::new_list},
};

use crate::taint::ui::window::Window;

const WELL_KNOWN_PACKAGES_JAVA: &[&'static str] = &["java.lang", "android.os", "android.content"];

fn only_indirect(
    _args: Vec<String>,
    _tools: &Tools,
    _cfg: &mut Config,
    state: &mut State,
) -> anyhow::Result<()> {
    state.filter_methods(Box::new(move |it| !it.spec.direct));
    Ok(())
}

fn clear_filter(
    _args: Vec<String>,
    _tools: &Tools,
    _cfg: &mut Config,
    state: &mut State,
) -> anyhow::Result<()> {
    state.clear_filters();
    Ok(())
}

fn do_filter(
    args: Vec<String>,
    tools: &Tools,
    _cfg: &mut Config,
    state: &mut State,
) -> anyhow::Result<()> {
    if args.len() == 0 {
        state.clear_filters();
        return Ok(());
    }
    let mut new_filters: Vec<Filter> = Vec::new();

    for arg in args {
        let filter = arg.parse::<Filter>()?;
        new_filters.push(filter);
    }

    let ids = tools.db.get_analysis_matching(&new_filters)?;

    state.filter_methods(Box::new(move |it| ids.contains(&it.analysis_id)));

    state.prev_filters = Some(new_filters);
    Ok(())
}

/// Shown by `keys` and `help keys`
static KEYS_HELP: &str = "\
j / k          move the cursor, arrows work too
^d / ^u        move half a screen
PgDn / PgUp    move a whole screen
Enter          open the selected method
/              filter by name, Enter keeps it, Esc drops it
d              change display detail
H              hide or unhide the selected method
.              show hidden methods
Esc            clear every filter
?              this list
:              command, try `help`
^C             quit";

static FILTER_LONG_HELP: &'static str = r#"Filter methods by their paths, multiple filters can be applied

The following filters retain methods that have at least one path that matches
the specification. All values are treated as if they were %VALUE% unless
numeric, ie class=Runtime matches Ljava/lang/Runtime; and
Ljava/lang/RuntimeException;

class=CLASS   - Method call or field read on CLASS
method=METHOD - Method call to METHOD
sig=SIG       - Method call to methods with SIG in signature
ret=RET       - Method call to methods returning RET
field=FIELD   - Field access on FIELD
min-len=N     - Minimum path length of N
max-len=N     - Maximum path length of N
nophi         - Path doesn't contain Phis
fts5=FTS5     - Arbitrary FTS5 search, see the SQL for info

Example usage:

class=Runtime method=exec
"#;

static COMMANDS: &[(&'static str, Command<State>)] = &[
    (
        "filter",
        Command {
            help: Some("filter methods"),
            long_help: Some(FILTER_LONG_HELP),
            func: do_filter,
        },
    ),
    (
        "clear",
        Command {
            help: Some("clear filters"),
            long_help: None,
            func: clear_filter,
        },
    ),
    (
        "indirect",
        Command {
            help: Some("only show methods the call graph led to"),
            long_help: None,
            func: only_indirect,
        },
    ),
];

pub struct State {
    methods: FilterVec<MethodData>,
    filters: Vec<Box<dyn Fn(&MethodData) -> bool>>,
    /// The name search, applied by [State::refilter] whenever it is set
    display_filter: Option<String>,
    /// Whether keystrokes are going into the name search. The text outlives the editing, which
    /// is what Enter does: stop typing but keep the list narrowed.
    editing_name: bool,
    show_hidden: bool,
    detail: Detail,
    hidden_methods: HashSet<AnalyzedMethodId>,
    /// Body height of the last draw, so the page keys know how far a page is
    page: Cell<usize>,
    /// The list's scroll offset, carried between draws.
    ///
    /// Rebuilding [ListState] each frame would start it at zero, and the widget only scrolls far
    /// enough to reveal the selection, which pins the selection to the bottom of the viewport.
    list_offset: Cell<usize>,
    prev_filters: Option<Vec<Filter>>,
}

pub struct SelectMethodWindow {
    state: State,
    command: CommandHandler<State>,
}

impl State {
    fn show_classes(&self) -> bool {
        self.detail == Detail::Moderate || self.detail == Detail::Full
    }

    fn show_packages(&self) -> bool {
        self.detail == Detail::Full
    }

    /// Call this instead of directly calling unfilter directly on [Self::methods], as there are
    /// some default filters we want to always apply
    fn clear_filters(&mut self) {
        self.filters.clear();
        // The parsed filters ride along to the graph view, so dropping the predicates without
        // dropping these would open every method filtered by a search the list no longer shows
        self.prev_filters = None;
        // A persisted name search is a filter like any other, and this is the only way out of one
        self.display_filter = None;
        self.editing_name = false;
        self.refilter();
    }

    /// Rerun all filters on the methods
    fn refilter(&mut self) {
        // The list is about to be a different length, so the old scroll offset means nothing
        self.list_offset.set(0);
        // Always filter out hidden things unless we're showing hidden things.
        let hidden = &self.hidden_methods;
        let show_hidden = self.show_hidden;
        let include_class = self.show_classes();
        self.methods.filter(|it| {
            (show_hidden || !hidden.contains(&it.analysis_id))
                && self
                    .display_filter
                    .as_ref()
                    .is_none_or(|display_filter| it.display.contains(display_filter, include_class))
                && (self.filters.is_empty() || self.filters.iter().any(|func| func(it)))
        });
    }

    /// Use this any time you want to filter [Self::methods], as there are some default filters we
    /// want to always apply
    fn filter_methods(&mut self, func: Box<dyn Fn(&MethodData) -> bool>) {
        self.filters.push(func);
        self.refilter();
    }

    fn update_display_filtered(&mut self) {
        self.refilter();
    }

    fn filtering_by_display(&self) -> bool {
        self.editing_name
    }

    /// Drop the name search entirely, leaving any other filters alone
    fn stop_filtering_by_display(&mut self) {
        self.display_filter = None;
        self.editing_name = false;
        self.refilter();
    }

    fn display_filter_delete(&mut self) {
        match &mut self.display_filter {
            None => return,
            Some(cur) if cur.len() > 1 => {
                cur.truncate(cur.len() - 1);
                self.update_display_filtered();
                return;
            }
            Some(_) => {}
        }

        // Falling through means we've deleted the last char
        self.stop_filtering_by_display();
    }

    fn display_filter_push(&mut self, c: char) {
        if let Some(cur) = self.display_filter.as_mut() {
            cur.push(c);
        } else {
            self.display_filter = Some(String::from(c));
        }
        self.update_display_filtered()
    }

    /// Stop typing but keep the list narrowed
    ///
    /// The text has to stay: [State::refilter] recomputes from scratch, so clearing it here
    /// would widen the list again on the next unrelated refilter.
    fn persist_display_filter(&mut self) {
        self.editing_name = false;
    }

    fn everything_hidden(&self) -> bool {
        self.hidden_methods.len() == self.methods.total_len()
    }

    fn get_current_method(&self) -> Option<AnalyzedMethodId> {
        self.methods.get_selected().map(|it| it.analysis_id)
    }

    fn toggle_show_hidden(&mut self) -> anyhow::Result<()> {
        if self.show_hidden && self.everything_hidden() {
            bail!("everything is hidden");
        }
        self.show_hidden = !self.show_hidden;
        self.refilter();
        Ok(())
    }

    fn prev_method(&mut self) {
        self.methods.dec_sel();
    }
    fn next_method(&mut self) {
        self.methods.inc_sel();
    }

    /// Step `count` entries, used by the page and half page keys
    fn move_method(&mut self, count: usize, forward: bool) {
        for _ in 0..count {
            if forward {
                self.methods.inc_sel();
            } else {
                self.methods.dec_sel();
            }
        }
    }

    fn toggle_method_hidden(&mut self, tools: &Tools) -> anyhow::Result<()> {
        let Some(current) = self.get_current_method() else {
            return Ok(());
        };

        if self.hidden_methods.insert(current) {
            tools.db.hide_analyzed_method(current)?;
            self.refilter();
            return Ok(());
        }

        self.hidden_methods.remove(&current);
        tools.db.unhide_analyzed_method(current)?;
        self.refilter();

        Ok(())
    }
}

impl Deref for SelectMethodWindow {
    type Target = State;
    fn deref(&self) -> &Self::Target {
        &self.state
    }
}

impl DerefMut for SelectMethodWindow {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.state
    }
}

enum JavaTypeDisplay {
    Class(OwnedJavaClassName, u8),
    Primitive(Cow<'static, str>),
}

impl JavaTypeDisplay {
    fn contains(&self, needle: &str) -> bool {
        match self {
            Self::Primitive(p) => p.contains(needle),
            Self::Class(c, _) => c.as_str().contains(needle),
        }
    }

    fn width(&self) -> usize {
        match self {
            Self::Primitive(p) => p.width(),
            Self::Class(class, dim) => {
                let simple = class.get_simple_class();
                let mut base = simple.width();
                if let Some(pkg) = class.get_java_package() {
                    if !WELL_KNOWN_PACKAGES_JAVA.contains(&pkg.as_ref()) {
                        base += 1 + pkg.width();
                    }
                }
                base + ((*dim as usize) * 2)
            }
        }
    }

    fn from_smali<T>(value: &T) -> Self
    where
        T: AsRef<str> + ?Sized,
    {
        let value = value.as_ref();
        let trimmed = value.trim_start_matches('[');
        let dim = value.len() - trimmed.len();
        if trimmed.starts_with('L') {
            let class = JavaClassName::from_raw(trimmed).into_owned();
            return Self::Class(class, dim as u8);
        }

        let base = match trimmed {
            "I" => "int",
            "J" => "long",
            "S" => "short",
            "B" => "byte",
            "C" => "char",
            "F" => "float",
            "D" => "double",
            "Z" => "bool",
            "V" => "void",
            _ => "?",
        };
        let as_str = if dim > 0 {
            let mut s = base.to_string();
            s.reserve(2 * dim);
            for _ in 0..dim {
                s.push_str("[]");
            }
            Cow::Owned(s)
        } else {
            Cow::Borrowed(base)
        };

        Self::Primitive(as_str)
    }
}

struct MethodDisplay {
    class: JavaTypeDisplay,
    name: String,
    args: Vec<JavaTypeDisplay>,
    ret: JavaTypeDisplay,
    width: usize,
}

impl MethodDisplay {
    fn contains(&self, needle: &str, include_class: bool) -> bool {
        if include_class && self.class.contains(needle) {
            return true;
        }

        self.name.contains(needle)
            || self.ret.contains(needle)
            || self.args.iter().any(|it| it.contains(needle))
    }
}

impl From<&MethodSpec> for MethodDisplay {
    fn from(value: &MethodSpec) -> Self {
        // It is possible to have methods like [Lfoo/bar/Baz;->quux()
        let class = JavaTypeDisplay::from_smali(value.class.as_str());
        let name = value.name.clone();
        let ret = JavaTypeDisplay::from_smali(&value.ret);

        let mut width = class.width() + ret.width();

        if value.signature.is_empty() {
            return Self {
                class,
                name,
                ret,
                args: Vec::new(),
                width,
            };
        }

        let Ok(it) = SmaliMethodSignatureIterator::new(&value.signature) else {
            // This branch should never be hit
            let sig = JavaTypeDisplay::from_smali(&value.signature);
            width += sig.width();
            return Self {
                class,
                ret,
                name,
                width,
                args: vec![sig],
            };
        };

        let mut args = Vec::new();

        for arg in it {
            let disp = JavaTypeDisplay::from_smali(&arg.as_smali_str());
            // +2 for ,
            width += disp.width() + 2;
            args.push(disp);
        }

        width -= 2;

        Self {
            class,
            name,
            args,
            ret,
            width,
        }
    }
}

struct MethodData {
    spec: AnalyzedMethodSpec,
    display: MethodDisplay,
}

impl Deref for MethodData {
    type Target = AnalyzedMethodSpec;
    fn deref(&self) -> &Self::Target {
        &self.spec
    }
}

impl SelectMethodWindow {
    pub fn new(db: &GraphTaintAnalysisDb, cfg: &Config) -> anyhow::Result<Self> {
        let methods = FilterContainer::new_vec(
            db.get_all_analyzed_method_specs()?
                .into_iter()
                .filter_map(|spec| {
                    if spec.ngraphs == 0 {
                        None
                    } else {
                        let display = MethodDisplay::from(&spec.spec);
                        Some(MethodData { spec, display })
                    }
                })
                .collect::<Vec<_>>(),
        );

        let hidden_methods = HashSet::from_iter(db.get_hidden_analyzed_methods()?.into_iter());
        // Show hidden if they all are hidden
        let show_hidden = hidden_methods.len() == methods.len();
        let command = CommandHandler::new(HashMap::from_iter(COMMANDS.iter().copied()), KEYS_HELP);
        let mut state = State {
            methods,
            detail: cfg.methods.detail,
            show_hidden,
            hidden_methods,
            page: Cell::new(1),
            list_offset: Cell::new(0),
            prev_filters: None,
            display_filter: None,
            editing_name: false,
            filters: Vec::new(),
        };
        state.refilter();
        Ok(Self { command, state })
    }

    fn open_paths_window(&self, cfg: &mut Config, tools: &Tools) -> anyhow::Result<WindowAction> {
        let Some(method) = self.methods.get_selected() else {
            return Ok(WindowAction::Nothing);
        };
        let filters = self.state.prev_filters.clone();
        let window = Box::new(GraphViewWindow::new(
            tools,
            cfg,
            method.analysis_id,
            filters,
        )?);
        Ok(WindowAction::PushWindow(window))
    }
}

impl Window for SelectMethodWindow {
    fn on_key_event(
        &mut self,
        tools: &Tools,
        cfg: &mut Config,
        evt: KeyEvent,
    ) -> anyhow::Result<WindowAction> {
        if self.command.is_active() {
            let state = &mut self.state;
            let command = &mut self.command;
            return command.on_key_event(evt, tools, cfg, state);
        }

        let mycfg = &mut cfg.methods;

        if evt.modifiers == KeyModifiers::CONTROL {
            match evt.code {
                KeyCode::Char('d') => {
                    let half = (self.page.get() / 2).max(1);
                    self.move_method(half, true);
                }
                KeyCode::Char('u') => {
                    let half = (self.page.get() / 2).max(1);
                    self.move_method(half, false);
                }
                _ => return Ok(WindowAction::Nothing),
            }
            return Ok(WindowAction::Redraw);
        };

        if evt.modifiers != KeyModifiers::NONE && evt.modifiers != KeyModifiers::SHIFT {
            return Ok(WindowAction::Nothing);
        }

        if self.filtering_by_display() {
            match evt.code {
                KeyCode::Esc => self.stop_filtering_by_display(),
                KeyCode::Enter => self.persist_display_filter(),
                KeyCode::Backspace => self.display_filter_delete(),
                KeyCode::Char(c) => self.display_filter_push(c),

                _ => return Ok(WindowAction::Nothing),
            }
            return Ok(WindowAction::Redraw);
        }

        match evt.code {
            KeyCode::Char('?') => self.command.show_keys(),
            KeyCode::Char(':') => self.command.activate(),
            KeyCode::Char('H') => self.toggle_method_hidden(tools)?,
            KeyCode::Esc if self.methods.is_filtered() => self.clear_filters(),
            KeyCode::Char('/') => {
                // Clear first: clearing now drops the name search too
                self.clear_filters();
                self.display_filter = Some(String::new());
                self.editing_name = true;
                self.refilter();
            }

            KeyCode::Enter => return self.open_paths_window(cfg, tools),
            KeyCode::Char('.') => self.toggle_show_hidden()?,
            KeyCode::Char('j') | KeyCode::Down => self.next_method(),
            KeyCode::Char('k') | KeyCode::Up => self.prev_method(),
            KeyCode::PageDown => {
                let page = self.page.get();
                self.move_method(page, true);
            }
            KeyCode::PageUp => {
                let page = self.page.get();
                self.move_method(page, false);
            }
            KeyCode::Char('d') => {
                mycfg.detail.next();
                self.detail = mycfg.detail;
            }
            _ => return Ok(WindowAction::default()),
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
            MouseEventKind::ScrollUp => self.prev_method(),
            MouseEventKind::ScrollDown => self.next_method(),
            _ => return Ok(WindowAction::default()),
        }

        Ok(WindowAction::Redraw)
    }

    fn draw(&self, _tools: &Tools, _cfg: &mut Config, frame: &mut Frame) {
        if self.command.draw(frame) {
            return;
        }

        let layout = Layout::vertical(&[
            Constraint::Length(1),
            Constraint::Fill(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ]);
        let [title_area, body_area, filter_area, status_area] = frame.area().layout(&layout);

        self.page.set((body_area.height as usize).max(1));

        let width = body_area.width as usize;

        let list_items = self.methods.iter().map(|method| {
            let is_hidden = self.hidden_methods.contains(&method.analysis_id);

            let txt = if method.display.width < width {
                method_names_fmt(&self.state, &method.display, is_hidden)
            } else {
                method_names_fmt_multi_line(&self.state, &method.display, width, is_hidden)
            };
            txt
        });

        let list = new_list(list_items);
        let mut state = ListState::default()
            .with_offset(self.list_offset.get())
            .with_selected(Some(self.methods.sel_index()));

        let counts = format!(
            "  [{} of {}{}]",
            self.methods.len(),
            self.methods.total_len(),
            if self.show_hidden {
                ", showing hidden"
            } else {
                ""
            }
        );
        let title = Line::from(vec![
            Span::styled("Select method", Style::new().bold()),
            Span::raw(counts),
        ])
        .centered();
        frame.render_widget(title, title_area);
        frame.render_stateful_widget(list, body_area, &mut state);
        self.list_offset.set(state.offset());

        if let Some(filter) = &self.display_filter {
            let mut line = Line::raw("/");
            line.push_span(Span::raw(filter));
            // The text outlives the typing, so say which one this is
            if !self.editing_name {
                line.push_span(Span::raw("  (applied, Esc to clear)").dim());
            }
            frame.render_widget(line, filter_area);
        }

        let status = "j/k move | Enter open | / filter | : command | H hide | . show-hidden | Esc clear | ^C quit | ? help";
        frame.render_widget(
            Line::raw(fit(status, status_area.width as usize)).dim(),
            status_area,
        );
    }
}

fn set_gray(txt: &mut Text) {
    txt.style = txt.style.fg(Color::Gray);
    for line in txt.lines.iter_mut() {
        line.style = line.style.fg(Color::Gray);
        for span in line.spans.iter_mut() {
            span.style = span.style.fg(Color::Gray);
        }
    }
}

fn sig_span<'a>(line: &mut Line<'a>, show_pkg: bool, args: &'a [JavaTypeDisplay]) -> usize {
    if args.len() == 0 {
        return 0;
    }

    let mut width = 0;
    let nargs = args.len();

    for (i, arg) in args.iter().enumerate() {
        width += arg.to_line(line, show_pkg, Style::default().fg(Color::Yellow));
        if i < nargs - 1 {
            line.push_span(Span::raw(", "));
            width += 2;
        }
    }

    width
}

fn method_names_fmt<'a>(state: &State, method: &'a MethodDisplay, is_hidden: bool) -> Text<'a> {
    let mut line = Line::default();

    let MethodDisplay {
        class,
        ret,
        args,
        name,
        ..
    } = method;

    let include_class = state.show_classes();
    let show_pkg = state.show_packages();

    if include_class {
        class.to_line(&mut line, show_pkg, Style::default());
        line.push_span(Span::raw("."));
    }
    line.push_span(Span::styled(name, Style::default().fg(Color::Green)));
    line.push_span(Span::raw("("));
    sig_span(&mut line, show_pkg, args);
    line.push_span(Span::raw("): "));

    ret.to_line(&mut line, show_pkg, Style::default());

    let mut text = Text::from(line);
    if is_hidden {
        set_gray(&mut text);
    }
    text
}

fn method_names_fmt_multi_line<'a>(
    state: &State,
    method: &'a MethodDisplay,
    width: usize,
    is_hidden: bool,
) -> Text<'a> {
    let mut txt = Text::default();

    let MethodDisplay {
        class,
        ret,
        args,
        name,
        ..
    } = method;
    let include_class = state.show_classes();
    let show_pkg = state.show_packages();

    let mut cur_width = 0;
    let name_width = name.width();
    let mut line = Line::default();
    if include_class {
        cur_width = class.to_line(&mut line, show_pkg, Style::default()) + 1;
        if cur_width + name_width >= width {
            txt.push_line(line);
            line = Line::default();
            line.push_span(Span::raw("   "));
            cur_width = 3;
        } else {
            line.push_span(Span::raw("."));
        }
    }
    line.push_span(Span::styled(name, Style::default().fg(Color::Green)));
    cur_width += name_width;

    let mut sig_line = Line::default();
    sig_line.push_span(Span::raw("("));
    let sig_width = sig_span(&mut sig_line, show_pkg, args) + 2;
    sig_line.push_span(Span::raw(")"));

    if cur_width + sig_width <= width {
        line.spans.reserve(sig_line.spans.len());
        line.spans.extend(sig_line.spans.into_iter());
        cur_width += sig_width;
    } else {
        txt.push_line(line);
        line = Line::default();
        line.spans.reserve(sig_line.spans.len() + 1);
        line.push_span(Span::raw("   "));
        line.spans.extend(sig_line.spans.into_iter());
        cur_width = 0;
    };

    let mut ret_line = Line::default();
    ret_line.push_span(": ");
    let ret_width = ret.to_line(&mut ret_line, show_pkg, Style::default()) + 2;

    if cur_width + ret_width <= width {
        line.spans.reserve(ret_line.spans.len());
        line.spans.extend(ret_line.into_iter());
        txt.push_line(line);
    } else {
        txt.push_line(line);
        txt.push_line(ret_line);
    }

    if is_hidden {
        set_gray(&mut txt);
    }

    txt
}

impl JavaTypeDisplay {
    fn to_line<'a>(&'a self, line: &mut Line<'a>, show_pkg: bool, style: Style) -> usize {
        match self {
            JavaTypeDisplay::Class(jc, dim) => {
                let mut width = (*dim as usize) * 2;

                let class = jc.get_simple_class();

                if show_pkg {
                    match jc.get_java_package() {
                        Some(pkg) if !WELL_KNOWN_PACKAGES_JAVA.contains(&pkg.as_ref()) => {
                            line.push_span(Span::styled(class, style));
                            width += class.width();
                        }
                        _ => {
                            let as_java = jc.as_str();
                            line.push_span(Span::styled(as_java, style));
                            width += as_java.width();
                        }
                    }
                } else {
                    line.push_span(Span::styled(class, style));
                    width += class.width();
                }
                if *dim > 0 {
                    for _ in 0..*dim {
                        line.push_span(Span::raw("[]"));
                    }
                }

                width
            }
            JavaTypeDisplay::Primitive(p) => {
                let as_str = p.as_ref();
                line.push_span(Span::styled(as_str, style));
                as_str.width()
            }
        }
    }
}
