use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    layout::{Alignment, Constraint, Layout},
    style::Stylize,
    text::Line,
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame,
};

use crate::taint::ui::{Tools, Window, WindowAction};

pub struct ErrorWindow {
    msg: String,
}

impl ErrorWindow {
    pub fn new(msg: String) -> Self {
        Self { msg }
    }
}

impl Window for ErrorWindow {
    fn draw(&self, _tools: &Tools, frame: &mut Frame) {
        let layout = Layout::vertical(&[
            Constraint::Length(1),
            Constraint::Fill(1),
            Constraint::Length(1),
        ]);
        let [title_area, body_area, esc_area] = frame.area().layout(&layout);
        let title = Line::from("Error").centered().bold().red();
        let esc = Line::from("Esc to go back").centered();
        let body = Paragraph::new(self.msg.as_str())
            .wrap(Wrap::default())
            .alignment(Alignment::Center)
            .block(Block::default().borders(Borders::ALL));
        frame.render_widget(title, title_area);
        frame.render_widget(body, body_area);
        frame.render_widget(esc, esc_area);
    }

    fn on_key_event(&mut self, _tools: &Tools, evt: KeyEvent) -> anyhow::Result<WindowAction> {
        if matches!(evt.modifiers, KeyModifiers::NONE) && matches!(evt.code, KeyCode::Esc) {
            return Ok(WindowAction::PopWindow);
        }
        Ok(WindowAction::default())
    }
}
