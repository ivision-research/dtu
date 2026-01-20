use diesel::backend::Backend;
use diesel::deserialize::FromSql;
use diesel::expression::AsExpression;
use diesel::serialize::{Output, ToSql};
use diesel::sql_types::Integer;
use diesel::FromSqlRow;
use serde::{Deserialize, Serialize};
use std::fmt::{Display, Formatter};

use std::ops::{BitAnd, BitOr};

const UB_TRUE: i32 = 1;
const UB_FALSE: i32 = -1;
const UB_UNKNOWN: i32 = 0;

#[repr(i32)]
#[derive(PartialEq, Debug, Clone, Copy, AsExpression, FromSqlRow)]
#[diesel(sql_type = Integer)]
pub enum UnknownBool {
    Unknown = UB_UNKNOWN,
    True = UB_TRUE,
    False = UB_FALSE,
}

impl Serialize for UnknownBool {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let as_num: i32 = (*self).into();
        serializer.serialize_i32(as_num)
    }
}

impl<'de> Deserialize<'de> for UnknownBool {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        Ok(i32::deserialize(deserializer)?.into())
    }
}

impl Display for UnknownBool {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            match self {
                UnknownBool::Unknown => "unknown",
                UnknownBool::True => "true",
                UnknownBool::False => "false",
            }
        )
    }
}

impl Default for UnknownBool {
    fn default() -> Self {
        Self::Unknown
    }
}

impl From<i32> for UnknownBool {
    fn from(value: i32) -> Self {
        if value == UB_UNKNOWN {
            UnknownBool::Unknown
        } else if value == UB_FALSE {
            UnknownBool::False
        } else if value == UB_TRUE {
            UnknownBool::True
        } else {
            UnknownBool::Unknown
        }
    }
}

#[test]
fn test_ub_from_i32() {
    assert_eq!(UnknownBool::from(UB_TRUE), UnknownBool::True);
    assert_eq!(UnknownBool::from(UB_FALSE), UnknownBool::False);
    assert_eq!(UnknownBool::from(UB_UNKNOWN), UnknownBool::Unknown);
    assert_eq!(UnknownBool::from(1000i32), UnknownBool::Unknown);
    assert_eq!(UnknownBool::from(-1000i32), UnknownBool::Unknown);
}

impl From<Option<bool>> for UnknownBool {
    fn from(value: Option<bool>) -> Self {
        match value {
            None => Self::Unknown,
            Some(v) => v.into(),
        }
    }
}

#[test]
fn test_ub_from_op_bool() {
    assert_eq!(UnknownBool::from(Some(true)), UnknownBool::True);
    assert_eq!(UnknownBool::from(Some(false)), UnknownBool::False);
    assert_eq!(UnknownBool::from(None), UnknownBool::Unknown);
}

impl From<bool> for UnknownBool {
    fn from(value: bool) -> Self {
        if value {
            Self::True
        } else {
            Self::False
        }
    }
}

#[test]
fn test_ub_from_bool() {
    assert_eq!(UnknownBool::from(true), UnknownBool::True);
    assert_eq!(UnknownBool::from(false), UnknownBool::False);
}

impl Into<Option<bool>> for UnknownBool {
    fn into(self) -> Option<bool> {
        match self {
            Self::Unknown => None,
            Self::False => Some(false),
            Self::True => Some(true),
        }
    }
}

impl<DB> ToSql<Integer, DB> for UnknownBool
where
    DB: Backend,
    i32: ToSql<Integer, DB>,
{
    fn to_sql<'b>(&'b self, out: &mut Output<'b, '_, DB>) -> diesel::serialize::Result {
        match self {
            Self::True => UB_TRUE.to_sql(out),
            Self::False => UB_FALSE.to_sql(out),
            Self::Unknown => UB_UNKNOWN.to_sql(out),
        }
    }
}

impl<DB> FromSql<Integer, DB> for UnknownBool
where
    DB: Backend,
    i32: FromSql<Integer, DB>,
{
    fn from_sql(bytes: DB::RawValue<'_>) -> diesel::deserialize::Result<Self> {
        let value: i32 = i32::from_sql(bytes)?;
        Ok(Self::from(value))
    }
}

#[test]
fn test_ub_into_op_bool() {
    assert_eq!(Into::<Option<bool>>::into(UnknownBool::Unknown), None);
    assert_eq!(Into::<Option<bool>>::into(UnknownBool::True), Some(true));
    assert_eq!(Into::<Option<bool>>::into(UnknownBool::False), Some(false));
}

impl Into<i32> for UnknownBool {
    fn into(self) -> i32 {
        self.to_numeric()
    }
}

#[test]
fn test_ub_into_i32() {
    assert_eq!(Into::<i32>::into(UnknownBool::Unknown), UB_UNKNOWN);
    assert_eq!(Into::<i32>::into(UnknownBool::True), UB_TRUE);
    assert_eq!(Into::<i32>::into(UnknownBool::False), UB_FALSE);
}

impl UnknownBool {
    pub fn from_numeric(num: i32) -> UnknownBool {
        num.into()
    }

