use anyhow::bail;
use clap::{self, Args, Subcommand};

use crate::diff::get_diff_source;
use dtu::db::device::models::{
    Apk, DiffSource, DiffedActivity, DiffedProvider, DiffedReceiver, DiffedService, Permission,
};
use dtu::db::{DeviceDatabase, MetaDatabase};
use dtu::Context;

use crate::parsers::{ApkValueParser, DiffSourceValueParser};
use crate::printer::{color, Printer};

#[derive(Args)]
pub struct Apks {
    #[command(subcommand)]
    command: Option<Command>,

    /// Set the diff source, note this doesn't set it on subcommands
    #[arg(short = 'S', long, value_parser = DiffSourceValueParser)]
    diff_source: Option<DiffSource>,
}

#[derive(Subcommand)]
enum Command {
    /// Display ContentProviders that don't exist in AOSP
    #[command()]
    Providers(Providers),

    /// Display BroadcastReceivers that don't exist in AOSP
    #[command()]
    Receivers(Receivers),

    /// Display Services that don't exist in AOSP
    #[command()]
    Services(Services),

    /// Display Activities that don't exist in AOSP
    #[command()]
    Activities(Activities),
}

macro_rules! simple_ipc {
    ($name:ident, $get_all:ident, $get_by_apk:ident, $ty:ty) => {
        #[derive(Args)]
        struct $name {
            /// Optional APK
            #[arg(long, value_parser = ApkValueParser)]
            apk: Option<Apk>,

            /// Only show entries that don't require permissions
            #[arg(long)]
            no_perms: bool,

            /// Allow entries that require normal level permissions
            #[arg(long)]
            allow_normal_perms: bool,

            /// Set the diff source, defaults to the emulator
            #[arg(short = 'S', long, value_parser = DiffSourceValueParser)]
            diff_source: Option<DiffSource>,
        }

        impl $name {
            fn run(
                &self,
                ctx: &dyn Context,
                meta: &dyn MetaDatabase,
                db: &DeviceDatabase,
            ) -> anyhow::Result<()> {
                let diff_source = get_diff_source(ctx, meta, &self.diff_source)?.id;

                let (mut apk_id, items) = match self.apk.as_ref() {
                    None => {
                        let mut items = db.$get_all(diff_source)?;
                        items.sort_by(|lhs, rhs| lhs.apk_id.cmp(&rhs.apk_id));
                        (-1, items)
                    }
                    Some(a) => (a.id, db.$get_by_apk(a.id, diff_source)?),
                };
                if items.len() == 0 {
                    bail!("no {}", stringify!($name).to_lowercase());
                }

                let printer = Printer::new();

                if let Some(apk) = self.apk.as_ref() {
                    print_apk(&printer, apk);
                }

                let normal_perms = db.get_normal_permissions()?;

                let iter = items.iter().filter(|it| {
                    if self.no_perms && it.permission.is_some() {
                        return false;
                    }
                    if self.allow_normal_perms && it.permission.is_some() {
                        let perm = it.permission.as_ref().unwrap().as_str();
                        let is_normal = normal_perms.iter().find(|it| it.name == perm).is_some();
                        if !is_normal {
                            return false;
                        }
                    }
                    !it.exists_in_diff && it.exported && it.enabled
                });

                for it in iter {
                    if it.apk_id != apk_id {
                        let apk = db.get_apk_by_id(it.apk_id)?;
                        print_apk(&printer, &apk);
                        apk_id = apk.id;
                    }
                    self.print_item(&printer, it, &normal_perms);
                }

                Ok(())
            }

            fn print_item(&self, printer: &Printer, it: &$ty, normal_perms: &Vec<Permission>) {
                let has_perm = it.permission.is_some();
                if self.no_perms && has_perm {
                    return;
                }

                printer.print("   - ");

                if has_perm {
                    let perm = it.permission.as_ref().unwrap().as_str();
                    let is_normal = normal_perms.iter().find(|it| it.name == perm).is_some();
                    if is_normal {
                        printer.print_colored(&it.class_name, color::INTERESTING);
                    } else {
                        printer.print(&it.class_name);
                    }
                    printer.print(" - ");
                    let print_color = if is_normal { color::GREY } else { color::RED };
                    printer.println_colored(&it.permission.as_ref().unwrap(), print_color);
                } else {
                    printer.println_colored(&it.class_name, color::INTERESTING);
                }
            }
        }
    };
}

simple_ipc!(
    Receivers,
    get_receiver_diffs_by_diff_id,
    get_receiver_diffs_for_apk,
    DiffedReceiver
);
simple_ipc!(
    Activities,
    get_activity_diffs_by_diff_id,
    get_activity_diffs_for_apk,
    DiffedActivity
);
simple_ipc!(
    Services,
    get_service_diffs_by_diff_id,
    get_service_diffs_for_apk,
    DiffedService
);

