pub mod widgets;

use std::io::{self, Stdout};

use anyhow;

use crossterm::{
    event::{DisableMouseCapture, EnableMouseCapture},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::{backend::CrosstermBackend, Terminal};

pub type TerminalImpl = Terminal<CrosstermBackend<Stdout>>;

pub fn setup_terminal() -> anyhow::Result<TerminalImpl> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture).map_err(|e| {
        let _ = disable_raw_mode();
        e
    })?;

    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic| {
        panic_restore();
        original_hook(panic);
    }));

    let backend = CrosstermBackend::new(stdout);
    Ok(Terminal::new(backend)?)
}

pub fn panic_restore() {
    let stdout = io::stdout();
    let mut term = Terminal::new(CrosstermBackend::new(stdout)).unwrap();
    restore_terminal(&mut term).unwrap()
}

pub fn restore_terminal(terminal: &mut TerminalImpl) -> anyhow::Result<()> {
    let mut res: anyhow::Result<()> = Ok(());

    if let Err(e) = disable_raw_mode() {
        res = Err(anyhow::anyhow!("{}", e));
    }

    if let Err(e) = execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    ) {
        if res.is_ok() {
            res = Err(anyhow::anyhow!("{}", e));
        }
    }
    if let Err(e) = terminal.show_cursor() {
        if res.is_ok() {
            res = Err(anyhow::anyhow!("{}", e));
        }
    }
    res
}

pub type RenderFunc = dyn FnOnce(Rect, &mut Buffer);
