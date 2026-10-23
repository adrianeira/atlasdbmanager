//! Provedor PostgreSQL do Atlas (tokio-postgres).

mod apply;
mod dump;
mod find;
mod import;
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
pub use dump::{DumpOptions, DumpPlan, DumpScope, DumpTable, begin_snapshot, copy_header, copy_table, data_select, end_snapshot, header as dump_header, plan as dump_plan};
pub use import::{Feed, Hooks as ImportHooks, ImportMode, ImportSpec, ImportStats, TargetColumn, run as run_import};
pub use find::{FindHit, FindOptions, FindResult, find};
pub use meta::{
    available_extensions, database_detail, extension_detail, function_detail, list_databases, list_objects, list_roles, list_schemas, matview_detail, role_detail,
    schema_detail, sequence_detail, table_detail, type_detail, view_detail,
};