#[derive(Args)]
struct Providers {
    /// Optional APK
    #[arg(long, value_parser = ApkValueParser)]
    apk: Option<Apk>,

    /// Set the diff source, defaults to the emulator
    #[arg(short = 'S', long, value_parser = DiffSourceValueParser)]
    diff_source: Option<DiffSource>,
}

impl Providers {
    fn run(
        &self,
        ctx: &dyn Context,
        meta: &dyn MetaDatabase,
        db: &DeviceDatabase,
    ) -> anyhow::Result<()> {
        let diff_source = get_diff_source(ctx, meta, &self.diff_source)?.id;

        let (mut apk_id, providers) = match self.apk.as_ref() {
            None => {
                let mut provs = db.get_provider_diffs_by_diff_id(diff_source)?;
                provs.sort_by(|lhs, rhs| lhs.apk_id.cmp(&rhs.apk_id));
                (-1, provs)
            }
            Some(a) => (a.id, db.get_provider_diffs_for_apk(a.id, diff_source)?),
        };
        if providers.len() == 0 {
            bail!("no providers");
        }

        let printer = Printer::new();

        if let Some(apk) = self.apk.as_ref() {
            print_apk(&printer, apk);
        }

        let iter = providers
            .iter()
            .filter(|it| !it.exists_in_diff && it.exported && it.enabled);

        for p in iter {
            if p.apk_id != apk_id {
                let apk = db.get_apk_by_id(p.apk_id)?;
                print_apk(&printer, &apk);
                apk_id = apk.id;
            }
            self.print_provider(&printer, p);
        }

        Ok(())
    }

    fn print_provider(&self, printer: &Printer, provider: &DiffedProvider) {
        let has_perm = provider.read_permission.is_some()
            || provider.write_permission.is_some()
            || provider.permission.is_some();

        printer.print("   - ");

        if has_perm {
            printer.print(&provider.name);
        } else {
            printer.print_colored(&provider.name, color::INTERESTING);
        }

        printer.print(" [");
        if has_perm {
            printer.print(&provider.authorities);
        } else {
            printer.print_colored(&provider.authorities, color::INTERESTING);
        }

        printer.println("]");

        let general_perm = provider
            .permission
            .as_ref()
            .map(|it| it.as_str())
            .unwrap_or("");

        if let Some(perm) = provider.permission.as_ref() {
            printer.println(&format!("      - PERM = {}", perm));
        }

        if let Some(read) = provider.read_permission.as_ref() {
            if read != general_perm {
                printer.println(&format!("      - READ = {}", read));
            }
        }

        if let Some(write) = provider.write_permission.as_ref() {
            if write != general_perm {
                printer.println(&format!("      - WRITE = {}", write));
            }
        }
    }
}

impl Apks {
    pub fn run(&self, ctx: &dyn Context, meta: &dyn MetaDatabase) -> anyhow::Result<()> {
        let db = DeviceDatabase::new(ctx)?;
        let ran_cmd = self.run_command(&ctx, meta, &db)?;
        if ran_cmd {
            return Ok(());
        }
        self.show_apks(ctx, meta, &db)
    }

    fn run_command(
        &self,
        ctx: &dyn Context,
        meta: &dyn MetaDatabase,
        db: &DeviceDatabase,
    ) -> anyhow::Result<bool> {
        let cmd = match self.command.as_ref() {
            None => return Ok(false),
            Some(c) => c,
        };
        match cmd {
            Command::Providers(c) => c.run(ctx, meta, db),
            Command::Receivers(c) => c.run(ctx, meta, db),
            Command::Services(c) => c.run(ctx, meta, db),
            Command::Activities(c) => c.run(ctx, meta, db),
        }?;
        Ok(true)
    }

    fn show_apks(
        &self,
        ctx: &dyn Context,
        meta: &dyn MetaDatabase,
        db: &DeviceDatabase,
    ) -> anyhow::Result<()> {
        let printer = Printer::new();
        let diff_id = get_diff_source(ctx, meta, &self.diff_source)?.id;
        let apks = db.get_apk_diffs_by_diff_id(diff_id)?;

        let filtered = apks.iter().filter(|it| !it.exists_in_diff);

        for apk in filtered {
            print_apk(&printer, apk);
        }
        Ok(())
    }
}

fn print_apk(printer: &Printer, apk: &Apk) {
    printer.print_colored(&apk.name, color::CYAN);
    printer.print(" [");
    printer.print_colored(&apk.app_name, color::GREEN);
    printer.println("]");
}
