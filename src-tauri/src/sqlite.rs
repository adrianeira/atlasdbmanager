//! Comandos do SQLite: espelham os `pg_*` que a interface usa (mesmos argumentos e formatos de resposta) com o prefixo `sqlite_`.
//! A interface escolhe o prefixo pelo id da conexão (`sqlite_...`), então as telas não mudam.

use crate::commands::{TABLE_ROWS_CAP, TableRows};
use crate::export::{ExportRequest, ExportResult, PartFile, Progress, io_error, part_path, validate_path};
use atlas_export::ExportColumn;
use std::fs::{self, File};
use std::time::{Duration, Instant};
use atlas_core::{ConnectionParams, DatabaseError, DatabaseList, ErrorKind, OrderBy, QueryOutcome, RoleInfo, SchemaInfo, SchemaObjects, ServerInfo, TableDetail, TableFilter};
use atlas_sqlite::{PlannedStatement, SqliteSession, SqliteTx};
use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
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
    /// Buscas nos dados em andamento (id da busca → pedido de parada).
    finds: Mutex<HashMap<String, Arc<AtomicBool>>>,
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

/// Tabelas (com colunas) e chaves estrangeiras, para o diagrama ER.
#[tauri::command]
pub async fn sqlite_schema_diagram(sessions: State<'_, SqliteSessions>, session_id: String, _database: String, _schema: String) -> Result<atlas_core::SchemaDiagram, DatabaseError> {
    let s = sessions.session(&session_id)?;
    blocking(move || s.schema_diagram()).await
}

/// "Find in Database", Look In = Structure.
#[tauri::command]
pub async fn sqlite_find(sessions: State<'_, SqliteSessions>, session_id: String, _database: String, options: atlas_sqlite::FindOptions) -> Result<atlas_sqlite::FindResult, DatabaseError> {
    let s = sessions.session(&session_id)?;
    blocking(move || s.find(&options)).await
}

/// "Find in Database", Look In = Data. O botão Parar (`sqlite_export_cancel` com o mesmo `find_id`) interrompe entre uma tabela e outra.
#[tauri::command]
pub async fn sqlite_find_data(
    sessions: State<'_, SqliteSessions>,
    session_id: String,
    _database: String,
    find_id: String,
    options: atlas_sqlite::FindOptions,
) -> Result<atlas_sqlite::FindDataResult, DatabaseError> {
    let s = sessions.session(&session_id)?;
    let stop = Arc::new(AtomicBool::new(false));
    if let Ok(mut m) = sessions.finds.lock() {
        m.insert(find_id.clone(), stop.clone());
    }
    let flag = stop.clone();
    let out = blocking(move || s.find_data(&options, &move || flag.load(Ordering::Relaxed))).await;
    if let Ok(mut m) = sessions.finds.lock() {
        m.remove(&find_id);
    }
    out
}

/// O tipo declarado no SQLite com o nome que os gravadores de exportação conhecem (os do PostgreSQL): números saem como número,
/// booleanos como `true`/`false`, JSON como JSON.
fn export_type(decl: &str) -> String {
    let d = decl.to_ascii_lowercase();
    let has = |words: &[&str]| words.iter().any(|w| d.contains(w));
    let name = if has(&["bool"]) {
        "bool"
    } else if has(&["int"]) {
        "int8"
    } else if has(&["real", "floa", "doub"]) {
        "float8"
    } else if has(&["numeric", "decimal"]) {
        "numeric"
    } else if has(&["json"]) {
        "json"
    } else if d == "date" {
        "date"
    } else if has(&["datetime", "timestamp"]) {
        "timestamp"
    } else if has(&["blob"]) {
        "bytea"
    } else {
        "text"
    };
    name.to_string()
}

/// Exporta uma tabela, view ou consulta para arquivo (CSV, JSON, SQL, Excel...), em streaming e numa conexão só de leitura.
#[tauri::command]
pub async fn sqlite_export(app: tauri::AppHandle, sessions: State<'_, SqliteSessions>, req: ExportRequest) -> Result<ExportResult, DatabaseError> {
    let started = Instant::now();
    let dest = validate_path(&req.path)?;
    let sql = match &req.query {
        Some(q) => {
            if q.sql.trim().is_empty() || !atlas_pg::is_single_statement(&q.sql) {
                return Err(DatabaseError::new(ErrorKind::InvalidInput, "a exportação aceita uma única consulta (um SELECT): selecione só o comando desejado"));
            }
            atlas_pg::wrap_query(&q.sql, &req.columns)
        }
        None => atlas_sqlite::select_columns_sql(&req.table, &req.columns, &req.order_by, req.filter.as_ref()),
    };
    let s = sessions.session(&req.session_id)?;
    let cancel = Arc::new(AtomicBool::new(false));
    if let Ok(mut m) = sessions.finds.lock() {
        m.insert(req.export_id.clone(), cancel.clone());
    }
    let id = req.export_id.clone();
    let cleanup_id = id.clone();
    let task = tokio::task::spawn_blocking(move || -> Result<ExportResult, DatabaseError> {
        let mut part = PartFile::new(part_path(&dest));
        let file = File::create(&part.path).map_err(io_error)?;
        let target = atlas_pg::quote_ident(&req.table);
        let writer = Mutex::new(atlas_export::create(&req.format, &atlas_export::WriterTarget { sql: &target, title: &req.table }, file).map_err(io_error)?);
        let began = AtomicBool::new(false);
        let mut last_emit = Instant::now();
        let mut rows = 0u64;
        let streamed = atlas_sqlite::stream_rows(
            &s.path,
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
                    let _ = app.emit("export-progress", Progress { export_id: &id, rows });
                }
                Ok(true)
            },
        );
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
        let _ = app.emit("export-progress", Progress { export_id: &id, rows });
        Ok(ExportResult { rows, bytes: fs::metadata(&dest).map(|m| m.len()).unwrap_or(0), elapsed_ms: started.elapsed().as_millis() as u64, path: dest.to_string_lossy().into_owned() })
    });
    let out = task.await.map_err(|e| DatabaseError::new(ErrorKind::Internal, e.to_string()))?;
    if let Ok(mut m) = sessions.finds.lock() {
        m.remove(&cleanup_id);
    }
    out
}

/// Pede para parar uma busca nos dados.
#[tauri::command]
pub async fn sqlite_export_cancel(sessions: State<'_, SqliteSessions>, export_id: String) -> Result<(), DatabaseError> {
    if let Some(stop) = sessions.finds.lock().ok().and_then(|m| m.get(&export_id).cloned()) {
        stop.store(true, Ordering::Relaxed);
    }
    Ok(())
}

/// Perfil da tabela: tamanhos, contagens da estrutura e total de linhas.
#[tauri::command]
pub async fn sqlite_table_profile(sessions: State<'_, SqliteSessions>, session_id: String, _database: String, _schema: String, table: String) -> Result<atlas_core::TableProfile, DatabaseError> {
    let s = sessions.session(&session_id)?;
    blocking(move || s.table_profile(&table)).await
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
