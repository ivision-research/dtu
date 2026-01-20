use anyhow::bail;
use clap::{self, Args, Subcommand};
use dtu::app::server::AppServer;
use dtu::db;
use dtu::db::device::models::{self, Apk, SystemServiceMethod};
use dtu::db::{DeviceDatabase, DeviceSqliteDatabase, MetaDatabase, MetaSqliteDatabase};
use dtu::prereqs::Prereq;
use dtu::utils::ClassName;
use dtu::DefaultContext;

use crate::parsers::{parse_parcel_string, ApkValueParser};
use crate::utils::{get_app_server, prompt_choice};

#[derive(Args)]
pub struct Call {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Call a method on a system service
    SystemService(SystemService),
    /// Call a method on an application service
    AppService(AppService),
}

impl Call {
    pub fn run(&self) -> anyhow::Result<()> {
        let ctx = DefaultContext::new();
        let meta = MetaSqliteDatabase::new(&ctx)?;
        meta.ensure_prereq(Prereq::AppSetup)?;
        match &self.command {
            Command::SystemService(c) => c.run(),
            Command::AppService(c) => c.run(),
        }
    }
}

#[derive(Args)]
pub struct AppService {
    /// The APK the service belongs to
    #[arg(short = 'A', long, value_parser = ApkValueParser)]
    apk: Apk,

    /// The service class
    #[arg(short, long)]
    class: ClassName,

    /// Optional interface name for the AIDL
    #[arg(short = 'I', long)]
    iface: Option<ClassName>,

    /// The transaction number
    #[arg(short, long)]
    txn: u32,

    /// An optional action to set on the intent when binding
    #[arg(short, long)]
    action: Option<String>,

    /// The Parcel string for defining the Parcel
    #[arg(last = true)]
    parcel: Vec<String>,
}

impl AppService {
    pub fn run(&self) -> anyhow::Result<()> {
        let ctx = DefaultContext::new();
        let qa = if self.parcel.len() > 0 {
            Some(parse_parcel_string(self.parcel.as_slice())?)
        } else {
            None
        };
        let mut srv = get_app_server(&ctx)?;
        let res = srv.call_app_service(
            self.txn,
            &self.apk.app_name,
            self.class.as_ref(),
            self.iface.as_ref(),
            self.action.as_ref().map(|it| it.as_str()),
            qa.as_ref(),
        )?;
        println!("{}", res);
        Ok(())
    }
}

#[derive(Args)]
pub struct SystemService {
    /// The system service name
    #[arg(short, long)]
    service: String,

    /// Optional interface name for the AIDL
    #[arg(short = 'I', long)]
    interface: Option<ClassName>,

    /// The system service method name, if None, --txn must be supplied
    #[arg(short, long)]
    method: Option<String>,

    /// Optional transaction number if it isn't in the database
    #[arg(short, long)]
    txn: Option<u32>,

    /// The Parcel string for defining the Parcel
    #[arg(last = true)]
    parcel: Vec<String>,
}

impl SystemService {
    pub fn run(&self) -> anyhow::Result<()> {
        let ctx = DefaultContext::new();
        let db = DeviceSqliteDatabase::new(&ctx)?;

        let service = match db.get_system_service_by_name(&self.service) {
            Ok(v) => Some(v),
            Err(db::Error::NotFound) if self.interface.is_some() => None,
            Err(e) => return Err(e.into()),
        };

        if service.is_none() && self.txn.is_none() {
            bail!(
                "the service `{}` is not in the database, must provide --txn",
                self.service
            );
        }

        if self.txn.is_none() && self.method.is_none() {
            bail!("need either -t/--txn or -m/--method");
        }

        let iface = match &self.interface {
            Some(v) => v,
            None => match service.as_ref().unwrap().iface.as_ref() {
                Some(v) => v,
                None => bail!(
                    "no known interface for service {}, use -I if one is known",
                    self.service
                ),
            },
        };

        let txn_number = self.get_transaction_id(&db, &service)?;

        if self.method.is_none() && self.txn.is_none() {
            bail!("need either -m/--method or -t/--txn");
        }

        let qa = if self.parcel.len() > 0 {
            Some(parse_parcel_string(self.parcel.as_slice())?)
        } else {
            None
        };
        let mut srv = get_app_server(&ctx)?;
        let res = srv.call_system_service(&self.service, txn_number, Some(iface), qa.as_ref())?;
        println!("{}", res);

        Ok(())
    }
    fn get_transaction_id(
        &self,
        db: &DeviceSqliteDatabase,
        service: &Option<models::SystemService>,
    ) -> anyhow::Result<u32> {
        if let Some(id) = self.txn {
            if id < 1 {
                bail!("invalid transaction id {}", id);
            }
            return Ok(id);
        }

        let svc = match service.as_ref() {
            None => bail!(
                "service {} not found in database and --txn not set",
                self.service
            ),
            Some(v) => v,
        };

        let name = self.method.as_ref().unwrap().as_str();

        let method = db.get_system_service_methods_by_service_id(svc.id)?;
        let methods = method
            .iter()
            .filter(|it| it.name == name)
            .collect::<Vec<&SystemServiceMethod>>();
        let count = methods.len();
        if count == 0 {
            bail!("no method named {} found on service {}", name, self.service);
        } else if count == 1 {
            return Ok(methods.get(0).unwrap().transaction_id as u32);
        }

        Ok(prompt_choice(
            &methods,
            &format!("Multiple methods named {} found for {}", name, self.service),
            "Choice: ",
        )?
        .transaction_id as u32)
    }
}
