//! Comandos do MySQL. Espelham os `pg_*` que as telas usam (mesmos argumentos e mesmos formatos de resposta), com o prefixo
//! `mysql_`; o roteador da interface (`providerRoute.ts`) manda a chamada para cá quando o id da conexão começa por `mysql_`.
//! Os comandos do PostgreSQL não são tocados.

use crate::commands::{TABLE_ROWS_CAP, TableRows};
use crate::local::Local;
use atlas_core::{ConnectionParams, DatabaseError, DatabaseList, ErrorKind, OrderBy, QueryOutcome, RoleInfo, SchemaInfo, SchemaObjects, ServerInfo, TableDetail, TableFilter};
use atlas_mysql::{MysqlSession, MysqlTx, PlannedStatement};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tauri::{Emitter, State};

/// Linhas máximas de uma consulta na grade.
const MAX_ROWS_CAP: usize = 100_000;

#[derive(Default)]
pub struct MysqlSessions {
    sessions: Mutex<HashMap<String, Arc<MysqlSession>>>,
    txs: Mutex<HashMap<String, Arc<MysqlTx>>>,
    next_tx: AtomicU64,
    /// Túneis SSH das conexões abertas (vivem enquanto a conexão existir).
    tunnels: Mutex<HashMap<String, atlas_ssh::Tunnel>>,
}

impl MysqlSessions {
    fn session(&self, id: &str) -> Result<Arc<MysqlSession>, DatabaseError> {
        self.sessions.lock().ok().and_then(|m| m.get(id).cloned()).ok_or_else(|| DatabaseError::new(ErrorKind::SessionNotFound, "connection is not open"))
    }

    fn tx(&self, id: &str) -> Result<Arc<MysqlTx>, DatabaseError> {
        self.txs.lock().ok().and_then(|m| m.get(id).cloned()).ok_or_else(|| DatabaseError::new(ErrorKind::SessionNotFound, "transaction is not open"))
    }
}

#[tauri::command]
pub async fn mysql_test_connection(local: State<'_, Local>, params: ConnectionParams, profile_id: Option<String>) -> Result<ServerInfo, DatabaseError> {
    let started = Instant::now();
    let mut params = local.resolve_password(params, profile_id.as_deref())?;
    // O túnel só vive durante o teste.
    let _tunnel = crate::tunnel::start(&local, &mut params).await?;
    params.ssh = None;
    let s = MysqlSession::open(params).await?;
    Ok(s.server_info(started.elapsed().as_millis() as u64))
}

#[tauri::command]
pub async fn mysql_open(
    app: tauri::AppHandle,
    local: State<'_, Local>,
    sessions: State<'_, MysqlSessions>,
    session_id: String,
    params: ConnectionParams,
    profile_id: Option<String>,
) -> Result<ServerInfo, DatabaseError> {
    let started = Instant::now();
    // A senha guardada no cofre do sistema e o túnel SSH, como no PostgreSQL.
    let mut params = local.resolve_password(params, profile_id.as_deref())?;
    let tunnel = crate::tunnel::start(&local, &mut params).await?;
    params.ssh = None;
    let session = Arc::new(MysqlSession::open(params).await?);
    if let (Some(t), Ok(mut map)) = (tunnel, sessions.tunnels.lock()) {
        map.insert(session_id.clone(), t);
    }
    let info = session.server_info(started.elapsed().as_millis() as u64);
    if let Ok(mut map) = sessions.sessions.lock() {
        map.insert(session_id.clone(), session);
    }
    let _ = app.emit("session-opened", serde_json::json!({ "id": session_id, "version": info.version, "versionNum": info.version_num }));
    Ok(info)
}

#[tauri::command]
pub async fn mysql_close(sessions: State<'_, MysqlSessions>, session_id: String) -> Result<(), DatabaseError> {
    if let Ok(mut map) = sessions.sessions.lock() {
        map.remove(&session_id);
    }
    if let Ok(mut map) = sessions.tunnels.lock() {
        map.remove(&session_id);
    }
    Ok(())
}

#[tauri::command]
pub async fn mysql_close_database(sessions: State<'_, MysqlSessions>, session_id: String, database: String) -> Result<(), DatabaseError> {
    sessions.session(&session_id)?.close_database(&database).await;
    Ok(())
}

#[tauri::command]
pub async fn mysql_list_databases(sessions: State<'_, MysqlSessions>, session_id: String, include_hidden: bool) -> Result<DatabaseList, DatabaseError> {
    sessions.session(&session_id)?.list_databases(include_hidden).await
}

#[tauri::command]
pub async fn mysql_list_schemas(sessions: State<'_, MysqlSessions>, session_id: String, database: String, _include_hidden: bool) -> Result<Vec<SchemaInfo>, DatabaseError> {
    Ok(sessions.session(&session_id)?.list_schemas(&database).await)
}

#[tauri::command]
pub async fn mysql_list_objects(sessions: State<'_, MysqlSessions>, session_id: String, database: String, _schema: String) -> Result<SchemaObjects, DatabaseError> {
    sessions.session(&session_id)?.list_objects(&database).await
}

#[tauri::command]
pub async fn mysql_table_detail(sessions: State<'_, MysqlSessions>, session_id: String, database: String, _schema: String, table: String) -> Result<TableDetail, DatabaseError> {
    sessions.session(&session_id)?.table_detail(&database, &table).await
}

#[tauri::command]
pub async fn mysql_list_roles(sessions: State<'_, MysqlSessions>, session_id: String) -> Result<Vec<RoleInfo>, DatabaseError> {
    sessions.session(&session_id)?.list_roles().await
}

