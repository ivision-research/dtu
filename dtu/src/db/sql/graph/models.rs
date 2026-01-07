use diesel::prelude::*;
use dtu_proc_macro::sql_db_row;

use super::schema::*;

#[sql_db_row]
#[diesel(table_name = calls)]
pub struct Call {
    pub caller: i32,
    pub callee: i32,
}

#[sql_db_row]
#[diesel(table_name = supers)]
pub struct Super {
    pub parent: i32,
    pub child: i32,
}

#[sql_db_row]
#[diesel(table_name = interfaces)]
pub struct Interface {
    pub interface: i32,
    pub class: i32,
}

#[sql_db_row]
#[diesel(table_name = methods)]
pub struct Method {
    pub id: i32,
    pub class: i32,
    pub name: String,
    pub args: String,
    pub ret: String,
    pub access_flags: i64,
    pub source: i32,
}

#[sql_db_row]
#[diesel(table_name = classes)]
pub struct Class {
    pub id: i32,
    pub name: String,
    pub access_flags: i64,
    pub source: i32,
}

#[sql_db_row]
#[diesel(table_name = sources)]
pub struct Source {
    pub id: i32,
    pub name: String,
}

#[sql_db_row]
#[diesel(table_name = _load_status)]
pub struct LoadStatus {
    pub source: i32,
    pub kind: i32,
}
