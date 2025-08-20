use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::widgets::{List, ListItem, ListState, StatefulWidget, Widget};
use std::borrow::Cow;
use std::fmt::Display;
use std::ops::{Deref, DerefMut};

pub struct ComboBoxItem<'a>(Cow<'a, str>);

impl<'a, T> From<T> for ComboBoxItem<'a>
where
    T: Display,
{
    fn from(value: T) -> Self {
        Self(Cow::Owned(value.to_string()))
    }
}

impl<'a> ComboBoxItem<'a> {
    pub fn new<T: Into<Cow<'a, str>>>(it: T) -> Self {
        Self(it.into())
    }
}

/// A ComboBox is similar to the HTML equivalent. When it is expanded, it
/// displays a dropdown of items. When it is not expanded, it just displays
/// a text box with the currently selected item.
pub struct ComboBox<'a, T>
where
    [T]: ToOwned,
    for<'b> &'b T: Into<ComboBoxItem<'a>>,
{
    items: Cow<'a, [T]>,
    title: Option<Cow<'a, str>>,
    selected: usize,
    is_expanded: bool,
    style: Style,
    highlight_style: Style,
}

impl<'a, T> ComboBox<'a, T>
where
    [T]: ToOwned,
    for<'b> &'b T: Into<ComboBoxItem<'a>>,
{
    pub fn new(items: Cow<'a, [T]>) -> Self {
        Self {
            items: items.into(),
            is_expanded: false,
            title: None,
            selected: 0,
            style: Style::default(),
            highlight_style: Style::default(),
        }
    }

    pub fn with_style(mut self, style: Style) -> Self {
        self.style = style;
        self
    }

    pub fn with_highlight_style(mut self, style: Style) -> Self {
        self.highlight_style = style;
        self
    }

    pub fn with_title<C: Into<Cow<'a, str>>>(mut self, title: C) -> Self {
        self.title = Some(title.into());
        self
    }

    pub fn with_expanded(mut self, expanded: bool) -> Self {
        self.is_expanded = expanded;
        self
    }

    pub fn with_selected(mut self, selected: usize) -> Self {
        self.selected = selected;
        self
    }

    fn render_expanded(
        self,
        area: Rect,
        buf: &mut Buffer,
        state: &mut <Self as StatefulWidget>::State,
    ) {
        let items = self
            .items
            .iter()
            .map(|it| {
                let combo_box_item = it.into();
                ListItem::new(combo_box_item.0)
            })
            .collect::<Vec<ListItem>>();

        let list = List::new(items)
            .style(self.style)
            .highlight_style(self.highlight_style);
        StatefulWidget::render(list, area, buf, state.as_mut())
    }

    fn render_unexpanded(self, area: Rect, buf: &mut Buffer) {
        let text = match self.items.get(self.selected) {
            None => Cow::Borrowed("- None -"),
            Some(it) => Into::<ComboBoxItem<'a>>::into(it).0,
        };
        buf.set_string(area.x, area.y, text.as_ref(), self.style)
    }
}

pub struct ComboBoxState(ListState);

impl Deref for ComboBoxState {
    type Target = ListState;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for ComboBoxState {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl AsRef<ListState> for ComboBoxState {
    fn as_ref(&self) -> &ListState {
        &self.0
    }
}

impl AsMut<ListState> for ComboBoxState {
    fn as_mut(&mut self) -> &mut ListState {
        &mut self.0
    }
}

impl Default for ComboBoxState {
    fn default() -> Self {
        Self(ListState::default())
    }
}

impl<'a, T> StatefulWidget for ComboBox<'a, T>
where
    [T]: ToOwned,
    for<'b> &'b T: Into<ComboBoxItem<'a>>,
{
    type State = ComboBoxState;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut Self::State) {
        if self.is_expanded {
            self.render_expanded(area, buf, state)
        } else {
            self.render_unexpanded(area, buf)
        }
    }
}

impl<'a, T> Widget for ComboBox<'a, T>
where
    [T]: ToOwned,
    for<'b> &'b T: Into<ComboBoxItem<'a>>,
{
    fn render(self, area: Rect, buf: &mut Buffer) {
        let mut state = ComboBoxState::default();
        StatefulWidget::render(self, area, buf, &mut state)
    }
}
