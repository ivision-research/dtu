use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};
use dtu::db::{ApkIPC, PermissionMode};
use ratatui::layout::{Constraint, Layout};
use ratatui::style::Style;
use ratatui::widgets::{StatefulWidget, Widget};
use std::borrow::Cow;
use std::collections::HashSet;
use std::fmt::Display;
use std::marker::PhantomData;
use std::ops::Deref;
use std::sync::Arc;

use crate::circular::CircularVec;
use dtu::db::device::models::*;

use crate::diff::ui::applet::{FilterBox, FilterBoxFunction};
use crate::ui::widgets::{CheckBox, ClosureWidget, ComboBox, ComboBoxState, BG_COLOR, FG_COLOR};

#[derive(Default, PartialEq, Eq, Clone, Copy, Debug)]
enum Focus {
    #[default]
    Combo,
    NoPerms,
    OnlyNormalPerms,
    Custom(usize),
}

impl Focus {
    fn next(&mut self, custom: usize) {
        *self = match self {
            Focus::Combo => Focus::NoPerms,
            Focus::NoPerms => Focus::OnlyNormalPerms,
            Focus::OnlyNormalPerms => {
                if custom > 0 {
                    Focus::Custom(0)
                } else {
                    Focus::Combo
                }
            }
            Focus::Custom(c) => {
                if custom > 0 {
                    if *c < custom - 1 {
                        Focus::Custom(*c + 1)
                    } else {
                        Focus::Combo
                    }
                } else {
                    Focus::Combo
                }
            }
        }
    }

    fn prev(&mut self, custom: usize) {
        *self = match self {
            Focus::Combo => {
                if custom > 0 {
                    Focus::Custom(custom - 1)
                } else {
                    Focus::OnlyNormalPerms
                }
            }
            Focus::OnlyNormalPerms => Focus::NoPerms,
            Focus::NoPerms => Focus::Combo,
            Focus::Custom(c) => {
                if *c == 0 {
                    Focus::OnlyNormalPerms
                } else {
                    Focus::Custom(*c - 1)
                }
            }
        }
    }
}

struct SelectedApk {
    id: i32,
    display: String,
}

impl From<&Apk> for SelectedApk {
    fn from(value: &Apk) -> Self {
        Self {
            id: value.id,
            display: value.to_string(),
        }
    }
}

impl SelectedApk {
    fn none() -> Self {
        Self {
            id: -1,
            display: String::from("None"),
        }
    }
}

type CustomCheckboxAction<U> = FilterBoxFunction<U>;

pub struct CustomCheckboxFilter<U> {
    text: String,
    enabled: bool,
    action: Arc<CustomCheckboxAction<U>>,
}

impl<U> CustomCheckboxFilter<U> {
    fn toggle_enabled(&mut self) {
        self.enabled = !self.enabled;
    }
}

pub struct ApkIPCFilterBox<U> {
    normal_permissions: HashSet<String>,

    apks: CircularVec<SelectedApk>,

    focus: Focus,

    combo_expanded: bool,
    no_perms: bool,
    only_normal_perms: bool,

    combo_search: String,

    custom_filters: Vec<CustomCheckboxFilter<U>>,

    marker: PhantomData<U>,
}

