//! Comandos do Oracle. Espelham os `pg_*` que as telas usam (mesmos argumentos e mesmos formatos de resposta), com o prefixo
//! `oracle_`; o roteador da interface (`providerRoute.ts`) manda a chamada para cá quando o id da conexão começa por `oracle_`.
//! Os comandos do PostgreSQL não são tocados. No Oracle o "banco" é o serviço da conexão e os schemas são os usuários.

use crate::commands::{TABLE_ROWS_CAP, TableRows};
use crate::local::Local;
use atlas_core::{ConnectionParams, DatabaseError, DatabaseList, ErrorKind, OrderBy, QueryOutcome, RoleInfo, SchemaInfo, SchemaObjects, ServerInfo, TableDetail, TableFilter};
use atlas_oracle::{OracleSession, OracleTx, PlannedStatement};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tauri::{Emitter, State};

/// Linhas máximas de uma consulta na grade.
const MAX_ROWS_CAP: usize = 100_000;

#[derive(Default)]
pub struct OracleSessions {
    sessions: Mutex<HashMap<String, Arc<OracleSession>>>,
    txs: Mutex<HashMap<String, Arc<OracleTx>>>,
    next_tx: AtomicU64,
    /// Túneis SSH das conexões abertas (vivem enquanto a conexão existir).
    tunnels: Mutex<HashMap<String, atlas_ssh::Tunnel>>,
}

impl OracleSessions {
    fn session(&self, id: &str) -> Result<Arc<OracleSession>, DatabaseError> {
        self.sessions.lock().ok().and_then(|m| m.get(id).cloned()).ok_or_else(|| DatabaseError::new(ErrorKind::SessionNotFound, "connection is not open"))
    }

    fn tx(&self, id: &str) -> Result<Arc<OracleTx>, DatabaseError> {
        self.txs.lock().ok().and_then(|m| m.get(id).cloned()).ok_or_else(|| DatabaseError::new(ErrorKind::SessionNotFound, "transaction is not open"))
    }
}

#[tauri::command]
pub async fn oracle_test_connection(local: State<'_, Local>, params: ConnectionParams, profile_id: Option<String>) -> Result<ServerInfo, DatabaseError> {
    let started = Instant::now();
    let mut params = local.resolve_password(params, profile_id.as_deref())?;
    // O túnel só vive durante o teste.
    let _tunnel = crate::tunnel::start(&local, &mut params).await?;
    params.ssh = None;
    let s = OracleSession::open(params).await?;
    let info = s.server_info(started.elapsed().as_millis() as u64);
    s.close().await;
    Ok(info)
}

