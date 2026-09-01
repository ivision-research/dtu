use std::{
    collections::{HashMap, HashSet},
    ops::{Deref, DerefMut},
};

use anyhow::bail;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use dtu::{
    analysis::db::taint::{
        db::{
            Filter, MethodSpecAndDisplay, ResolvableIds, ResolvedOrigin, ResolvedTaintRoute,
            ResolvedTaintRoutes, YokedResolvedTaintRoutes,
        },
        models::{AnalyzedMethodId, RouteId},
    },
    utils::{CircularIndex, FilterContainer},
};
use ratatui::{
    layout::{Constraint, Layout},
    style::{Color, Stylize},
    text::{Line, Text},
    widgets::{ListItem, ListState},
    Frame,
};

use crate::{
    taint::ui::{Command, CommandFunc, Tools, Window, WindowAction},
    ui::widgets::list::new_list,
    utils::dtu_open_method_spec,
};

pub struct State {
    analysis_id: AnalyzedMethodId,
    method: String,
    show_chains: bool,
    pathno: usize,
    chainno: CircularIndex,
    nchains: usize,
    show_hidden: bool,
    chain_sel_idx: CircularIndex,
    prev_filters: Option<Vec<Filter>>,
    resolved: FilterContainer<YokedResolvedTaintRoutes>,
    hidden_routes: HashSet<RouteId>,
    list_sel_idx: CircularIndex,
}

pub struct MethodPathsWindow {
    command: Command<State>,
    state: State,
}

fn clear_filter(_args: Vec<String>, _tools: &Tools, state: &mut State) -> anyhow::Result<()> {
    state.clear_filters();
    Ok(())
}

fn show_chains(_args: Vec<String>, _tools: &Tools, state: &mut State) -> anyhow::Result<()> {
    state.switch_to_chains();
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

    state.apply_filters(tools, &new_filters)?;
    state.prev_filters = Some(new_filters);
    Ok(())
}

static COMMANDS: &[(&'static str, CommandFunc<State>)] = &[
    ("filter", do_filter),
    ("clear", clear_filter),
    ("chains", show_chains),
];

impl Deref for MethodPathsWindow {
    type Target = State;
    fn deref(&self) -> &Self::Target {
        &self.state
    }
}

impl DerefMut for MethodPathsWindow {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.state
    }
}

impl MethodPathsWindow {
    pub fn new(
        tools: &Tools,
        ana_id: AnalyzedMethodId,
        filters: Option<Vec<Filter>>,
    ) -> anyhow::Result<Self> {
        let db = tools.db;
        let ana = db.get_analysis_info(ana_id)?;
        ana.ensure_valid()?;
        let paths = db.get_routes_for_analysis(&ana)?;
        if paths.len() == 0 {
            bail!("no paths for analysis entry: {ana_id}");
        }
        let hidden_routes = HashSet::from_iter(db.get_hidden_routes()?.into_iter());

        // If we enter here and everything is hidden we'd never be able to unhide things without
        // this
        let show_hidden = hidden_routes.len() == paths.len();

        let report_map = db.get_report_id_map(&ana, ResolvableIds::All)?;
        let Ok(analyzed) = report_map.get_method(ana.method) else {
            bail!("BUG! the analyzed method itself wasn't included in all referenced methods...");
        };
        let method = analyzed.as_smali();

        let resolved = FilterContainer::new(ResolvedTaintRoutes::resolve_yoked(paths, report_map)?);
        let command = Command::new(HashMap::from_iter(COMMANDS.iter().copied()));
        let list_sel_idx = match resolved.first() {
            Some(v) => CircularIndex::new_for_slice(&v.sinks),
            None => CircularIndex::new(0),
        };
        let chain_sel_idx = CircularIndex::new(0);

        let nchains = match &resolved.container().get().origin {
            ResolvedOrigin::Direct => 0,
            ResolvedOrigin::CallGraph { chains } => chains.len(),
        };

        let prev_filters = filters;
        let mut state = State {
            method,
            hidden_routes,
            nchains,
            analysis_id: ana_id,
            prev_filters: None,
            resolved,
            show_hidden,
            show_chains: false,
            pathno: 0,
            chainno: CircularIndex::new(nchains.saturating_sub(1)),
            list_sel_idx,
            chain_sel_idx,
        };

        let chain_len = state
            .get_chain()
            .map(|it| it.len().saturating_sub(1))
            .unwrap_or(0);
        state.chain_sel_idx.set_max(chain_len);

        if let Some(filters) = &prev_filters {
            state.apply_filters(tools, filters)?;
        } else {
            state.init_hidden_filter();
        }
        state.prev_filters = prev_filters;

        Ok(Self { command, state })
    }

