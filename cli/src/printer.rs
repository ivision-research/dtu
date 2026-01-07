#![allow(dead_code)]

use crossterm::cursor::{Hide, MoveTo, MoveToNextLine, RestorePosition, SavePosition, Show};
use crossterm::tty::IsTty;
use crossterm::{cursor, queue, terminal};
use std::cell::RefCell;
use std::env::{self, VarError};
use std::fmt::Display;
use std::io::{stdout, Stdout, Write};
use std::sync::atomic::{AtomicBool, AtomicU16, Ordering};

use crossterm::style::{
    Attribute, Color, ContentStyle, Print, PrintStyledContent, StyledContent, Stylize,
};
use crossterm::terminal::{Clear, ClearType, ScrollUp};

type StatusLine = PrintStyledContent<String>;

/// A StatusPrinter sets up the UI to also print a status line at the bottom
///
/// Note that the printer doesn't use a mutex on stdout!
pub struct StatusPrinter {
    is_tty: bool,
    silent: bool,
    style_enabled: AtomicBool,
    status_line: RefCell<Option<StatusLine>>,
    num_status_lines: AtomicU16,
}

impl Drop for StatusPrinter {
    fn drop(&mut self) {
        self.restore();
    }
}

pub(crate) fn no_color_set() -> bool {
    match env::var("NO_COLOR") {
        Err(VarError::NotPresent) => false,
        _ => true,
    }
}

#[cfg(feature = "unicode")]
static LINE_CHARACTER: &'static str = "⎯";

#[cfg(not(feature = "unicode"))]
static LINE_CHARACTER: &'static str = "=";

impl StatusPrinter {
    pub fn new() -> Self {
        let is_tty = stdout().is_tty();
        let style_enabled = !no_color_set();

        if is_tty && style_enabled {
            with_stdout(|s| {
                _ = queue!(s, Hide, SavePosition);
            });
        }

        Self {
            is_tty,
            style_enabled: AtomicBool::new(style_enabled),
            silent: false,
            status_line: RefCell::new(None),
            num_status_lines: AtomicU16::new(0),
        }
    }

    pub fn clear_below(&self) {
        if self.silent {
            return;
        }
        with_stdout(|s| {
            _ = queue!(s, Clear(ClearType::FromCursorDown));
            _ = s.flush();
        });
    }

    pub fn advance_line(&self) {
        if self.silent {
            return;
        }
        with_stdout(|s| {
            _ = queue!(s, MoveToNextLine(1));
            _ = s.flush();
        });
    }

    fn restore(&self) {
        if self.is_tty && self.style_enabled.load(Ordering::Relaxed) {
            with_stdout(|s| {
                _ = queue!(s, Show, RestorePosition);
            });
        }
        self.flush();
    }

    pub fn set_silent(&mut self, silent: bool) {
        self.silent = silent;
    }

    /// Returns (current_col, current_row, last_row)
    fn get_location_info(&self) -> Option<(u16, u16, u16)> {
        let (_, last) = terminal::size().ok()?;
        let (cur_col, cur_row) = cursor::position().ok()?;

        Some((cur_col, cur_row, last))
    }

    fn get_num_lines(&self, s: &str) -> Option<u16> {
        let (cols, _) = terminal::size().ok()?;
        let cols = usize::try_from(cols).ok()?;

        let mut len = s.len();
        let mut num_lines = 1;

        while len > cols {
            num_lines += 1;
            len -= cols;
        }

        Some(num_lines)
    }

    fn can_fit_line(&self, num_lines: u16) -> Option<bool> {
        let (cur_col, mut cur_row, last) = self.get_location_info()?;
        if cur_col != 0 {
            cur_row += 1;
        }
        Some(
            last.checked_sub(cur_row)
                .map(|it| it >= num_lines)
                .unwrap_or(false),
        )
    }

