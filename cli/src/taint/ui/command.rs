use std::collections::HashMap;

use anyhow::bail;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use itertools::Itertools;
use ratatui::{
    layout::{Constraint, Layout},
    style::Stylize,
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::taint::ui::{Tools, WindowAction};

pub type CommandFunc<T> = fn(Vec<String>, &Tools, &mut T) -> anyhow::Result<()>;

pub struct Command<T> {
    command: Option<String>,
    commands: HashMap<&'static str, CommandFunc<T>>,
}

impl<T> Command<T> {
    pub fn new(commands: HashMap<&'static str, CommandFunc<T>>) -> Self {
        Self {
            command: None,
            commands,
        }
    }

    pub fn activate(&mut self) {
        self.command = Some(String::new());
    }

    #[allow(unused)]
    pub fn deactivate(&mut self) {
        self.command = None;
    }

    pub fn is_active(&self) -> bool {
        self.command.is_some()
    }

    fn push(&mut self, c: char) {
        if let Some(cmd) = &mut self.command {
            cmd.push(c);
        }
    }

    fn clear(&mut self) {
        self.command = None;
    }

    fn delete(&mut self) {
        let Some(cmd) = &mut self.command else {
            return;
        };

        if cmd.len() == 0 {
            return;
        }
        cmd.truncate(cmd.len() - 1);
    }

    pub fn on_key_event(
        &mut self,
        evt: KeyEvent,
        tools: &Tools,
        state: &mut T,
    ) -> anyhow::Result<WindowAction> {
        match evt.modifiers {
            KeyModifiers::SHIFT => match evt.code {
                KeyCode::Char(c) => self.push(c),
                _ => return Ok(WindowAction::default()),
            },
            KeyModifiers::NONE => match evt.code {
                KeyCode::Esc => self.clear(),
                KeyCode::Backspace => self.delete(),
                KeyCode::Char(c) => self.push(c),
                KeyCode::Enter => return self.submit(tools, state).and(Ok(WindowAction::Redraw)),
                _ => return Ok(WindowAction::default()),
            },
            _ => return Ok(WindowAction::default()),
        }

        Ok(WindowAction::Redraw)
    }

    pub fn draw(&self, frame: &mut Frame) -> bool {
        let Some(cmd) = &self.command else {
            return false;
        };

        let layout = Layout::horizontal(&[
            Constraint::Length(1),
            Constraint::Fill(1),
            Constraint::Length(1),
        ]);

        let [_, center, _] = frame.area().layout(&layout);

        let layout = Layout::vertical(&[
            Constraint::Fill(1),
            Constraint::Max(10),
            Constraint::Fill(1),
        ]);

        let [_, block_area, _] = center.layout(&layout);

        let mut cmd_box = Paragraph::new(cmd.as_str())
            .left_aligned()
            .block(Block::default().borders(Borders::ALL).title("command"));

        let command_name = match cmd.split_once(' ') {
            Some((v, _)) => v,
            None => cmd,
        };

        if !self.commands.keys().any(|it| it.starts_with(command_name)) {
            cmd_box = cmd_box.red();
        }

        frame.render_widget(cmd_box, block_area);

        true
    }

    fn split_args(args: &str) -> Vec<String> {
        let mut argv = Vec::new();
        let mut cur_arg = String::new();
        let mut in_dquote = false;
        let mut in_squote = false;
        let mut escaped = false;
        let mut chars = args.chars();
        while let Some(c) = chars.next() {
            // Escaping
            if escaped {
                escaped = false;
                cur_arg.push(c);
                continue;
            }
            // Escaping
            if c == '\\' {
                escaped = true;
                continue;
            }

            // Quoting
            if c == '\'' {
                if in_squote {
                    argv.push(cur_arg.clone());
                    cur_arg.clear();
                    continue;
                } else if !in_dquote {
                    in_squote = true;
                    continue;
                }
            } else if c == '"' {
                if in_dquote {
                    argv.push(cur_arg.clone());
                    cur_arg.clear();
                    continue;
                } else if !in_squote {
                    in_dquote = true;
                    continue;
                }
            }
            // Whitespace splitting
            if c.is_whitespace() {
                argv.push(cur_arg.clone());
                cur_arg.clear();
                continue;
            }

            // Push everything else
            cur_arg.push(c);
        }

        if cur_arg.len() > 0 {
            argv.push(cur_arg);
        }

        argv
    }

    fn on_command_submitted(
        &mut self,
        state: &mut T,
        tools: &Tools,
        command: String,
    ) -> anyhow::Result<()> {
        log::debug!("Handling command: `{command}`");
        let (cmd, args) = match command.split_once(' ') {
            Some((cmd, rem)) => (cmd, Some(rem)),
            None => (command.as_str(), None),
        };

        let func = match self.commands.get(cmd) {
            Some(func) => *func,
            None => {
                let prefixed = self
                    .commands
                    .iter()
                    .filter(|(k, _)| k.starts_with(cmd))
                    .collect::<Vec<_>>();
                if prefixed.len() > 1 {
                    bail!(
                        "ambiguous command, matches {}",
                        prefixed.iter().map(|(k, _)| k).join(", ")
                    );
                }
                let Some((_, func)) = prefixed.first() else {
                    bail!("no command matches: {cmd}");
                };
                **func
            }
        };

        let argv = match args {
            None => Vec::new(),
            Some(v) => Self::split_args(v),
        };

        func(argv, tools, state)
    }

    fn submit(&mut self, tools: &Tools, state: &mut T) -> anyhow::Result<()> {
        if let Some(cmd) = self.command.take() {
            return self.on_command_submitted(state, tools, cmd);
        }
        Ok(())
    }
}