#[tauri::command]
pub async fn mysql_execute(
    sessions: State<'_, MysqlSessions>,
    session_id: String,
    database: String,
    sql: String,
    max_rows: usize,
    _schema: Option<String>,
) -> Result<QueryOutcome, DatabaseError> {
    sessions.session(&session_id)?.execute(&database, &sql, max_rows.clamp(1, MAX_ROWS_CAP)).await
}

/// Botão Parar: `KILL QUERY` da consulta em andamento.
#[tauri::command]
pub async fn mysql_cancel(sessions: State<'_, MysqlSessions>, session_id: String, database: String) -> Result<(), DatabaseError> {
    sessions.session(&session_id)?.cancel(&database).await
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn mysql_table_rows(
    sessions: State<'_, MysqlSessions>,
    session_id: String,
    database: String,
    _schema: String,
    table: String,
    order_by: Vec<OrderBy>,
    filter: Option<TableFilter>,
    limit: Option<u32>,
    offset: u64,
) -> Result<TableRows, DatabaseError> {
    let s = sessions.session(&session_id)?;
    let limit = limit.map(|l| l.clamp(1, TABLE_ROWS_CAP as u32));
    let (sql, outcome) = s.table_rows(&database, &table, &order_by, filter.as_ref(), limit, offset, TABLE_ROWS_CAP).await?;
    Ok(TableRows { sql, outcome })
}

#[tauri::command]
pub async fn mysql_table_count(sessions: State<'_, MysqlSessions>, session_id: String, database: String, _schema: String, table: String, filter: Option<TableFilter>) -> Result<u64, DatabaseError> {
    sessions.session(&session_id)?.table_count(&database, &table, filter.as_ref()).await
}

#[tauri::command]
pub async fn mysql_apply(sessions: State<'_, MysqlSessions>, session_id: String, database: String, statements: Vec<PlannedStatement>) -> Result<Vec<u64>, DatabaseError> {
    sessions.session(&session_id)?.apply(&database, &statements).await
}

/// Rótulos de chaves estrangeiras.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn mysql_fk_labels(
    sessions: State<'_, MysqlSessions>,
    session_id: String,
    database: String,
    _schema: String,
    table: String,
    key_column: String,
    keys: Vec<String>,
) -> Result<atlas_mysql::FkLabels, DatabaseError> {
    sessions.session(&session_id)?.fk_labels(&database, &table, &key_column, &keys).await
}

/// Tabelas (com colunas) e chaves estrangeiras de um banco, para o diagrama ER.
#[tauri::command]
pub async fn mysql_schema_diagram(sessions: State<'_, MysqlSessions>, session_id: String, database: String, _schema: String) -> Result<atlas_core::SchemaDiagram, DatabaseError> {
    sessions.session(&session_id)?.schema_diagram(&database).await
}

/// Definição de uma view para o designer.
#[tauri::command]
pub async fn mysql_view_detail(sessions: State<'_, MysqlSessions>, session_id: String, database: String, _schema: String, view: String) -> Result<atlas_core::ViewDetail, DatabaseError> {
    sessions.session(&session_id)?.view_detail(&database, &view).await
}

/// Tudo que o MySQL ainda não tem nesta interface: a tela recebe "indisponível".
#[tauri::command]
pub async fn mysql_unsupported(what: String) -> Result<(), DatabaseError> {
    Err(DatabaseError::new(ErrorKind::UnsupportedFeature, format!("{what} is not available for MySQL yet")))
}

/* ---------- Transação manual ---------- */

#[tauri::command]
pub async fn mysql_tx_begin(sessions: State<'_, MysqlSessions>, session_id: String, database: String) -> Result<String, DatabaseError> {
    let s = sessions.session(&session_id)?;
    let tx = Arc::new(s.begin(&database).await?);
    let id = format!("mysql_tx{}", sessions.next_tx.fetch_add(1, Ordering::Relaxed) + 1);
    if let Ok(mut map) = sessions.txs.lock() {
        map.insert(id.clone(), tx);
    }
    Ok(id)
}

#[tauri::command]
pub async fn mysql_tx_apply(sessions: State<'_, MysqlSessions>, tx_id: String, statements: Vec<PlannedStatement>) -> Result<Vec<u64>, DatabaseError> {
    sessions.tx(&tx_id)?.apply(&statements).await
}

#[tauri::command]
pub async fn mysql_tx_execute(sessions: State<'_, MysqlSessions>, tx_id: String, sql: String, max_rows: usize, _schema: Option<String>) -> Result<QueryOutcome, DatabaseError> {
    sessions.tx(&tx_id)?.execute(&sql, max_rows.clamp(1, MAX_ROWS_CAP)).await
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn mysql_tx_table_rows(
    sessions: State<'_, MysqlSessions>,
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
    // No MySQL o schema é o próprio banco.
    let (sql, outcome) = tx.table_rows(&schema, &table, &order_by, filter.as_ref(), limit, offset, TABLE_ROWS_CAP).await?;
    Ok(TableRows { sql, outcome })
}

#[tauri::command]
pub async fn mysql_tx_table_count(sessions: State<'_, MysqlSessions>, tx_id: String, schema: String, table: String, filter: Option<TableFilter>) -> Result<u64, DatabaseError> {
    sessions.tx(&tx_id)?.table_count(&schema, &table, filter.as_ref()).await
}

#[tauri::command]
pub async fn mysql_tx_end(sessions: State<'_, MysqlSessions>, tx_id: String, commit: bool) -> Result<(), DatabaseError> {
    let tx = sessions.tx(&tx_id)?;
    let result = tx.finish(commit).await;
    if let Ok(mut map) = sessions.txs.lock() {
        map.remove(&tx_id);
    }
    result
}
