//! Comandos do Firebird: espelham os `pg_*` que a interface usa (mesmos argumentos e formatos de resposta) com o prefixo `firebird_`.
//! A interface escolhe o prefixo pelo id da conexão (`firebird_...`), então as telas não mudam.
//! O Firebird (até a versão 4) não tem schemas: o arquivo aparece como o banco (pelo nome do arquivo) com um schema `main`.

use crate::commands::{TABLE_ROWS_CAP, TableRows};
use atlas_core::{ConnectionParams, DatabaseError, DatabaseList, ErrorKind, OrderBy, QueryOutcome, RoleInfo, SchemaInfo, SchemaObjects, ServerInfo, TableDetail, TableFilter};
use atlas_firebird::{FirebirdSession, FirebirdTx, PlannedStatement, Target};
use crate::local::Local;
use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tauri::{Emitter, State};

const MAX_ROWS_CAP: usize = 100_000;
/// O Firebird não tem schemas: tudo fica num só, com este nome.
const MAIN: &str = "main";

#[derive(Default)]
pub struct FirebirdSessions {
    sessions: Mutex<HashMap<String, Arc<FirebirdSession>>>,
    txs: Mutex<HashMap<String, Arc<FirebirdTx>>>,
    next_tx: AtomicU64,
    /// Túneis SSH das conexões abertas (vivem enquanto a conexão existir).
    tunnels: Mutex<HashMap<String, atlas_ssh::Tunnel>>,
    /// Operações longas em andamento (id → pedido de parada).
    finds: Mutex<HashMap<String, Arc<AtomicBool>>>,
}

impl FirebirdSessions {
    /// Registra o pedido de parada de uma operação longa (geração, transferência) pelo id dela.
    pub(crate) fn register_cancel(&self, id: &str, flag: Arc<AtomicBool>) {
        if let Ok(mut m) = self.finds.lock() {
            m.insert(id.to_string(), flag);
        }
    }

    pub(crate) fn unregister_cancel(&self, id: &str) {
        if let Ok(mut m) = self.finds.lock() {
            m.remove(id);
        }
    }

    pub(crate) fn session(&self, id: &str) -> Result<Arc<FirebirdSession>, DatabaseError> {
        self.sessions.lock().ok().and_then(|m| m.get(id).cloned()).ok_or_else(|| DatabaseError::new(ErrorKind::SessionNotFound, "connection is not open"))
    }

    fn tx(&self, id: &str) -> Result<Arc<FirebirdTx>, DatabaseError> {
        self.txs.lock().ok().and_then(|m| m.get(id).cloned()).ok_or_else(|| DatabaseError::new(ErrorKind::SessionNotFound, "transaction is not open"))
    }
}

/// O cliente do Firebird é síncrono: roda numa thread própria para não travar a interface.
async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T, DatabaseError> + Send + 'static) -> Result<T, DatabaseError> {
    tokio::task::spawn_blocking(f).await.map_err(|e| DatabaseError::new(ErrorKind::Internal, e.to_string()))?
}

/// O nome que a árvore mostra para o banco: o arquivo sem pasta e sem extensão (ou o apelido).
fn display_name(database: &str) -> String {
    let file = database.rsplit(['/', '\\']).next().unwrap_or(database);
    file.rsplit_once('.').map_or(file, |(stem, _)| stem).to_string()
}

#[tauri::command]
pub async fn firebird_test_connection(local: State<'_, Local>, params: ConnectionParams, profile_id: Option<String>) -> Result<ServerInfo, DatabaseError> {
    let started = Instant::now();
    let mut params = local.resolve_password(params, profile_id.as_deref())?;
    // O túnel só vive durante o teste.
    let _tunnel = crate::tunnel::start(&local, &mut params).await?;
    params.ssh = None;
    blocking(move || {
        let s = FirebirdSession::open(Target::from_params(&params))?;
        let mut info = s.server_info();
        info.latency_ms = started.elapsed().as_millis() as u64;
        Ok(info)
    })
    .await
}

#[tauri::command]
pub async fn firebird_open(
    app: tauri::AppHandle,
    local: State<'_, Local>,
    sessions: State<'_, FirebirdSessions>,
    session_id: String,
    params: ConnectionParams,
    profile_id: Option<String>,
) -> Result<ServerInfo, DatabaseError> {
    let started = Instant::now();
    // A senha guardada no cofre do sistema e o túnel SSH, como nos outros bancos.
    let mut params = local.resolve_password(params, profile_id.as_deref())?;
    let tunnel = crate::tunnel::start(&local, &mut params).await?;
    params.ssh = None;
    let session = Arc::new(blocking(move || FirebirdSession::open(Target::from_params(&params))).await?);
    if let (Some(t), Ok(mut map)) = (tunnel, sessions.tunnels.lock()) {
        map.insert(session_id.clone(), t);
    }
    let mut info = session.server_info();
    info.latency_ms = started.elapsed().as_millis() as u64;
    if let Ok(mut map) = sessions.sessions.lock() {
        map.insert(session_id.clone(), session);
    }
    let _ = app.emit("session-opened", serde_json::json!({ "id": session_id, "version": info.version, "versionNum": info.version_num }));
    Ok(info)
}

#[tauri::command]
pub async fn firebird_close(sessions: State<'_, FirebirdSessions>, session_id: String) -> Result<(), DatabaseError> {
    if let Ok(mut map) = sessions.sessions.lock() {
        map.remove(&session_id);
    }
    if let Ok(mut map) = sessions.tunnels.lock() {
        map.remove(&session_id);
    }
    Ok(())
}

