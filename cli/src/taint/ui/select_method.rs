use std::{
    collections::{HashMap, HashSet},
    ops::{Deref, DerefMut},
};

use anyhow::bail;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};
use dtu::{
    analysis::db::taint::{
        db::{AnalyzedMethodSpec, Filter, GraphTaintAnalysisDb},
        models::{AnalyzedMethodId, MethodStatus},
    },
    db::graph::MethodSpec,
    smalisa::Type,
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
    taint::ui::{window::WindowAction, Command, CommandHandler, MethodPathsWindow, Tools},
    ui::widgets::list::new_list,
};

use crate::taint::ui::window::Window;

fn has_chains(_args: Vec<String>, _tools: &Tools, state: &mut State) -> anyhow::Result<()> {
    state.filter_methods(Box::new(move |it| it.nchains > 0));
    Ok(())
}

fn clear_filter(_args: Vec<String>, _tools: &Tools, state: &mut State) -> anyhow::Result<()> {
    state.clear_filters();
    Ok(())
}

fn do_filter(args: Vec<String>, tools: &Tools, state: &mut State) -> anyhow::Result<()> {
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
        "has-chains",
        Command {
            help: Some("only show methods with chains"),
            long_help: None,
            func: has_chains,
        },
    ),
];

pub struct State {
    methods: FilterVec<MethodData>,
    filters: Vec<Box<dyn Fn(&MethodData) -> bool>>,
    name_filter: Option<String>,
    show_hidden: bool,
    hidden_methods: HashSet<AnalyzedMethodId>,
    prev_filters: Option<Vec<Filter>>,
}

pub struct SelectMethodWindow {
    state: State,
    command: CommandHandler<State>,
}

impl State {
    /// Call this instead of directly calling unfilter directly on [Self::methods], as there are
    /// some default filters we want to always apply
    fn clear_filters(&mut self) {
        self.filters.clear();
        self.refilter();
    }

    /// Rerun all filters on the methods
    fn refilter(&mut self) {
        // Always filter out hidden things unless we're showing hidden things.
        let hidden = &self.hidden_methods;
        let show_hidden = self.show_hidden;
        self.methods.filter(|it| {
            (show_hidden || !hidden.contains(&it.analysis_id))
                && self
                    .name_filter
                    .as_ref()
                    .is_none_or(|name_filter| it.smali.contains(name_filter))
                && (self.filters.is_empty() || self.filters.iter().any(|func| func(it)))
        });
    }

    /// Use this any time you want to filter [Self::methods], as there are some default filters we
    /// want to always apply
    fn filter_methods(&mut self, func: Box<dyn Fn(&MethodData) -> bool>) {
        self.filters.push(func);
        self.refilter();
    }

    fn update_name_filtered(&mut self) {
        let Some(filter) = self.name_filter.take() else {
            self.clear_filters();
            return;
        };

        self.name_filter = Some(filter);
        self.refilter();
    }

    fn filering_by_name(&self) -> bool {
        self.name_filter.is_some()
    }

    fn stop_filering_by_name(&mut self) {
        self.name_filter = None;
        self.update_name_filtered();
    }

    fn name_filter_delete(&mut self) {
        match &mut self.name_filter {
            None => return,
            Some(cur) if cur.len() > 1 => {
                cur.truncate(cur.len() - 1);
                self.update_name_filtered();
                return;
            }
            Some(_) => {}
        }

        // Falling through means we've deleted the last char
        self.stop_filering_by_name();
    }

    fn name_filter_push(&mut self, c: char) {
        if let Some(cur) = self.name_filter.as_mut() {
            cur.push(c);
        } else {
            self.name_filter = Some(String::from(c));
        }
        self.update_name_filtered()
    }

    fn persist_name_filter(&mut self) {
        // Remove the filter but leave the list filtered
        self.name_filter = None;
    }

