use crate::{
    pull::applet::Applet,
    ui::widgets::list::HIGHLIGHT_SYMBOL,
    ui::widgets::{ACTIVE_COLOR, BG_COLOR, BORDER_TYPE, FG_COLOR},
};
use ratatui::{
    layout::{Constraint, Layout},
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, List, ListItem, ListState},
    Frame,
};

/*
Want the UI to roughly look like:
+----------------------------------------------------------------------------------------+
| Progress | Log |                                                                       |
+--Pull (..)-----------------------------------------------------------------------------+
| X/Y [path ...]                                                                         |
| X/Y [path ...]                                                                         |
| X/Y [path ...]                                                                         |
| X/Y [path ...]                                                                         |
| X/Y [path ...]                                                                         |
+---Decompile (..)-----------------------------------------------------------------------+
| X/Y [path ...]                                                                         |
| X/Y [path ...]                                                                         |
| X/Y [path ...]                                                                         |
| X/Y [path ...]                                                                         |
| X/Y [path ...]                                                                         |
+---------------------------------------------+------------------------------------------+
 */

#[derive(Default, PartialEq)]
pub enum FocusedBlock {
    Pull,
    #[default]
    Decompile,
}

impl FocusedBlock {
    pub fn down(&mut self) {
        *self = match self {
            Self::Pull => Self::Decompile,
            Self::Decompile => Self::Pull,
        }
    }
    pub fn up(&mut self) {
        *self = match self {
            Self::Pull => Self::Decompile,
            Self::Decompile => Self::Pull,
        }
    }
}

pub fn draw(f: &mut Frame, applet: &Applet) {
    let chunks = Layout::default()
        .constraints(vec![
            Constraint::Percentage(40), // Pulls
            Constraint::Percentage(60), // Decompiles
        ])
        .split(f.area());

    let pulls = get_pull_list(&applet);
    let mut pull_list_state = ListState::default();

    let decomp = get_decompile_list(&applet);
    let mut decomp_list_state = ListState::default();

    let sel_idx = Some(applet.sel_idx);

    match applet.focused_block {
        FocusedBlock::Decompile => decomp_list_state.select(sel_idx),
        FocusedBlock::Pull => pull_list_state.select(sel_idx),
    }

    f.render_stateful_widget(pulls, chunks[0], &mut pull_list_state);
    f.render_stateful_widget(decomp, chunks[1], &mut decomp_list_state);
}

fn get_border_color(block: FocusedBlock, applet: &Applet) -> Color {
    if applet.focused_block == block {
        ACTIVE_COLOR
    } else {
        FG_COLOR
    }
}

fn get_pull_list<'a>(applet: &Applet) -> List<'a> {
    List::new(
        applet
            .pull_list
            .iter()
            .rev()
            .map(|ps| Into::<ListItem>::into(ps))
            .collect::<Vec<ListItem>>(),
    )
    .highlight_symbol(HIGHLIGHT_SYMBOL)
    .highlight_style(Style::default().add_modifier(Modifier::BOLD))
    .block(
        Block::default()
            .title(format!(
                "{} Pull Status ({})",
                applet.get_spin_state(),
                applet.pull_list.len()
            ))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(get_border_color(FocusedBlock::Pull, applet)))
            .border_type(BORDER_TYPE)
            .style(Style::default().bg(BG_COLOR)),
    )
}

fn get_decompile_list<'a>(applet: &Applet) -> List<'a> {
    List::new(
        applet
            .decompile_list
            .iter()
            .rev()
            .map(|ps| Into::<ListItem>::into(ps))
            .collect::<Vec<ListItem>>(),
    )
    .highlight_symbol(HIGHLIGHT_SYMBOL)
    .highlight_style(Style::default().add_modifier(Modifier::BOLD))
    .block(
        Block::default()
            .title(format!(
                "{} Decompile Status ({})",
                applet.get_spin_state(),
                applet.decompile_list.len()
            ))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(get_border_color(FocusedBlock::Decompile, applet)))
            .border_type(BORDER_TYPE)
            .style(Style::default().bg(BG_COLOR)),
    )
}
