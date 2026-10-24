//! Provedor PostgreSQL do Atlas (tokio-postgres).

mod apply;
mod dump;
mod find;
mod import;
mod transfer;
mod connect;
mod console;
mod datagen;
mod datasync;
mod error;
mod exec;
mod history;
mod logged;
mod meta;
mod monitor;
mod structsync;

pub use apply::{PlannedStatement, apply_in_open_transaction, apply_in_transaction, begin_manual_transaction, end_manual_transaction};
pub use connect::{PgSession, connect};
pub use error::map_error;
pub use exec::{
    execute, execute_in_schema, is_single_statement, quote_ident, search_path_for, select_sql, stream_rows, table_count, table_rows, wrap_query,
};
pub use dump::{DumpOptions, DumpPlan, DumpScope, DumpTable, begin_snapshot, copy_header, copy_table, data_select, end_snapshot, header as dump_header, plan as dump_plan};
pub use console::{Completion as ConsoleCompletion, ConsoleOptions, ConsoleOutput, complete as console_complete, is_complete as console_is_complete, run as console_run};
pub use datagen::{ColumnMeta as GenColumnMeta, ColumnSpec as GenColumnSpec, GenHooks, GenOptions, GenSpec, GenStats, Generator, Preview as GenPreview, TableMeta as GenTableMeta, TableSpec as GenTableSpec, TextStyle, describe as describe_generation, preview as preview_generation, run as run_generation};
pub use datasync::{CompareHooks, Comparison, DeployHooks, DeployStats, DiffKind, DiffRow, SyncSpec, SyncTable, TableDiff, TableSelection, compare as compare_data, deploy as deploy_data, script as data_script};
pub use history::{Entry as HistoryEntry, clear as history_clear, is_paused as history_is_paused, read as history_read, set_paused as history_set_paused};
pub use monitor::{ProcessRow as MonitorProcess, Snapshot as MonitorSnapshot, end_process as monitor_end_process, snapshot as monitor_snapshot};
pub use structsync::{Op as StructOp, ObjectDiff as StructObject, Part as StructPart, StructComparison, StructSelection, StructSpec, compare as compare_structure, script as structure_script};
pub use transfer::{FieldMap, Hooks as TransferHooks, TableOverride, TransferSpec, TransferStats, rewrite_schema, run as run_transfer};
pub use import::{Feed, Hooks as ImportHooks, ImportMode, ImportSpec, ImportStats, TargetColumn, run as run_import};
pub use find::{FindHit, FindOptions, FindResult, find};
pub use meta::{
    available_extensions, database_detail, extension_detail, foreign_server_detail, foreign_table_detail, foreign_wrappers, function_detail, schema_diagram, list_databases, list_objects, list_roles, list_schemas, matview_detail, role_detail,
    schema_detail, sequence_detail, table_detail, type_detail, view_detail,
};
