//! Comandos do SQLite: espelham os `pg_*` que a interface usa (mesmos argumentos e formatos de resposta) com o prefixo `sqlite_`.
//! A interface escolhe o prefixo pelo id da conexão (`sqlite_...`), então as telas não mudam.

use crate::commands::{TABLE_ROWS_CAP, TableRows};
use crate::export::{ExportRequest, ExportResult, PartFile, Progress, io_error, part_path, validate_path};
use crate::import::{ImportRequest, ImportSummary, JobResult, LogEvent, ProgressEvent, failed_job};
use crate::dump::{DumpRequest, DumpResult};
use crate::backup::{BackupEntry, BackupRequest, BackupResult, LogLine, RestoreRequest, Sidecar, safe_segment, store_dir};
use atlas_export::ExportColumn;
use std::path::PathBuf;
use atlas_import::convert::{TargetKind, convert, kind_of};
use atlas_sqlite::{ImportFeed, ImportHooks, ImportMode, ImportSpec, TargetColumn, declared_type};
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

/// Importa arquivos (TXT/CSV, JSON, XML, Excel, dBase) para tabelas do SQLite, nos seis modos do assistente.
#[tauri::command]
pub async fn sqlite_import(app: tauri::AppHandle, sessions: State<'_, SqliteSessions>, req: ImportRequest) -> Result<ImportSummary, DatabaseError> {
    let started = Instant::now();
    let s = sessions.session(&req.session_id)?;
    let cancel = Arc::new(AtomicBool::new(false));
    if let Ok(mut m) = sessions.finds.lock() {
        m.insert(req.import_id.clone(), cancel.clone());
    }
    let import_id = req.import_id.clone();
    let task = tokio::task::spawn_blocking(move || -> Result<ImportSummary, DatabaseError> {
        let mut results = Vec::new();
        for (job_no, job) in req.jobs.iter().enumerate() {
            if cancel.load(Ordering::Relaxed) {
                return Err(DatabaseError::new(ErrorKind::Canceled, "import canceled"));
            }
            let _ = app.emit("import-log", LogEvent { import_id: &req.import_id, line: format!("[IMP] {}", job.target) });
            // Colunas do destino: as da tabela (lidas do arquivo) ou as que serão criadas.
            let target_columns: Vec<(String, String, bool)> = match &job.create {
                Some(cols) => cols.iter().map(|c| (c.name.clone(), declared_type(&c.type_name), false)).collect(),
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
            let mut on_progress = move |st: &atlas_sqlite::ImportStats| {
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
    if let Ok(mut m) = sessions.finds.lock() {
        m.remove(&import_id);
    }
    out
}

/// Backup do arquivo: uma cópia compactada guardada na pasta de backups do app (ou na pasta escolhida), com o comentário ao lado.
/// A extensão `.dump` é a mesma dos backups do PostgreSQL, para a lista e a exclusão serem as mesmas.
#[tauri::command]
pub async fn sqlite_backup(app: tauri::AppHandle, sessions: State<'_, SqliteSessions>, req: BackupRequest) -> Result<BackupResult, DatabaseError> {
    let started = Instant::now();
    let s = sessions.session(&req.session_id)?;
    let store = store_dir(&app, &req.session_id, &req.database, req.schema.as_deref())?;
    fs::create_dir_all(&store).map_err(io_error)?;
    let folder = match req.folder.as_deref().map(str::trim).filter(|f| !f.is_empty()) {
        Some(f) => {
            let p = PathBuf::from(f);
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
    let job_id = req.job_id.clone();
    let exclude = req.exclude.clone();
    let part_dest = dest.clone();
    let log_app = app.clone();
    let task = tokio::task::spawn_blocking(move || -> Result<(), DatabaseError> {
        let mut part = PartFile::new(part_path(&part_dest));
        let mut log = |line: String| {
            let _ = log_app.emit("backup-log", LogLine { job_id: &job_id, line: &line });
        };
        s.backup_to(&part.path, &exclude, &mut log)?;
        fs::rename(&part.path, &part_dest).map_err(io_error)?;
        part.keep = true;
        Ok(())
    });
    task.await.map_err(|e| DatabaseError::new(ErrorKind::Internal, e.to_string()))??;
    let external = (folder != store).then(|| dest.to_string_lossy().into_owned());
    let sidecar = serde_json::to_string(&Sidecar { comment: req.comment, path: external }).unwrap_or_default();
    let _ = fs::write(&meta, sidecar);
    Ok(BackupResult { bytes: fs::metadata(&dest).map(|m| m.len()).unwrap_or(0), elapsed_ms: started.elapsed().as_millis() as u64, path: dest.to_string_lossy().into_owned(), tool: "sqlite".into(), name })
}

/// Backups guardados do banco (a lista é a mesma do PostgreSQL: pastas e registros do app).
#[tauri::command]
pub async fn sqlite_backup_list(app: tauri::AppHandle, session_id: String, database: String, schema: String) -> Result<Vec<BackupEntry>, DatabaseError> {
    crate::backup::pg_backup_list(app, session_id, database, schema).await
}

/// Restaura um backup: o conteúdo do banco passa a ser o do arquivo.
#[tauri::command]
pub async fn sqlite_restore(app: tauri::AppHandle, sessions: State<'_, SqliteSessions>, req: RestoreRequest) -> Result<BackupResult, DatabaseError> {
    let started = Instant::now();
    let s = sessions.session(&req.session_id)?;
    if req.extract_to.is_some() {
        return Err(atlas_sqlite::unsupported_feature("extract SQL from a backup"));
    }
    let source = PathBuf::from(&req.path);
    if !source.exists() {
        return Err(DatabaseError::new(ErrorKind::InvalidInput, "o arquivo de backup não existe"));
    }
    let _ = app.emit("backup-log", LogLine { job_id: &req.job_id, line: &format!("restoring {}", source.display()) });
    let from = source.clone();
    tokio::task::spawn_blocking(move || s.restore_from(&from)).await.map_err(|e| DatabaseError::new(ErrorKind::Internal, e.to_string()))??;
    let _ = app.emit("backup-log", LogLine { job_id: &req.job_id, line: "restore finished" });
    Ok(BackupResult { bytes: fs::metadata(&source).map(|m| m.len()).unwrap_or(0), elapsed_ms: started.elapsed().as_millis() as u64, path: source.to_string_lossy().into_owned(), tool: "sqlite".into(), name: String::new() })
}

/// Destino compartilhado pelo dump e pelo gravador de `INSERT` (cada um escreve sem o outro estar no meio de uma linha).
#[derive(Clone)]
struct DumpSink(Arc<Mutex<std::io::BufWriter<File>>>);

impl std::io::Write for DumpSink {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().write(data)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.0.lock().unwrap().flush()
    }
}

/// Dump SQL: os `CREATE` guardados no arquivo e os dados como `INSERT`, num script que o `sqlite3` e o Atlas executam de volta.
#[tauri::command]
pub async fn sqlite_dump_sql(app: tauri::AppHandle, sessions: State<'_, SqliteSessions>, req: DumpRequest) -> Result<DumpResult, DatabaseError> {
    use std::io::Write;
    let started = Instant::now();
    let dest = validate_path(&req.path)?;
    if !req.options.structure && !req.options.data {
        return Err(DatabaseError::new(ErrorKind::InvalidInput, "escolha ao menos estrutura ou dados"));
    }
    let s = sessions.session(&req.session_id)?;
    let cancel = Arc::new(AtomicBool::new(false));
    if let Ok(mut m) = sessions.finds.lock() {
        m.insert(req.dump_id.clone(), cancel.clone());
    }
    let dump_id = req.dump_id.clone();
    let task = tokio::task::spawn_blocking(move || -> Result<DumpResult, DatabaseError> {
        let (tables, views): (Option<Vec<String>>, Option<Vec<String>>) = match &req.options.scope {
            atlas_pg::DumpScope::Database | atlas_pg::DumpScope::Schema { .. } => (None, None),
            atlas_pg::DumpScope::Tables { tables, .. } => (Some(tables.clone()), Some(Vec::new())),
            atlas_pg::DumpScope::Selection { tables, views, .. } => (Some(tables.clone()), Some(views.clone())),
        };
        let plan = s.dump_plan(tables.as_deref(), views.as_deref(), req.options.clean)?;
        let mut part = PartFile::new(part_path(&dest));
        let out = DumpSink(Arc::new(Mutex::new(std::io::BufWriter::with_capacity(256 * 1024, File::create(&part.path).map_err(io_error)?))));
        let write = |text: &str| -> Result<(), DatabaseError> { out.clone().write_all(text.as_bytes()).map_err(io_error) };
        let canceled = || DatabaseError::new(ErrorKind::Canceled, "dump canceled");
        write(&format!("-- Atlas DB Manager: SQL dump\n-- SQLite {}; arquivo: {}\n\nPRAGMA foreign_keys = OFF;\nBEGIN TRANSACTION;\n", s.version, s.path.display()))?;
        if req.options.structure {
            for stmt in &plan.pre_data {
                write(&format!("\n{stmt}\n"))?;
            }
        }
        let mut rows = 0u64;
        let mut last_emit = Instant::now();
        if req.options.data {
            for table in &plan.data_tables {
                if cancel.load(Ordering::Relaxed) {
                    return Err(canceled());
                }
                let quoted = atlas_pg::quote_ident(table);
                write(&format!("\n-- Dados de {quoted}\n"))?;
                let format = atlas_export::ExportFormat::Sql { rows_per_insert: req.rows_per_insert.clamp(1, 1000) };
                let writer = Mutex::new(atlas_export::create_stream(&format, &atlas_export::WriterTarget { sql: &quoted, title: table }, Box::new(out.clone())).map_err(io_error)?);
                let mut table_rows = 0u64;
                atlas_sqlite::stream_rows(
                    &s.path,
                    &atlas_sqlite::select_sql(table, &[], None),
                    |cols| {
                        let cols: Vec<ExportColumn> = cols.iter().map(|c| ExportColumn { name: c.name.clone(), type_name: c.type_name.as_deref().map(export_type) }).collect();
                        writer.lock().unwrap().begin(&cols).map_err(io_error)
                    },
                    |values| {
                        if cancel.load(Ordering::Relaxed) {
                            return Ok(false);
                        }
                        writer.lock().unwrap().row(values).map_err(io_error)?;
                        table_rows += 1;
                        if last_emit.elapsed() >= Duration::from_millis(200) {
                            last_emit = Instant::now();
                            let _ = app.emit("export-progress", Progress { export_id: &req.dump_id, rows: rows + table_rows });
                        }
                        Ok(true)
                    },
                )?;
                if cancel.load(Ordering::Relaxed) {
                    return Err(canceled());
                }
                writer.into_inner().unwrap().finish().map_err(io_error)?;
                rows += table_rows;
            }
        }
        if req.options.structure {
            for stmt in &plan.post_data {
                write(&format!("\n{stmt}\n"))?;
            }
        }
        write("\nCOMMIT;\nPRAGMA foreign_keys = ON;\n")?;
        out.clone().flush().map_err(io_error)?;
        fs::rename(&part.path, &dest).map_err(io_error)?;
        part.keep = true;
        let _ = app.emit("export-progress", Progress { export_id: &req.dump_id, rows });
        Ok(DumpResult {
            objects: plan.objects,
            tables: plan.data_tables.len(),
            rows,
            bytes: fs::metadata(&dest).map(|m| m.len()).unwrap_or(0),
            elapsed_ms: started.elapsed().as_millis() as u64,
            path: dest.to_string_lossy().into_owned(),
        })
    });
    let out = task.await.map_err(|e| DatabaseError::new(ErrorKind::Internal, e.to_string()))?;
    if let Ok(mut m) = sessions.finds.lock() {
        m.remove(&dump_id);
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