    fn draw_chains(&self, frame: &mut Frame) {
        let layout = Layout::vertical(&[
            Constraint::Length(1),
            Constraint::Fill(1),
            Constraint::Length(1),
        ]);

        let [header, chain_area, footer] = frame.area().layout(&layout);

        let State { method, .. } = &self.state;

        frame.render_widget(
            Line::raw(format!("{method} [Chains]")).centered().cyan(),
            header,
        );

        let Some(chain) = self.state.get_chain() else {
            frame.render_widget(Text::raw("no chains").centered().red(), chain_area);
            return;
        };

        let list_items = chain
            .iter()
            .map(|it| ListItem::new(Text::raw(it.display.as_str()).centered()));

        let chain_methods = new_list(list_items);
        let mut state = ListState::default().with_selected(Some(self.chain_sel_idx.pos()));

        frame.render_stateful_widget(chain_methods, chain_area, &mut state);
        frame.render_widget(
            Line::raw(format!("{}/{}", self.chainno.index(), self.nchains,)).centered(),
            footer,
        );
    }

    fn draw_sinks(&self, frame: &mut Frame) {
        let layout = Layout::vertical(&[
            Constraint::Length(1),
            Constraint::Fill(1),
            Constraint::Length(1),
        ]);

        let [header, path_area, footer] = frame.area().layout(&layout);

        let State { method, pathno, .. } = &self.state;

        let path = self.resolved.get_selected();

        let color =
            if self.show_hidden && path.is_some_and(|it| self.hidden_routes.contains(&it.id)) {
                Color::Gray
            } else {
                Color::Cyan
            };

        frame.render_widget(Line::raw(method.as_str()).centered().fg(color), header);

        let Some(path) = self.resolved.get_selected() else {
            frame.render_widget(Text::raw("no available paths").centered().red(), path_area);
            return;
        };

        let source = path.source;
        let list_items = path.sinks.iter().map(|it| {
            let as_str = it.sink.as_ref();
            ListItem::new(Text::raw(as_str).centered())
        });

        let sinks = new_list(list_items);
        let mut state = ListState::default().with_selected(Some(self.list_sel_idx.pos()));

        frame.render_stateful_widget(sinks, path_area, &mut state);
        frame.render_widget(
            Line::raw(format!(
                "{} | {} | {}/{}{}",
                source,
                if path.incomplete {
                    "truncated"
                } else {
                    "not truncated"
                },
                pathno,
                self.resolved.len().saturating_sub(1),
                if self.resolved.is_filtered() { "*" } else { "" }
            ))
            .centered(),
            footer,
        );
    }

    fn on_chains_key_event(
        &mut self,
        tools: &Tools,
        evt: KeyEvent,
    ) -> anyhow::Result<WindowAction> {
        match evt.modifiers {
            KeyModifiers::SHIFT => match evt.code {
                KeyCode::Char('O') => {
                    return self
                        .dtu_open_chain_link(tools)
                        .and(Ok(WindowAction::Nothing))
                }
                _ => return Ok(WindowAction::default()),
            },
            KeyModifiers::NONE => match evt.code {
                KeyCode::Esc => self.leave_chains(),
                KeyCode::Char('h') | KeyCode::Left => self.prev_chain(),
                KeyCode::Char('l') | KeyCode::Right => self.next_chain(),
                KeyCode::Char('j') | KeyCode::Down => self.next_chain_link(),
                KeyCode::Char('k') | KeyCode::Up => self.prev_chain_link(),

                _ => return Ok(WindowAction::default()),
            },
            _ => return Ok(WindowAction::default()),
        }

        Ok(WindowAction::Redraw)
    }

    fn dtu_open_sink(&self, tools: &Tools) -> anyhow::Result<()> {
        let Some(path) = self.resolved.get_selected() else {
            bail!("no paths selected");
        };

        let Some(sink) = path.sinks.get(self.list_sel_idx.index()) else {
            bail!("no sink selected");
        };

        dtu_open_method_spec(tools.ctx, &sink.in_method.spec)
    }

    fn dtu_open_chain_link(&self, tools: &Tools) -> anyhow::Result<()> {
        let Some(chain) = self.get_chain() else {
            bail!("no chains");
        };

        let Some(link) = chain.get(self.chain_sel_idx.index()) else {
            bail!("no chain selected");
        };

        dtu_open_method_spec(tools.ctx, &link.spec)
    }
}

impl State {
    fn switch_to_chains(&mut self) {
        self.chainno.reset();
        self.show_chains = true;
        self.chain_sel_idx.reset();
    }

    fn leave_chains(&mut self) {
        self.show_chains = false;
    }

    fn prev_chain(&mut self) {
        self.chainno.dec();
        self.chain_changed();
    }
    fn next_chain(&mut self) {
        self.chainno.inc();
        self.chain_changed();
    }

    fn next_chain_link(&mut self) {
        self.chain_sel_idx.inc();
    }

    fn prev_chain_link(&mut self) {
        self.chain_sel_idx.dec();
    }

    fn get_chain(&self) -> Option<&'_ Vec<&'_ MethodSpecAndDisplay>> {
        let chainno = self.chainno;
        let routes = self.resolved.container().get();
        match &routes.origin {
            ResolvedOrigin::CallGraph { chains } => chains.get(chainno.index()),
            ResolvedOrigin::Direct => None,
        }
    }

