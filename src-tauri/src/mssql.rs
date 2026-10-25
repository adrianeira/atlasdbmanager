//! Comandos do SQL Server. Espelham os `pg_*` que as telas usam (mesmos argumentos e mesmos formatos de resposta), com o prefixo
//! `mssql_`; o roteador da interface (`providerRoute.ts`) manda a chamada para cá quando o id da conexão começa por `mssql_`.
//! Os comandos do PostgreSQL não são tocados.

use crate::commands::{TABLE_ROWS_CAP, TableRows};
use atlas_core::{ConnectionParams, DatabaseError, DatabaseList, ErrorKind, OrderBy, QueryOutcome, RoleInfo, SchemaInfo, SchemaObjects, ServerInfo, TableDetail, TableFilter};
use atlas_mssql::{MssqlSession, MssqlTx, PlannedStatement};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tauri::{Emitter, State};

/// Linhas máximas de uma consulta na grade.
const MAX_ROWS_CAP: usize = 100_000;

#[derive(Default)]
pub struct MssqlSessions {
    sessions: Mutex<HashMap<String, Arc<MssqlSession>>>,
    txs: Mutex<HashMap<String, Arc<MssqlTx>>>,
    next_tx: AtomicU64,
    /// Buscas nos dados em andamento (id da busca → pedido de parada).
    finds: Mutex<HashMap<String, Arc<AtomicBool>>>,
}

impl MssqlSessions {
    fn session(&self, id: &str) -> Result<Arc<MssqlSession>, DatabaseError> {
        self.sessions.lock().ok().and_then(|m| m.get(id).cloned()).ok_or_else(|| DatabaseError::new(ErrorKind::SessionNotFound, "connection is not open"))
    }

    fn tx(&self, id: &str) -> Result<Arc<MssqlTx>, DatabaseError> {
        self.txs.lock().ok().and_then(|m| m.get(id).cloned()).ok_or_else(|| DatabaseError::new(ErrorKind::SessionNotFound, "transaction is not open"))
    }
}

#[tauri::command]
pub async fn mssql_test_connection(params: ConnectionParams, _profile_id: Option<String>) -> Result<ServerInfo, DatabaseError> {
    let started = Instant::now();
    let s = MssqlSession::open(params).await?;
    Ok(s.server_info(started.elapsed().as_millis() as u64))
}

#[tauri::command]
pub async fn mssql_open(
    app: tauri::AppHandle,
    sessions: State<'_, MssqlSessions>,
    session_id: String,
    params: ConnectionParams,
    _profile_id: Option<String>,
) -> Result<ServerInfo, DatabaseError> {
    let started = Instant::now();
    let session = Arc::new(MssqlSession::open(params).await?);
    let info = session.server_info(started.elapsed().as_millis() as u64);
    if let Ok(mut map) = sessions.sessions.lock() {
        map.insert(session_id.clone(), session);
    }
    let _ = app.emit("session-opened", serde_json::json!({ "id": session_id, "version": info.version, "versionNum": info.version_num }));
    Ok(info)
}

#[tauri::command]
pub async fn mssql_close(sessions: State<'_, MssqlSessions>, session_id: String) -> Result<(), DatabaseError> {
    if let Ok(mut map) = sessions.sessions.lock() {
        map.remove(&session_id);
    }
    Ok(())
}

#[tauri::command]
pub async fn mssql_close_database(sessions: State<'_, MssqlSessions>, session_id: String, database: String) -> Result<(), DatabaseError> {
    sessions.session(&session_id)?.close_database(&database).await;
    Ok(())
}

#[tauri::command]
pub async fn mssql_list_databases(sessions: State<'_, MssqlSessions>, session_id: String, include_hidden: bool) -> Result<DatabaseList, DatabaseError> {
    sessions.session(&session_id)?.list_databases(include_hidden).await
}

#[tauri::command]
pub async fn mssql_list_schemas(sessions: State<'_, MssqlSessions>, session_id: String, database: String, include_hidden: bool) -> Result<Vec<SchemaInfo>, DatabaseError> {
    sessions.session(&session_id)?.list_schemas(&database, include_hidden).await
}

