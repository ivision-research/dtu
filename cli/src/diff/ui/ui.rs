use std::borrow::Cow;

use crate::ui::widgets::list::new_list;
use crate::ui::widgets::{ClosureWidget, BG_COLOR, FG_COLOR};
use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, StatefulWidget, Widget};
use ratatui::Frame;

use super::applet::Applet;

#[repr(u8)]
#[derive(Default, PartialEq, Clone, Copy)]
pub enum ActiveSection {
    #[default]
    ItemList,
    FilterBox,
}

#[repr(u8)]
#[derive(Default, PartialEq, Clone, Copy)]
pub enum ActiveTab {
    #[default]
    SystemServices,
    SystemServiceMethods,
    Apks,
    Providers,
    Receivers,
    Services,
    Activities,
}

pub fn draw(f: &mut Frame, applet: &Applet) {
    let active = applet.get_active_tab();

    let tabs = List::new(vec![
        ListItem::new("System Services"),
        ListItem::new("System Service Methods"),
        ListItem::new("APKs"),
        ListItem::new("Providers"),
        ListItem::new("Receivers"),
        ListItem::new("Services"),
        ListItem::new("Activities"),
    ])
    .block(Block::default().borders(Borders::ALL).title("Types"))
    .style(Style::default().fg(FG_COLOR))
    .highlight_style(
        Style::default()
            .add_modifier(Modifier::BOLD)
            .bg(FG_COLOR)
            .fg(BG_COLOR),
    );

    let mut tabs_state = ListState::default();
    tabs_state.select(Some(active as usize));

    let chunks = Layout::default()
        .constraints(vec![Constraint::Percentage(10), Constraint::Percentage(90)])
        .direction(Direction::Horizontal)
        .split(f.area());

    let content = ContentWidget::new(applet);
    let mut lst_state = ListState::default();
    lst_state.select(Some(applet.get_selection_idx()));
    f.render_stateful_widget(tabs, chunks[0], &mut tabs_state);
    f.render_stateful_widget(content, chunks[1], &mut lst_state);

    if let Some(pop) = applet.get_popup() {
        let popup_area = get_popup_area(f.area());
        f.render_widget(Clear, popup_area);
        f.render_widget(pop, popup_area);
    }
}

pub fn get_popup_area(area: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints(
            [
                Constraint::Percentage(25),
                Constraint::Percentage(50),
                Constraint::Percentage(25),
            ]
            .as_ref(),
        )
        .split(area);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints(
            [
                Constraint::Percentage(25),
                Constraint::Percentage(50),
                Constraint::Percentage(25),
            ]
            .as_ref(),
        )
        .split(popup_layout[1])[1]
}

struct ContentWidget<'a> {
    items: Vec<ListItem<'a>>,
    filter_widget: Option<ClosureWidget>,
    search_string: Option<String>,
    editing_search: bool,
}

impl<'a> ContentWidget<'a> {
    fn new(applet: &'a Applet) -> Self {
        let items = applet.get_active_items_list();
        let filter_widget = applet.get_filter_widget();
        let search_string = applet.get_search_string();
        let editing_search = applet.get_editing_search_string();

        Self {
            items,
            filter_widget,
            search_string,
            editing_search,
        }
    }
}

impl<'a> StatefulWidget for ContentWidget<'a> {
    type State = ListState;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut Self::State) {
        let lst = new_list(self.items);

        let has_filter = self.filter_widget.is_some();
        let has_search = self.search_string.is_some() || self.editing_search;

        if !(has_filter || has_search) {
            StatefulWidget::render(lst, area, buf, state);
            return;
        }

        let constraints = if has_filter && has_search {
            vec![
                Constraint::Percentage(75),
                Constraint::Percentage(20),
                Constraint::Percentage(5),
            ]
        } else if has_filter {
            vec![Constraint::Percentage(80), Constraint::Percentage(20)]
        } else {
            vec![Constraint::Percentage(95), Constraint::Percentage(5)]
        };

        let chunks = Layout::default()
            .constraints(constraints)
            .direction(Direction::Vertical)
            .split(area);

        StatefulWidget::render(lst, chunks[0], buf, state);

        if let Some(filter) = self.filter_widget {
            filter.render(chunks[1], buf);
        }

        if has_search {
            let search_string = if let Some(search) = self.search_string {
                Cow::Owned(format!("Search: {}", search))
            } else {
                Cow::Borrowed("Search: ")
            };

            let style = if self.editing_search {
                Style::default().bg(FG_COLOR).fg(BG_COLOR)
            } else {
                Style::default().fg(FG_COLOR).bg(BG_COLOR)
            };

            let area = if has_filter { chunks[2] } else { chunks[1] };
            buf.set_string(
                area.x,
                area.y,
                search_string,
                style.add_modifier(Modifier::ITALIC),
            );
        }
    }
}

impl ActiveSection {
    pub fn next(&self) -> ActiveSection {
        match self {
            ActiveSection::ItemList => ActiveSection::FilterBox,
            ActiveSection::FilterBox => ActiveSection::ItemList,
        }
    }
}
