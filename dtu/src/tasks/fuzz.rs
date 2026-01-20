use std::io::Read;

use csv::{ReaderBuilder, StringRecord};

use dtu_proc_macro::{wraps_base_error, wraps_decompile_error};

use crate::db::device::db::Database;
use crate::db::device::models::{FuzzResult, InsertFuzzResult};
use crate::db::{self, DeviceSqliteDatabase};
use crate::Context;

#[wraps_base_error]
#[wraps_decompile_error]
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid CSV file")]
    CSVError,

    #[error("{0}")]
    DBError(db::Error),
}

impl From<db::Error> for Error {
    fn from(value: db::Error) -> Self {
        Self::DBError(value)
    }
}

impl From<csv::Error> for Error {
    fn from(_value: csv::Error) -> Self {
        Self::CSVError
    }
}

pub type Result<T> = std::result::Result<T, Error>;

/// FuzzInfo holds stored results from fuzzing, such as from ssfuzz or fast
#[derive(Debug, Eq, PartialEq)]
pub struct FuzzInfo {
    pub service: String,
    pub method: String,
    pub threw_exception: bool,
    pub threw_security_exception: bool,
}

fn extract_csv_info<R: Read>(csv: &mut R) -> Result<Vec<FuzzInfo>> {
    let mut res: Vec<FuzzInfo> = Vec::new();

    let mut reader = ReaderBuilder::new().has_headers(false).from_reader(csv);

    let mut record = StringRecord::new();

    while reader.read_record(&mut record)? {
        res.push(FuzzInfo::try_from(&record)?);
    }

    Ok(res)
}

impl TryFrom<&StringRecord> for FuzzInfo {
    type Error = Error;

    fn try_from(value: &StringRecord) -> std::result::Result<Self, Self::Error> {
        Ok(Self {
            service: get_record_idx(&value, 0)?.to_string(),
            method: get_record_idx(&value, 1)?.to_string(),
            threw_exception: get_record_bool(&value, 2)?,
            threw_security_exception: get_record_bool(&value, 3)?,
        })
    }
}

fn get_record_bool(record: &StringRecord, idx: usize) -> Result<bool> {
    let as_str = get_record_idx(record, idx)?;
    Ok(as_str == "1")
}

fn get_record_idx(record: &StringRecord, idx: usize) -> Result<&str> {
    record.get(idx).ok_or(Error::CSVError)
}

/// Parse a provided CSV file provided by a fuzzing tool, adding the
/// reported information into the device's database
///
/// The CSV file is expected to not have headers, and be in the following
/// format, with boolean fields marked by a `0` or a `1`:
/// `service,method,exceptionThrown?,exceptionWasSecurityException?,rawExceptionCode,parcelData`
///
/// Examples of programs that generate this file are ssfuzz and fast
pub fn parse_csv<R>(ctx: &dyn Context, csv: &mut R) -> Result<()>
where
    R: Read,
{
    let info = extract_csv_info(csv)?;

    let dev_db = DeviceSqliteDatabase::new(ctx)?;

    for res in info {
        let ins = InsertFuzzResult::new(
            &res.service,
            &res.method,
            res.threw_exception,
            res.threw_security_exception,
        );

        dev_db.add_fuzz_result(&ins)?;
    }

    Ok(())
}

/// Search the SQL database for endpoints that did not throw a security exception
pub fn get_no_security(ctx: &dyn Context) -> Result<Vec<FuzzResult>> {
    let dev_db = DeviceSqliteDatabase::new(ctx)?;

    Ok(dev_db.get_endpoints_by_security(false)?)
}

#[cfg(test)]
mod test {
    use super::*;
    use rstest::*;

    #[rstest]
    fn test_csv_info() {
        let csv = "s1,m1,0,0\ns2,m2,1,1\n";
        let info = extract_csv_info(&mut csv.as_bytes()).unwrap();

        let control = vec![
            FuzzInfo {
                service: "s1".into(),
                method: "m1".into(),
                threw_exception: false,
                threw_security_exception: false,
            },
            FuzzInfo {
                service: "s2".into(),
                method: "m2".into(),
                threw_exception: true,
                threw_security_exception: true,
            },
        ];

        assert_eq!(control.len(), info.len());

        for (c, t) in control.iter().zip(info.iter()) {
            assert_eq!(c, t);
        }
    }
}
