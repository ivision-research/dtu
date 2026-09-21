use std::collections::HashMap;

use anyhow::bail;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use itertools::Itertools;
use ratatui::{
    layout::{Constraint, Layout},
    style::{Style, Stylize},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::taint::ui::{config::Config, Tools, WindowAction};

pub type CommandFunc<T> = fn(Vec<String>, &Tools, &mut Config, &mut T) -> anyhow::Result<()>;

pub struct Command<T> {
    pub help: Option<&'static str>,
    pub long_help: Option<&'static str>,
    pub func: CommandFunc<T>,
}

impl<T> Clone for Command<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for Command<T> {}

enum Help {
    Inactive,
    All,
    Command(Option<&'static str>),
}

pub struct CommandHandler<T> {
    help: Help,
    keys_help: &'static str,
    command: Option<String>,
    commands: HashMap<&'static str, Command<T>>,
}

impl<T> CommandHandler<T> {
    /// `keys_help` is what `keys` and `help keys` display. Each window binds its own keys, so
    /// the handler cannot know them.
    pub fn new(commands: HashMap<&'static str, Command<T>>, keys_help: &'static str) -> Self {
        Self {
            help: Help::Inactive,
            command: None,
            commands,
            keys_help,
        }
    }

    /// Show the key list, as `keys` and `help keys` do
    pub fn show_keys(&mut self) {
        self.help = Help::Command(Some(self.keys_help));
    }

    pub fn activate(&mut self) {
        self.command = Some(String::new());
    }

    #[allow(unused)]
    pub fn deactivate(&mut self) {
        self.command = None;
        self.help = Help::Inactive;
    }

    pub fn is_active(&self) -> bool {
        !matches!(self.help, Help::Inactive) || self.command.is_some()
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
        cfg: &mut Config,
        state: &mut T,
    ) -> anyhow::Result<WindowAction> {
        match evt.modifiers {
            KeyModifiers::SHIFT => match evt.code {
                KeyCode::Char(c) => self.push(c),
                _ => return Ok(WindowAction::default()),
            },
            KeyModifiers::NONE => match evt.code {
                KeyCode::Esc | KeyCode::Enter if !matches!(self.help, Help::Inactive) => {
                    self.help = Help::Inactive
                }
                KeyCode::Esc => self.clear(),
                KeyCode::Backspace => self.delete(),
                KeyCode::Char(c) => self.push(c),
                KeyCode::Enter => {
                    return self.submit(tools, cfg, state).and(Ok(WindowAction::Redraw))
                }
                _ => return Ok(WindowAction::default()),
            },
            _ => return Ok(WindowAction::default()),
        }

        Ok(WindowAction::Redraw)
    }

    fn draw_help(&self, frame: &mut Frame) {
        let layout = Layout::horizontal(&[
            Constraint::Length(1),
            Constraint::Fill(1),
            Constraint::Length(1),
        ]);

        let [_, center, _] = frame.area().layout(&layout);

        let layout = Layout::vertical(&[
            Constraint::Length(1),
            Constraint::Fill(1),
            Constraint::Length(1),
        ]);

        let [_, block_area, _] = center.layout(&layout);

        let txt = match &self.help {
            Help::Inactive => Text::raw("BUG! unreachable state for help"),
            Help::All => {
                let mut txt = Text::default();
                txt.lines.push(Line::from(vec![
                    Span::styled("keys", Style::new().bold()),
                    Span::raw(": every bound key"),
                ]));
                for (cmdname, cmd) in &self.commands {
                    let Some(shorthelp) = cmd.help else {
                        txt.lines.push(Line::styled(*cmdname, Style::new().bold()));
                        continue;
                    };
                    let mut line = Line::default();
                    line.spans.push(Span::styled(*cmdname, Style::new().bold()));
                    line.spans.push(Span::raw(": "));
                    line.spans.push(Span::raw(shorthelp));
                    txt.lines.push(line);
                }
                txt
            }
            Help::Command(Some(helptxt)) => Text::raw(*helptxt),
            Help::Command(None) => Text::raw("no help text available for command"),
        };

        let widget = Paragraph::new(txt)
            .left_aligned()
            .block(Block::default().borders(Borders::ALL).title("help"));

        frame.render_widget(widget, block_area);
    }

    pub fn draw(&self, frame: &mut Frame) -> bool {
        if !matches!(self.help, Help::Inactive) {
            self.draw_help(frame);
            return true;
        }

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

        if !self.commands.keys().any(|it| it.starts_with(command_name))
            && !"help".starts_with(command_name)
            && !"keys".starts_with(command_name)
        {
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

    fn on_help(&mut self, name: Option<&str>) {
        let Some(name) = name else {
            self.help = Help::All;
            return;
        };

        if name == "keys" {
            self.help = Help::Command(Some(self.keys_help));
            return;
        }

        if let Some(cmd) = self.commands.get(name) {
            let display = match cmd.long_help {
                Some(v) => Some(v),
                None => cmd.help,
            };

            self.help = Help::Command(display);
            return;
        }

        let mut byprefix = None;

        for (cmdname, cmd) in &self.commands {
            if cmdname.starts_with(name) {
                if byprefix.is_some() {
                    self.help = Help::All;
                    return;
                }

                let display = match cmd.long_help {
                    Some(v) => Some(v),
                    None => cmd.help,
                };

                byprefix = Some(display);
            }
        }

        if let Some(helptxt) = byprefix {
            self.help = Help::Command(helptxt);
        } else {
            self.help = Help::All;
        }
    }

    fn on_command_submitted(
        &mut self,
        state: &mut T,
        tools: &Tools,
        cfg: &mut Config,
        command: String,
    ) -> anyhow::Result<()> {
        log::debug!("Handling command: `{command}`");

        let (cmdstr, args) = match command.split_once(' ') {
            Some((cmd, rem)) => (cmd, Some(rem)),
            None => (command.as_str(), None),
        };

        if cmdstr == "help" {
            self.on_help(args);
            return Ok(());
        }

        if cmdstr == "keys" {
            self.help = Help::Command(Some(self.keys_help));
            return Ok(());
        }

        let func: &CommandFunc<T> = match self.commands.get(cmdstr) {
            Some(cmd) => &cmd.func,
            None => {
                let prefixed = self
                    .commands
                    .iter()
                    .filter(|(k, _)| k.starts_with(cmdstr))
                    .collect::<Vec<_>>();
                if prefixed.len() > 1 {
                    bail!(
                        "ambiguous command, matches {}",
                        prefixed.iter().map(|(k, _)| k).join(", ")
                    );
                }
                let Some((_, cmd)) = prefixed.first() else {
                    bail!("no command matches: {cmdstr}");
                };
                &cmd.func
            }
        };

        let argv = match args {
            None => Vec::new(),
            Some(v) => Self::split_args(v),
        };

        func(argv, tools, cfg, state)
    }

    fn submit(&mut self, tools: &Tools, cfg: &mut Config, state: &mut T) -> anyhow::Result<()> {
        if let Some(cmd) = self.command.take() {
            return self.on_command_submitted(state, tools, cfg, cmd);
        }
        Ok(())
    }
}
