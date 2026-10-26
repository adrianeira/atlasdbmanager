//! Comandos do Firebird: espelham os `pg_*` que a interface usa (mesmos argumentos e formatos de resposta) com o prefixo `firebird_`.
//! A interface escolhe o prefixo pelo id da conexão (`firebird_...`), então as telas não mudam.
//! O Firebird (até a versão 4) não tem schemas: o arquivo aparece como o banco (pelo nome do arquivo) com um schema `main`.

use crate::commands::{TABLE_ROWS_CAP, TableRows};
use crate::backup::{BackupEntry, BackupRequest, BackupResult, LogLine, RestoreRequest, Sidecar, safe_segment, store_dir};
use crate::dump::{DumpRequest, DumpResult};
use crate::export::{ExportRequest, ExportResult, PartFile, Progress, io_error, part_path, validate_path};
use crate::import::{ImportRequest, ImportSummary, JobResult, LogEvent, ProgressEvent, failed_job};
use atlas_export::ExportColumn;
use atlas_firebird::{ImportFeed, ImportHooks, ImportMode, ImportSpec, TargetColumn, declared_type};
use atlas_import::convert::{TargetKind, convert, kind_of};
use std::fs::{self, File};
use std::time::Duration;
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

/// Cria um banco novo no servidor (o arquivo não pode existir; a pasta é a do servidor).
#[tauri::command]
pub async fn firebird_create_database(local: State<'_, Local>, params: ConnectionParams, profile_id: Option<String>) -> Result<(), DatabaseError> {
    let mut params = local.resolve_password(params, profile_id.as_deref())?;
    let _tunnel = crate::tunnel::start(&local, &mut params).await?;
    params.ssh = None;
    blocking(move || atlas_firebird::create_database(&Target::from_params(&params), None)).await
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
pub async fn firebird_routine_definition(sessions: State<'_, FirebirdSessions>, session_id: String, _database: String, _schema: String, name: String) -> Result<String, DatabaseError> {
    let s = sessions.session(&session_id)?;
    blocking(move || s.routine_definition(&name)).await
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

/* ---------- Exportar e importar ---------- */

/// O tipo do Firebird com o nome que os gravadores de exportação conhecem (os do PostgreSQL).
fn export_type(name: &str) -> String {
    let d = name.to_ascii_lowercase();
    let t = match d.as_str() {
        "smallint" => "int2",
        "integer" => "int4",
        "bigint" | "int128" => "int8",
        "float" => "float4",
        "double precision" => "float8",
        "numeric" | "decimal" => "numeric",
        "boolean" => "bool",
        "date" => "date",
        "time" => "time",
        "timestamp" => "timestamp",
        "blob" => "bytea",
        _ => "text",
    };
    t.to_string()
}

/// Exporta uma tabela, view ou consulta para arquivo (CSV, JSON, SQL, Excel...), em streaming e numa conexão própria.
#[tauri::command]
pub async fn firebird_export(app: tauri::AppHandle, sessions: State<'_, FirebirdSessions>, req: ExportRequest) -> Result<ExportResult, DatabaseError> {
    let started = Instant::now();
    let dest = validate_path(&req.path)?;
    let sql = match &req.query {
        Some(q) => {
            if q.sql.trim().is_empty() || atlas_firebird::split_statements(&q.sql).len() != 1 {
                return Err(DatabaseError::new(ErrorKind::InvalidInput, "a exportação aceita uma única consulta (um SELECT): selecione só o comando desejado"));
            }
            let list = if req.columns.is_empty() { "*".to_string() } else { req.columns.iter().map(|c| atlas_firebird::quote_ident(c)).collect::<Vec<_>>().join(", ") };
            format!("SELECT {list} FROM ({}) ATLAS_Q", q.sql.trim().trim_end_matches(';'))
        }
        None => atlas_firebird::select_columns_sql(&req.table, &req.columns, &req.order_by, req.filter.as_ref()),
    };
    let s = sessions.session(&req.session_id)?;
    let cancel = Arc::new(AtomicBool::new(false));
    sessions.register_cancel(&req.export_id, cancel.clone());
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
        let streamed = atlas_firebird::stream_rows(
            &s.target,
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
    sessions.unregister_cancel(&cleanup_id);
    out
}

/// O botão Parar de uma exportação, importação ou busca em andamento.
#[tauri::command]
pub async fn firebird_export_cancel(sessions: State<'_, FirebirdSessions>, export_id: String) -> Result<(), DatabaseError> {
    if let Ok(m) = sessions.finds.lock() {
        if let Some(flag) = m.get(&export_id) {
            flag.store(true, Ordering::Relaxed);
        }
    }
    Ok(())
}

/// Importa arquivos (TXT/CSV, JSON, XML, Excel, dBase) para tabelas do Firebird, nos seis modos do assistente.
#[tauri::command]
pub async fn firebird_import(app: tauri::AppHandle, sessions: State<'_, FirebirdSessions>, req: ImportRequest) -> Result<ImportSummary, DatabaseError> {
    let started = Instant::now();
    let s = sessions.session(&req.session_id)?;
    let cancel = Arc::new(AtomicBool::new(false));
    sessions.register_cancel(&req.import_id, cancel.clone());
    let import_id = req.import_id.clone();
    let task = tokio::task::spawn_blocking(move || -> Result<ImportSummary, DatabaseError> {
        let boolean_ok = s.version_num >= 3_000_000;
        let mut results = Vec::new();
        for (job_no, job) in req.jobs.iter().enumerate() {
            if cancel.load(Ordering::Relaxed) {
                return Err(DatabaseError::new(ErrorKind::Canceled, "import canceled"));
            }
            let _ = app.emit("import-log", LogEvent { import_id: &req.import_id, line: format!("[IMP] {}", job.target) });
            // Colunas do destino: as da tabela (lidas do servidor) ou as que serão criadas.
            let target_columns: Vec<(String, String, bool)> = match &job.create {
                Some(cols) => cols.iter().map(|c| (c.name.clone(), declared_type(&c.type_name, boolean_ok), false)).collect(),
                None => match s.table_detail(&job.target) {
                    Ok(d) => d.columns.iter().map(|c| (c.name.clone(), c.data_type.clone(), c.generated.as_deref() == Some("s"))).collect(),
                    Err(e) => {
                        results.push(failed_job(&job.target, e.message, &app, &req.import_id));
                        continue;
                    }
                },
            };
            let mut source = match atlas_import::open(&job.source) {
                Ok(src) => src,
                Err(e) => {
                    results.push(failed_job(&job.target, format!("{}: {e}", job.source.path.display()), &app, &req.import_id));
                    continue;
                }
            };
            let source_cols = source.columns().to_vec();
            let mut columns = Vec::new();
            let mut source_index = Vec::new();
            let mut kinds: Vec<TargetKind> = Vec::new();
            let mut problem: Option<String> = None;
            for m in job.mappings.iter().filter(|m| !m.target.is_empty()) {
                let Some(si) = source_cols.iter().position(|c| *c == m.source) else {
                    problem = Some(format!("source field \"{}\" not found in the file", m.source));
                    break;
                };
                let Some((name, ty, generated)) = target_columns.iter().find(|(n, ..)| *n == m.target) else {
                    problem = Some(format!("destination field \"{}\" does not exist in {}", m.target, job.target));
                    break;
                };
                if *generated {
                    problem = Some(format!("destination field \"{name}\" is a generated column and cannot receive data"));
                    break;
                }
                columns.push(TargetColumn { name: name.clone(), declared_type: ty.clone() });
                kinds.push(kind_of(ty));
                source_index.push(si);
            }
            let mut key_columns = Vec::new();
            if problem.is_none() {
                for k in &job.keys {
                    match columns.iter().position(|c| c.name == *k) {
                        Some(i) => key_columns.push(i),
                        None => {
                            problem = Some(format!("key field \"{k}\" is not mapped"));
                            break;
                        }
                    }
                }
            }
            if let Some(p) = problem {
                results.push(failed_job(&job.target, p, &app, &req.import_id));
                continue;
            }
            let mode = match job.mode {
                atlas_pg::ImportMode::Append => ImportMode::Append,
                atlas_pg::ImportMode::Update => ImportMode::Update,
                atlas_pg::ImportMode::AppendUpdate => ImportMode::AppendUpdate,
                atlas_pg::ImportMode::AppendNoUpdate => ImportMode::AppendNoUpdate,
                atlas_pg::ImportMode::Delete => ImportMode::Delete,
                atlas_pg::ImportMode::Copy => ImportMode::Copy,
            };
            let spec = ImportSpec {
                table: job.target.clone(),
                create: job.create.as_ref().map(|c| c.iter().map(|x| (x.name.clone(), x.type_name.clone())).collect()),
                columns,
                key_columns,
                mode,
                extended_insert: req.advanced.extended_insert,
                max_statement_bytes: req.advanced.max_statement_kb.max(1) as usize * 1024,
                continue_on_error: req.advanced.continue_on_error,
            };
            let mut convert_opts = req.convert.clone();
            convert_opts.empty_as_null = req.advanced.empty_as_null;
            let mut feed = || -> ImportFeed {
                match source.next_row() {
                    Err(e) => ImportFeed::Bad(e.to_string()),
                    Ok(None) => ImportFeed::End,
                    Ok(Some(row)) => {
                        let mut out = Vec::with_capacity(spec.columns.len());
                        for (n, &si) in source_index.iter().enumerate() {
                            let converted = match row[si].as_deref() {
                                None => Ok(None),
                                Some(v) => convert(v, kinds[n], &convert_opts),
                            };
                            match converted {
                                Ok(v) => out.push(v),
                                Err(e) => return ImportFeed::Bad(format!("{} ({})", e, spec.columns[n].name)),
                            }
                        }
                        ImportFeed::Row(out)
                    }
                }
            };
            let target = job.target.clone();
            let log_app = app.clone();
            let log_id = req.import_id.clone();
            let mut on_log = move |line: String| {
                let _ = log_app.emit("import-log", LogEvent { import_id: &log_id, line });
            };
            let progress_app = app.clone();
            let progress_id = req.import_id.clone();
            let progress_target = target.clone();
            let mut on_progress = move |st: &atlas_firebird::ImportStats| {
                let counts = JobResult { table: progress_target.clone(), processed: st.processed, added: st.added, updated: st.updated, deleted: st.deleted, errors: st.errors, rolled_back: false };
                let _ = progress_app.emit("import-progress", ProgressEvent { import_id: &progress_id, job: job_no, counts });
            };
            let cancelled = || cancel.load(Ordering::Relaxed);
            let stats = s.import(&spec, &mut feed, ImportHooks { log: &mut on_log, progress: &mut on_progress, cancelled: &cancelled })?;
            let result = JobResult { table: target, processed: stats.processed, added: stats.added, updated: stats.updated, deleted: stats.deleted, errors: stats.errors, rolled_back: stats.rolled_back };
            let _ = app.emit("import-progress", ProgressEvent { import_id: &req.import_id, job: job_no, counts: result.clone() });
            results.push(result);
        }
        Ok(ImportSummary { jobs: results, elapsed_ms: started.elapsed().as_millis() as u64 })
    });
    let out = task.await.map_err(|e| DatabaseError::new(ErrorKind::Internal, e.to_string()))?;
    sessions.unregister_cancel(&import_id);
    out
}

/* ---------- Perfil, diagrama, rótulos e busca ---------- */

/// Data Profiling: estatísticas de cada coluna sobre os registros que o filtro, a ordenação e o limite escolhem.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn firebird_data_profile(
    sessions: State<'_, FirebirdSessions>,
    session_id: String,
    _database: String,
    _schema: String,
    table: String,
    columns: Vec<String>,
    order_by: Vec<OrderBy>,
    filter: Option<TableFilter>,
    limit: Option<u64>,
) -> Result<atlas_firebird::DataProfile, DatabaseError> {
    let s = sessions.session(&session_id)?;
    blocking(move || s.data_profile(&table, &columns, &order_by, filter.as_ref(), limit)).await
}

#[tauri::command]
pub async fn firebird_table_profile(sessions: State<'_, FirebirdSessions>, session_id: String, _database: String, _schema: String, table: String) -> Result<atlas_core::TableProfile, DatabaseError> {
    let s = sessions.session(&session_id)?;
    blocking(move || s.table_profile(&table)).await
}

/// Tabelas (com colunas) e chaves estrangeiras, para o diagrama ER.
#[tauri::command]
pub async fn firebird_schema_diagram(sessions: State<'_, FirebirdSessions>, session_id: String, _database: String, _schema: String) -> Result<atlas_core::SchemaDiagram, DatabaseError> {
    let s = sessions.session(&session_id)?;
    blocking(move || s.schema_diagram()).await
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn firebird_fk_labels(
    sessions: State<'_, FirebirdSessions>,
    session_id: String,
    _database: String,
    _schema: String,
    table: String,
    key_column: String,
    keys: Vec<String>,
) -> Result<atlas_firebird::FkLabels, DatabaseError> {
    let s = sessions.session(&session_id)?;
    blocking(move || s.fk_labels(&table, &key_column, &keys)).await
}

/// "Find in Database", Look In = Structure.
#[tauri::command]
pub async fn firebird_find(sessions: State<'_, FirebirdSessions>, session_id: String, _database: String, options: atlas_firebird::FindOptions) -> Result<atlas_firebird::FindResult, DatabaseError> {
    let s = sessions.session(&session_id)?;
    blocking(move || s.find(&options)).await
}

/// "Find in Database", Look In = Data. O botão Parar (`firebird_export_cancel` com o mesmo `find_id`) interrompe entre uma tabela e outra.
#[tauri::command]
pub async fn firebird_find_data(
    sessions: State<'_, FirebirdSessions>,
    session_id: String,
    _database: String,
    find_id: String,
    options: atlas_firebird::FindOptions,
) -> Result<atlas_firebird::FindDataResult, DatabaseError> {
    let s = sessions.session(&session_id)?;
    let stop = Arc::new(AtomicBool::new(false));
    sessions.register_cancel(&find_id, stop.clone());
    let flag = stop.clone();
    let out = blocking(move || s.find_data(&options, &move || flag.load(Ordering::Relaxed))).await;
    sessions.unregister_cancel(&find_id);
    out
}

/* ---------- Dump SQL, backup e restauração ---------- */

/// Primeira linha de um backup do Atlas: o arquivo é o script do dump compactado.
const BACKUP_MARK: &str = "-- Atlas DB Manager: SQL dump (Firebird)";

/// O que o dump leva e como sai.
struct DumpSpec {
    structure: bool,
    data: bool,
    clean: bool,
    tables: Option<Vec<String>>,
    views: Option<Vec<String>>,
    functions: Option<Vec<String>>,
}

/// Escreve o script do dump em `out`. Devolve (objetos, tabelas, linhas).
fn write_dump<W: std::io::Write>(
    s: &FirebirdSession,
    spec: &DumpSpec,
    out: &mut W,
    cancel: &AtomicBool,
    mut progress: impl FnMut(u64),
) -> Result<(usize, usize, u64), DatabaseError> {
    let plan = s.dump_plan(spec.tables.as_deref(), spec.views.as_deref(), spec.functions.as_deref(), spec.clean)?;
    let canceled = || DatabaseError::new(ErrorKind::Canceled, "dump canceled");
    let boolean_ok = s.version_num >= 3_000_000;
    out.write_all(format!("{BACKUP_MARK}\n-- Firebird {}; banco: {}\n", s.version, s.target.database).as_bytes()).map_err(io_error)?;
    if spec.structure {
        for stmt in &plan.pre_data {
            out.write_all(format!("\n{stmt}\n").as_bytes()).map_err(io_error)?;
        }
    }
    let mut rows = 0u64;
    if spec.data {
        for table in &plan.data_tables {
            if cancel.load(Ordering::Relaxed) {
                return Err(canceled());
            }
            if table.columns.is_empty() {
                continue;
            }
            let target = atlas_firebird::quote_ident(&table.name);
            let list = table.columns.iter().map(|(n, _)| atlas_firebird::quote_ident(n)).collect::<Vec<_>>().join(", ");
            out.write_all(format!("\n-- Dados de {target}\n").as_bytes()).map_err(io_error)?;
            let mut table_rows = 0u64;
            atlas_firebird::stream_rows(
                &s.target,
                &format!("SELECT {list} FROM {target}"),
                |_| Ok(()),
                |values| {
                    if cancel.load(Ordering::Relaxed) {
                        return Ok(false);
                    }
                    let cells: Vec<String> = values.iter().zip(&table.columns).map(|(v, (_, ty))| atlas_firebird::literal(*v, ty, boolean_ok)).collect();
                    out.write_all(format!("INSERT INTO {target} ({list}) VALUES ({});\n", cells.join(", ")).as_bytes()).map_err(io_error)?;
                    table_rows += 1;
                    progress(rows + table_rows);
                    Ok(true)
                },
            )?;
            if cancel.load(Ordering::Relaxed) {
                return Err(canceled());
            }
            rows += table_rows;
        }
    }
    if spec.structure {
        for stmt in &plan.post_data {
            out.write_all(format!("\n{stmt}\n").as_bytes()).map_err(io_error)?;
        }
    }
    out.flush().map_err(io_error)?;
    Ok((plan.objects, plan.data_tables.len(), rows))
}

/// Dump SQL: o DDL montado das tabelas de sistema e os dados em `INSERT`, num script que o `isql` e o Atlas executam de volta.
#[tauri::command]
pub async fn firebird_dump_sql(app: tauri::AppHandle, sessions: State<'_, FirebirdSessions>, req: DumpRequest) -> Result<DumpResult, DatabaseError> {
    let started = Instant::now();
    let dest = validate_path(&req.path)?;
    if !req.options.structure && !req.options.data {
        return Err(DatabaseError::new(ErrorKind::InvalidInput, "escolha ao menos estrutura ou dados"));
    }
    let s = sessions.session(&req.session_id)?;
    let cancel = Arc::new(AtomicBool::new(false));
    sessions.register_cancel(&req.dump_id, cancel.clone());
    let dump_id = req.dump_id.clone();
    let task = tokio::task::spawn_blocking(move || -> Result<DumpResult, DatabaseError> {
        let (tables, views, functions): (Option<Vec<String>>, Option<Vec<String>>, Option<Vec<String>>) = match &req.options.scope {
            atlas_pg::DumpScope::Database | atlas_pg::DumpScope::Schema { .. } => (None, None, None),
            atlas_pg::DumpScope::Tables { tables, .. } => (Some(tables.clone()), Some(Vec::new()), Some(Vec::new())),
            atlas_pg::DumpScope::Selection { tables, views, functions, .. } => (Some(tables.clone()), Some(views.clone()), Some(functions.clone())),
        };
        let spec = DumpSpec { structure: req.options.structure, data: req.options.data, clean: req.options.clean, tables, views, functions };
        let mut part = PartFile::new(part_path(&dest));
        let mut out = std::io::BufWriter::with_capacity(256 * 1024, File::create(&part.path).map_err(io_error)?);
        let mut last_emit = Instant::now();
        let (objects, tables, rows) = write_dump(&s, &spec, &mut out, &cancel, |n| {
            if last_emit.elapsed() >= Duration::from_millis(200) {
                last_emit = Instant::now();
                let _ = app.emit("export-progress", Progress { export_id: &req.dump_id, rows: n });
            }
        })?;
        drop(out);
        fs::rename(&part.path, &dest).map_err(io_error)?;
        part.keep = true;
        let _ = app.emit("export-progress", Progress { export_id: &req.dump_id, rows });
        Ok(DumpResult { objects, tables, rows, bytes: fs::metadata(&dest).map(|m| m.len()).unwrap_or(0), elapsed_ms: started.elapsed().as_millis() as u64, path: dest.to_string_lossy().into_owned() })
    });
    let out = task.await.map_err(|e| DatabaseError::new(ErrorKind::Internal, e.to_string()))?;
    sessions.unregister_cancel(&dump_id);
    out
}

/// Backup do banco: o script do dump (estrutura + dados, com `DROP` antes de cada objeto) compactado em `.dump`, guardado na pasta de
/// backups do app (ou na escolhida), com o comentário ao lado. A extensão é a mesma dos outros backups, para a lista e a exclusão serem as mesmas.
#[tauri::command]
pub async fn firebird_backup(app: tauri::AppHandle, sessions: State<'_, FirebirdSessions>, req: BackupRequest) -> Result<BackupResult, DatabaseError> {
    let started = Instant::now();
    let s = sessions.session(&req.session_id)?;
    let store = store_dir(&app, &req.session_id, &req.database, req.schema.as_deref())?;
    fs::create_dir_all(&store).map_err(io_error)?;
    let folder = match req.folder.as_deref().map(str::trim).filter(|f| !f.is_empty()) {
        Some(f) => {
            let p = std::path::PathBuf::from(f);
            if !p.is_absolute() || !p.is_dir() {
                return Err(DatabaseError::new(ErrorKind::InvalidInput, "a pasta do backup precisa existir"));
            }
            p
        }
        None => store.clone(),
    };
    let name = safe_segment(&req.file_name);
    let dest = folder.join(format!("{name}.dump"));
    let meta = store.join(format!("{name}.json"));
    if dest.exists() || meta.exists() {
        return Err(DatabaseError::new(ErrorKind::InvalidInput, format!("já existe um backup chamado {name}")));
    }
    let exclude = req.exclude.clone();
    let job_id = req.job_id.clone();
    let part_dest = dest.clone();
    let log_app = app.clone();
    let task = tokio::task::spawn_blocking(move || -> Result<(usize, u64), DatabaseError> {
        // O que o usuário desmarcou (`schema.nome`) fica de fora.
        let objects = s.list_objects()?;
        let excluded = |n: &str| exclude.iter().any(|e| e.rsplit('.').next() == Some(n));
        let spec = DumpSpec {
            structure: true,
            data: true,
            clean: true,
            tables: Some(objects.tables.iter().filter(|t| !excluded(&t.name)).map(|t| t.name.clone()).collect()),
            views: Some(objects.views.iter().filter(|v| !excluded(&v.name)).map(|v| v.name.clone()).collect()),
            functions: Some(objects.functions.iter().filter(|f| !excluded(&f.name)).map(|f| f.name.clone()).collect()),
        };
        let cancel = AtomicBool::new(false);
        let mut part = PartFile::new(part_path(&part_dest));
        let file = File::create(&part.path).map_err(io_error)?;
        let mut gz = flate2::write::GzEncoder::new(std::io::BufWriter::new(file), flate2::Compression::default());
        let _ = log_app.emit("backup-log", LogLine { job_id: &job_id, line: &format!("dumping database {}", s.target.database) });
        let (_, tables, rows) = write_dump(&s, &spec, &mut gz, &cancel, |_| {})?;
        gz.finish().map_err(io_error)?;
        fs::rename(&part.path, &part_dest).map_err(io_error)?;
        part.keep = true;
        let _ = log_app.emit("backup-log", LogLine { job_id: &job_id, line: &format!("{tables} table(s), {rows} row(s)") });
        Ok((tables, rows))
    });
    task.await.map_err(|e| DatabaseError::new(ErrorKind::Internal, e.to_string()))??;
    let external = (folder != store).then(|| dest.to_string_lossy().into_owned());
    let sidecar = serde_json::to_string(&Sidecar { comment: req.comment, path: external }).unwrap_or_default();
    let _ = fs::write(&meta, sidecar);
    Ok(BackupResult { bytes: fs::metadata(&dest).map(|m| m.len()).unwrap_or(0), elapsed_ms: started.elapsed().as_millis() as u64, path: dest.to_string_lossy().into_owned(), tool: "firebird".into(), name })
}

/// Backups guardados do banco (a lista é a mesma dos outros bancos: pastas e registros do app).
#[tauri::command]
pub async fn firebird_backup_list(app: tauri::AppHandle, session_id: String, database: String, schema: String) -> Result<Vec<BackupEntry>, DatabaseError> {
    crate::backup::pg_backup_list(app, session_id, database, schema).await
}

/// Lê o script de um backup `.dump` (gzip) para a memória.
fn read_backup(path: &std::path::Path) -> Result<String, DatabaseError> {
    use std::io::Read;
    let file = File::open(path).map_err(io_error)?;
    let mut text = String::new();
    flate2::read::GzDecoder::new(std::io::BufReader::new(file))
        .read_to_string(&mut text)
        .map_err(|_| DatabaseError::new(ErrorKind::InvalidInput, "o arquivo não é um backup do Firebird feito pelo Atlas"))?;
    if !text.starts_with(BACKUP_MARK) {
        return Err(DatabaseError::new(ErrorKind::InvalidInput, "o arquivo não é um backup do Firebird feito pelo Atlas"));
    }
    Ok(text)
}

/// Restaura um backup (os objetos do arquivo substituem os do banco) ou, com `extract_to`, grava o SQL do backup num arquivo.
#[tauri::command]
pub async fn firebird_restore(app: tauri::AppHandle, sessions: State<'_, FirebirdSessions>, req: RestoreRequest) -> Result<BackupResult, DatabaseError> {
    let started = Instant::now();
    let source = std::path::PathBuf::from(&req.path);
    if !source.exists() {
        return Err(DatabaseError::new(ErrorKind::InvalidInput, "o arquivo de backup não existe"));
    }
    let text = read_backup(&source)?;
    if let Some(target) = &req.extract_to {
        let dest = validate_path(target)?;
        fs::write(&dest, &text).map_err(io_error)?;
        return Ok(BackupResult { bytes: fs::metadata(&dest).map(|m| m.len()).unwrap_or(0), elapsed_ms: started.elapsed().as_millis() as u64, path: dest.to_string_lossy().into_owned(), tool: "firebird".into(), name: String::new() });
    }
    let s = sessions.session(&req.session_id)?;
    let job_id = req.job_id.clone();
    let log_app = app.clone();
    let _ = app.emit("backup-log", LogLine { job_id: &req.job_id, line: &format!("restoring {}", source.display()) });
    let done = tokio::task::spawn_blocking(move || {
        let mut log = |line: String| {
            let _ = log_app.emit("backup-log", LogLine { job_id: &job_id, line: &line });
        };
        s.run_script(&text, &mut log)
    })
    .await
    .map_err(|e| DatabaseError::new(ErrorKind::Internal, e.to_string()))??;
    let _ = app.emit("backup-log", LogLine { job_id: &req.job_id, line: &format!("{done} command(s) executed") });
    Ok(BackupResult { bytes: fs::metadata(&source).map(|m| m.len()).unwrap_or(0), elapsed_ms: started.elapsed().as_millis() as u64, path: source.to_string_lossy().into_owned(), tool: "firebird".into(), name: String::new() })
}

/* ---------- Console e monitor ---------- */

/// Console: `input` é um comando completo, SQL ou com `.`.
#[tauri::command]
pub async fn firebird_console_run(sessions: State<'_, FirebirdSessions>, session_id: String, _database: String, input: String, options: atlas_firebird::ConsoleOptions) -> Result<atlas_firebird::ConsoleOutput, DatabaseError> {
    let s = sessions.session(&session_id)?;
    blocking(move || s.console_run(&input, options)).await
}

/// O texto do prompt (`main=>`).
#[tauri::command]
pub async fn firebird_console_prompt(sessions: State<'_, FirebirdSessions>, session_id: String, _database: String) -> Result<String, DatabaseError> {
    let s = sessions.session(&session_id)?;
    blocking(move || s.console_prompt()).await
}

/// Tab no console.
#[tauri::command]
pub async fn firebird_console_complete(sessions: State<'_, FirebirdSessions>, session_id: String, _database: String, text: String) -> Result<atlas_firebird::ConsoleCompletion, DatabaseError> {
    let s = sessions.session(&session_id)?;
    blocking(move || s.console_complete(&text)).await
}

/// O texto forma um comando completo? (A tela decide com isso quando enviar.)
#[tauri::command]
pub fn firebird_console_is_complete(input: String) -> bool {
    atlas_firebird::console_is_complete(&input)
}

/// Server Monitor: conexões, configurações e contadores.
#[tauri::command]
pub async fn firebird_monitor_snapshot(sessions: State<'_, FirebirdSessions>, session_id: String) -> Result<atlas_core::MonitorSnapshot, DatabaseError> {
    let s = sessions.session(&session_id)?;
    blocking(move || s.monitor_snapshot()).await
}

/// Server Monitor: encerra a conexão `pid`.
#[tauri::command]
pub async fn firebird_monitor_end_process(sessions: State<'_, FirebirdSessions>, session_id: String, pid: i64) -> Result<bool, DatabaseError> {
    let s = sessions.session(&session_id)?;
    blocking(move || s.monitor_end_process(pid)).await
}
