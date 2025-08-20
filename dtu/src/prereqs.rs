use diesel::backend::Backend;
use diesel::deserialize::FromSql;
use diesel::expression::AsExpression;
use diesel::serialize::{Output, ToSql};
use diesel::sql_types::Integer;
use diesel::FromSqlRow;
use std::fmt::{Display, Formatter};
use std::str::FromStr;

#[repr(i32)]
#[derive(Clone, Copy, Debug, PartialEq, AsExpression, FromSqlRow)]
#[diesel(sql_type = Integer)]
pub enum Prereq {
    /// Framework and APKs have been pulled and decompiled
    PullAndDecompile = 1,

    /// The SQL database is populated with data
    SQLDatabaseSetup = 2,

    /// The diff with an eumulator is done
    EmulatorDiff = 3,

    /// The Graph database is accessible and populated with inheritance and
    /// implementation data
    GraphDatabasePartialSetup = 4,

    /// fa.st has had a successful autocall run and Autocall.csv has been
    /// pulled
    FastAutocallResults = 5,

    /// The testing application was created
    AppSetup = 6,

    /// The SELinux policy has been pulled off the device and recompiled
    AcquiredSelinuxPolicy = 7,

    /// The Graph database is populated with all data
    GraphDatabaseFullSetup = 8,

    #[cfg(test)]
    TestComplete = 0x1000,
    #[cfg(test)]
    TestIncomplete = 0x1001,
}

impl FromStr for Prereq {
    type Err = &'static str;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "pull" => Self::PullAndDecompile,
            "db-setup" => Self::SQLDatabaseSetup,
            "graphdb-partial-setup" => Self::GraphDatabasePartialSetup,
            "graphdb-full-setup" => Self::GraphDatabaseFullSetup,
            "app-setup" => Self::AppSetup,
            "fast-results" => Self::FastAutocallResults,
            "emulator-diff" => Self::EmulatorDiff,
            "acquired-selinux-policy" => Self::AcquiredSelinuxPolicy,
            _ => {
                return Err(
                    "valid values are 'pull', 'db-setup', 'graphdb-partial-setup', 'graphdb-full-setup', 'app-setup', 'emulator-diff', 'fast-results', and 'acquired-selinux-policy'"
                )
            }
        })
    }
}

impl Display for Prereq {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let as_str = match self {
            Self::PullAndDecompile => "PullAndDecompile",
            Self::SQLDatabaseSetup => "SQLDatabaseSetup",
            Self::EmulatorDiff => "EmulatorDiff",
            Self::GraphDatabasePartialSetup => "GraphDatabasePartialSetup",
            Self::GraphDatabaseFullSetup => "GraphDatabaseFullSetup",
            Self::FastAutocallResults => "FastAutocallResults",
            Self::AppSetup => "AppSetup",
            Self::AcquiredSelinuxPolicy => "AcquiredSelinuxPolicy",
            #[cfg(test)]
            Self::TestComplete => "TestComplete",
            #[cfg(test)]
            Self::TestIncomplete => "TestIncomplete",
        };
        write!(f, "{}", as_str)
    }
}

impl<DB> ToSql<Integer, DB> for Prereq
where
    DB: Backend,
    i32: ToSql<Integer, DB>,
{
    fn to_sql<'b>(&'b self, out: &mut Output<'b, '_, DB>) -> diesel::serialize::Result {
        match self {
            Self::PullAndDecompile => 1.to_sql(out),
            Self::SQLDatabaseSetup => 2.to_sql(out),
            Self::EmulatorDiff => 3.to_sql(out),
            Self::GraphDatabasePartialSetup => 4.to_sql(out),
            Self::FastAutocallResults => 5.to_sql(out),
            Self::AppSetup => 6.to_sql(out),
            Self::AcquiredSelinuxPolicy => 7.to_sql(out),
            Self::GraphDatabaseFullSetup => 8.to_sql(out),
            #[cfg(test)]
            Self::TestComplete => 0x1000.to_sql(out),
            #[cfg(test)]
            Self::TestIncomplete => 0x1001.to_sql(out),
        }
    }
}

impl<DB> FromSql<Integer, DB> for Prereq
where
    DB: Backend,
    i32: FromSql<Integer, DB>,
{
    fn from_sql(bytes: DB::RawValue<'_>) -> diesel::deserialize::Result<Self> {
        let value: i32 = i32::from_sql(bytes)?;
        Ok(match value {
            1 => Self::PullAndDecompile,
            2 => Self::SQLDatabaseSetup,
            3 => Self::EmulatorDiff,
            4 => Self::GraphDatabasePartialSetup,
            5 => Self::FastAutocallResults,
            6 => Self::AppSetup,
            7 => Self::AcquiredSelinuxPolicy,
            8 => Self::GraphDatabaseFullSetup,
            #[cfg(test)]
            0x1000 => Self::TestComplete,
            #[cfg(test)]
            0x1001 => Self::TestIncomplete,
            _ => {
                panic!("todo")
                //return Err(Box::new(format!("invalid prereq value {}", value)))
            }
        })
    }
}