    pub fn print_divider(&self) {
        if self.silent {
            return;
        }
        let cols = match terminal::size() {
            Ok((cols, _)) => cols,
            Err(_) => {
                // Do something I guess :)
                self.println("-----");
                return;
            }
        };
        let mut line = LINE_CHARACTER.repeat(cols as usize);
        line.push('\n');
        let mut style = ContentStyle::default();
        style.attributes.set(Attribute::Bold);
        self.do_styled_print(line, style, false)
    }

    pub fn println(&self, content: impl Display) {
        self.println_styled(content, ContentStyle::default())
    }

    pub fn print(&self, content: impl Display) {
        self.print_styled(content, ContentStyle::default())
    }

    pub fn println_colored(&self, content: impl Display, color: Color) {
        let style = ContentStyle::default().with(color);
        self.println_styled(content, style)
    }

    pub fn print_colored(&self, content: impl Display, color: Color) {
        let style = ContentStyle::default().with(color);
        self.print_styled(content, style)
    }

    pub fn println_styled(&self, content: impl Display, style: ContentStyle) {
        self.do_styled_print(content, style, true)
    }

    pub fn print_styled(&self, content: impl Display, style: ContentStyle) {
        self.do_styled_print(content, style, false)
    }

    fn should_style(&self) -> bool {
        self.is_tty && self.style_enabled.load(Ordering::Relaxed)
    }

    fn do_styled_print(&self, content: impl Display, style: ContentStyle, with_nl: bool) {
        if self.silent {
            return;
        }

        let should_style = self.should_style();
        if should_style {
            self.ensure_status_line();
        }
        with_stdout(|s| {
            if should_style {
                _ = queue!(s, PrintStyledContent(StyledContent::new(style, content)));
            } else {
                _ = queue!(s, Print(content));
            }
            if with_nl {
                _ = queue!(s, Print("\n"));
            }
        });
    }

    fn get_last_row(&self) -> Option<u16> {
        // terminal::size returns the top left as (1, 1), but pretty much everything else
        // wants top left to be (0, 0). Just account for that here.
        Some(terminal::size().ok()?.1 - 1)
    }

    fn scroll_and_print_status_line(&self, num_lines: u16) {
        let _status_line = self.status_line.borrow();
        let status_line = match _status_line.as_ref() {
            None => return,
            Some(v) => v,
        };

        let last_row = match self.get_last_row() {
            None => return,
            Some(v) => v,
        };

        let (cur_col, mut cur_row) = match cursor::position() {
            Err(_) => return,
            Ok((col, row)) => (col, row),
        };

        let remaining_lines = last_row - cur_row;
        let write_at = last_row - num_lines + 1;

        with_stdout(|s| {
            if remaining_lines < num_lines {
                _ = queue!(s, ScrollUp(num_lines));
                cur_row = cur_row.checked_sub(num_lines).unwrap_or(0);
            }

            _ = queue!(
                s,
                MoveTo(0, write_at),
                status_line,
                MoveTo(cur_col, cur_row)
            );
            _ = s.flush();
        });
    }

    fn ensure_status_line(&self) {
        let num_lines = self.num_status_lines.load(Ordering::Relaxed);

        if num_lines == 0 || self.can_fit_line(num_lines).unwrap_or(true) {
            return;
        }
        with_stdout(|s| {
            _ = queue!(s, Clear(ClearType::FromCursorDown));
        });
        self.scroll_and_print_status_line(num_lines);
    }

    pub fn flush(&self) {
        if self.silent {
            return;
        }

        with_stdout(|s| {
            _ = s.flush();
        })
    }

    pub fn update_status_line_styled(&self, content: impl Display, style: ContentStyle) {
        if !self.should_style() || self.silent {
            return;
        }

        // Just clear everything after where we are, it's easier and we'll hit the bottom
        // of the terminal pretty quickly anyway.
        with_stdout(|s| {
            _ = queue!(s, Clear(ClearType::FromCursorDown));
        });

        let as_string = content.to_string();

        let nls = self.get_num_lines(&as_string).unwrap_or(0);
        self.num_status_lines.store(nls, Ordering::Relaxed);
        let psc = PrintStyledContent(StyledContent::new(style, as_string));
        _ = self.status_line.borrow_mut().insert(psc.clone());

        self.scroll_and_print_status_line(nls);
    }

