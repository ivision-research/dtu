use std::{cmp::Ordering, time::Duration};

use crate::{
    progress::{TickSpinner, FAIL_MARKER, SUCCESS_MARKER},
    pull::{ui::FocusedBlock, Pull},
    ui::widgets::{ACTIVE_COLOR, ERR_COLOR, OK_COLOR},
};
use crossbeam::channel::Receiver;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};
use dtu::tasks::pull::Event;
use dtu::utils::DevicePath;
use ratatui::{style::Style, text::Span, text::Text, widgets::ListItem};

#[derive(PartialEq)]
enum QuitAction {
    None,
    WaitQuit,
    ImmediateQuit,
}

pub struct Applet {
    evt_rx: Receiver<Event>,
    quit: QuitAction,

    spinner: TickSpinner<'static>,
    spin_state: &'static str,
    pub is_done: bool,
    pub pull_list: Vec<PullDecompileAction>,
    pub decompile_list: Vec<PullDecompileAction>,
    pub focused_block: FocusedBlock,
    pub sel_idx: usize,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum PullDecompileStatus {
    InProgress,
    Failed,
    Succeeded,
}

#[derive(Clone)]
pub struct PullDecompileAction {
    spin: &'static str,
    item: String,
    status: PullDecompileStatus,
}

impl PullDecompileAction {
    fn new(spin: &'static str, item: String, status: PullDecompileStatus) -> Self {
        Self { spin, item, status }
    }
}

impl<'a> Into<ListItem<'a>> for &PullDecompileAction {
    fn into(self) -> ListItem<'a> {
        let (marker, color) = match self.status {
            PullDecompileStatus::InProgress => (self.spin, ACTIVE_COLOR),
            PullDecompileStatus::Failed => (FAIL_MARKER, ERR_COLOR),
            PullDecompileStatus::Succeeded => (SUCCESS_MARKER, OK_COLOR),
        };
        let mut text = Text::from(Span::raw(marker));
        text.push_span(Span::styled(self.item.clone(), Style::default().fg(color)));
        ListItem::new(text)
    }
}

impl Applet {
    pub fn new(_pull: &Pull, evt_rx: Receiver<Event>) -> Self {
        let mut spinner = TickSpinner::new_rand(Duration::from_millis(200));
        let spin_state = spinner.get();
        Self {
            evt_rx,
            spinner,
            spin_state,
            quit: QuitAction::None,
            is_done: false,
            pull_list: Vec::new(),
            decompile_list: Vec::new(),
            focused_block: FocusedBlock::default(),
            sel_idx: 0,
        }
    }

    pub fn update_spinners(&mut self, spin_state: &'static str) {
        for ps in &mut self.pull_list {
            if let PullDecompileStatus::InProgress = ps.status {
                ps.spin = spin_state;
            }
        }

        for d in &mut self.decompile_list {
            if let PullDecompileStatus::InProgress = d.status {
                d.spin = spin_state;
            }
        }
    }

