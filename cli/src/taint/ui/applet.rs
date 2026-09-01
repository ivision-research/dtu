use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent};
use dtu::analysis::db::taint::db::GraphTaintAnalysisDb;
use dtu::Context;
use ratatui::Frame;

use crate::taint::ui::{ErrorWindow, SelectMethodWindow, Tools, Window, WindowAction};

pub struct Applet<'a> {
    tools: Tools<'a>,
    windows: Vec<Box<dyn Window>>,
    should_quit: bool,
}

impl<'a> Applet<'a> {
    pub fn new(ctx: &'a dyn Context, db: &'a GraphTaintAnalysisDb) -> anyhow::Result<Self> {
        let windows: Vec<Box<dyn Window>> = vec![Box::new(SelectMethodWindow::new(&db)?)];
        Ok(Self {
            tools: Tools { ctx, db },
            should_quit: false,
            windows,
        })
    }

    fn with_active_window<R>(&mut self, f: impl FnOnce(&mut dyn Window, &Tools) -> R) -> R {
        let tools = &self.tools;
        let window = self
            .windows
            .last_mut()
            .expect("should never pop all windows")
            .as_mut();

        f(window, tools)
    }
    pub fn draw(&mut self, frame: &mut Frame) {
        self.with_active_window(|window, tools| window.draw(tools, frame))
    }

    fn on_quit(&mut self) {
        self.should_quit = true;
    }

    pub fn on_key_event(&mut self, evt: KeyEvent) -> bool {
        match evt.modifiers {
            KeyModifiers::CONTROL if matches!(evt.code, KeyCode::Char('c')) => self.on_quit(),
            _ => {}
        }

        let res = self.with_active_window(|window, tools| window.on_key_event(tools, evt));

        self.on_event_result(res)
    }

    fn on_event_result(&mut self, res: anyhow::Result<WindowAction>) -> bool {
        let action = match res {
            Ok(a) => a,
            Err(e) => {
                self.windows.push(Box::new(ErrorWindow::new(e.to_string())));
                return true;
            }
        };

        match action {
            WindowAction::Nothing => false,
            WindowAction::Redraw => true,
            WindowAction::PushWindow(window) => {
                self.windows.push(window);
                true
            }
            WindowAction::PopWindow if self.windows.len() > 1 => {
                self.windows.pop();
                true
            }
            // This should never happen...
            WindowAction::PopWindow => {
                self.windows.push(Box::new(
                    SelectMethodWindow::new(self.tools.db)
                        .expect("everything has gone horribly wrong"),
                ));
                true
            }
            WindowAction::ReplaceWindow(window) => {
                let idx = self.windows.len() - 1;
                self.windows[idx] = window;
                true
            }
        }
    }

    pub fn on_mouse_event(&mut self, evt: MouseEvent) -> bool {
        let res = self.with_active_window(|window, tools| window.on_mouse_event(tools, evt));
        self.on_event_result(res)
    }

    pub fn should_quit(&self) -> bool {
        self.should_quit
    }
}