#[tauri::command]
pub async fn oracle_open(
    app: tauri::AppHandle,
    local: State<'_, Local>,
    sessions: State<'_, OracleSessions>,
    session_id: String,
    params: ConnectionParams,
    profile_id: Option<String>,
) -> Result<ServerInfo, DatabaseError> {
    let started = Instant::now();
    // A senha guardada no cofre do sistema e o túnel SSH, como nos outros bancos.
    let mut params = local.resolve_password(params, profile_id.as_deref())?;
    let tunnel = crate::tunnel::start(&local, &mut params).await?;
    params.ssh = None;
    let session = Arc::new(OracleSession::open(params).await?);
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
pub async fn oracle_close(sessions: State<'_, OracleSessions>, session_id: String) -> Result<(), DatabaseError> {
    let session = sessions.sessions.lock().ok().and_then(|mut m| m.remove(&session_id));
    if let Some(s) = session {
        s.close().await;
    }
    if let Ok(mut map) = sessions.tunnels.lock() {
        map.remove(&session_id);
    }
    Ok(())
}

/// A conexão é uma só (o serviço); não há o que fechar por banco.
#[tauri::command]
pub async fn oracle_close_database(_session_id: String, _database: String) -> Result<(), DatabaseError> {
    Ok(())
}

#[tauri::command]
pub async fn oracle_list_databases(sessions: State<'_, OracleSessions>, session_id: String, include_hidden: bool) -> Result<DatabaseList, DatabaseError> {
    sessions.session(&session_id)?.list_databases(include_hidden).await
}

#[tauri::command]
pub async fn oracle_list_schemas(sessions: State<'_, OracleSessions>, session_id: String, _database: String, include_hidden: bool) -> Result<Vec<SchemaInfo>, DatabaseError> {
    sessions.session(&session_id)?.list_schemas(include_hidden).await
}

#[tauri::command]
pub async fn oracle_list_objects(sessions: State<'_, OracleSessions>, session_id: String, _database: String, schema: String) -> Result<SchemaObjects, DatabaseError> {
    sessions.session(&session_id)?.list_objects(&schema).await
}

#[tauri::command]
pub async fn oracle_table_detail(sessions: State<'_, OracleSessions>, session_id: String, _database: String, schema: String, table: String) -> Result<TableDetail, DatabaseError> {
    sessions.session(&session_id)?.table_detail(&schema, &table).await
}

#[tauri::command]
pub async fn oracle_view_detail(sessions: State<'_, OracleSessions>, session_id: String, _database: String, schema: String, view: String) -> Result<atlas_core::ViewDetail, DatabaseError> {
    sessions.session(&session_id)?.view_detail(&schema, &view).await
}

#[tauri::command]
pub async fn oracle_schema_diagram(sessions: State<'_, OracleSessions>, session_id: String, _database: String, schema: String) -> Result<atlas_core::SchemaDiagram, DatabaseError> {
    sessions.session(&session_id)?.schema_diagram(&schema).await
}

#[tauri::command]
pub async fn oracle_list_roles(sessions: State<'_, OracleSessions>, session_id: String) -> Result<Vec<RoleInfo>, DatabaseError> {
    sessions.session(&session_id)?.list_roles().await
}

#[tauri::command]
pub async fn oracle_execute(
    sessions: State<'_, OracleSessions>,
    session_id: String,
    _database: String,
    sql: String,
    max_rows: usize,
    schema: Option<String>,
) -> Result<QueryOutcome, DatabaseError> {
    sessions.session(&session_id)?.execute(schema.as_deref().unwrap_or(""), &sql, max_rows.clamp(1, MAX_ROWS_CAP)).await
}

/// Botão Parar: `ALTER SYSTEM CANCEL SQL` da consulta em andamento (exige o privilégio `ALTER SYSTEM`).
#[tauri::command]
pub async fn oracle_cancel(sessions: State<'_, OracleSessions>, session_id: String, _database: String) -> Result<(), DatabaseError> {
    sessions.session(&session_id)?.cancel().await
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn oracle_table_rows(
    sessions: State<'_, OracleSessions>,
    session_id: String,
    _database: String,
    schema: String,
    table: String,
    order_by: Vec<OrderBy>,
    filter: Option<TableFilter>,
    limit: Option<u32>,
    offset: u64,
) -> Result<TableRows, DatabaseError> {
    let s = sessions.session(&session_id)?;
    let limit = limit.map(|l| l.clamp(1, TABLE_ROWS_CAP as u32));
    let (sql, outcome) = s.table_rows(&schema, &table, &order_by, filter.as_ref(), limit, offset, TABLE_ROWS_CAP).await?;
    Ok(TableRows { sql, outcome })
}

#[tauri::command]
pub async fn oracle_table_count(sessions: State<'_, OracleSessions>, session_id: String, _database: String, schema: String, table: String, filter: Option<TableFilter>) -> Result<u64, DatabaseError> {
    sessions.session(&session_id)?.table_count(&schema, &table, filter.as_ref()).await
}

#[tauri::command]
pub async fn oracle_apply(sessions: State<'_, OracleSessions>, session_id: String, _database: String, statements: Vec<PlannedStatement>) -> Result<Vec<u64>, DatabaseError> {
    sessions.session(&session_id)?.apply(&statements).await
}

/// Rótulos de chaves estrangeiras: ainda sem suporte, a grade mostra a chave mesmo.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn oracle_fk_labels(
    _session_id: String,
    _database: String,
    _schema: String,
    _table: String,
    _key_column: String,
    _keys: Vec<String>,
) -> Result<serde_json::Value, DatabaseError> {
    Ok(serde_json::json!({ "labelColumn": null, "labels": [] }))
}

/// Tudo que o Oracle ainda não tem nesta interface: a tela recebe "indisponível".
#[tauri::command]
pub async fn oracle_unsupported(what: String) -> Result<(), DatabaseError> {
    Err(atlas_oracle::unsupported_feature(&what))
}

/* ---------- Transação manual ---------- */

#[tauri::command]
pub async fn oracle_tx_begin(sessions: State<'_, OracleSessions>, session_id: String, _database: String) -> Result<String, DatabaseError> {
    let s = sessions.session(&session_id)?;
    let tx = Arc::new(s.begin().await?);
    let id = format!("oracle_tx{}", sessions.next_tx.fetch_add(1, Ordering::Relaxed) + 1);
    if let Ok(mut map) = sessions.txs.lock() {
        map.insert(id.clone(), tx);
    }
    Ok(id)
}

#[tauri::command]
pub async fn oracle_tx_apply(sessions: State<'_, OracleSessions>, tx_id: String, statements: Vec<PlannedStatement>) -> Result<Vec<u64>, DatabaseError> {
    sessions.tx(&tx_id)?.apply(&statements).await
}

#[tauri::command]
pub async fn oracle_tx_execute(sessions: State<'_, OracleSessions>, tx_id: String, sql: String, max_rows: usize, schema: Option<String>) -> Result<QueryOutcome, DatabaseError> {
    sessions.tx(&tx_id)?.execute(schema.as_deref().unwrap_or(""), &sql, max_rows.clamp(1, MAX_ROWS_CAP)).await
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn oracle_tx_table_rows(
    sessions: State<'_, OracleSessions>,
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
pub async fn oracle_tx_table_count(sessions: State<'_, OracleSessions>, tx_id: String, schema: String, table: String, filter: Option<TableFilter>) -> Result<u64, DatabaseError> {
    sessions.tx(&tx_id)?.table_count(&schema, &table, filter.as_ref()).await
}

#[tauri::command]
pub async fn oracle_tx_end(sessions: State<'_, OracleSessions>, tx_id: String, commit: bool) -> Result<(), DatabaseError> {
    let tx = sessions.tx(&tx_id)?;
    let result = tx.finish(commit).await;
    if let Ok(mut map) = sessions.txs.lock() {
        map.remove(&tx_id);
    }
    result
}
