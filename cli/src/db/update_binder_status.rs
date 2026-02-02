use anyhow::bail;
use clap::{self, Args};
use dtu::{
    app_server::AppServer,
    db::{
        device::{schema::system_services, ServiceMeta},
        DeviceDatabase,
    },
    diesel::{prelude::*, update},
    prereqs::Prereq,
    utils::ensure_prereq,
    DefaultContext, UnknownBool,
};

use crate::utils::get_app_server;
/// Calls `service list` from the application via the application server to determine whether
/// binders are available for system services or not.
#[derive(Args)]
pub struct UpdateBinderAvailability {}

impl UpdateBinderAvailability {
    pub fn run(&self) -> anyhow::Result<()> {
        let ctx = DefaultContext::new();
        ensure_prereq(&ctx, Prereq::SQLDatabaseSetup)?;
        ensure_prereq(&ctx, Prereq::AppSetup)?;
        let db = DeviceDatabase::new(&ctx)?;
        let mut app_server = get_app_server(&ctx)?;
        let res = app_server.sh("service list")?;
        if !res.ok() {
            bail!(
                "failed to run `service list` from test application: {}",
                res.stderr_string()
            );
        }

        let services = res
            .stdout_string()
            .split('\n')
            .filter_map(|it| {
                let sm = ServiceMeta::from_line(it)?;
                if sm.iface.is_some() {
                    Some(sm.service_name)
                } else {
                    None
                }
            })
            .collect::<Vec<String>>();

        db.with_connection(|c| {
            update(system_services::table)
                .filter(system_services::name.eq_any(&services))
                .set(system_services::can_get_binder.eq(UnknownBool::True.to_numeric()))
                .execute(c)
        })?;

        Ok(())
    }
}
