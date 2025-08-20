use crate::ui::widgets::{BG_COLOR, BORDER_TYPE};
use ratatui::style::Style;
use ratatui::widgets::{block::title::Title, Block, BorderType, Borders};

pub struct BlockBuilder<'a> {
    borders: Borders,
    border_style: Style,
    border_type: BorderType,
    style: Style,
    text: Option<Title<'a>>,
}

impl<'a> Default for BlockBuilder<'a> {
    fn default() -> Self {
        Self {
            borders: Borders::ALL,
            border_style: Style::default(),
            border_type: BORDER_TYPE,
            style: Style::default().bg(BG_COLOR),
            text: None,
        }
    }
}

impl<'a> BlockBuilder<'a> {
    pub fn default_block() -> Block<'a> {
        Self::default().build()
    }

    pub fn with_borders(mut self, val: Borders) -> Self {
        self.borders = val;
        self
    }
    pub fn with_border_style(mut self, val: Style) -> Self {
        self.border_style = val;
        self
    }
    pub fn with_border_type(mut self, val: BorderType) -> Self {
        self.border_type = val;
        self
    }
    pub fn with_style(mut self, val: Style) -> Self {
        self.style = val;
        self
    }
    pub fn with_text<T: Into<Title<'a>>>(mut self, val: T) -> Self {
        self.text = Some(val.into());
        self
    }

    pub fn build(self) -> Block<'a> {
        let block = Block::default()
            .borders(self.borders)
            .border_style(self.border_style)
            .border_type(self.border_type)
            .style(self.style);

        if let Some(txt) = self.text {
            block.title(txt)
        } else {
            block
        }
    }
}
