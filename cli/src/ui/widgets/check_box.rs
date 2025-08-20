use std::borrow::Cow;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::widgets::Widget;

pub struct CheckBox<'a> {
    text: Cow<'a, str>,
    checked: bool,
    style: Style,
}

impl<'a> CheckBox<'a> {
    pub fn new<T: Into<Cow<'a, str>>>(text: T) -> Self {
        Self {
            text: text.into(),
            checked: false,
            style: Style::default(),
        }
    }

    pub fn with_style(mut self, style: Style) -> Self {
        self.style = style;
        self
    }

    pub fn with_checked(mut self, checked: bool) -> Self {
        self.checked = checked;
        self
    }

    #[cfg(feature = "unicode")]
    fn draw_checkbox(&self, area: Rect, buf: &mut Buffer) -> u16 {
        const EMPTY: &'static str = "☐";
        const CHECKED: &'static str = "🗹";

        let check_box = if self.checked { CHECKED } else { EMPTY };
        let middle_height = area.top() + ((area.bottom() - area.top()) / 2);

        buf.set_string(area.left(), middle_height, check_box, self.style);

        area.left() + 1
    }

    // This doesn't really look good

    #[cfg(not(feature = "unicode"))]
    fn draw_checkbox(&self, area: Rect, buf: &mut Buffer) -> u16 {
        use crate::ui::widgets::{invert_color, BlockBuilder, BG_COLOR, FG_COLOR, TEXT_STYLE};
        use ratatui::style::Modifier;
        use ratatui::widgets::BorderType;

        let middle_height = area.top() + ((area.bottom() - area.top()) / 2);

        let bg = invert_color(self.style.bg.unwrap_or(BG_COLOR));

        let check_box = BlockBuilder::default()
            .with_border_type(BorderType::Plain)
            .with_style(Style::default().bg(bg))
            .build();

        let checkbox_area = Rect {
            x: area.left(),
            y: middle_height,
            width: 2,
            height: 1,
        };

        let inner = check_box.inner(checkbox_area);
        check_box.render(checkbox_area, buf);

        if self.checked {
            let fg = invert_color(self.style.fg.unwrap_or(FG_COLOR));

            let x = inner.left() + ((inner.right() - inner.left()) / 2);

            buf.set_string(
                x,
                middle_height,
                "x",
                self.style.fg(fg).add_modifier(Modifier::BOLD),
            );
        }

        checkbox_area.right()
    }
}

impl<'a> Widget for CheckBox<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let middle_height = area.top() + ((area.bottom() - area.top()) / 2);
        buf.set_style(area, self.style);
        let checkbox_right = self.draw_checkbox(area, buf);
        buf.set_string(checkbox_right + 1, middle_height, self.text, self.style);
    }
}
