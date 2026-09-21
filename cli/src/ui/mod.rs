pub mod widgets;

/// Shorten `text` to `width` columns, dropping from the middle so that both the package a name
/// starts with and the method it ends with survive
pub fn fit(text: &str, width: usize) -> String {
    let len = text.chars().count();
    if len <= width {
        return text.to_string();
    }
    // Below this there is no room for an ellipsis plus a character either side of it
    if width < 3 {
        return text.chars().take(width).collect();
    }

    let keep = width - 1;
    // The tail gets the odd character: a smali display ends with the method name
    let head = keep / 2;
    let tail = keep - head;
    let mut out: String = text.chars().take(head).collect();
    out.push('\u{2026}');
    out.extend(text.chars().skip(len - tail));
    out
}

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

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn fit_keeps_both_ends_of_a_long_name() {
        assert_eq!(fit("short", 10), "short");
        assert_eq!(fit("exact", 5), "exact");
        assert_eq!(fit("abcdefghij", 5), "ab\u{2026}ij");
        assert_eq!(fit("abcdefghij", 4), "a\u{2026}ij");
        // No room for an ellipsis with a character either side, so just cut
        assert_eq!(fit("abcdefghij", 2), "ab");
        assert_eq!(fit("abcdefghij", 0), "");
        assert_eq!(fit("", 0), "");
    }

    #[test]
    fn fit_never_exceeds_the_width() {
        let name = "Lcom/example/really/long/package/Class;->method(Ljava/lang/String;)V";
        for width in 0..name.chars().count() + 4 {
            assert!(fit(name, width).chars().count() <= width.max(0));
        }
    }
}