    pub const fn to_numeric(&self) -> i32 {
        match self {
            Self::Unknown => UB_UNKNOWN,
            Self::False => UB_FALSE,
            Self::True => UB_TRUE,
        }
    }

    #[inline]
    pub fn is_known(&self) -> bool {
        *self != Self::Unknown
    }

    #[inline]
    pub fn is_unknown(&self) -> bool {
        *self == Self::Unknown
    }

    #[inline]
    pub fn is_true(&self) -> bool {
        *self == Self::True
    }

    #[inline]
    pub fn is_false(&self) -> bool {
        *self == Self::False
    }

    #[inline]
    pub fn is_false_or_unknown(&self) -> bool {
        self.is_false() || self.is_unknown()
    }

    #[inline]
    pub fn is_true_or_unknown(&self) -> bool {
        self.is_true() || self.is_unknown()
    }

    #[inline]
    pub fn set(&mut self, val: bool) {
        *self = UnknownBool::from(val);
    }

    #[inline]
    pub fn set_false(&mut self) {
        *self = UnknownBool::False;
    }

    #[inline]
    pub fn set_true(&mut self) {
        *self = UnknownBool::True;
    }

    #[inline]
    pub fn set_unknown(&mut self) {
        *self = UnknownBool::Unknown;
    }
}

#[test]
fn test_ub_methods() {
    macro_rules! test_ub_methods {
        ($ub:ident, is_known:$ik:literal, is_true:$it:literal) => {
            assert_eq!(
                UnknownBool::$ub.is_known(),
                $ik,
                concat!("UnknownBool::", stringify!($ub), ".is_known()")
            );
            assert_eq!(
                UnknownBool::$ub.is_unknown(),
                !$ik,
                concat!("UnknownBool::", stringify!($ub), ".is_unknown()",)
            );
            assert_eq!(
                UnknownBool::$ub.is_false(),
                $ik && !$it,
                concat!("UnknownBool::", stringify!($ub), ".is_false()",)
            );
            assert_eq!(
                UnknownBool::$ub.is_true(),
                $ik && $it,
                concat!("UnknownBool::", stringify!($ub), ".is_true()",)
            );
            assert_eq!(
                UnknownBool::$ub.is_false_or_unknown(),
                !$it || !$ik,
                concat!("UnknownBool::", stringify!($ub), ".is_false_or_unknown()",)
            );
            assert_eq!(
                UnknownBool::$ub.is_true_or_unknown(),
                $it || !$ik,
                concat!("UnknownBool::", stringify!($ub), ".is_true_or_unknown()",)
            );
        };
    }

    test_ub_methods!(True, is_known:true, is_true:true);
    test_ub_methods!(False, is_known:true, is_true:false);
    test_ub_methods!(Unknown, is_known:false, is_true:false);
}

impl BitAnd for UnknownBool {
    type Output = Self;

    fn bitand(self, rhs: Self) -> Self::Output {
        match self {
            Self::False => Self::False,
            Self::Unknown => Self::Unknown,
            Self::True => rhs,
        }
    }
}

impl BitOr for UnknownBool {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        match self {
            Self::True => Self::True,
            Self::Unknown => match rhs {
                Self::Unknown => Self::Unknown,
                Self::False => Self::Unknown,
                Self::True => Self::True,
            },
            Self::False => rhs,
        }
    }
}

#[test]
fn test_ub_bitwise() {
    assert_eq!(UnknownBool::True & UnknownBool::True, UnknownBool::True);
    assert_eq!(UnknownBool::True & UnknownBool::False, UnknownBool::False);
    assert_eq!(
        UnknownBool::True & UnknownBool::Unknown,
        UnknownBool::Unknown
    );

    assert_eq!(UnknownBool::False & UnknownBool::True, UnknownBool::False);
    assert_eq!(UnknownBool::False & UnknownBool::False, UnknownBool::False);
    assert_eq!(
        UnknownBool::False & UnknownBool::Unknown,
        UnknownBool::False
    );

    assert_eq!(
        UnknownBool::Unknown & UnknownBool::True,
        UnknownBool::Unknown
    );
    assert_eq!(
        UnknownBool::Unknown & UnknownBool::False,
        UnknownBool::Unknown
    );
    assert_eq!(
        UnknownBool::Unknown & UnknownBool::Unknown,
        UnknownBool::Unknown
    );

    assert_eq!(UnknownBool::True | UnknownBool::True, UnknownBool::True);
    assert_eq!(UnknownBool::True | UnknownBool::False, UnknownBool::True);
    assert_eq!(UnknownBool::True | UnknownBool::Unknown, UnknownBool::True);

    assert_eq!(UnknownBool::False | UnknownBool::True, UnknownBool::True);
    assert_eq!(UnknownBool::False | UnknownBool::False, UnknownBool::False);
    assert_eq!(
        UnknownBool::False | UnknownBool::Unknown,
        UnknownBool::Unknown
    );

    assert_eq!(UnknownBool::Unknown | UnknownBool::True, UnknownBool::True);
    assert_eq!(
        UnknownBool::Unknown | UnknownBool::False,
        UnknownBool::Unknown
    );
    assert_eq!(
        UnknownBool::Unknown | UnknownBool::Unknown,
        UnknownBool::Unknown
    );
}