impl<U, T> ApkIPCFilterBox<U>
where
    T: ApkIPC + Display,
    U: Deref<Target = T>,
{
    pub fn new(apks: Vec<Apk>, normal_permissions: Vec<String>) -> Self {
        let mut selected_apks = Vec::with_capacity(apks.len() + 1);
        selected_apks.push(SelectedApk::none());
        selected_apks.extend(apks.iter().map(|it| SelectedApk::from(it)));
        let mut perm_set = HashSet::new();
        perm_set.extend(normal_permissions);
        Self {
            apks: CircularVec::new(selected_apks),
            normal_permissions: perm_set,
            focus: Focus::default(),
            combo_expanded: false,
            combo_search: String::new(),
            custom_filters: Vec::new(),
            no_perms: false,
            only_normal_perms: false,
            marker: PhantomData::default(),
        }
    }

    pub fn add_custom(&mut self, text: String, action: Arc<CustomCheckboxAction<U>>) {
        self.custom_filters.push(CustomCheckboxFilter {
            text,
            action,
            enabled: false,
        })
    }

    fn handle_control_keys(&mut self, evt: KeyEvent) -> bool {
        if self.focus == Focus::Combo && self.combo_expanded {
            match evt.code {
                KeyCode::Char('j') => {
                    self.apks.inc();
                    return true;
                }
                KeyCode::Char('k') => {
                    self.apks.dec();
                    return true;
                }
                _ => {}
            }
        }
        false
    }

    fn handle_unmodified_key(&mut self, evt: KeyEvent) -> bool {
        match evt.code {
            KeyCode::Backspace if self.focus == Focus::Combo && self.combo_expanded => {
                self.combo_search.pop();
                if self.combo_search.len() > 0 {
                    self.update_combo_search();
                }
            }
            KeyCode::Down => {
                if self.focus == Focus::Combo && self.combo_expanded {
                    self.apks.inc();
                } else {
                    self.focus.next(self.custom_filters.len());
                }
            }
            KeyCode::Up => {
                if self.focus == Focus::Combo && self.combo_expanded {
                    self.apks.dec();
                } else {
                    self.focus.prev(self.custom_filters.len());
                }
            }
            KeyCode::Enter => match self.focus {
                Focus::NoPerms => self.no_perms = !self.no_perms,
                Focus::OnlyNormalPerms => {
                    self.only_normal_perms = !self.only_normal_perms;
                }
                Focus::Combo => {
                    self.combo_search.clear();
                    self.combo_expanded = !self.combo_expanded;
                }
                Focus::Custom(v) => {
                    match self.custom_filters.get_mut(v) {
                        Some(f) => f.toggle_enabled(),
                        // BUG
                        None => {}
                    }
                }
            },
            KeyCode::Char(c) => return self.handle_char_key(c),
            _ => return false,
        }
        true
    }

    fn update_combo_search(&mut self) {
        self.apks
            .goto_first(|it| it.display.starts_with(&self.combo_search));
    }

    fn handle_char_key_combo_expanded(&mut self, c: char) {
        self.combo_search.push(c);
        self.update_combo_search();
    }

    fn handle_char_key(&mut self, c: char) -> bool {
        if self.combo_expanded {
            self.handle_char_key_combo_expanded(c);
            return true;
        }
        match c {
            'j' => self.focus.next(self.custom_filters.len()),
            'k' => self.focus.prev(self.custom_filters.len()),
            _ => return false,
        }
        true
    }
}

