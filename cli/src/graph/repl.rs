use std::{
    borrow::Cow,
    fs,
    io::{stdin, stdout, Read},
    path::PathBuf,
};

use clap::{self, Args};
use dtu::{
    db::{
        graph::{get_default_graphdb, GraphDatabase},
        sql::{meta::get_default_metadb, MetaDatabase},
    },
    prereqs::Prereq,
    Context, DefaultContext,
};
use rustyline::{
    completion::Completer,
    highlight::Highlighter,
    hint::Hinter,
    history::{FileHistory, History, MemHistory},
    validate::{MatchingBracketValidator, Validator},
    ColorMode, Config, EditMode, Editor, Helper,
};

use crate::printer::no_color_set;

#[derive(Args)]
pub struct Eval {
    #[arg(short, long, help = "Script to run. If this is -, read from stdin")]
    script: String,
}

impl Eval {
    fn get_script<'a>(&'a self) -> anyhow::Result<Cow<'a, str>> {
        if self.script != "-" {
            return Ok(Cow::Borrowed(self.script.as_str()));
        }

        let mut sin = stdin().lock();
        let mut script = String::new();
        sin.read_to_string(&mut script)?;
        Ok(Cow::Owned(script))
    }

    pub fn run(self) -> anyhow::Result<()> {
        let script = self.get_script()?;
        let ctx = DefaultContext::new();
        let graph = get_default_graphdb(&ctx)?;
        let mut out = stdout().lock();
        graph.eval(script.as_ref(), &mut out)?;
        Ok(())
    }
}

#[derive(Args)]
pub struct Repl {
    /// Disable history
    #[arg(short, long)]
    no_history: bool,
}

impl Repl {
    fn get_history_file(&self, ctx: &dyn Context) -> anyhow::Result<PathBuf> {
        Ok(ctx.get_cache_dir()?.join("graphdb_history"))
    }

    fn get_config(&self, ctx: &dyn Context) -> anyhow::Result<Config> {
        let mode = match ctx.get_env("EDITOR") {
            Ok(v) if v.contains("vi") => EditMode::Vi,
            _ => EditMode::Emacs,
        };

        let color_mode = if no_color_set() {
            ColorMode::Disabled
        } else {
            ColorMode::Enabled
        };

        Ok(Config::builder()
            .edit_mode(mode)
            .max_history_size(128)?
            .color_mode(color_mode)
            .build())
    }

    fn on_line(
        &self,
        graph: &dyn GraphDatabase,
        line: String,
        hist: &mut dyn History,
        hist_path: &Option<PathBuf>,
    ) -> anyhow::Result<()> {
        let mut out = stdout();
        if let Err(e) = graph.eval(&line, &mut out) {
            eprintln!("{}", e);
        }

        let added = hist.add_owned(line).unwrap_or(false);

        if added {
            if let Some(path) = hist_path.as_ref() {
                _ = hist.append(path);
            }
        }
        Ok(())
    }

    fn run_repl<H: History>(
        &self,
        graph: &dyn GraphDatabase,
        mut ed: Editor<ReplHelper, H>,
        hist_path: Option<PathBuf>,
    ) -> anyhow::Result<()> {
        loop {
            match ed.readline("graph> ") {
                Ok(line) => {
                    let history = ed.history_mut();
                    self.on_line(graph, line, history, &hist_path)?;
                }
                Err(rustyline::error::ReadlineError::Eof) => break,
                Err(e) => return Err(e.into()),
            }
        }
        Ok(())
    }

    pub fn run(self) -> anyhow::Result<()> {
        let ctx = DefaultContext::new();
        let meta = get_default_metadb(&ctx)?;
        meta.ensure_prereq(Prereq::GraphDatabasePartialSetup)?;
        let graph = get_default_graphdb(&ctx)?;
        let cfg = self.get_config(&ctx)?;

        let (history, hist_path) = if self.no_history {
            (None, None)
        } else {
            let mut hist = FileHistory::with_config(cfg.clone());
            let path = self.get_history_file(&ctx)?;
            if path.exists() {
                if let Err(_) = hist.load(&path) {
                    _ = fs::remove_file(&path);
                }
            }
            (Some(hist), Some(path))
        };

        if let Some(h) = history {
            self.run_repl(&graph, Editor::with_history(cfg, h)?, hist_path)?;
        } else {
            self.run_repl(
                &graph,
                Editor::with_history(cfg, MemHistory::new())?,
                hist_path,
            )?;
        }

        Ok(())
    }
}

struct ReplHelper {
    validator: MatchingBracketValidator,
}

impl Helper for ReplHelper {}

impl Validator for ReplHelper {
    fn validate(
        &self,
        ctx: &mut rustyline::validate::ValidationContext,
    ) -> rustyline::Result<rustyline::validate::ValidationResult> {
        self.validator.validate(ctx)
    }

    fn validate_while_typing(&self) -> bool {
        false
    }
}

impl Highlighter for ReplHelper {}

impl Hinter for ReplHelper {
    type Hint = String;
}

impl Completer for ReplHelper {
    type Candidate = String;
}