    fn chain_changed(&mut self) {
        self.chain_sel_idx.reset();
        let chain_len = self
            .get_chain()
            .map(|it| it.len().saturating_sub(1))
            .unwrap_or(0);
        self.chain_sel_idx.set_max(chain_len);
    }

    fn prev_route(&mut self) {
        self.pathno = self.resolved.dec_sel_get();
        self.path_changed();
    }
    fn next_route(&mut self) {
        self.pathno = self.resolved.inc_sel_get();
        self.path_changed();
    }

    fn next_sink(&mut self) {
        self.list_sel_idx.inc();
    }

    fn prev_sink(&mut self) {
        self.list_sel_idx.dec();
    }

    fn path_changed(&mut self) {
        self.list_sel_idx.reset();
        match self.resolved.get_selected() {
            Some(v) => self.list_sel_idx.max_to_slice(&v.sinks),
            None => self.list_sel_idx.set_max(0),
        }
    }

    fn init_hidden_filter(&mut self) {
        self.clear_filters();
    }

    /// Call this instead of directly calling unfilter directly on [Self::resolved], as there are
    /// some default filters we want to always apply
    fn clear_filters(&mut self) {
        let hidden = &self.hidden_routes;
        let show_hidden = self.show_hidden;
        self.resolved
            .filter(|it| show_hidden || !hidden.contains(&it.id));
    }

    /// Use this any time you want to filter [Self::resolved], as there are some default filters we
    /// want to always apply
    fn filter_resolved<F>(&mut self, func: F)
    where
        F: Fn(&ResolvedTaintRoute) -> bool,
    {
        let hidden = &self.hidden_routes;
        let show_hidden = self.show_hidden;
        // Always filter out hidden things unless we're showing hidden things.
        self.resolved
            .filter(|it| (show_hidden || !hidden.contains(&it.id)) && func(it))
    }

    fn apply_filters(&mut self, tools: &Tools, filters: &[Filter]) -> anyhow::Result<()> {
        let run_ids = tools.db.get_routes_matching(self.analysis_id, filters)?;
        self.filter_resolved(|it| run_ids.contains(&it.id));
        Ok(())
    }

    fn get_current_route(&self) -> Option<RouteId> {
        self.resolved.get_selected().map(|it| it.id)
    }

    fn everything_hidden(&self) -> bool {
        self.hidden_routes.len() == self.resolved.total_len()
    }

    fn toggle_show_hidden(&mut self) -> anyhow::Result<()> {
        if self.show_hidden && self.everything_hidden() {
            bail!("everything is hidden");
        }
        self.show_hidden = !self.show_hidden;
        Ok(())
    }

    fn toggle_route_hidden(&mut self, tools: &Tools) -> anyhow::Result<()> {
        let Some(current) = self.get_current_route() else {
            return Ok(());
        };

        if self.hidden_routes.insert(current) {
            tools.db.hide_route(current)?;
            self.next_route();
            return Ok(());
        }

        self.hidden_routes.remove(&current);
        tools.db.unhide_route(current)?;

        Ok(())
    }
}

impl Window for MethodPathsWindow {
    fn draw(&self, _tools: &Tools, frame: &mut Frame) {
        if self.command.draw(frame) {
            return;
        }

        if self.show_chains {
            self.draw_chains(frame);
            return;
        }

        self.draw_sinks(frame);
    }

    fn on_key_event(&mut self, tools: &Tools, evt: KeyEvent) -> anyhow::Result<WindowAction> {
        if self.command.is_active() {
            let state = &mut self.state;
            let command = &mut self.command;
            return command.on_key_event(evt, tools, state);
        }

        if self.show_chains {
            return self.on_chains_key_event(tools, evt);
        }

        match evt.modifiers {
            KeyModifiers::SHIFT => match evt.code {
                KeyCode::Char('O') => {
                    return self.dtu_open_sink(tools).and(Ok(WindowAction::Nothing))
                }
                KeyCode::Char('H') => self.toggle_route_hidden(tools)?,
                _ => return Ok(WindowAction::default()),
            },

            KeyModifiers::NONE => match evt.code {
                KeyCode::Esc => return Ok(WindowAction::PopWindow),
                KeyCode::Char(':') => self.command.activate(),
                KeyCode::Char('.') => self.toggle_show_hidden()?,
                KeyCode::Char('h') | KeyCode::Left => self.prev_route(),
                KeyCode::Char('l') | KeyCode::Right => self.next_route(),
                KeyCode::Char('j') | KeyCode::Down => self.next_sink(),
                KeyCode::Char('k') | KeyCode::Up => self.prev_sink(),

                _ => return Ok(WindowAction::default()),
            },
            _ => return Ok(WindowAction::default()),
        }
        Ok(WindowAction::Redraw)
    }
}
