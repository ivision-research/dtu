use crate::consts::NEO4J_CONTAINER_NAME;
use crate::parsers::HeapSizeValueParser;
use crate::utils::ensure_neo4j;
use anyhow::bail;
use clap::Args;
use dtu::utils::{ensure_dir_exists, path_must_str};
use dtu::{run_cmd, Context, DefaultContext};
use std::fs;
use std::path::PathBuf;

#[cfg(not(windows))]
use libc;

#[derive(Args)]
pub struct StartNeo4j {
    /// Invoke the `docker` commands with `sudo`
    #[arg(long, action = clap::ArgAction::SetTrue, default_value_t = false)]
    sudo: bool,

    /// Set the heap size allocated to the graph container
    ///
    /// Defaults to 8G. Raise or lower the amount of heap space that is allocated
    /// to the Neo4j container.
    #[arg(
        short='H',
        long,
        action = clap::ArgAction::Set,
        default_value = "8G",
        value_parser = HeapSizeValueParser,
    )]
    heap_size: String,
}

const NEO4J_DOCKER_TAG: &'static str = "5.18.1-community";
const APOC_FILE: &'static str = "apoc-5.18.0-core.jar";

#[cfg(target_os = "macos")]
macro_rules! add_user_flag {
    ($args:ident) => {};
}

#[cfg(not(target_os = "macos"))]
macro_rules! add_user_flag {
    ($args:ident) => {
        let euid = unsafe { libc::geteuid() };
        let egid = unsafe { libc::getegid() };
        let u_flag = format!("{}:{}", euid, egid);
        $args.extend(&["-u", &u_flag]);
    };
}

#[cfg(not(target_os = "linux"))]
macro_rules! add_selinux_flags {
    ($args:ident) => {};
}

#[cfg(target_os = "linux")]
macro_rules! add_selinux_flags {
    ($args:ident) => {
        let mut sec_opt = selinux::SecurityOpt::Disabled;
        if (selinux::is_enabled()) {
            selinux::get_security_opt(&mut sec_opt);
            match &sec_opt {
                selinux::SecurityOpt::Disabled => {
                    $args.extend(&["--security-opt", "label=disable"]);
                }

                selinux::SecurityOpt::Pieces {
                    user,
                    role,
                    type_,
                    level,
                } => {
                    $args.extend(&["--security-opt", user]);
                    $args.extend(&["--security-opt", role]);
                    $args.extend(&["--security-opt", type_]);
                    $args.extend(&["--security-opt", level]);
                }
            }
        }
    };
}

#[cfg(target_os = "linux")]
mod selinux {

    use dtu::run_cmd;
    use std::fs;
    use std::io::Read;

    pub enum SecurityOpt {
        Pieces {
            user: String,
            role: String,
            type_: String,
            level: String,
        },
        Disabled,
    }

    fn get_label_from_id_cmd(into: &mut SecurityOpt) -> bool {
        let out = match run_cmd("id", &["-Z"]) {
            Ok(out) => {
                if out.ok() {
                    out
                } else {
                    return false;
                }
            }
            Err(_) => return false,
        };

        let label = out.stdout_utf8_lossy();
        let trimmed = label.trim_end();

        let mut iter = trimmed.splitn(4, ':');

        macro_rules! get_next {
            ($it:ident) => {
                match $it.next() {
                    None => {
                        return false;
                    }
                    Some(v) => v,
                }
            };
        }

        *into = SecurityOpt::Pieces {
            user: format!("label=user:{}", get_next!(iter)),
            role: format!("label=role:{}", get_next!(iter)),
            type_: format!("label=type:{}", get_next!(iter)),
            level: format!("label=level:{}", get_next!(iter)),
        };

        true
    }

    pub(crate) fn get_security_opt(into: &mut SecurityOpt) {
        if get_label_from_id_cmd(into) {
            return;
        }
        *into = SecurityOpt::Disabled;
    }

    pub(crate) fn is_enabled() -> bool {
        // Determine if SELinux is enabled:
        //      1. /sys/fs/selinux/enforce must enxist
        //      2. /sys/fs/selinux/enforce must return `1` when read

        let mut file = match fs::File::open("/sys/fs/selinux/enforce") {
            Err(_) => return false,
            Ok(f) => f,
        };

        let into = &mut [0u8];

        match file.read(into) {
            Err(_) => return false,
            Ok(_) => {}
        }

        let content = into[0];

        content == b'1'
    }
}

impl StartNeo4j {
    #[cfg(windows)]
    pub fn run(&self) -> anyhow::Result<()> {
        bail!("why are you running windows?");
    }