    fn filter_status(&mut self, status: MethodStatus) {
        self.filter_methods(Box::new(move |it| it.status == status))
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

struct MethodData {
    spec: AnalyzedMethodSpec,
    smali: String,
    width: usize,
}

impl Deref for MethodData {
    type Target = AnalyzedMethodSpec;
    fn deref(&self) -> &Self::Target {
        &self.spec
    }
}

impl SelectMethodWindow {
    pub fn new(db: &GraphTaintAnalysisDb) -> anyhow::Result<Self> {
        let methods = FilterContainer::new_vec(
            db.get_all_analyzed_method_specs()?
                .into_iter()
                .filter_map(|spec| {
                    if spec.nroutes == 0 {
                        None
                    } else {
                        let smali = spec.as_smali();
                        let width = smali.width();
                        Some(MethodData { spec, smali, width })
                    }
                })
                .collect::<Vec<_>>(),
        );

        let hidden_methods = HashSet::from_iter(db.get_hidden_analyzed_methods()?.into_iter());
        // Show hidden if they all are hidden
        let show_hidden = hidden_methods.len() == methods.len();
        let command = CommandHandler::new(HashMap::from_iter(COMMANDS.iter().copied()));
        let mut state = State {
            methods,
            show_hidden,
            hidden_methods,
            prev_filters: None,
            name_filter: None,
            filters: Vec::new(),
        };
        state.refilter();
        Ok(Self { command, state })
    }

    fn open_paths_window(&self, tools: &Tools) -> anyhow::Result<WindowAction> {
        let Some(method) = self.methods.get_selected() else {
            return Ok(WindowAction::Nothing);
        };
        let filters = self.state.prev_filters.clone();
        let window = Box::new(MethodPathsWindow::new(tools, method.analysis_id, filters)?);
        Ok(WindowAction::PushWindow(window))
    }
}

impl Window for SelectMethodWindow {
    fn on_key_event(&mut self, tools: &Tools, evt: KeyEvent) -> anyhow::Result<WindowAction> {
        if self.command.is_active() {
            let state = &mut self.state;
            let command = &mut self.command;
            return command.on_key_event(evt, tools, state);
        }

        match evt.modifiers {
            KeyModifiers::SHIFT if self.filering_by_name() => match evt.code {
                KeyCode::Char(c) => self.name_filter_push(c),
                _ => return Ok(WindowAction::default()),
            },
            KeyModifiers::SHIFT => match evt.code {
                KeyCode::Char('H') => self.toggle_method_hidden(tools)?,
                _ => return Ok(WindowAction::default()),
            },
            KeyModifiers::NONE => match evt.code {
                KeyCode::Esc if self.methods.is_filtered() => self.clear_filters(),
                KeyCode::Enter if self.filering_by_name() => self.persist_name_filter(),
                KeyCode::Backspace if self.filering_by_name() => self.name_filter_delete(),
                KeyCode::Char(c) if self.filering_by_name() => self.name_filter_push(c),
                KeyCode::Char('/') if !self.filering_by_name() => {
                    self.name_filter = Some(String::new());
                    self.clear_filters();
                }

                KeyCode::Enter => return self.open_paths_window(tools),
                KeyCode::Char(':') => self.command.activate(),
                KeyCode::Char('.') => self.toggle_show_hidden()?,
                KeyCode::Char('j') | KeyCode::Down => self.next_method(),
                KeyCode::Char('k') | KeyCode::Up => self.prev_method(),
                KeyCode::Char('p') => self.filter_status(MethodStatus::Pending),
                KeyCode::Char('d') => self.filter_status(MethodStatus::Done),
                KeyCode::Char('f') => self.filter_status(MethodStatus::Failed),
                _ => return Ok(WindowAction::default()),
            },
            _ => return Ok(WindowAction::default()),
        }
        Ok(WindowAction::Redraw)
    }

    fn on_mouse_event(&mut self, _tools: &Tools, evt: MouseEvent) -> anyhow::Result<WindowAction> {
        match evt.kind {
            MouseEventKind::ScrollUp => self.prev_method(),
            MouseEventKind::ScrollDown => self.next_method(),
            _ => return Ok(WindowAction::default()),
        }

        Ok(WindowAction::Redraw)
    }

    fn draw(&self, _tools: &Tools, frame: &mut Frame) {
        if self.command.draw(frame) {
            return;
        }

        let layout = Layout::vertical(&[
            Constraint::Length(1),
            Constraint::Fill(1),
            Constraint::Length(1),
        ]);
        let [title_area, body_area, filter_area] = frame.area().layout(&layout);

        let width = body_area.width as usize;

        let list_items = self.methods.iter().map(|method| {
            let is_hidden = self.hidden_methods.contains(&method.analysis_id);

            let txt = if method.width < width {
                method_names_fmt(method, is_hidden)
            } else {
                method_names_fmt_multi_line(&method.spec.spec, width, is_hidden)
            };
            txt
        });

        let list = new_list(list_items);
        let mut state = ListState::default().with_selected(Some(self.methods.sel_index()));

        let title = Line::from("Select method").centered().bold();
        frame.render_widget(title, title_area);
        frame.render_stateful_widget(list, body_area, &mut state);
        if let Some(filter) = &self.name_filter {
            let mut line = Line::raw("/");
            line.push_span(Span::raw(filter));
            frame.render_widget(line, filter_area);
        }
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

fn class_span<'a>(line: &mut Line<'a>, class: &'a str) {
    let class_style = Style::default().fg(Color::Magenta);
    line.push_span(Span::raw("L"));
    line.push_span(Span::styled(&class[1..class.len() - 1], class_style));
    line.push_span(Span::raw(";"));
}

fn method_names_fmt<'a>(method: &'a MethodSpec, is_hidden: bool) -> Text<'a> {
    let mut line = Line::default();

    let MethodSpec {
        name,
        signature: sig,
        ret,
        ..
    } = method;
    let class = method.class.as_str();

    class_span(&mut line, class);

    line.push_span(Span::raw("->"));
    line.push_span(Span::styled(name, Style::default().fg(Color::Green)));
    line.push_span(Span::raw("("));

    if sig.len() > 0 {
        if let Ok(iter) = SmaliMethodSignatureIterator::new(&sig) {
            for arg in iter {
                match arg {
                    Type::Class(class, dim) => {
                        if dim > 0 {
                            line.push_span(Span::raw("[".repeat(dim as usize)));
                        }
                        class_span(&mut line, class.as_str());
                    }
                    Type::Primitive(prim, dim) => {
                        if dim > 0 {
                            line.push_span(Span::raw("[".repeat(dim as usize)));
                        }
                        line.push_span(Span::raw(prim.as_smali_str()));
                    }
                    Type::Unknown => {}
                }
            }
        } else {
            line.push_span(Span::raw(sig));
        }
    }
    line.push_span(Span::raw(")"));

    if ret.starts_with('L') {
        class_span(&mut line, ret);
    } else {
        line.push_span(Span::raw(ret));
    }

    let mut text = Text::from(line);
    if is_hidden {
        set_gray(&mut text);
    }
    text
}

fn method_names_fmt_multi_line<'a>(
    method: &'a MethodSpec,
    width: usize,
    is_hidden: bool,
) -> Text<'a> {
    let mut txt = Text::default();

    let MethodSpec {
        name,
        signature: sig,
        ret,
        ..
    } = method;
    let class = method.class.as_str();

    let mut line = Line::default();
    class_span(&mut line, class);
    txt.push_line(line);

    let mut line = Line::default();
    line.push_span(Span::raw("   "));
    line.push_span(Span::styled(name, Style::default().fg(Color::Green)));

    let name_args_width = name.width() + sig.width() + 5;

    let (mut line, cur_width) = if name_args_width <= width {
        (line, name_args_width)
    } else {
        txt.push_line(line);
        let mut line = Line::default();
        line.push_span(Span::raw("   "));
        (line, name_args_width - name.width())
    };

    line.push_span(Span::raw("("));

    if sig.len() > 0 {
        if let Ok(iter) = SmaliMethodSignatureIterator::new(&sig) {
            for arg in iter {
                match arg {
                    Type::Class(class, dim) => {
                        if dim > 0 {
                            line.push_span(Span::raw("[".repeat(dim as usize)));
                        }
                        class_span(&mut line, class.as_str());
                    }
                    Type::Primitive(prim, dim) => {
                        if dim > 0 {
                            line.push_span(Span::raw("[".repeat(dim as usize)));
                        }
                        line.push_span(Span::raw(prim.as_smali_str()));
                    }
                    Type::Unknown => {}
                }
            }
        } else {
            line.push_span(Span::raw(sig));
        }
    }
    line.push_span(Span::raw(")"));

    let mut line = if cur_width + ret.width() <= width {
        line
    } else {
        txt.push_line(line);
        let mut line = Line::default();
        line.push_span(Span::raw("   "));
        line
    };

    if ret.starts_with('L') {
        class_span(&mut line, ret);
    } else {
        line.push_span(Span::raw(ret));
    }

    txt.push_line(line);

    if is_hidden {
        set_gray(&mut txt);
    }

    txt
}
