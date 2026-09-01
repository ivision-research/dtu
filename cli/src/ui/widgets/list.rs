use crate::ui::widgets::block::BlockBuilder;
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{List, ListItem};

#[cfg(not(feature = "unicode"))]
pub static HIGHLIGHT_SYMBOL: &'static str = ">";
#[cfg(feature = "unicode")]
pub static HIGHLIGHT_SYMBOL: &'static str = "➤";

pub fn new_list<'a, T>(items: T) -> List<'a>
where
    T: IntoIterator,
    T::Item: Into<ListItem<'a>>,
{
    List::new(items)
        .highlight_symbol(HIGHLIGHT_SYMBOL)
        .highlight_style(Style::default().add_modifier(Modifier::BOLD))
        .block(BlockBuilder::default_block())
}
