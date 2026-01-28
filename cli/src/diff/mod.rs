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

pub fn get_diff_source(
    ctx: &dyn Context,
    meta: &dyn MetaDatabase,
    src: &Option<DiffSource>,
) -> anyhow::Result<DiffSource> {
    let diff_source = match src {
        Some(it) => {
            if it.name == EMULATOR_DIFF_SOURCE {
                meta.ensure_prereq(Prereq::EmulatorDiff)?;
            }
            (*it).clone()
        }
        None => {
            meta.ensure_prereq(Prereq::EmulatorDiff)?;
            let db = DeviceDatabase::new(ctx)?;
            db.get_diff_source_by_name(EMULATOR_DIFF_SOURCE)?
        }
    };
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
