use std::collections::HashSet;
use std::fmt::Display;

use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::ListItem;
use regex::Regex;

use crate::diff::ui::applet::FilterBox;
use crate::diff::ui::customizer::Customizer;
use crate::diff::ui::ui::ActiveTab;
use crate::ui::widgets::{ClosureWidget, GREY};
use crossterm::event::{KeyEvent, MouseEvent};
use dtu::db::Idable;
use dtu::Context;

pub trait Tab {
    fn inc_selection(&mut self);
    fn dec_selection(&mut self);
    fn select_start(&mut self);
    fn select_end(&mut self);
    fn toggle_selected_hidden(&mut self);
    fn set_show_hidden(&mut self, show_hidden: bool);

    fn get_type(&self) -> ActiveTab;
    fn get_selected_idx(&self) -> usize;
    fn get_hidden_set(&self) -> HashSet<i32>;
    fn get_item_count(&self) -> usize;
    fn get_list_items(&self) -> Vec<ListItem<'_>>;
    fn open_selection(&self, ctx: &dyn Context) -> anyhow::Result<()>;
    fn clipboard_selection(&self, ctx: &dyn Context) -> anyhow::Result<()>;
    fn clipboard_logcat_selection(&self, ctx: &dyn Context) -> anyhow::Result<()>;

    fn on_filter_box_key_event(&mut self, evt: KeyEvent) -> bool;
    fn on_filter_box_mouse_event(&mut self, evt: MouseEvent) -> bool;
    fn get_filter_box(&self) -> Option<ClosureWidget>;

    fn has_filter_box(&self) -> bool {
        self.get_filter_box().is_some()
    }

    fn get_info_popup(&self) -> Option<ClosureWidget>;

    fn set_search_string(&mut self, search: Option<String>);
}

pub struct TabContainer<E>
where
    E: Display + Idable,
{
    sel: usize,
    selected_id: i32,
    items: Vec<E>,
    hidden_set: HashSet<i32>,
    tab_type: ActiveTab,
    show_hidden: bool,

    filter_box_filter: Option<Box<dyn Fn(&E) -> bool>>,

    pub filter_box: Option<Box<dyn FilterBox<E>>>,
    pub customizer: Option<Box<dyn Customizer<E>>>,

    // Extra filter added on top
    pub filter: Option<Box<dyn Fn(&E) -> bool>>,
}

impl<E> TabContainer<E>
where
    E: Display + Idable,
{
    pub fn new(
        items: Vec<E>,
        hidden_list: HashSet<i32>,
        tab_type: ActiveTab,
        show_hidden: bool,
    ) -> Self {
        Self {
            items,
            hidden_set: hidden_list,
            tab_type,
            sel: 0,
            selected_id: 0,
            show_hidden,
            customizer: None,
            filter_box: None,
            filter_box_filter: None,
            filter: None,
        }
    }

    pub fn set_filter_box(&mut self, filter_box: Option<Box<dyn FilterBox<E>>>) {
        self.filter_box_filter = match filter_box.as_ref() {
            None => None,
            Some(v) => v.make_filter(),
        };
        self.filter_box = filter_box;
    }

    fn ignore_item(&self, item: &E) -> bool {
        if self.filtered(item) {
            return true;
        }
        if self.show_hidden {
            false
        } else {
            self.hidden_set.contains(&item.get_id())
        }
    }

    fn ignore_idx(&self, idx: usize) -> bool {
        let item = self.items.get(idx);
        match item {
            Some(it) => self.ignore_item(it),
            None => true,
        }
    }

    fn filtered(&self, item: &E) -> bool {
        let filtered = self.filter.as_ref().map(|it| it(item)).unwrap_or(false);
        if filtered {
            return true;
        }

        self.customizer
            .as_ref()
            .map(|f| f.filter(item))
            .unwrap_or(false)
            || self
                .filter_box_filter
                .as_ref()
                .map(|f| f(item))
                .unwrap_or(false)
    }

    fn update_selected_id(&mut self) {
        self.selected_id = self.items.get(self.sel).map(|it| it.get_id()).unwrap_or(0);
    }
}

