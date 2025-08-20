use std::fmt::{Display, Formatter};
use std::path::{Path, PathBuf};
use std::str::FromStr;

use diesel::backend::Backend;
use diesel::deserialize::FromSql;
use diesel::expression::AsExpression;
use diesel::prelude::*;
use diesel::serialize::{Output, ToSql};
use diesel::sql_types::Integer;
use diesel::FromSqlRow;
use dtu_proc_macro::sql_db_row;

use super::schema::*;
use crate::prereqs::Prereq;
use crate::utils::DevicePath;

#[derive(Queryable, Insertable, AsChangeset)]
#[diesel(table_name = progress)]
pub struct ProgressStep {
    pub step: Prereq,
    pub completed: bool,
}

#[sql_db_row]
pub struct KeyValue {
    pub id: i32,
    pub key: String,
    pub value: String,
}

#[sql_db_row]
#[diesel(table_name = app_activities)]
pub struct AppActivity {
    pub id: i32,
    /// The name of the activity (a simple class name)
    pub name: String,
    /// The Android ID for the activity's button
    pub button_android_id: String,
    /// The text to display on the button
    pub button_text: String,
    pub status: AppTestStatus,
}

#[sql_db_row]
pub struct AppPermission {
    pub id: i32,
    pub permission: String,
    pub usable: bool,
}

/// Database entry for every pull
#[sql_db_row]
#[diesel(table_name = decompile_status)]
pub struct DecompileStatus {
    pub id: i32,
    pub device_path: DevicePath,
    pub host_path: Option<String>,
    pub decompiled: bool,
    pub decompile_attempts: i32,
}

impl DecompileStatus {
    pub fn host_path_as_pb(&self) -> Option<PathBuf> {
        self.host_path.as_ref().map(|p| PathBuf::from(p))
    }

    #[inline]
    pub fn should_decompile(&self) -> bool {
        !self.decompiled
    }

    /// Returns whether the entry should be pulled.
    pub fn should_pull(&self) -> bool {
        self.host_path
            .as_ref()
            .map_or(true, |p| !Path::new(p).exists())
    }
}

#[repr(i32)]
#[derive(Clone, Copy, Debug, PartialEq, AsExpression, FromSqlRow)]
#[diesel(sql_type = Integer)]
pub enum AppTestStatus {
    Experimenting = 1,
    Failed = 2,
    Confirmed = 3,
}

impl FromStr for AppTestStatus {
    type Err = &'static str;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "exp" => Self::Experimenting,
            "fail" => Self::Failed,
            "conf" => Self::Confirmed,
            _ => return Err("valid values are 'exp', 'fail', and 'conf'"),
        })
    }
}

impl Display for AppTestStatus {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            match self {
                Self::Experimenting => "exp",
                Self::Failed => "fail",
                Self::Confirmed => "conf",
            }
        )
    }
}

impl Default for AppTestStatus {
    fn default() -> Self {
        Self::Experimenting
    }
}

impl<DB> ToSql<Integer, DB> for AppTestStatus
where
    DB: Backend,
    i32: ToSql<Integer, DB>,
{
    fn to_sql<'b>(&'b self, out: &mut Output<'b, '_, DB>) -> diesel::serialize::Result {
        match self {
            Self::Experimenting => 1.to_sql(out),
            Self::Failed => 2.to_sql(out),
            Self::Confirmed => 3.to_sql(out),
        }
    }
}

impl<DB> FromSql<Integer, DB> for AppTestStatus
where
    DB: Backend,
    i32: FromSql<Integer, DB>,
{
    fn from_sql(bytes: DB::RawValue<'_>) -> diesel::deserialize::Result<Self> {
        let value: i32 = i32::from_sql(bytes)?;
        Ok(match value {
            1 => Self::Experimenting,
            2 => Self::Failed,
            3 => Self::Confirmed,
            // TODO
            _ => todo!("bad database"),
        })
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::testing::{tmp_context, TestContext};
    use crate::Context;
    use rstest::*;
    use std::fs::OpenOptions;

    #[rstest]
    fn test_decompile_status(tmp_context: TestContext) {
        let mut status = DecompileStatus {
            id: 10,
            device_path: DevicePath::new("/system/framework/framework.jar"),
            host_path: None,
            decompiled: false,
            decompile_attempts: 0,
        };

        assert_eq!(status.should_pull(), true, "should need to pull");
        assert_eq!(status.should_decompile(), true, "should need to decompile");

        let pb = tmp_context.get_project_dir().unwrap().join("test");
        {
            let _f = OpenOptions::new()
                .write(true)
                .create(true)
                .open(&pb)
                .unwrap();
        }
        status.host_path = Some(pb.to_str().unwrap().into());
        assert_eq!(status.should_pull(), false, "shouldn't need to pull");
        assert_eq!(status.should_decompile(), true, "should need to decompile");
    }
}
