use std::collections::HashSet;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use dtu::utils::ensure_dir_exists;
use dtu::Context;

/// State just contains some of the ui state between runs.
#[derive(Serialize, Deserialize, Default)]
pub struct State {
    pub hidden_system_services: HashSet<i32>,
    pub hidden_system_service_methods: HashSet<i32>,
    pub hidden_apks: HashSet<i32>,
    pub hidden_providers: HashSet<i32>,
    pub hidden_activities: HashSet<i32>,
    pub hidden_receivers: HashSet<i32>,
    pub hidden_services: HashSet<i32>,
}

impl State {
    /// Load the state from the default location provided by the [Context]
    pub fn load(ctx: &dyn Context) -> anyhow::Result<Self> {
        let path = get_state_file(ctx)?;
        if !path.exists() {
            return Ok(Self::default());
        }
        let content = fs::read_to_string(&path)?;
        Ok(serde_json::from_str(&content)?)
    }

    /// Store the state in a default location provided by the [Context]
    pub fn store(&self, ctx: &dyn Context) -> anyhow::Result<()> {
        let path = get_state_file(ctx)?;
        let mut file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&path)?;
        let json = serde_json::to_string(self)?;
        file.write_all(json.as_bytes())?;
        Ok(())
    }
}

fn get_state_file(ctx: &dyn Context) -> anyhow::Result<PathBuf> {
    let dir = ctx.get_output_dir_child(".diff")?;
    ensure_dir_exists(&dir)?;
    Ok(dir.join("state.json"))
}
