//! Provedor PostgreSQL do Atlas (tokio-postgres).

mod apply;
mod connect;
mod error;
mod exec;
mod meta;

pub use apply::{PlannedStatement, apply_in_transaction};
pub use connect::{PgSession, connect};
pub use error::map_error;
pub use exec::{execute, execute_in_schema, quote_ident, search_path_for, select_sql, stream_rows, table_count, table_rows};
pub use meta::{list_databases, list_objects, list_roles, list_schemas, table_detail, view_detail};
