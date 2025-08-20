use ratatui::style::{Modifier, Style};
use ratatui::{style::Color, widgets::BorderType};

use crate::printer::color::{self, to_tui_rgb};

pub mod block;
pub use block::BlockBuilder;
pub mod closure_widget;
pub mod list;
pub use closure_widget::ClosureWidget;
pub mod combo_box;
pub use combo_box::*;
pub mod check_box;
pub use check_box::*;

pub const BG_COLOR: Color = Color::Black;
pub const FG_COLOR: Color = Color::Rgb(0xFF, 0xFF, 0xFF);
pub const ERR_COLOR: Color = to_tui_rgb(color::ERROR);
pub const OK_COLOR: Color = to_tui_rgb(color::OK);
pub const ACTIVE_COLOR: Color = to_tui_rgb(color::CYAN);
pub const INACTIVE_COLOR: Color = to_tui_rgb(color::GREY);
pub const ATTENTION_COLOR: Color = to_tui_rgb(color::YELLOW);
pub const INTERESTING_COLOR: Color = to_tui_rgb(color::INTERESTING);
pub const PURPLE: Color = to_tui_rgb(color::PURPLE);
pub const GREY: Color = to_tui_rgb(color::GREY);
pub const BORDER_TYPE: BorderType = BorderType::Rounded;

pub const TEXT_STYLE: Style = Style {
    fg: Some(FG_COLOR),
    bg: None,
    underline_color: None,
    add_modifier: Modifier::empty(),
    sub_modifier: Modifier::empty(),
};

/// Invert a color if it is RGB. Currently can't handle non RGB colors.
pub const fn invert_color(color: Color) -> Color {
    if let Color::Rgb(r, g, b) = color {
        Color::Rgb(0xFF - r, 0xFF - g, 0xFF - b)
    } else {
        color
    }
}
