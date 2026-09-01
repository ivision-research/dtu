use clap::{self, Args};
use crossterm::event;
use dtu::{analysis::db::taint::db::GraphTaintAnalysisDb, Context};

use crate::ui::{restore_terminal, setup_terminal, TerminalImpl};

mod applet;
mod command;
mod error_window;
mod method_paths;
mod select_method;
mod window;
use applet::Applet;

pub(super) use command::*;
pub(super) use error_window::*;
pub(super) use method_paths::*;
pub(super) use select_method::*;
pub(super) use window::*;

#[derive(Args)]
pub struct Ui {
    /// Input result file
    #[arg()]
    pub file: String,
}

impl Ui {
    pub fn run(self, ctx: &dyn Context) -> anyhow::Result<()> {
        let db = GraphTaintAnalysisDb::new_from_path(ctx, &self.file)?;
        db.get_validity_err()?;
        let mut term = setup_terminal()?;
        let applet = Applet::new(ctx, &db)?;
        let res = tui_loop(&mut term, applet);
        let restore_res = restore_terminal(&mut term);

        res?;
        restore_res
    }
}

fn tui_loop(term: &mut TerminalImpl, mut applet: Applet) -> anyhow::Result<()> {
    term.draw(|f| applet.draw(f))?;

    loop {
        let needs_redraw = match event::read()? {
            event::Event::Key(evt) => applet.on_key_event(evt),
            event::Event::Mouse(evt) => applet.on_mouse_event(evt),
            event::Event::Resize(_, _) => true,
            _ => false,
        };
        if applet.should_quit() {
            break;
        }
        if needs_redraw {
            term.draw(|f| applet.draw(f))?;
        }
    }
    Ok(())
}