    #[cfg(not(windows))]
    pub fn run(&self) -> anyhow::Result<()> {
        let ctx = DefaultContext::new();

        if let Ok(()) = ensure_neo4j(&ctx, self.sudo) {
            bail!("graph docker container already running");
        }

        let docker = ctx.get_bin("docker")?;
        let n4j_dir = ctx.get_neo4j_dir()?;
        let data_dir = n4j_dir.join("data");
        let import_dir = ctx.get_graph_import_dir()?;
        let plugins_dir = get_plugins_dir(&ctx)?;
        let logs_dir = n4j_dir.join("logs");
        ensure_dir_exists(&data_dir)?;
        ensure_dir_exists(&import_dir)?;
        ensure_dir_exists(&logs_dir)?;
        ensure_apoc(&ctx, &plugins_dir)?;
        let mut args = Vec::with_capacity(32);
        let cmd = if self.sudo {
            args.push(docker.as_str());
            ctx.get_bin("sudo")?
        } else {
            docker
        };
        let logs_mount = format!("{}:/logs", path_must_str(&logs_dir));
        let data_mount = format!("{}:/data", path_must_str(&data_dir));
        let import_mount = format!("{}:/var/lib/graph/import", path_must_str(&import_dir));
        let plugin_mount = format!("{}:/plugins", path_must_str(&plugins_dir));
        let container = format!("graph:{}", NEO4J_DOCKER_TAG);

        let env_heap_size1 = format!("HEAP_SIZE={}", self.heap_size);
        let env_heap_size2 = format!("NEO4J_dbms_memory_heap_max__size={}", self.heap_size);
        args.extend(&[
            "run",
            "--name",
            NEO4J_CONTAINER_NAME,
            "-p7474:7474",
            "-p7687:7687",
            "--rm",
            "-v",
            &logs_mount,
            "-v",
            &data_mount,
            "-v",
            &import_mount,
            "-v",
            &plugin_mount,
        ]);

        add_user_flag!(args);
        add_selinux_flags!(args);

        args.extend(&[
            "-d",
            "--env",
            "NEO4J_AUTH=none",
            "--env",
            &env_heap_size1,
            "--env",
            &env_heap_size2,
            &container,
        ]);

        let out = run_cmd(&cmd, args.as_slice())?;
        if !out.ok() {
            let serr = out.stderr_utf8_lossy();
            if serr.is_empty() {
                bail!("docker run failed (no stderr)");
            } else {
                bail!("docker run failed:\n{}", serr);
            }
        }
        Ok(())
    }
}

#[derive(Args)]
pub struct StopNeo4j {
    /// Invoke the `docker` commands with `sudo`
    #[arg(long, action = clap::ArgAction::SetTrue, default_value_t = false)]
    sudo: bool,
}

impl StopNeo4j {
    pub fn run(&self) -> anyhow::Result<()> {
        let ctx = DefaultContext::new();
        ensure_neo4j(&ctx, self.sudo)?;
        let docker = ctx.get_bin("docker")?;
        let mut args = Vec::with_capacity(8);
        let cmd = if self.sudo {
            args.push(docker.as_str());
            ctx.get_bin("sudo")?
        } else {
            docker
        };
        args.extend(&["kill", NEO4J_CONTAINER_NAME]);
        let out = run_cmd(&cmd, args.as_slice())?;
        if !out.ok() {
            let serr = out.stderr_utf8_lossy();
            if serr.is_empty() {
                bail!("docker kill failed (no stderr)");
            } else {
                bail!("docker kill failed:\n{}", serr);
            }
        }
        Ok(())
    }
}
fn ensure_apoc(ctx: &dyn Context, dir: &PathBuf) -> anyhow::Result<()> {
    let apoc_file = dir.join(APOC_FILE);
    if apoc_file.exists() {
        return Ok(());
    }
    ensure_dir_exists(dir)?;

    // Ensure we only have the most recent APOC plugin in there

    for ent in fs::read_dir(dir)? {
        let entry = ent?;
        log::debug!("removing old apoc file: {}", entry.path().to_string_lossy());
        fs::remove_file(entry.path())?;
    }

    log::info!("Downloading APOC plugin");
    let as_str = apoc_file.to_str().expect("bad path");
    let store = get_filestore(ctx)?;

    if let Err(e) = store.get_file(ctx, APOC_FILE, as_str) {
        let _ = fs::remove_file(&apoc_file);
        return Err(e.into());
    }
    Ok(())
}

fn get_plugins_dir(ctx: &dyn Context) -> anyhow::Result<PathBuf> {
    Ok(ctx.get_user_local_dir()?.join("neo4j").join("plugins"))
}