    pub fn get_spin_state(&self) -> &'static str {
        if self.is_done {
            SUCCESS_MARKER
        } else {
            self.spin_state
        }
    }

    pub fn on_tick(&mut self) {
        let spin_state = self.spinner.get();
        self.spin_state = spin_state;
        self.update_spinners(spin_state);
        loop {
            match self.evt_rx.try_recv() {
                Err(_) => break,
                Ok(evt) => self.on_event(spin_state, evt),
            }
        }
    }

    fn track_selection(&mut self, focused: FocusedBlock) {
        if self.sel_idx > 0 && self.focused_block == focused {
            self.sel_idx += 1;
        }
    }

    fn on_event(&mut self, spin_state: &'static str, evt: Event) {
        match evt {
            Event::FrameworkStarted => {}
            Event::FrameworkEnded => {}
            Event::ApksStarted => {}
            Event::ApksEnded => {}
            Event::FindingDirectories => {}
            Event::DirectoryFound { .. } => {}
            Event::DirectoryDone { .. } => {}
            Event::Pulling {
                device,
                local: _local,
            } => {
                let act = PullDecompileAction::new(
                    spin_state,
                    device.get_device_string(),
                    PullDecompileStatus::InProgress,
                );
                self.pull_list.push(act);
                self.track_selection(FocusedBlock::Pull);
            }
            Event::PullSuccess { device } => {
                self.update_pull_list(device, PullDecompileStatus::Succeeded);
            }
            Event::PullFailed { device } => {
                self.update_pull_list(device, PullDecompileStatus::Failed);
            }
            Event::Decompiling { local } => {
                let device_path = match DevicePath::from_path(&local) {
                    Ok(p) => p.get_device_string(),
                    _ => return,
                };
                self.decompile_list.push(PullDecompileAction::new(
                    spin_state,
                    device_path,
                    PullDecompileStatus::InProgress,
                ));
                self.track_selection(FocusedBlock::Decompile);
            }
            Event::DecompileFailed { local } => {
                let device_path = match DevicePath::from_path(&local) {
                    Ok(p) => p.get_device_string(),
                    _ => return,
                };
                self.update_decompile_list(device_path, PullDecompileStatus::Failed);
            }
            Event::DecompileSuccess { local } => {
                let device_path = match DevicePath::from_path(&local) {
                    Ok(p) => p.get_device_string(),
                    _ => return,
                };
                self.update_decompile_list(device_path, PullDecompileStatus::Succeeded);
            }
        }
    }

    fn update_list(
        lst: &mut Vec<PullDecompileAction>,
        path: String,
        new_state: PullDecompileStatus,
    ) -> bool {
        let ret = if let Some(ps) = lst.iter_mut().find(|ps| ps.item == path) {
            ps.status = new_state;
            false
        } else {
            lst.push(PullDecompileAction::new("", path, new_state));
            true
        };
        // Define a sort such that `InProgress` items are always at the
        // end of the list
        lst.sort_by(|lhs, rhs| {
            if rhs.status == PullDecompileStatus::InProgress {
                if lhs.status == PullDecompileStatus::InProgress {
                    Ordering::Equal
                } else {
                    Ordering::Less
                }
            } else if lhs.status == PullDecompileStatus::InProgress {
                Ordering::Greater
            } else {
                Ordering::Equal
            }
        });
        ret
    }

    fn update_decompile_list(&mut self, path: String, new_state: PullDecompileStatus) {
        if Self::update_list(&mut self.decompile_list, path, new_state) {
            self.track_selection(FocusedBlock::Decompile);
        }
    }

    fn update_pull_list(&mut self, path: DevicePath, new_state: PullDecompileStatus) {
        let path = path.get_device_string();
        if Self::update_list(&mut self.pull_list, path, new_state) {
            self.track_selection(FocusedBlock::Pull);
        }
    }
    pub fn handle_mouse_event(&mut self, evt: MouseEvent) {
        match evt.kind {
            MouseEventKind::ScrollUp => self.dec_selection(),
            MouseEventKind::ScrollDown => self.inc_selection(),
            _ => {}
        }
    }

    pub fn handle_key_event(&mut self, key: KeyEvent) {
        match key.modifiers {
            KeyModifiers::CONTROL => match key.code {
                KeyCode::Char('u') => self.sel_idx = 0,
                KeyCode::Char('c') => self.quit = QuitAction::ImmediateQuit,
                _ => {}
            },
            KeyModifiers::SHIFT => match key.code {
                KeyCode::Char('J') => {
                    self.focused_block.down();
                    self.sel_idx = 0;
                }
                KeyCode::Char('K') => {
                    self.focused_block.up();
                    self.sel_idx = 0;
                }
                _ => {}
            },
            KeyModifiers::NONE => match key.code {
                KeyCode::Tab => {}
                KeyCode::Esc => {
                    if self.quit == QuitAction::None {
                        self.quit = QuitAction::WaitQuit
                    }
                }
                KeyCode::Char('0') => self.sel_idx = 0,
                KeyCode::Char('j') => self.inc_selection(),
                KeyCode::Char('k') => self.dec_selection(),
                _ => {}
            },
            _ => {}
        }
    }

    fn inc_selection(&mut self) {
        let max_idx = match self.focused_block {
            FocusedBlock::Pull => self.pull_list.len(),
            FocusedBlock::Decompile => self.decompile_list.len(),
        } - 1;
        if self.sel_idx >= max_idx {
            return;
        }
        self.sel_idx += 1;
    }

    fn dec_selection(&mut self) {
        if self.sel_idx == 0 {
            return;
        }
        self.sel_idx -= 1;
    }

    pub fn should_quit(&self) -> bool {
        self.quit != QuitAction::None
    }

    pub fn should_quit_immediately(&self) -> bool {
        self.quit == QuitAction::ImmediateQuit
    }
}