    pub fn update_status_line(&self, content: impl Display) {
        self.update_status_line_styled(content, ContentStyle::default())
    }
}

/// A Printer allows for printing styled content to stdout
///
/// Note that the printer doesn't use a mutex on stdout!
pub struct Printer {
    is_tty: bool,
    style_enabled: bool,
}

impl Drop for Printer {
    fn drop(&mut self) {
        // Ensure we've flushed everything
        self.flush()
    }
}

pub mod color {
    use crossterm::style::Color;

    pub const fn to_tui_rgb(color: Color) -> ratatui::style::Color {
        match color {
            Color::Rgb { r, g, b } => ratatui::style::Color::Rgb(r, g, b),
            _ => panic!("unreachable"),
        }
    }

    // Colorscheme credit to Paul Tol https://personal.sron.nl/~pault/#sec:qualitative

    pub const YELLOW: Color = Color::Rgb {
        r: 0xCC,
        g: 0xBB,
        b: 0x44,
    };
    pub const CYAN: Color = Color::Rgb {
        r: 0x66,
        g: 0xCC,
        b: 0xEE,
    };
    pub const RED: Color = Color::Rgb {
        r: 0xEE,
        g: 0x66,
        b: 0x77,
    };
    pub const GREEN: Color = Color::Rgb {
        r: 0x22,
        g: 0x88,
        b: 0x33,
    };
    pub const PURPLE: Color = Color::Rgb {
        r: 0xAA,
        g: 0x33,
        b: 0x77,
    };
    pub const GREY: Color = Color::Rgb {
        r: 0xBB,
        g: 0xBB,
        b: 0xBB,
    };

    pub const INTERESTING: Color = YELLOW;
    #[allow(dead_code)]
    pub const ERROR: Color = RED;
    #[allow(dead_code)]
    pub const OK: Color = GREEN;
}

impl Printer {
    pub fn new() -> Self {
        let is_tty = stdout().is_tty();
        let style_enabled = !no_color_set();
        Self {
            is_tty,
            style_enabled,
        }
    }

    pub fn println(&self, content: impl Display) {
        self.println_styled(content, ContentStyle::default())
    }

    pub fn print(&self, content: impl Display) {
        self.print_styled(content, ContentStyle::default())
    }

    pub fn println_colored(&self, content: impl Display, color: Color) {
        let style = ContentStyle::default().with(color);
        self.println_styled(content, style)
    }

    pub fn print_colored(&self, content: impl Display, color: Color) {
        let style = ContentStyle::default().with(color);
        self.print_styled(content, style)
    }

    pub fn println_styled(&self, content: impl Display, style: ContentStyle) {
        self.do_styled_print(content, style, true)
    }

    pub fn print_styled(&self, content: impl Display, style: ContentStyle) {
        self.do_styled_print(content, style, false)
    }

    fn should_style(&self) -> bool {
        self.is_tty && self.style_enabled
    }

    fn do_styled_print(&self, content: impl Display, style: ContentStyle, with_nl: bool) {
        let should_style = self.should_style();
        with_stdout(|s| {
            if should_style {
                _ = queue!(s, PrintStyledContent(StyledContent::new(style, content)));
            } else {
                _ = queue!(s, Print(content));
            }
            if with_nl {
                _ = queue!(s, Print("\n"));
            }
        });
    }

    pub fn flush(&self) {
        with_stdout(|s| {
            _ = s.flush();
        })
    }
}

#[inline]
fn with_stdout<F: FnOnce(&mut Stdout)>(func: F) {
    let mut stdout = stdout();
    func(&mut stdout);
}
