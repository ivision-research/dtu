use std::fmt::Display;

use dtu::db::device::schema::{
    system_service_method_diffs, system_service_methods, system_services,
};
use dtu::db::Idable;
use dtu::{
    db::{self, DeviceDatabase},
    UnknownBool,
};

use dtu::diesel::prelude::*;

use crate::utils::ostr;

pub struct DiffedSystemServiceMethodData {
    pub id: i32,
    pub system_service_id: i32,
    pub transaction_id: i32,
    pub name: String,
    pub signature: Option<String>,
    pub return_type: Option<String>,
    pub service_binder_available: UnknownBool,
    pub exists_in_diff: bool,
    pub hash_matches_diff: UnknownBool,
}

impl Display for DiffedSystemServiceMethodData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let sig = ostr(&self.signature).unwrap_or("?");
        let ret = ostr(&self.return_type).unwrap_or("?");
        write!(f, "{}({}) -> {}", self.name, sig, ret,)
    }
}

impl Idable for DiffedSystemServiceMethodData {
    fn get_id(&self) -> i32 {
        self.id
    }
}

impl DiffedSystemServiceMethodData {
    pub fn vec_from_db(db: &DeviceDatabase, diffid: i32) -> anyhow::Result<Vec<Self>> {
        Ok(
            db.with_connection(|c| -> db::Result<Vec<DiffedSystemServiceMethodData>> {
                let res = system_service_method_diffs::table
                    .filter(
                        system_service_method_diffs::exists_in_diff
                            .eq(false)
                            .or(system_service_method_diffs::hash_matches_diff
                                .eq(UnknownBool::False)),
                    )
                    .filter(system_service_method_diffs::diff_source.eq(diffid))
                    .inner_join(system_service_methods::table.inner_join(system_services::table))
                    .select((
                        system_services::id,
                        system_service_methods::name,
                        system_service_methods::signature,
                        system_service_methods::return_type,
                        system_services::can_get_binder,
                        system_service_method_diffs::exists_in_diff,
                        system_service_method_diffs::hash_matches_diff,
                        system_service_methods::transaction_id,
                        system_service_methods::id,
                    ))
                    .load::<(
                        i32,
                        String,
                        Option<String>,
                        Option<String>,
                        UnknownBool,
                        bool,
                        UnknownBool,
                        i32,
                        i32,
                    )>(c)?;

                Ok(res
                    .into_iter()
                    .map(DiffedSystemServiceMethodData::from)
                    .collect())
            })?,
        )
    }
}

impl
    From<(
        i32,
        String,
        Option<String>,
        Option<String>,
        UnknownBool,
        bool,
        UnknownBool,
        i32,
        i32,
    )> for DiffedSystemServiceMethodData
{
    fn from(
        value: (
            i32,
            String,
            Option<String>,
            Option<String>,
            UnknownBool,
            bool,
            UnknownBool,
            i32,
            i32,
        ),
    ) -> Self {
        Self {
            system_service_id: value.0,
            name: value.1,
            signature: value.2,
            return_type: value.3,
            service_binder_available: value.4,
            exists_in_diff: value.5,
            hash_matches_diff: value.6,
            transaction_id: value.7,
            id: value.8,
        }
    }
}