impl<E> Tab for TabContainer<E>
where
    E: Display + Idable,
{
    fn inc_selection(&mut self) {
        let count = self.get_item_count();
        if count == 0 {
            return;
        }

        let max = self.items.len() - 1;

        loop {
            let sel = self.sel.wrapping_add(1);
            self.sel = if sel > max { 0 } else { sel };
            if !self.ignore_idx(self.sel) {
                self.update_selected_id();
                return;
            }
        }
    }

    fn dec_selection(&mut self) {
        let count = self.get_item_count();
        if count == 0 {
            return;
        }
        loop {
            self.sel = self.sel.checked_sub(1).unwrap_or(self.items.len() - 1);
            if !self.ignore_idx(self.sel) {
                self.update_selected_id();
                return;
            }
        }
    }

    fn select_start(&mut self) {
        self.sel = 0;

        let max = self.items.len() - 1;
        loop {
            if !self.ignore_idx(self.sel) {
                self.update_selected_id();
                return;
            }

            let sel = self.sel.wrapping_add(1);
            self.sel = if sel > max { 0 } else { sel };
        }
    }

    fn select_end(&mut self) {
        let max = self.items.len() - 1;
        self.sel = max;

        loop {
            if !self.ignore_idx(self.sel) {
                self.update_selected_id();
                return;
            }

            self.sel.checked_sub(1).unwrap_or(max);
        }
    }

    fn toggle_selected_hidden(&mut self) {
        let id = self.items.get(self.sel).map(|it| it.get_id()).unwrap_or(-1);
        if id == -1 {
            log::trace!("no id for hiding");
            return;
        }

        if self.hidden_set.contains(&id) {
            log::trace!("unhiding item {}", id);
            self.hidden_set.remove(&id);
        } else {
            log::trace!("hiding item {}", id);
            self.hidden_set.insert(id);
        }

        if !self.show_hidden {
            self.inc_selection();
        }
    }

    fn set_show_hidden(&mut self, show_hidden: bool) {
        self.show_hidden = show_hidden;
    }

    fn get_type(&self) -> ActiveTab {
        self.tab_type
    }

    fn get_selected_idx(&self) -> usize {
        let visible = self.items.iter().filter(|it| !self.ignore_item(*it));

        let mut sel = 0;

        for v in visible {
            if v.get_id() == self.selected_id {
                return sel;
            }
            sel += 1;
        }
        return 0;
    }

    fn get_hidden_set(&self) -> HashSet<i32> {
        self.hidden_set.clone()
    }

    fn get_item_count(&self) -> usize {
        self.items
            .iter()
            .filter(|it| !self.ignore_item(*it))
            .count()
    }

    fn get_list_items(&self) -> Vec<ListItem<'_>> {
        let list_items = self
            .items
            .iter()
            .filter(|it| !self.ignore_item(*it))
            .map(|it| {
                let (content, mut style) = if let Some(customizer) = &self.customizer {
                    (
                        customizer.display(it),
                        customizer
                            .style(it)
                            .unwrap_or_else(|| Style::default().fg(Color::Cyan)),
                    )
                } else {
                    (it.to_string(), Style::default().fg(Color::Cyan))
                };
                let list_item = ListItem::new(content);

                if self.hidden_set.contains(&it.get_id()) {
                    log::trace!("{} is hidden", it);
                    style = Style::default().fg(GREY).add_modifier(Modifier::ITALIC)
                };
                list_item.style(style)
            });
        list_items.collect()
    }

    fn on_filter_box_key_event(&mut self, evt: KeyEvent) -> bool {
        let filter_box = match self.filter_box.as_mut() {
            None => return false,
            Some(v) => v,
        };
        let handled = filter_box.on_key_event(evt);
        if handled {
            self.filter_box_filter = filter_box.make_filter();
        }
        handled
    }

    fn on_filter_box_mouse_event(&mut self, evt: MouseEvent) -> bool {
        let filter_box = match self.filter_box.as_mut() {
            None => return false,
            Some(v) => v,
        };
        let handled = filter_box.on_mouse_event(evt);
        if handled {
            self.filter_box_filter = filter_box.make_filter();
        }
        handled
    }

    fn get_filter_box(&self) -> Option<ClosureWidget> {
        match &self.filter_box {
            None => None,
            Some(v) => v.get_widget(),
        }
    }

    fn clipboard_selection(&self, ctx: &dyn Context) -> anyhow::Result<()> {
        match &self.customizer {
            None => anyhow::bail!("tab doesn't support clipboard"),
            Some(it) => match self.items.get(self.sel) {
                Some(item) => it.clipboard_selection(ctx, item),
                None => anyhow::bail!("invalid selection"),
            },
        }
    }

    fn clipboard_logcat_selection(&self, ctx: &dyn Context) -> anyhow::Result<()> {
        match &self.customizer {
            None => anyhow::bail!("tab doesn't support clipboard"),
            Some(it) => match self.items.get(self.sel) {
                Some(item) => it.clipboard_logcat_selection(ctx, item),
                None => anyhow::bail!("invalid selection"),
            },
        }
    }

    fn open_selection(&self, ctx: &dyn Context) -> anyhow::Result<()> {
        match &self.customizer {
            None => anyhow::bail!("tab doesn't support opening"),
            Some(it) => match self.items.get(self.sel) {
                Some(item) => it.open_selection(ctx, item),
                None => anyhow::bail!("invalid selection"),
            },
        }
    }

    fn get_info_popup(&self) -> Option<ClosureWidget> {
        match &self.customizer {
            None => None,
            Some(it) => {
                let item = self.items.get(self.sel)?;
                it.get_popup(item)
            }
        }
    }

    fn set_search_string(&mut self, search: Option<String>) {
        match search {
            None => self.filter = None,
            Some(v) if v.len() == 0 => self.filter = None,
            Some(v) => {
                let regex = match Regex::new(&v) {
                    Err(_) => return,
                    Ok(v) => v,
                };
                self.filter = Some(Box::new(move |it: &E| -> bool {
                    let as_string = it.to_string();
                    if regex.is_match(&as_string) {
                        return false;
                    }
                    !as_string.contains(&v)
                }));
            }
        }
    }
}