impl<U, T> FilterBox<U> for ApkIPCFilterBox<U>
where
    T: ApkIPC + Display,
    for<'a> U: Deref<Target = T> + 'a,
{
    fn on_key_event(&mut self, evt: KeyEvent) -> bool {
        match evt.modifiers {
            KeyModifiers::NONE => self.handle_unmodified_key(evt),
            KeyModifiers::CONTROL => self.handle_control_keys(evt),
            KeyModifiers::SHIFT => match evt.code {
                KeyCode::Char(c) => self.handle_char_key(c),
                _ => false,
            },
            _ => false,
        }
    }

    fn on_mouse_event(&mut self, evt: MouseEvent) -> bool {
        match evt.kind {
            MouseEventKind::ScrollUp => {
                if self.focus == Focus::Combo && self.combo_expanded {
                    self.apks.dec();
                } else {
                    self.focus.prev(self.custom_filters.len());
                }
            }
            MouseEventKind::ScrollDown => {
                if self.focus == Focus::Combo && self.combo_expanded {
                    self.apks.inc();
                } else {
                    self.focus.next(self.custom_filters.len());
                }
            }
            _ => return false,
        }
        true
    }

    fn make_filter(&self) -> Option<Box<FilterBoxFunction<U>>> {
        let apk_id = match self.apks.current() {
            Some(v) => {
                if v.id == -1 {
                    None
                } else {
                    Some(v.id)
                }
            }
            None => None,
        };
        let no_perms = self.no_perms;

        let perms = if self.only_normal_perms {
            Some(self.normal_permissions.clone())
        } else {
            None
        };

        let checks = self
            .custom_filters
            .iter()
            .filter(|it| it.enabled)
            .map(|it| Arc::clone(&it.action))
            .collect::<Vec<Arc<CustomCheckboxAction<U>>>>();

        let check_custom = move |it: &U| -> bool {
            for c in &checks {
                if c(it) {
                    return true;
                }
            }
            false
        };

        Some(Box::new(move |e: &U| -> bool {
            // If we're filtering by APK, only show items belonging to that APK
            if let Some(id) = apk_id {
                if e.get_apk_id() != id {
                    return true;
                }
            }

            // If we want only items that don't require permissions, ensure that
            // here
            if no_perms && e.requires_permission() {
                return true;
            }

            // If we have a list of perms we allow, check
            if let Some(allowed_permissions) = &perms {
                let modes = &[
                    PermissionMode::Generic,
                    PermissionMode::Write,
                    PermissionMode::Read,
                ];

                let has_mode_with_perm = modes.iter().any(|it| {
                    if let Some(p) = e.get_permission_for_mode(*it) {
                        allowed_permissions.contains(p)
                    } else {
                        false
                    }
                });

                if !has_mode_with_perm {
                    return true;
                }
            }

            check_custom(e)
        }))
    }

    fn get_widget(&self) -> Option<ClosureWidget> {
        let items = Cow::Owned(
            self.apks
                .iter()
                .map(|it| it.display.clone())
                .collect::<Vec<String>>(),
        );
        let sel = self.apks.idx();
        let focus = self.focus;
        let combo_expanded = self.combo_expanded;
        let no_perms = self.no_perms;
        let only_normal = self.only_normal_perms;

        let (constraint_count, constraint_percent) = if self.custom_filters.len() == 0 {
            (3, 33u16)
        } else {
            let count = 3 + self.custom_filters.len();
            (count, 100u16 / (count as u16))
        };

        let custom_checkboxes = self
            .custom_filters
            .iter()
            .map(|it| (it.text.clone(), it.enabled))
            .collect::<Vec<(String, bool)>>();

        Some(ClosureWidget::new(Box::new(move |area, buf| {
            let mut combo = ComboBox::new(items)
                .with_selected(sel)
                .with_expanded(combo_expanded)
                .with_style(Style::default().fg(FG_COLOR).bg(BG_COLOR))
                .with_highlight_style(Style::default().fg(BG_COLOR).bg(FG_COLOR));

            let mut constraints = Vec::with_capacity(constraint_count);
            for _c in 0..constraint_count {
                constraints.push(Constraint::Percentage(constraint_percent));
            }

            let chunks = Layout::default().constraints(constraints).split(area);

            let mut state = ComboBoxState::default();
            state.select(Some(sel));

            let focused_style = Style::default().bg(FG_COLOR).fg(BG_COLOR);

            let mut no_perms_box = CheckBox::new("No Permissions").with_checked(no_perms);
            let mut only_normal_box =
                CheckBox::new("Only Normal Permissions").with_checked(only_normal);

            let custom_focused = match focus {
                Focus::NoPerms => {
                    no_perms_box = no_perms_box.with_style(focused_style);
                    -1
                }
                Focus::OnlyNormalPerms => {
                    only_normal_box = only_normal_box.with_style(focused_style);
                    -1
                }
                Focus::Combo => {
                    if !combo_expanded {
                        combo = combo.with_style(focused_style);
                    }
                    -1
                }
                Focus::Custom(v) => v as isize,
            };

            StatefulWidget::render(combo, chunks[0], buf, &mut state);
            no_perms_box.render(chunks[1], buf);
            only_normal_box.render(chunks[2], buf);

            for (i, (text, enabled)) in custom_checkboxes.into_iter().enumerate() {
                let mut cb = CheckBox::new(text).with_checked(enabled);
                if custom_focused != -1 && custom_focused == i as isize {
                    cb = cb.with_style(focused_style);
                }
                cb.render(chunks[i + 3], buf);
            }
        })))
    }
}
