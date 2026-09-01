use std::ops::Deref;

use crossterm::event::{KeyEvent, MouseEvent};
use dtu::{analysis::db::taint::db::GraphTaintAnalysisDb, Context};
use ratatui::Frame;

#[derive(Default)]
pub enum WindowAction {
    /// Do nothing after the event
    #[default]
    Nothing,
    /// Redraw the UI after the event
    Redraw,
    /// Pop the current window off the stack and redraaw
    PopWindow,
    /// Push a new window onto the stack and redrawn
    PushWindow(Box<dyn Window>),
    #[allow(unused)]
    /// Replace the current window
    ReplaceWindow(Box<dyn Window>),
}

pub struct Tools<'a> {
    #[allow(unused)]
    pub ctx: &'a dyn Context,
    pub db: &'a GraphTaintAnalysisDb,
}

impl<'a> Deref for Tools<'a> {
    type Target = GraphTaintAnalysisDb;
    fn deref(&self) -> &Self::Target {
        &self.db
    }
}

pub trait Window {
    fn on_key_event(&mut self, _tools: &Tools, _evt: KeyEvent) -> anyhow::Result<WindowAction> {
        Ok(WindowAction::default())
    }
    fn on_mouse_event(&mut self, _tools: &Tools, _evt: MouseEvent) -> anyhow::Result<WindowAction> {
        Ok(WindowAction::default())
    }
    fn draw(&self, tools: &Tools, frame: &mut Frame);
}
