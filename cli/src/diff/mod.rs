use clap::{self, Args, Subcommand};
use dtu::db::{DeviceDatabase, MetaDatabase, MetaSqliteDatabase};
use dtu::prereqs::Prereq;
use dtu::{Context, DefaultContext};

mod apks;
use apks::Apks;
use dtu::db::device::models::DiffSource;
use dtu::db::device::EMULATOR_DIFF_SOURCE;
use dtu::utils::{ClassName, SmaliMethodSignatureIterator};

mod system_services;
use system_services::SystemServices;

mod ui;
use ui::UI;

#[derive(Args)]
pub struct Diff {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    #[command()]
    UI(UI),

    #[command()]
    SystemServices(SystemServices),

    #[command()]
    Apks(Apks),
}

impl Diff {
    pub fn run(&self) -> anyhow::Result<()> {
        let ctx = DefaultContext::new();
        let meta = MetaSqliteDatabase::new(&ctx)?;
        match &self.command {
            Command::UI(c) => c.run(&ctx, &meta),
            Command::SystemServices(c) => c.run(&ctx, &meta),
            Command::Apks(c) => c.run(&ctx, &meta),
        }
    }
}

/// Retrieve a diff source to use
///
/// The source is resolved in this order:
///
///     (1) If the provided src is Some it is returned
///     (2) The DTU_DIFF_SOURCE env var is checked and used as the default diff source if it exists
///     (3) The EMULATOR_DIFF_SOURCE is returned after ensuring the emulator diff is available
pub fn get_diff_source(
    ctx: &dyn Context,
    meta: &dyn MetaDatabase,
    db: &DeviceDatabase,
    src: &Option<DiffSource>,
) -> anyhow::Result<DiffSource> {
    let diff_source = match src {
        Some(it) => (*it).clone(),
        None => match ctx.maybe_get_env("DTU_DIFF_SOURCE") {
            Some(v) => {
                log::info!("Using DTU_DIFF_SOURCE {v}");
                db.get_diff_source_by_name(&v)?
            }
            None => {
                meta.ensure_prereq(Prereq::EmulatorDiff)?;
                return Ok(db.get_diff_source_by_name(EMULATOR_DIFF_SOURCE)?);
            }
        },
    };

    if diff_source.name == EMULATOR_DIFF_SOURCE {
        meta.ensure_prereq(Prereq::EmulatorDiff)?;
    }
    Ok(diff_source)
}

pub fn smali_sig_contains_class(vals: &str) -> bool {
    if vals == "?" {
        return false;
    }
    let mut iter = match SmaliMethodSignatureIterator::new(vals) {
        Err(_) => return false,
        Ok(v) => v,
    };
    iter.find(|it| {
        if let dtu::smalisa::Type::Class(_, _) = it {
            true
        } else {
            false
        }
    })
    .is_some()
}

pub fn smali_sig_looks_like_binder(vals: &str) -> bool {
    if vals == "?" {
        return false;
    }
    let mut iter = match SmaliMethodSignatureIterator::new(vals) {
        Err(_) => return false,
        Ok(v) => v,
    };
    iter.find(|it| {
        if let dtu::smalisa::Type::Class(name, _) = it {
            let cname = ClassName::from(*name);
            let simple = cname.get_simple_class_name();
            simple.starts_with("I") || simple.contains("Callback") || simple.contains("Listener")
        } else {
            false
        }
    })
    .is_some()
}
