use std::fmt;

use clap::{self, Args};
use dtu::{Context, DefaultContext};

#[derive(Args)]
pub struct RunCheck {}

enum Importance {
    Optional,
    Required,
}

enum Status {
    Missing,
    Exists(String),
}

struct Info {
    name: String,
    status: Status,
    importance: Importance,
}

fn check_bin(ctx: &dyn Context, bin: &str, importance: Importance) -> Info {
    let status = match ctx.maybe_get_bin(bin) {
        None => Status::Missing,
        Some(path) => Status::Exists(path),
    };

    Info {
        name: bin.into(),
        status,
        importance,
    }
}

fn check_env(ctx: &dyn Context, env: &str, importance: Importance) -> Info {
    let status = match ctx.maybe_get_env(env) {
        None => Status::Missing,
        Some(env) => Status::Exists(env),
    };

    Info {
        name: env.into(),
        status,
        importance,
    }
}

#[cfg(feature = "emoji")]
mod status {
    pub const FAIL: &'static str = "💩";
    pub const OK: &'static str = "🚀";
    pub const MEH: &'static str = "😒";
}

#[cfg(not(feature = "emoji"))]
mod status {
    pub const FAIL: &'static str = "Fail";
    pub const OK: &'static str = "Ok";
    pub const MEH: &'static str = "Meh";
}

use status::*;

impl RunCheck {
    pub fn run(&self) -> anyhow::Result<()> {
        let ctx = DefaultContext::new();

        let mut checks = Vec::new();

        let required_bins = &["baksmali", "apktool", "jadx", "adb", "gradle"];
        let optional_bins = &[
            "secilc",
            "vdexExtractor",
            "compact_dex_converter",
            "dtu-open-file",
            "dtu-clipboard",
            "aws",
        ];

        for it in required_bins {
            checks.push(check_bin(&ctx, it, Importance::Required));
        }

        for it in optional_bins {
            checks.push(check_bin(&ctx, it, Importance::Optional));
        }
        println!("External programs:");
        #[cfg(feature = "emoji")]
        {
            println!("\n{} = Program present in PATH", OK);
            println!("{} = Required and missing", FAIL);
            println!("{} = Optional and missing\n", MEH);
        }

        for c in checks.iter() {
            println!("{}", c);
        }

        checks.clear();

        let required_envs = &["ANDROID_HOME"];
        let optional_envs = &["JAVA_HOME"];

        println!("Environmental variables:\n");

        for it in required_envs {
            checks.push(check_env(&ctx, it, Importance::Required));
        }

        for it in optional_envs {
            checks.push(check_env(&ctx, it, Importance::Optional));
        }

        for c in checks.iter() {
            println!("{}", c);
        }

        Ok(())
    }
}

impl fmt::Display for Info {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match &self.status {
            Status::Missing => {
                write!(
                    f,
                    "{}: {}",
                    match self.importance {
                        Importance::Optional => MEH,
                        Importance::Required => FAIL,
                    },
                    self.name
                )
            }
            Status::Exists(path) => {
                write!(f, "{}: {} ({})", OK, self.name, path)
            }
        }
    }
}
