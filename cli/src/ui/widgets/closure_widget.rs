use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::widgets::Widget;

use crate::ui::RenderFunc;

/// Allows using a closure as a [Widget]
pub struct ClosureWidget(Box<RenderFunc>);

impl ClosureWidget {
    pub fn new(closure: Box<RenderFunc>) -> Self {
        Self(closure)
    }
}

impl From<Box<RenderFunc>> for ClosureWidget {
    fn from(value: Box<RenderFunc>) -> Self {
        Self::new(value)
    }
}

impl Widget for ClosureWidget {
    fn render(self, area: Rect, buf: &mut Buffer) {
        self.0(area, buf)
    }
}
