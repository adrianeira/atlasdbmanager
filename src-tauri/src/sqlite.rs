//! Comandos do SQLite: espelham os `pg_*` que a interface usa (mesmos argumentos e formatos de resposta) com o prefixo `sqlite_`.
//! A interface escolhe o prefixo pelo id da conexão (`sqlite_...`), então as telas não mudam.

use crate::commands::{TABLE_ROWS_CAP, TableRows};
use atlas_core::{ConnectionParams, DatabaseError, DatabaseList, ErrorKind, OrderBy, QueryOutcome, RoleInfo, SchemaInfo, SchemaObjects, ServerInfo, TableDetail, TableFilter};
use atlas_sqlite::{PlannedStatement, SqliteSession, SqliteTx};
use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tauri::{Emitter, State};

const MAX_ROWS_CAP: usize = 100_000;
/// O SQLite não tem bancos nem schemas como o PostgreSQL: o arquivo aparece como o banco `main`, com um schema `main`.
const MAIN: &str = "main";

#[derive(Default)]
pub struct SqliteSessions {
    sessions: Mutex<HashMap<String, Arc<SqliteSession>>>,
    txs: Mutex<HashMap<String, Arc<SqliteTx>>>,
    next_tx: AtomicU64,
}

impl SqliteSessions {
    fn session(&self, id: &str) -> Result<Arc<SqliteSession>, DatabaseError> {
        self.sessions.lock().ok().and_then(|m| m.get(id).cloned()).ok_or_else(|| DatabaseError::new(ErrorKind::SessionNotFound, "connection is not open"))
    }

    fn tx(&self, id: &str) -> Result<Arc<SqliteTx>, DatabaseError> {
        self.txs.lock().ok().and_then(|m| m.get(id).cloned()).ok_or_else(|| DatabaseError::new(ErrorKind::SessionNotFound, "transaction is not open"))
    }
}

/// O SQLite é síncrono: roda numa thread própria para não travar a interface.
async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T, DatabaseError> + Send + 'static) -> Result<T, DatabaseError> {
    tokio::task::spawn_blocking(f).await.map_err(|e| DatabaseError::new(ErrorKind::Internal, e.to_string()))?
}

#[tauri::command]
pub async fn sqlite_test_connection(params: ConnectionParams, _profile_id: Option<String>) -> Result<ServerInfo, DatabaseError> {
    blocking(move || Ok(SqliteSession::open(&params.host)?.server_info())).await
}

#[tauri::command]
pub async fn sqlite_open(
    app: tauri::AppHandle,
    sessions: State<'_, SqliteSessions>,
    session_id: String,
    params: ConnectionParams,
    _profile_id: Option<String>,
) -> Result<ServerInfo, DatabaseError> {
    let session = Arc::new(blocking(move || SqliteSession::open(&params.host)).await?);
    let info = session.server_info();
    if let Ok(mut map) = sessions.sessions.lock() {
        map.insert(session_id.clone(), session);
    }
    // Avisa as outras janelas, como o PostgreSQL faz.
    let _ = app.emit("session-opened", serde_json::json!({ "id": session_id, "version": info.version, "versionNum": info.version_num }));
    Ok(info)
}

#[tauri::command]
pub async fn sqlite_close(sessions: State<'_, SqliteSessions>, session_id: String) -> Result<(), DatabaseError> {
    if let Ok(mut map) = sessions.sessions.lock() {
        map.remove(&session_id);
    }
    Ok(())
}

#[tauri::command]
pub async fn sqlite_close_database(_session_id: String, _database: String) -> Result<(), DatabaseError> {
    Ok(())
}

#[tauri::command]
pub async fn sqlite_list_databases(sessions: State<'_, SqliteSessions>, session_id: String, _include_hidden: bool) -> Result<DatabaseList, DatabaseError> {
    sessions.session(&session_id)?;
    Ok(DatabaseList { databases: vec![MAIN.to_string()], hidden: 0, comments: BTreeMap::new() })
}

#[tauri::command]
pub async fn sqlite_list_schemas(sessions: State<'_, SqliteSessions>, session_id: String, _database: String, _include_hidden: bool) -> Result<Vec<SchemaInfo>, DatabaseError> {
    sessions.session(&session_id)?;
    Ok(vec![SchemaInfo { name: MAIN.to_string(), comment: None }])
}

#[tauri::command]
pub async fn sqlite_list_objects(sessions: State<'_, SqliteSessions>, session_id: String, _database: String, _schema: String) -> Result<SchemaObjects, DatabaseError> {
    let s = sessions.session(&session_id)?;
    blocking(move || s.list_objects()).await
}

#[tauri::command]
pub async fn sqlite_table_detail(sessions: State<'_, SqliteSessions>, session_id: String, _database: String, _schema: String, table: String) -> Result<TableDetail, DatabaseError> {
    let s = sessions.session(&session_id)?;
    blocking(move || s.table_detail(&table)).await
}

#[tauri::command]
pub async fn sqlite_list_roles(_session_id: String) -> Result<Vec<RoleInfo>, DatabaseError> {
    Ok(Vec::new())
}

#[tauri::command]
pub async fn sqlite_execute(
    sessions: State<'_, SqliteSessions>,
    session_id: String,
    _database: String,
    sql: String,
    max_rows: usize,
    _schema: Option<String>,
) -> Result<QueryOutcome, DatabaseError> {
    let s = sessions.session(&session_id)?;
    blocking(move || s.execute(&sql, max_rows.clamp(1, MAX_ROWS_CAP))).await
}