#[tauri::command]
pub async fn firebird_close_database(_session_id: String, _database: String) -> Result<(), DatabaseError> {
    Ok(())
}

#[tauri::command]
pub async fn firebird_list_databases(sessions: State<'_, FirebirdSessions>, session_id: String, _include_hidden: bool) -> Result<DatabaseList, DatabaseError> {
    let s = sessions.session(&session_id)?;
    Ok(DatabaseList { databases: vec![display_name(&s.target.database)], hidden: 0, comments: BTreeMap::new() })
}

#[tauri::command]
pub async fn firebird_list_schemas(sessions: State<'_, FirebirdSessions>, session_id: String, _database: String, _include_hidden: bool) -> Result<Vec<SchemaInfo>, DatabaseError> {
    sessions.session(&session_id)?;
    Ok(vec![SchemaInfo { name: MAIN.to_string(), comment: None }])
}

#[tauri::command]
pub async fn firebird_list_objects(sessions: State<'_, FirebirdSessions>, session_id: String, _database: String, _schema: String) -> Result<SchemaObjects, DatabaseError> {
    let s = sessions.session(&session_id)?;
    blocking(move || s.list_objects()).await
}

#[tauri::command]
pub async fn firebird_table_detail(sessions: State<'_, FirebirdSessions>, session_id: String, _database: String, _schema: String, table: String) -> Result<TableDetail, DatabaseError> {
    let s = sessions.session(&session_id)?;
    blocking(move || s.table_detail(&table)).await
}

#[tauri::command]
pub async fn firebird_view_detail(sessions: State<'_, FirebirdSessions>, session_id: String, _database: String, _schema: String, view: String) -> Result<atlas_core::ViewDetail, DatabaseError> {
    let s = sessions.session(&session_id)?;
    blocking(move || s.view_detail(&view)).await
}

#[tauri::command]
pub async fn firebird_list_roles(_session_id: String) -> Result<Vec<RoleInfo>, DatabaseError> {
    Ok(Vec::new())
}

#[tauri::command]
pub async fn firebird_execute(
    sessions: State<'_, FirebirdSessions>,
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
pub async fn firebird_cancel(sessions: State<'_, FirebirdSessions>, session_id: String, _database: String) -> Result<(), DatabaseError> {
    let s = sessions.session(&session_id)?;
    blocking(move || s.cancel()).await
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn firebird_table_rows(
    sessions: State<'_, FirebirdSessions>,
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
pub async fn firebird_table_count(
    sessions: State<'_, FirebirdSessions>,
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
pub async fn firebird_apply(sessions: State<'_, FirebirdSessions>, session_id: String, _database: String, statements: Vec<PlannedStatement>) -> Result<Vec<u64>, DatabaseError> {
    let s = sessions.session(&session_id)?;
    blocking(move || s.apply(&statements)).await
}

/// Tudo que o Firebird não tem (extensões, tablespaces...): a interface recebe "indisponível".
#[tauri::command]
pub async fn firebird_unsupported(what: String) -> Result<(), DatabaseError> {
    Err(atlas_firebird::unsupported_feature(&what))
}

/* ---------- Transação manual ---------- */

#[tauri::command]
pub async fn firebird_tx_begin(sessions: State<'_, FirebirdSessions>, session_id: String, _database: String) -> Result<String, DatabaseError> {
    let s = sessions.session(&session_id)?;
    let tx = Arc::new(blocking(move || s.begin()).await?);
    let id = format!("firebird_tx{}", sessions.next_tx.fetch_add(1, Ordering::Relaxed) + 1);
    if let Ok(mut map) = sessions.txs.lock() {
        map.insert(id.clone(), tx);
    }
    Ok(id)
}

#[tauri::command]
pub async fn firebird_tx_apply(sessions: State<'_, FirebirdSessions>, tx_id: String, statements: Vec<PlannedStatement>) -> Result<Vec<u64>, DatabaseError> {
    let tx = sessions.tx(&tx_id)?;
    blocking(move || tx.apply(&statements)).await
}

#[tauri::command]
pub async fn firebird_tx_execute(sessions: State<'_, FirebirdSessions>, tx_id: String, sql: String, max_rows: usize, _schema: Option<String>) -> Result<QueryOutcome, DatabaseError> {
    let tx = sessions.tx(&tx_id)?;
    blocking(move || tx.execute(&sql, max_rows.clamp(1, MAX_ROWS_CAP))).await
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn firebird_tx_table_rows(
    sessions: State<'_, FirebirdSessions>,
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
pub async fn firebird_tx_table_count(sessions: State<'_, FirebirdSessions>, tx_id: String, _schema: String, table: String, filter: Option<TableFilter>) -> Result<u64, DatabaseError> {
    let tx = sessions.tx(&tx_id)?;
    blocking(move || tx.table_count(&table, filter.as_ref())).await
}

#[tauri::command]
pub async fn firebird_tx_end(sessions: State<'_, FirebirdSessions>, tx_id: String, commit: bool) -> Result<(), DatabaseError> {
    let tx = sessions.tx(&tx_id)?;
    let result = blocking(move || tx.finish(commit)).await;
    if let Ok(mut map) = sessions.txs.lock() {
        map.remove(&tx_id);
    }
    result
}
