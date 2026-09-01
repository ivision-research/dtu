use anyhow::{bail, Context as AnyhowContext};
use std::{
    io::Write,
    path::PathBuf,
    process::{Command, Stdio},
};

use clap::{self, Args};
use dtu::{db::graph::db::GRAPH_DATABASE_FILE_NAME, utils::path_must_str, Context};

#[derive(Args)]
pub struct Sqlite {
    /// Input result file
    #[arg()]
    pub file: String,

    #[arg(last = true)]
    pub sqlite_args: Vec<String>,
}

impl Sqlite {
    pub fn run(mut self, ctx: &dyn Context) -> anyhow::Result<()> {
        let path = ctx.get_env("PATH")?;
        let mut sqlite_bin = None;

        for dir in path.split(':').map(PathBuf::from) {
            let potential = dir.join("sqlite3");
            if potential.exists() {
                sqlite_bin = Some(String::from(path_must_str(&potential)));
                break;
            }
        }

        let Some(sqlite_bin) = sqlite_bin else {
            bail!("no sqlite3 on PATH");
        };

        let graph_path = ctx.get_sqlite_dir()?.join(GRAPH_DATABASE_FILE_NAME);
        let graph_path_str = path_must_str(&graph_path);

        let commands = vec![
            format!("ATTACH DATABASE '{graph_path_str}' AS graph"),
            "PRAGMA graph.query_only = true".into(),
        ];

        log::debug!("Adding SQL statements to init file:");
        let mut tf = tempfile::NamedTempFile::new()?;
        for cmd in commands {
            log::debug!("{cmd};");
            tf.write_all(cmd.as_bytes())
                .with_context(|| "writing command to tempfile")?;
            tf.write_all(&[b';', b'\n'])
                .with_context(|| "writing command to tempfile")?;
        }

        let init_file = path_must_str(tf.path());

        log::debug!("Running with -init {init_file}");

        self.sqlite_args.reserve(3);
        self.sqlite_args.push("-init".into());
        self.sqlite_args.push(init_file.into());
        self.sqlite_args.push(self.file.clone());

        let status = Command::new(&sqlite_bin)
            .args(&self.sqlite_args)
            .stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .status()?;

        drop(tf);

        if let Some(code) = status.code() {
            std::process::exit(code);
        }

        Ok(())
    }
}
