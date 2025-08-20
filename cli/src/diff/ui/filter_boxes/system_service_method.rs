use crate::circular::CircularVec;
use crate::diff::ui::applet::{FilterBox, FilterBoxFunction};
use crate::ui::widgets::{CheckBox, ClosureWidget, ComboBox, ComboBoxState, BG_COLOR, FG_COLOR};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};
use dtu::db::sql::device::models::{DiffedSystemServiceMethod, SystemService};
use ratatui::layout::{Constraint, Layout};
use ratatui::style::Style;
use ratatui::widgets::{StatefulWidget, Widget};
use std::borrow::Cow;

struct SelectedService {
    id: i32,
    display: String,
}

impl From<&SystemService> for SelectedService {
    fn from(value: &SystemService) -> Self {
        Self {
            id: value.id,
            display: value.name.clone(),
        }
    }
}

impl SelectedService {
    fn none() -> Self {
        Self {
            id: -1,
            display: String::from("None"),
        }
    }
}

#[derive(Default, PartialEq, Eq, Clone, Copy, Debug)]
enum Focus {
    #[default]
    Combo,
    OnlyNew,
    OnlyModified,
}

impl Focus {
    fn next(&mut self) {
        *self = match self {
            Focus::Combo => Focus::OnlyNew,
            Focus::OnlyNew => Focus::OnlyModified,
            Focus::OnlyModified => Focus::OnlyNew,
        }
    }

    fn prev(&mut self) {
        *self = match self {
            Focus::Combo => Focus::OnlyModified,
            Focus::OnlyModified => Focus::OnlyNew,
            Focus::OnlyNew => Focus::Combo,
        }
    }
}

pub struct SystemServiceMethodFilterBox {
    services: CircularVec<SelectedService>,

    focus: Focus,
    combo_expanded: bool,
    combo_search: String,
    only_new: bool,
    only_modified: bool,
}

impl SystemServiceMethodFilterBox {
    pub fn new(available_services: Vec<SystemService>) -> Self {
        let mut services = Vec::with_capacity(available_services.len() + 1);
        services.push(SelectedService::none());
        services.extend(
            available_services
                .iter()
                .map(|it| SelectedService::from(it)),
        );
        Self {
            services: CircularVec::new(services),
            focus: Focus::default(),
            combo_search: String::new(),
            combo_expanded: false,
            only_new: false,
            only_modified: false,
        }
    }

    fn handle_control_keys(&mut self, evt: KeyEvent) -> bool {
        if self.focus == Focus::Combo && self.combo_expanded {
            match evt.code {
                KeyCode::Char('j') => {
                    self.services.inc();
                    return true;
                }
                KeyCode::Char('k') => {
                    self.services.dec();
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
                    self.services.inc();
                } else {
                    self.focus.next();
                }
            }
            KeyCode::Up => {
                if self.focus == Focus::Combo && self.combo_expanded {
                    self.services.dec();
                } else {
                    self.focus.prev();
                }
            }
            KeyCode::Enter => match self.focus {
                Focus::Combo => {
                    self.combo_expanded = !self.combo_expanded;
                }
                Focus::OnlyNew => self.only_new = !self.only_new,
                Focus::OnlyModified => self.only_modified = !self.only_modified,
            },
            KeyCode::Char(c) => return self.handle_char_key(c),
            _ => return false,
        }
        true
    }

    fn update_combo_search(&mut self) {
        self.services
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
            'j' => self.focus.next(),
            'k' => self.focus.prev(),
            _ => return false,
        }
        true
    }
}

impl FilterBox<DiffedSystemServiceMethod> for SystemServiceMethodFilterBox {
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
                    self.services.dec();
                } else {
                    self.focus.prev();
                }
            }
            MouseEventKind::ScrollDown => {
                if self.focus == Focus::Combo && self.combo_expanded {
                    self.services.inc();
                } else {
                    self.focus.next();
                }
            }
            _ => return false,
        }
        true
    }

    fn make_filter(&self) -> Option<Box<FilterBoxFunction<DiffedSystemServiceMethod>>> {
        let service_id = match self.services.current() {
            Some(v) => {
                if v.id == -1 {
                    None
                } else {
                    Some(v.id)
                }
            }
            None => None,
        };

        let only_new = self.only_new;
        let only_modified = self.only_modified;

        Some(Box::new(move |it: &DiffedSystemServiceMethod| -> bool {
            if let Some(id) = service_id {
                if it.system_service_id != id {
                    return true;
                }
            }

            if only_new && it.exists_in_diff {
                return true;
            }

            if only_modified {
                if !it.exists_in_diff {
                    return true;
                } else if it.hash_matches_diff.is_true() {
                    return true;
                }
            }

            false
        }))
    }

    fn get_widget(&self) -> Option<ClosureWidget> {
        let items = Cow::Owned(
            self.services
                .iter()
                .map(|it| it.display.clone())
                .collect::<Vec<String>>(),
        );
        let sel = self.services.idx();
        let focus = self.focus;
        let combo_expanded = self.combo_expanded;
        let only_modified = self.only_modified;
        let only_new = self.only_new;

        Some(ClosureWidget::new(Box::new(move |area, buf| {
            let mut combo = ComboBox::new(items)
                .with_selected(sel)
                .with_expanded(combo_expanded)
                .with_style(Style::default().fg(FG_COLOR).bg(BG_COLOR))
                .with_highlight_style(Style::default().fg(BG_COLOR).bg(FG_COLOR));
            let chunks = Layout::default()
                .constraints(vec![
                    Constraint::Percentage(33),
                    Constraint::Percentage(33),
                    Constraint::Percentage(33),
                ])
                .split(area);

            let mut state = ComboBoxState::default();
            state.select(Some(sel));

            let focused_style = Style::default().bg(FG_COLOR).fg(BG_COLOR);

            let mut only_new_box = CheckBox::new("Only new").with_checked(only_new);
            let mut only_modified_box = CheckBox::new("Only modified").with_checked(only_modified);

            match focus {
                Focus::OnlyNew => only_new_box = only_new_box.with_style(focused_style),
                Focus::OnlyModified => {
                    only_modified_box = only_modified_box.with_style(focused_style)
                }
                Focus::Combo if !combo_expanded => combo = combo.with_style(focused_style),
                _ => {}
            }

            StatefulWidget::render(combo, chunks[0], buf, &mut state);
            only_new_box.render(chunks[1], buf);
            only_modified_box.render(chunks[2], buf);
        })))
    }
}
