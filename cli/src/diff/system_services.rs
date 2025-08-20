use crate::diff::{get_diff_source, smali_sig_contains_class, smali_sig_looks_like_binder};
use crate::parsers::DiffSourceValueParser;
use clap::{self, Args, Subcommand};
use crossterm::style::{Attribute, ContentStyle, Stylize};
use dtu::db::sql::device::models::{DiffSource, SystemService};
use dtu::db::sql::{DeviceDatabase, DeviceSqliteDatabase, MetaDatabase};
use dtu::Context;

use crate::parsers::SystemServiceValueParser;
use crate::printer::{color, Printer};

#[derive(Args)]
pub struct SystemServices {
    #[command(subcommand)]
    command: Option<Command>,

    /// Set the diff source, defaults to the emulator
    #[arg(short = 'S', long, value_parser = DiffSourceValueParser)]
    diff_source: Option<DiffSource>,
}

#[derive(Subcommand)]
enum Command {
    #[command()]
    Methods(Methods),
}

impl SystemServices {
    pub fn run(&self, ctx: &dyn Context, meta: &dyn MetaDatabase) -> anyhow::Result<()> {
        let db = DeviceSqliteDatabase::new(ctx)?;
        let ran_cmd = self.run_command(&ctx, meta, &db)?;
        if ran_cmd {
            return Ok(());
        }
        let diff_source = get_diff_source(ctx, meta, &self.diff_source)?.id;
        let services = db.get_system_service_diffs_by_diff_id(diff_source)?;
        for s in services {
            if s.exists_in_diff {
                continue;
            }
            println!("{}", s.name);
        }
        Ok(())
    }

    fn run_command(
        &self,
        ctx: &dyn Context,
        meta: &dyn MetaDatabase,
        db: &dyn DeviceDatabase,
    ) -> anyhow::Result<bool> {
        let cmd = match self.command.as_ref() {
            None => return Ok(false),
            Some(c) => c,
        };
        match cmd {
            Command::Methods(c) => c.run(ctx, meta, db),
        }?;
        Ok(true)
    }
}

#[derive(Args)]
struct Methods {
    /// Optional service
    #[arg(short, long, value_parser = SystemServiceValueParser)]
    service: Option<SystemService>,

    /// Only new methods
    #[arg(
        short = 'N',
        long,
        action = clap::ArgAction::SetTrue,
        default_value_t = false,
    )]
    only_new: bool,

    /// Only modified methods
    #[arg(
        short = 'M',
        long,
        action = clap::ArgAction::SetTrue,
        default_value_t = false,
    )]
    only_modified: bool,

    /// Only "interesting" methods
    ///
    /// An interesting method is defined as any method that takes non primitive input
    #[arg(
        short = 'I',
        long,
        action = clap::ArgAction::SetTrue,
        default_value_t = false,
    )]
    only_interesting: bool,

    /// Only binder methods
    ///
    /// Only methods that take or return objects that look like IBinders
    #[arg(
        short = 'B',
        long,
        action = clap::ArgAction::SetTrue,
        default_value_t = false,
    )]
    only_binders: bool,

    /// Set the diff source, defaults to the emulator
    #[arg(short = 'S', long, value_parser = DiffSourceValueParser)]
    diff_source: Option<DiffSource>,
}

impl Methods {
    fn run(
        &self,
        ctx: &dyn Context,
        meta: &dyn MetaDatabase,
        db: &dyn DeviceDatabase,
    ) -> anyhow::Result<()> {
        let diff_source = get_diff_source(ctx, meta, &self.diff_source)?.id;
        let printer = Printer::new();
        let mut methods = match self.service.as_ref() {
            None => db.get_system_service_method_diffs_by_diff_id(diff_source)?,
            Some(s) => db.get_system_service_method_diffs_for_service(s.id, diff_source)?,
        };

        methods.sort_by(|lhs, rhs| {
            if lhs.system_service_id != rhs.system_service_id {
                lhs.system_service_id.cmp(&rhs.system_service_id)
            } else {
                lhs.transaction_id.cmp(&rhs.transaction_id)
            }
        });

        let methods = methods.iter().filter(|it| {
            if self.only_binders {
                if !smali_sig_looks_like_binder(it.get_signature()) {
                    return false;
                }
            }

            if self.only_interesting {
                let sig = it.get_signature();
                if !smali_sig_contains_class(sig) {
                    return false;
                }
            }

            if self.only_new {
                !it.exists_in_diff
            } else if self.only_modified {
                it.exists_in_diff && it.hash_matches_diff.is_false()
            } else {
                true
            }
        });

        let mut current_service_id = -1;

        for m in methods {
            if current_service_id != m.system_service_id {
                let svc = db.get_system_service_by_id(m.system_service_id)?;
                current_service_id = svc.id;
                printer.println_colored(svc.name, color::CYAN);
            }
            let signature = m.get_signature();
            let ret_type = m.get_return_type();

            let display = format!(
                "  ({}) {}({}) -> {}",
                m.transaction_id, m.name, signature, ret_type
            );

            if smali_sig_looks_like_binder(signature) || smali_sig_looks_like_binder(ret_type) {
                let mut style = ContentStyle::new().with(color::PURPLE);
                style.attributes.set(Attribute::Bold);
                printer.println_styled(display, style);
            } else if signature.contains("Ljava/lang/String;") {
                let mut style = ContentStyle::new().with(color::INTERESTING);
                style.attributes.set(Attribute::Bold);
                printer.println_styled(display, style);
            } else {
                printer.println(display);
            }
        }

        Ok(())
    }
}
