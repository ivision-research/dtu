use clap::{self, Args};

use crossterm::event;
use dtu::Context;

mod filter_boxes;
mod state;
mod ui;

mod applet;
mod customizer;
mod tabs;

use crate::diff::get_diff_source;
use crate::parsers::DiffSourceValueParser;
use crate::ui::{restore_terminal, setup_terminal, TerminalImpl};
use applet::Applet;
use dtu::db::sql::device::models::DiffSource;
use dtu::db::sql::MetaDatabase;

#[derive(Args)]
pub struct UI {
    /// Set the diff source, defaults to the emulator
    #[arg(short = 'S', long, value_parser = DiffSourceValueParser)]
    diff_source: Option<DiffSource>,
}

impl UI {
    pub fn run(&self, ctx: &dyn Context, meta: &dyn MetaDatabase) -> anyhow::Result<()> {
        let diff_source = get_diff_source(ctx, meta, &self.diff_source)?;
        let mut applet = Applet::new(&ctx, diff_source)?;

        let mut term = setup_terminal()?;

        let res = tui_loop(&mut term, &mut applet);
        let restore_res = restore_terminal(&mut term);

        res?;
        restore_res
    }
}

fn tui_loop(term: &mut TerminalImpl, applet: &mut Applet) -> anyhow::Result<()> {
    term.draw(|f| ui::draw(f, &applet))?;

    loop {
        let needs_redraw = match event::read()? {
            event::Event::Key(evt) => applet.on_key_event(evt),
            event::Event::Mouse(evt) => applet.on_mouse_event(evt),
            _ => false,
        };
        if applet.should_quit() {
            break;
        }
        if needs_redraw {
            term.draw(|f| ui::draw(f, &applet))?;
        }
    }
    Ok(())
}
