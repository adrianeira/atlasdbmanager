//! Comandos do MySQL. Espelham os `pg_*` que as telas usam (mesmos argumentos e mesmos formatos de resposta), com o prefixo
//! `mysql_`; o roteador da interface (`providerRoute.ts`) manda a chamada para cá quando o id da conexão começa por `mysql_`.
//! Os comandos do PostgreSQL não são tocados.

use crate::commands::{TABLE_ROWS_CAP, TableRows};
use crate::export::{ExportRequest, ExportResult, PartFile, Progress, io_error, part_path, validate_path};
use crate::local::Local;
use atlas_export::ExportColumn;
use std::fs::{self, File};
use std::sync::atomic::AtomicBool;
use std::time::Duration;
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
    /// Exportações e buscas em andamento (id → pedido de parada).
    finds: Mutex<HashMap<String, Arc<AtomicBool>>>,
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

/// O tipo do MySQL com o nome que os gravadores de exportação conhecem (os do PostgreSQL).
fn export_type(t: &str) -> String {
    match t.to_ascii_lowercase().as_str() {
        "tinyint" | "smallint" | "mediumint" | "int" | "bigint" | "year" => "int8",
        "decimal" => "numeric",
        "float" | "double" => "float8",
        "bit" => "bool",
        "date" => "date",
        "datetime" | "timestamp" => "timestamp",
        "time" => "time",
        "json" => "json",
        "blob" | "binary" | "varbinary" => "bytea",
        _ => "text",
    }
    .to_string()
}

/// Exporta uma tabela, view ou consulta para arquivo (CSV, JSON, SQL, Excel...), em streaming e numa conexão só da exportação.
#[tauri::command]
pub async fn mysql_export(app: tauri::AppHandle, sessions: State<'_, MysqlSessions>, req: ExportRequest) -> Result<ExportResult, DatabaseError> {
    let started = Instant::now();
    let dest = validate_path(&req.path)?;
    // No MySQL o schema é o próprio banco.
    let database = if req.schema.is_empty() { req.database.clone() } else { req.schema.clone() };
    let sql = match &req.query {
        Some(q) => {
            if q.sql.trim().is_empty() || !atlas_pg::is_single_statement(&q.sql) {
                return Err(DatabaseError::new(ErrorKind::InvalidInput, "a exportação aceita uma única consulta (um SELECT): selecione só o comando desejado"));
            }
            let body = q.sql.trim().trim_end_matches(';').trim_end().to_string();
            if req.columns.is_empty() {
                body
            } else {
                format!("SELECT {} FROM (\n{body}\n) AS atlas_export", req.columns.iter().map(|c| atlas_mysql::quote_ident(c)).collect::<Vec<_>>().join(", "))
            }
        }
        None => atlas_mysql::select_columns_sql(&database, &req.table, &req.columns, &req.order_by, req.filter.as_ref()),
    };
    let s = sessions.session(&req.session_id)?;
    let cancel = Arc::new(AtomicBool::new(false));
    if let Ok(mut m) = sessions.finds.lock() {
        m.insert(req.export_id.clone(), cancel.clone());
    }
    let out = async {
        let mut part = PartFile::new(part_path(&dest));
        let file = File::create(&part.path).map_err(io_error)?;
        let target = format!("{}.{}", atlas_mysql::quote_ident(&database), atlas_mysql::quote_ident(&req.table));
        let writer = Mutex::new(atlas_export::create(&req.format, &atlas_export::WriterTarget { sql: &target, title: &req.table }, file).map_err(io_error)?);
        let began = AtomicBool::new(false);
        let mut last_emit = Instant::now();
        let mut rows = 0u64;
        let streamed = s
            .stream_rows(
                &req.database,
                &sql,
                |cols| {
                    let cols: Vec<ExportColumn> = cols.iter().map(|c| ExportColumn { name: c.name.clone(), type_name: c.type_name.as_deref().map(export_type) }).collect();
                    began.store(true, Ordering::Relaxed);
                    writer.lock().unwrap().begin(&cols).map_err(io_error)
                },
                |values| {
                    if cancel.load(Ordering::Relaxed) {
                        return Ok(false);
                    }
                    writer.lock().unwrap().row(values).map_err(io_error)?;
                    rows += 1;
                    if last_emit.elapsed() >= Duration::from_millis(200) {
                        last_emit = Instant::now();
                        let _ = app.emit("export-progress", Progress { export_id: &req.export_id, rows });
                    }
                    Ok(true)
                },
            )
            .await;
        if cancel.load(Ordering::Relaxed) {
            return Err(DatabaseError::new(ErrorKind::Canceled, "export canceled"));
        }
        streamed?;
        if !began.load(Ordering::Relaxed) {
            return Err(DatabaseError::new(ErrorKind::InvalidInput, "o comando não retorna linhas para exportar"));
        }
        writer.into_inner().unwrap().finish().map_err(io_error)?;
        fs::rename(&part.path, &dest).map_err(io_error)?;
        part.keep = true;
        let _ = app.emit("export-progress", Progress { export_id: &req.export_id, rows });
        Ok(ExportResult { rows, bytes: fs::metadata(&dest).map(|m| m.len()).unwrap_or(0), elapsed_ms: started.elapsed().as_millis() as u64, path: dest.to_string_lossy().into_owned() })
    }
    .await;
    if let Ok(mut m) = sessions.finds.lock() {
        m.remove(&req.export_id);
    }
    out
}

/// Pede para parar uma exportação.
#[tauri::command]
pub async fn mysql_export_cancel(sessions: State<'_, MysqlSessions>, export_id: String) -> Result<(), DatabaseError> {
    if let Some(stop) = sessions.finds.lock().ok().and_then(|m| m.get(&export_id).cloned()) {
        stop.store(true, Ordering::Relaxed);
    }
    Ok(())
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
