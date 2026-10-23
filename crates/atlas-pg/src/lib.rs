//! Provedor PostgreSQL do Atlas (tokio-postgres).

mod apply;
mod find;
mod connect;
mod error;
mod exec;
mod meta;

pub use apply::{PlannedStatement, apply_in_open_transaction, apply_in_transaction, begin_manual_transaction, end_manual_transaction};
pub use connect::{PgSession, connect};
pub use error::map_error;
pub use exec::{
    execute, execute_in_schema, is_single_statement, quote_ident, search_path_for, select_sql, stream_rows, table_count, table_rows, wrap_query,
};
pub use find::{FindHit, FindOptions, FindResult, find};
pub use meta::{role_detail, type_detail, sequence_detail, function_detail, list_databases, list_objects, list_roles, list_schemas, matview_detail, table_detail, view_detail};
