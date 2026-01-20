#[cfg(feature = "sql")]
use diesel::{
    backend::Backend,
    deserialize::FromSql,
    expression::AsExpression,
    serialize::{Output, ToSql},
    sql_types::Text,
    FromSqlRow,
};
use serde::{Deserialize, Serialize};
use std::fmt::{Display, Formatter};
use std::str::FromStr;

#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "sql", derive(AsExpression, FromSqlRow))]
#[cfg_attr(feature = "sql", diesel(sql_type = Text))]
pub enum Prereq {
    /// Framework and APKs have been pulled and decompiled
    PullAndDecompile,

    /// The SQL database is populated with data
    SQLDatabaseSetup,

    /// The diff with an eumulator is done
    EmulatorDiff,

    /// The Graph database is accessible and populated with all information
    GraphDatabaseSetup,

    /// fa.st has had a successful autocall run and Autocall.csv has been
    /// pulled
    FastAutocallResults,

    /// The testing application was created
    AppSetup,

    /// The SELinux policy has been pulled off the device and recompiled
    AcquiredSelinuxPolicy,

    /// Smalisa has been run on everything
    Smalisa,

    #[cfg(test)]
    TestComplete,
    #[cfg(test)]
    TestIncomplete,
}

impl Serialize for Prereq {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for Prereq {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = <&'_ str as Deserialize>::deserialize(deserializer)?;
        Self::from_str(s).map_err(|e| serde::de::Error::custom(e))
    }
}

impl FromStr for Prereq {
    type Err = &'static str;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "pull" => Self::PullAndDecompile,
            "db-setup" => Self::SQLDatabaseSetup,
            "graphdb-setup" => Self::GraphDatabaseSetup,
            "app-setup" => Self::AppSetup,
            "fast-results" => Self::FastAutocallResults,
            "emulator-diff" => Self::EmulatorDiff,
            "acquired-selinux-policy" => Self::AcquiredSelinuxPolicy,
            "smalisa" => Self::Smalisa,
            #[cfg(test)]
            "test-complete" => Self::TestComplete,
            #[cfg(test)]
            "test-incomplete" => Self::TestIncomplete,

            _ => {
                return Err(
                    "valid values are 'pull', 'db-setup', 'graphdb-setup', 'graphdb-setup', 'app-setup', 'emulator-diff', 'fast-results', 'smalisa', and 'acquired-selinux-policy'"
                )
            }
        })
    }
}

impl Prereq {
    fn as_str(&self) -> &'static str {
        match self {
            Self::PullAndDecompile => "pull",
            Self::SQLDatabaseSetup => "db-setup",
            Self::EmulatorDiff => "emulator-diff",
            Self::GraphDatabaseSetup => "graphdb-setup",
            Self::FastAutocallResults => "fast-results",
            Self::AppSetup => "app-setup",
            Self::AcquiredSelinuxPolicy => "acquired-selinux-policy",
            Self::Smalisa => "smalisa",
            #[cfg(test)]
            Self::TestComplete => "test-complete",
            #[cfg(test)]
            Self::TestIncomplete => "test-incomplete",
        }
    }
}

impl Display for Prereq {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

#[cfg(feature = "sql")]
impl<DB> ToSql<Text, DB> for Prereq
where
    DB: Backend,
    str: ToSql<Text, DB>,
{
    fn to_sql<'b>(&'b self, out: &mut Output<'b, '_, DB>) -> diesel::serialize::Result {
        let s: &'static str = self.as_str();
        s.to_sql(out)
    }
}

#[cfg(feature = "sql")]
impl<DB> FromSql<Text, DB> for Prereq
where
    DB: Backend,
    String: FromSql<Text, DB>,
{
    fn from_sql(bytes: DB::RawValue<'_>) -> diesel::deserialize::Result<Self> {
        let value = String::from_sql(bytes)?;
        Self::from_str(value.as_str()).map_err(|_| panic!("TODO invalid prereq"))
    }
}