#[tauri::command]
pub async fn sqlite_cancel(sessions: State<'_, SqliteSessions>, session_id: String, _database: String) -> Result<(), DatabaseError> {
    sessions.session(&session_id)?.cancel();
    Ok(())
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn sqlite_table_rows(
    sessions: State<'_, SqliteSessions>,
    session_id: String,
    _database: String,
    _schema: String,
    table: String,
    order_by: Vec<OrderBy>,
    filter: Option<TableFilter>,
    limit: Option<u32>,
    offset: u64,
) -> Result<TableRows, DatabaseError> {
    let s = sessions.session(&session_id)?;
    let limit = limit.map(|l| l.clamp(1, TABLE_ROWS_CAP as u32));
    let (sql, outcome) = blocking(move || s.table_rows(&table, &order_by, filter.as_ref(), limit, offset, TABLE_ROWS_CAP)).await?;
    Ok(TableRows { sql, outcome })
}

#[tauri::command]
pub async fn sqlite_table_count(
    sessions: State<'_, SqliteSessions>,
    session_id: String,
    _database: String,
    _schema: String,
    table: String,
    filter: Option<TableFilter>,
) -> Result<u64, DatabaseError> {
    let s = sessions.session(&session_id)?;
    blocking(move || s.table_count(&table, filter.as_ref())).await
}

#[tauri::command]
pub async fn sqlite_apply(sessions: State<'_, SqliteSessions>, session_id: String, _database: String, statements: Vec<PlannedStatement>) -> Result<Vec<u64>, DatabaseError> {
    let s = sessions.session(&session_id)?;
    blocking(move || s.apply(&statements)).await
}

/// Data Profiling: estatísticas de cada coluna sobre os registros que o filtro, a ordenação e o limite escolhem.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn sqlite_data_profile(
    sessions: State<'_, SqliteSessions>,
    session_id: String,
    _database: String,
    _schema: String,
    table: String,
    columns: Vec<String>,
    order_by: Vec<OrderBy>,
    filter: Option<TableFilter>,
    limit: Option<u64>,
) -> Result<atlas_sqlite::DataProfile, DatabaseError> {
    let s = sessions.session(&session_id)?;
    blocking(move || s.data_profile(&table, &columns, &order_by, filter.as_ref(), limit)).await
}

/// Rótulos de chaves estrangeiras visíveis no grid ("mostrar o valor da chave estrangeira").
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn sqlite_fk_labels(
    sessions: State<'_, SqliteSessions>,
    session_id: String,
    _database: String,
    _schema: String,
    table: String,
    key_column: String,
    keys: Vec<String>,
) -> Result<atlas_sqlite::FkLabels, DatabaseError> {
    let s = sessions.session(&session_id)?;
    blocking(move || s.fk_labels(&table, &key_column, &keys)).await
}

/// Tudo que o SQLite não tem (extensões, tablespaces, backup do PostgreSQL...): a interface recebe "indisponível".
#[tauri::command]
pub async fn sqlite_unsupported(what: String) -> Result<(), DatabaseError> {
    Err(atlas_sqlite::unsupported_feature(&what))
}

/* ---------- Transação manual ---------- */

#[tauri::command]
pub async fn sqlite_tx_begin(sessions: State<'_, SqliteSessions>, session_id: String, _database: String) -> Result<String, DatabaseError> {
    let s = sessions.session(&session_id)?;
    let tx = Arc::new(blocking(move || s.begin()).await?);
    let id = format!("sqlite_tx{}", sessions.next_tx.fetch_add(1, Ordering::Relaxed) + 1);
    if let Ok(mut map) = sessions.txs.lock() {
        map.insert(id.clone(), tx);
    }
    Ok(id)
}

#[tauri::command]
pub async fn sqlite_tx_apply(sessions: State<'_, SqliteSessions>, tx_id: String, statements: Vec<PlannedStatement>) -> Result<Vec<u64>, DatabaseError> {
    let tx = sessions.tx(&tx_id)?;
    blocking(move || tx.apply(&statements)).await
}

#[tauri::command]
pub async fn sqlite_tx_execute(sessions: State<'_, SqliteSessions>, tx_id: String, sql: String, max_rows: usize, _schema: Option<String>) -> Result<QueryOutcome, DatabaseError> {
    let tx = sessions.tx(&tx_id)?;
    blocking(move || tx.execute(&sql, max_rows.clamp(1, MAX_ROWS_CAP))).await
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn sqlite_tx_table_rows(
    sessions: State<'_, SqliteSessions>,
    tx_id: String,
    _schema: String,
    table: String,
    order_by: Vec<OrderBy>,
    filter: Option<TableFilter>,
    limit: Option<u32>,
    offset: u64,
) -> Result<TableRows, DatabaseError> {
    let tx = sessions.tx(&tx_id)?;
    let limit = limit.map(|l| l.clamp(1, TABLE_ROWS_CAP as u32));
    let (sql, outcome) = blocking(move || tx.table_rows(&table, &order_by, filter.as_ref(), limit, offset, TABLE_ROWS_CAP)).await?;
    Ok(TableRows { sql, outcome })
}

#[tauri::command]
pub async fn sqlite_tx_table_count(sessions: State<'_, SqliteSessions>, tx_id: String, _schema: String, table: String, filter: Option<TableFilter>) -> Result<u64, DatabaseError> {
    let tx = sessions.tx(&tx_id)?;
    blocking(move || tx.table_count(&table, filter.as_ref())).await
}

#[tauri::command]
pub async fn sqlite_tx_end(sessions: State<'_, SqliteSessions>, tx_id: String, commit: bool) -> Result<(), DatabaseError> {
    let tx = sessions.tx(&tx_id)?;
    let result = blocking(move || tx.finish(commit)).await;
    if let Ok(mut map) = sessions.txs.lock() {
        map.remove(&tx_id);
    }
    result
}
