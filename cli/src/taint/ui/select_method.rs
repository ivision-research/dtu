use std::{
    collections::HashMap,
    ops::{Deref, DerefMut},
};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};
use dtu::{
    analysis::db::taint::{
        db::{AnalyzedMethodSpec, Filter, GraphTaintAnalysisDb},
        models::MethodStatus,
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
    taint::ui::{window::WindowAction, Command, CommandFunc, MethodPathsWindow, Tools},
    ui::widgets::list::new_list,
};

use crate::taint::ui::window::Window;

fn has_chains(_args: Vec<String>, _tools: &Tools, state: &mut State) -> anyhow::Result<()> {
    state.methods.filter(|it| it.0.nchains > 0);
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
        if arg == "$$" {
            if let Some(old) = state.prev_filters.take() {
                new_filters.extend(old.into_iter());
            }
        } else {
            let filter = arg.parse::<Filter>()?;
            new_filters.push(filter);
        }
    }

    let ids = tools.db.get_analysis_matching(&new_filters)?;

    state.methods.filter(|it| ids.contains(&it.0.analysis_id));

    state.prev_filters = Some(new_filters);
    Ok(())
}

static COMMANDS: &[(&'static str, CommandFunc<State>)] = &[
    ("filter", do_filter),
    ("clear", clear_filter),
    ("has-chains", has_chains),
];

pub struct State {
    methods: FilterVec<(AnalyzedMethodSpec, String, usize)>,
    filter: Option<String>,
    prev_filters: Option<Vec<Filter>>,
}

pub struct SelectMethodWindow {
    command: Command<State>,
    state: State,
}

impl State {
    fn clear_filters(&mut self) {
        self.methods.unfilter();
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

impl SelectMethodWindow {
    pub fn new(db: &GraphTaintAnalysisDb) -> anyhow::Result<Self> {
        let methods = FilterContainer::new_vec(
            db.get_all_analyzed_method_specs()?
                .into_iter()
                .filter_map(|it| {
                    if it.nroutes == 0 {
                        None
                    } else {
                        let smali = it.as_smali();
                        let width = smali.width();
                        Some((it, smali, width))
                    }
                })
                .collect::<Vec<_>>(),
        );

        let command = Command::new(HashMap::from_iter(COMMANDS.iter().copied()));
        Ok(Self {
            command,
            state: State {
                methods,
                prev_filters: None,
                filter: None,
            },
        })
    }

    fn open_paths_window(&self, tools: &Tools) -> anyhow::Result<WindowAction> {
        let Some((ana_method, _, _)) = self.methods.get_selected() else {
            return Ok(WindowAction::Nothing);
        };
        let filters = self.state.prev_filters.clone();
        let window = Box::new(MethodPathsWindow::new(
            tools,
            ana_method.analysis_id,
            filters,
        )?);
        Ok(WindowAction::PushWindow(window))
    }

    fn update_filtered(&mut self) {
        let Some(filter) = self.filter.take() else {
            self.methods.unfilter();
            return;
        };

        self.state
            .methods
            .filter(|(_, smali, _)| smali.contains(&filter));
        self.filter = Some(filter);
    }

    fn filtering(&self) -> bool {
        self.filter.is_some()
    }

    fn stop_filtering(&mut self) {
        self.filter = None;
        self.update_filtered();
    }

    fn filter_delete(&mut self) {
        match &mut self.filter {
            None => return,
            Some(cur) if cur.len() > 1 => {
                cur.truncate(cur.len() - 1);
                self.update_filtered();
                return;
            }
            Some(_) => {}
        }

        // Falling through means we've deleted the last char
        self.stop_filtering();
    }

    fn filter_push(&mut self, c: char) {
        if let Some(cur) = self.filter.as_mut() {
            cur.push(c);
        } else {
            self.filter = Some(String::from(c));
        }
        self.update_filtered()
    }

    fn persist_filter(&mut self) {
        // Remove the filter but leave the list filtered
        self.filter = None;
    }

    fn filter_status(&mut self, status: MethodStatus) {
        self.methods.filter(|it| it.0.status == status)
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
            KeyModifiers::SHIFT if self.filtering() => match evt.code {
                KeyCode::Char(c) => self.filter_push(c),
                _ => return Ok(WindowAction::default()),
            },
            KeyModifiers::NONE => match evt.code {
                KeyCode::Esc if self.methods.is_filtered() => self.clear_filters(),
                KeyCode::Enter if self.filtering() => self.persist_filter(),
                KeyCode::Backspace if self.filtering() => self.filter_delete(),
                KeyCode::Char(c) if self.filtering() => self.filter_push(c),
                KeyCode::Char('/') if !self.filtering() => {
                    self.filter = Some(String::new());
                    self.methods.unfilter();
                }

                KeyCode::Enter => return self.open_paths_window(tools),
                KeyCode::Char(':') => self.command.activate(),
                KeyCode::Char('j') | KeyCode::Down => self.methods.inc_sel(),
                KeyCode::Char('k') | KeyCode::Up => self.methods.dec_sel(),
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
            MouseEventKind::ScrollUp => self.methods.dec_sel(),
            MouseEventKind::ScrollDown => self.methods.inc_sel(),
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

        let list_items = self.methods.iter().map(|(method, _, smali_width)| {
            let txt = if *smali_width < width {
                method_names_fmt(method)
            } else {
                method_names_fmt_multi_line(method, width)
            };
            txt
        });

        let list = new_list(list_items);
        let mut state = ListState::default().with_selected(Some(self.methods.sel_index()));

        let title = Line::from("Select method").centered().bold();
        frame.render_widget(title, title_area);
        frame.render_stateful_widget(list, body_area, &mut state);
        if let Some(filter) = &self.filter {
            let mut line = Line::raw("/");
            line.push_span(Span::raw(filter));
            frame.render_widget(line, filter_area);
        }
    }
}

fn class_span<'a>(line: &mut Line<'a>, class: &'a str) {
    let class_style = Style::default().fg(Color::Magenta);
    line.push_span(Span::raw("L"));
    line.push_span(Span::styled(&class[1..class.len() - 1], class_style));
    line.push_span(Span::raw(";"));
}

fn method_names_fmt<'a>(method: &'a MethodSpec) -> Text<'a> {
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

    Text::from(line)
}

fn method_names_fmt_multi_line<'a>(method: &'a MethodSpec, width: usize) -> Text<'a> {
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

    txt
}