#[tauri::command]
pub async fn mssql_list_objects(sessions: State<'_, MssqlSessions>, session_id: String, database: String, schema: String) -> Result<SchemaObjects, DatabaseError> {
    sessions.session(&session_id)?.list_objects(&database, &schema).await
}

#[tauri::command]
pub async fn mssql_table_detail(sessions: State<'_, MssqlSessions>, session_id: String, database: String, schema: String, table: String) -> Result<TableDetail, DatabaseError> {
    sessions.session(&session_id)?.table_detail(&database, &schema, &table).await
}

#[tauri::command]
pub async fn mssql_list_roles(sessions: State<'_, MssqlSessions>, session_id: String) -> Result<Vec<RoleInfo>, DatabaseError> {
    sessions.session(&session_id)?.list_roles().await
}

#[tauri::command]
pub async fn mssql_execute(
    sessions: State<'_, MssqlSessions>,
    session_id: String,
    database: String,
    sql: String,
    max_rows: usize,
    _schema: Option<String>,
) -> Result<QueryOutcome, DatabaseError> {
    sessions.session(&session_id)?.execute(&database, &sql, max_rows.clamp(1, MAX_ROWS_CAP)).await
}

/// O TDS cancela com um pacote de atenção que o driver ainda não expõe: por enquanto o botão Parar não interrompe.
#[tauri::command]
pub async fn mssql_cancel(sessions: State<'_, MssqlSessions>, session_id: String, _database: String) -> Result<(), DatabaseError> {
    sessions.session(&session_id)?;
    Ok(())
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn mssql_table_rows(
    sessions: State<'_, MssqlSessions>,
    session_id: String,
    database: String,
    schema: String,
    table: String,
    order_by: Vec<OrderBy>,
    filter: Option<TableFilter>,
    limit: Option<u32>,
    offset: u64,
) -> Result<TableRows, DatabaseError> {
    let s = sessions.session(&session_id)?;
    let limit = limit.map(|l| l.clamp(1, TABLE_ROWS_CAP as u32));
    let (sql, outcome) = s.table_rows(&database, &schema, &table, &order_by, filter.as_ref(), limit, offset, TABLE_ROWS_CAP).await?;
    Ok(TableRows { sql, outcome })
}

#[tauri::command]
pub async fn mssql_table_count(
    sessions: State<'_, MssqlSessions>,
    session_id: String,
    database: String,
    schema: String,
    table: String,
    filter: Option<TableFilter>,
) -> Result<u64, DatabaseError> {
    sessions.session(&session_id)?.table_count(&database, &schema, &table, filter.as_ref()).await
}

#[tauri::command]
pub async fn mssql_apply(sessions: State<'_, MssqlSessions>, session_id: String, database: String, statements: Vec<PlannedStatement>) -> Result<Vec<u64>, DatabaseError> {
    sessions.session(&session_id)?.apply(&database, &statements).await
}

/// Rótulos das chaves estrangeiras visíveis no grid ("mostrar o valor da chave estrangeira").
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn mssql_fk_labels(
    sessions: State<'_, MssqlSessions>,
    session_id: String,
    database: String,
    schema: String,
    table: String,
    key_column: String,
    keys: Vec<String>,
) -> Result<atlas_mssql::FkLabels, DatabaseError> {
    sessions.session(&session_id)?.fk_labels(&database, &schema, &table, &key_column, &keys).await
}

/// Tabelas (com colunas) e chaves estrangeiras de um schema, para o diagrama ER.
#[tauri::command]
pub async fn mssql_schema_diagram(sessions: State<'_, MssqlSessions>, session_id: String, database: String, schema: String) -> Result<atlas_core::SchemaDiagram, DatabaseError> {
    sessions.session(&session_id)?.schema_diagram(&database, &schema).await
}

/// "Find in Database", Look In = Structure.
#[tauri::command]
pub async fn mssql_find(sessions: State<'_, MssqlSessions>, session_id: String, database: String, options: atlas_mssql::FindOptions) -> Result<atlas_mssql::FindResult, DatabaseError> {
    sessions.session(&session_id)?.find(&database, &options).await
}

/// "Find in Database", Look In = Data. O botão Parar (`mssql_export_cancel` com o mesmo `find_id`) interrompe entre uma tabela e outra.
#[tauri::command]
pub async fn mssql_find_data(
    sessions: State<'_, MssqlSessions>,
    session_id: String,
    database: String,
    find_id: String,
    options: atlas_mssql::FindOptions,
) -> Result<atlas_mssql::FindDataResult, DatabaseError> {
    let s = sessions.session(&session_id)?;
    let stop = Arc::new(AtomicBool::new(false));
    if let Ok(mut m) = sessions.finds.lock() {
        m.insert(find_id.clone(), stop.clone());
    }
    let flag = stop.clone();
    let out = s.find_data(&database, &options, &move || flag.load(Ordering::Relaxed)).await;
    if let Ok(mut m) = sessions.finds.lock() {
        m.remove(&find_id);
    }
    out
}

/// Pede para parar uma busca nos dados.
#[tauri::command]
pub async fn mssql_export_cancel(sessions: State<'_, MssqlSessions>, export_id: String) -> Result<(), DatabaseError> {
    if let Some(stop) = sessions.finds.lock().ok().and_then(|m| m.get(&export_id).cloned()) {
        stop.store(true, Ordering::Relaxed);
    }
    Ok(())
}

/// Tudo que o SQL Server ainda não tem nesta interface: a tela recebe "indisponível".
#[tauri::command]
pub async fn mssql_unsupported(what: String) -> Result<(), DatabaseError> {
    Err(DatabaseError::new(ErrorKind::UnsupportedFeature, format!("{what} is not available for SQL Server yet")))
}

/* ---------- Transação manual ---------- */

#[tauri::command]
pub async fn mssql_tx_begin(sessions: State<'_, MssqlSessions>, session_id: String, database: String) -> Result<String, DatabaseError> {
    let s = sessions.session(&session_id)?;
    let tx = Arc::new(s.begin(&database).await?);
    let id = format!("mssql_tx{}", sessions.next_tx.fetch_add(1, Ordering::Relaxed) + 1);
    if let Ok(mut map) = sessions.txs.lock() {
        map.insert(id.clone(), tx);
    }
    Ok(id)
}

#[tauri::command]
pub async fn mssql_tx_apply(sessions: State<'_, MssqlSessions>, tx_id: String, statements: Vec<PlannedStatement>) -> Result<Vec<u64>, DatabaseError> {
    sessions.tx(&tx_id)?.apply(&statements).await
}

#[tauri::command]
pub async fn mssql_tx_execute(sessions: State<'_, MssqlSessions>, tx_id: String, sql: String, max_rows: usize, _schema: Option<String>) -> Result<QueryOutcome, DatabaseError> {
    sessions.tx(&tx_id)?.execute(&sql, max_rows.clamp(1, MAX_ROWS_CAP)).await
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn mssql_tx_table_rows(
    sessions: State<'_, MssqlSessions>,
    tx_id: String,
    schema: String,
    table: String,
    order_by: Vec<OrderBy>,
    filter: Option<TableFilter>,
    limit: Option<u32>,
    offset: u64,
) -> Result<TableRows, DatabaseError> {
    let tx = sessions.tx(&tx_id)?;
    let limit = limit.map(|l| l.clamp(1, TABLE_ROWS_CAP as u32));
    let (sql, outcome) = tx.table_rows(&schema, &table, &order_by, filter.as_ref(), limit, offset, TABLE_ROWS_CAP).await?;
    Ok(TableRows { sql, outcome })
}

#[tauri::command]
pub async fn mssql_tx_table_count(sessions: State<'_, MssqlSessions>, tx_id: String, schema: String, table: String, filter: Option<TableFilter>) -> Result<u64, DatabaseError> {
    sessions.tx(&tx_id)?.table_count(&schema, &table, filter.as_ref()).await
}

#[tauri::command]
pub async fn mssql_tx_end(sessions: State<'_, MssqlSessions>, tx_id: String, commit: bool) -> Result<(), DatabaseError> {
    let tx = sessions.tx(&tx_id)?;
    let result = tx.finish(commit).await;
    if let Ok(mut map) = sessions.txs.lock() {
        map.remove(&tx_id);
    }
    result
}
