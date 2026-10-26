//! Comandos do MySQL. Espelham os `pg_*` que as telas usam (mesmos argumentos e mesmos formatos de resposta), com o prefixo
//! `mysql_`; o roteador da interface (`providerRoute.ts`) manda a chamada para cá quando o id da conexão começa por `mysql_`.
//! Os comandos do PostgreSQL não são tocados.

use crate::commands::{TABLE_ROWS_CAP, TableRows};
use crate::import::{ImportRequest, ImportSummary, JobResult, LogEvent, ProgressEvent, failed_job};
use atlas_import::convert::{TargetKind, convert, kind_of};
use crate::dump::{DumpRequest, DumpResult};
use crate::backup::{BackupEntry, BackupRequest, BackupResult, LogLine, RestoreRequest, Sidecar, safe_segment, store_dir};
use crate::export::{ExportRequest, ExportResult, PartFile, Progress, io_error, part_path, validate_path};
use crate::local::Local;
use atlas_export::ExportColumn;
use std::fs::{self, File};
use std::sync::atomic::AtomicBool;
use std::time::Duration;
use atlas_core::{ConnectionParams, DatabaseError, DatabaseList, ErrorKind, OrderBy, QueryOutcome, RoleInfo, SchemaInfo, SchemaObjects, ServerInfo, TableDetail, TableFilter};
use atlas_mysql::{ImportFeed, ImportHooks, ImportMode, ImportSpec, MysqlSession, MysqlTx, PlannedStatement, TargetColumn};
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

/// "Find in Database", Look In = Structure.
#[tauri::command]
pub async fn mysql_find(sessions: State<'_, MysqlSessions>, session_id: String, database: String, options: atlas_mysql::FindOptions) -> Result<atlas_mysql::FindResult, DatabaseError> {
    sessions.session(&session_id)?.find(&database, &options).await
}

/// "Find in Database", Look In = Data. O botão Parar (`mysql_export_cancel` com o mesmo `find_id`) interrompe entre uma tabela e outra.
#[tauri::command]
pub async fn mysql_find_data(
    sessions: State<'_, MysqlSessions>,
    session_id: String,
    database: String,
    find_id: String,
    options: atlas_mysql::FindOptions,
) -> Result<atlas_mysql::FindDataResult, DatabaseError> {
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

/// Data Profiling: estatísticas de cada coluna sobre os registros que o filtro, a ordenação e o limite escolhem.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn mysql_data_profile(
    sessions: State<'_, MysqlSessions>,
    session_id: String,
    database: String,
    _schema: String,
    table: String,
    columns: Vec<String>,
    order_by: Vec<OrderBy>,
    filter: Option<TableFilter>,
    limit: Option<u64>,
) -> Result<atlas_mysql::DataProfile, DatabaseError> {
    sessions.session(&session_id)?.data_profile(&database, &table, &columns, &order_by, filter.as_ref(), limit).await
}

/// Table Profile: tamanhos, contagens da estrutura e total de linhas.
#[tauri::command]
pub async fn mysql_table_profile(sessions: State<'_, MysqlSessions>, session_id: String, database: String, _schema: String, table: String) -> Result<atlas_core::TableProfile, DatabaseError> {
    sessions.session(&session_id)?.table_profile(&database, &table).await
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

/// A categoria de conversão do assistente para um tipo do MySQL.
fn kind_for(ty: &str) -> TargetKind {
    let t = ty.trim().to_ascii_lowercase();
    let base = t.split(['(', ' ']).next().unwrap_or(&t);
    if t.starts_with("tinyint(1)") {
        return kind_of("boolean");
    }
    match base {
        "bit" => kind_of("boolean"),
        "tinyint" | "smallint" | "mediumint" | "int" | "year" => kind_of("bigint"),
        "double" => kind_of("numeric"),
        "datetime" | "timestamp" => kind_of("timestamp"),
        other => kind_of(other),
    }
}

/// Importa arquivos (TXT/CSV, JSON, XML, Excel, dBase) para tabelas do MySQL, nos seis modos do assistente.
#[tauri::command]
pub async fn mysql_import(app: tauri::AppHandle, sessions: State<'_, MysqlSessions>, req: ImportRequest) -> Result<ImportSummary, DatabaseError> {
    let started = Instant::now();
    let s = sessions.session(&req.session_id)?;
    let cancel = Arc::new(AtomicBool::new(false));
    if let Ok(mut m) = sessions.finds.lock() {
        m.insert(req.import_id.clone(), cancel.clone());
    }
    let out = async {
        let mut results = Vec::new();
        for (job_no, job) in req.jobs.iter().enumerate() {
            if cancel.load(Ordering::Relaxed) {
                return Err(DatabaseError::new(ErrorKind::Canceled, "import canceled"));
            }
            let _ = app.emit("import-log", LogEvent { import_id: &req.import_id, line: format!("[IMP] {}", job.target) });
            // Colunas do destino: as da tabela (lidas do servidor) ou as que serão criadas. Colunas calculadas não recebem dados (AUTO_INCREMENT aceita valor).
            // No MySQL o schema é o próprio banco.
            let database = if req.schema.is_empty() { req.database.clone() } else { req.schema.clone() };
            let target_columns: Vec<(String, String, bool)> = match &job.create {
                Some(cols) => cols.iter().map(|c| (c.name.clone(), atlas_mysql::declared_type(&c.type_name), false)).collect(),
                None => match s.table_detail(&database, &job.target).await {
                    Ok(d) => d.columns.iter().map(|c| (c.name.clone(), c.formatted_type.clone(), c.generated.is_some())).collect(),
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
                    problem = Some(format!("destination field \"{name}\" is a computed column and cannot receive data"));
                    break;
                }
                columns.push(TargetColumn { name: name.clone(), declared_type: ty.clone() });
                kinds.push(kind_for(ty));
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
                database: database.clone(),
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
            let mut on_progress = move |st: &atlas_mysql::ImportStats| {
                let counts = JobResult { table: progress_target.clone(), processed: st.processed, added: st.added, updated: st.updated, deleted: st.deleted, errors: st.errors, rolled_back: false };
                let _ = progress_app.emit("import-progress", ProgressEvent { import_id: &progress_id, job: job_no, counts });
            };
            let cancelled = || cancel.load(Ordering::Relaxed);
            let stats = s.import(&spec, &mut feed, ImportHooks { log: &mut on_log, progress: &mut on_progress, cancelled: &cancelled }).await?;
            let result = JobResult { table: target, processed: stats.processed, added: stats.added, updated: stats.updated, deleted: stats.deleted, errors: stats.errors, rolled_back: stats.rolled_back };
            let _ = app.emit("import-progress", ProgressEvent { import_id: &req.import_id, job: job_no, counts: result.clone() });
            results.push(result);
        }
        Ok(ImportSummary { jobs: results, elapsed_ms: started.elapsed().as_millis() as u64 })
    }
    .await;
    if let Ok(mut m) = sessions.finds.lock() {
        m.remove(&req.import_id);
    }
    out
}

/// Marca que separa os comandos de um backup (a restauração executa um comando por vez).
const STMT_MARK: &str = "-- ATLAS-STMT";

/// O que o dump leva e como sai.
struct DumpSpec {
    database: String,
    structure: bool,
    data: bool,
    clean: bool,
    rows_per_insert: usize,
    tables: Option<Vec<String>>,
    views: Option<Vec<String>>,
    /// Põe `STMT_MARK` depois de cada comando (formato do backup).
    marked: bool,
}

/// Escreve o script do dump em `out`: o DDL lido com `SHOW CREATE` e os dados em `INSERT`. Devolve (objetos, tabelas, linhas).
async fn write_dump<W: std::io::Write + Send>(
    s: &MysqlSession,
    spec: &DumpSpec,
    out: &mut W,
    cancel: &AtomicBool,
    mut progress: impl FnMut(u64) + Send,
) -> Result<(usize, usize, u64), DatabaseError> {
    let plan = s.dump_plan(&spec.database, spec.tables.as_deref(), spec.views.as_deref(), spec.clean).await?;
    let canceled = || DatabaseError::new(ErrorKind::Canceled, "dump canceled");
    let io = io_error;
    let emit = |out: &mut W, stmt: &str| -> Result<(), DatabaseError> {
        out.write_all(format!("\n{stmt}\n").as_bytes()).map_err(io)?;
        if spec.marked {
            out.write_all(format!("{STMT_MARK}\n").as_bytes()).map_err(io)?;
        }
        Ok(())
    };
    out.write_all(format!("-- Atlas DB Manager: SQL dump\n-- MySQL {}; banco: {}\n", s.version, spec.database).as_bytes()).map_err(io)?;
    emit(out, "SET NAMES utf8mb4;")?;
    emit(out, "SET FOREIGN_KEY_CHECKS = 0;")?;
    if spec.structure {
        for stmt in &plan.pre_data {
            emit(out, stmt)?;
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
            let target = atlas_mysql::quote_ident(&table.name);
            let list = table.columns.iter().map(|(n, _)| atlas_mysql::quote_ident(n)).collect::<Vec<_>>().join(", ");
            out.write_all(format!("\n-- Dados de {target}\n").as_bytes()).map_err(io)?;
            let mut batch: Vec<String> = Vec::new();
            let mut table_rows = 0u64;
            let flush = |batch: &mut Vec<String>, out: &mut W| -> Result<(), DatabaseError> {
                if batch.is_empty() {
                    return Ok(());
                }
                let text = format!("INSERT INTO {target} ({list}) VALUES\n{};", batch.join(",\n"));
                batch.clear();
                emit(out, &text)
            };
            s.stream_rows(
                &spec.database,
                &format!("SELECT {list} FROM {}.{target}", atlas_mysql::quote_ident(&spec.database)),
                |_| Ok(()),
                |values| {
                    if cancel.load(Ordering::Relaxed) {
                        return Ok(false);
                    }
                    let cells: Vec<String> = values.iter().zip(&table.columns).map(|(v, (_, ty))| atlas_mysql::literal(*v, ty)).collect();
                    batch.push(format!("({})", cells.join(", ")));
                    table_rows += 1;
                    if batch.len() >= spec.rows_per_insert {
                        flush(&mut batch, out)?;
                    }
                    progress(rows + table_rows);
                    Ok(true)
                },
            )
            .await?;
            if cancel.load(Ordering::Relaxed) {
                return Err(canceled());
            }
            flush(&mut batch, out)?;
            rows += table_rows;
        }
    }
    if spec.structure {
        for stmt in &plan.post_data {
            emit(out, stmt)?;
        }
    }
    emit(out, "SET FOREIGN_KEY_CHECKS = 1;")?;
    out.flush().map_err(io)?;
    Ok((plan.objects, plan.data_tables.len(), rows))
}

/// Destino do dump.
type DumpOut = std::io::BufWriter<File>;

/// Dump SQL: o DDL lido com `SHOW CREATE` e os dados em `INSERT`, num script que o `mysql` e o Atlas executam de volta.
#[tauri::command]
pub async fn mysql_dump_sql(app: tauri::AppHandle, sessions: State<'_, MysqlSessions>, req: DumpRequest) -> Result<DumpResult, DatabaseError> {
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
    let out = async {
        // No MySQL o schema é o próprio banco.
        let (database, tables, views): (String, Option<Vec<String>>, Option<Vec<String>>) = match &req.options.scope {
            atlas_pg::DumpScope::Database => (req.database.clone(), None, None),
            atlas_pg::DumpScope::Schema { schema } => (schema.clone(), None, None),
            atlas_pg::DumpScope::Tables { schema, tables } => (schema.clone(), Some(tables.clone()), Some(Vec::new())),
            atlas_pg::DumpScope::Selection { schema, tables, views, .. } => (schema.clone(), Some(tables.clone()), Some(views.clone())),
        };
        let spec = DumpSpec { database, structure: req.options.structure, data: req.options.data, clean: req.options.clean, rows_per_insert: req.rows_per_insert.clamp(1, 1000) as usize, tables, views, marked: false };
        let mut part = PartFile::new(part_path(&dest));
        let mut out: DumpOut = std::io::BufWriter::with_capacity(256 * 1024, File::create(&part.path).map_err(io_error)?);
        let mut last_emit = Instant::now();
        let (objects, tables, rows) = write_dump(&s, &spec, &mut out, &cancel, |n| {
            if last_emit.elapsed() >= Duration::from_millis(200) {
                last_emit = Instant::now();
                let _ = app.emit("export-progress", Progress { export_id: &req.dump_id, rows: n });
            }
        })
        .await?;
        drop(out);
        fs::rename(&part.path, &dest).map_err(io_error)?;
        part.keep = true;
        let _ = app.emit("export-progress", Progress { export_id: &req.dump_id, rows });
        Ok(DumpResult { objects, tables, rows, bytes: fs::metadata(&dest).map(|m| m.len()).unwrap_or(0), elapsed_ms: started.elapsed().as_millis() as u64, path: dest.to_string_lossy().into_owned() })
    }
    .await;
    if let Ok(mut m) = sessions.finds.lock() {
        m.remove(&req.dump_id);
    }
    out
}

/// Backup do banco: o script do dump (estrutura + dados, com `DROP` antes de cada objeto) compactado em `.dump`, guardado na pasta de
/// backups do app (ou na escolhida), com o comentário ao lado. A extensão é a mesma dos outros backups, para a lista e a exclusão serem as mesmas.
#[tauri::command]
pub async fn mysql_backup(app: tauri::AppHandle, sessions: State<'_, MysqlSessions>, req: BackupRequest) -> Result<BackupResult, DatabaseError> {
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
    let database = req.schema.clone().filter(|x| !x.is_empty()).unwrap_or_else(|| req.database.clone());
    // O que o usuário desmarcou (`banco.nome`) fica de fora.
    let objects = s.list_objects(&database).await?;
    let excluded = |n: &str| req.exclude.iter().any(|e| e.rsplit('.').next() == Some(n));
    let spec = DumpSpec {
        database: database.clone(),
        structure: true,
        data: true,
        clean: true,
        rows_per_insert: 200,
        tables: Some(objects.tables.iter().filter(|t| !excluded(&t.name)).map(|t| t.name.clone()).collect()),
        views: if objects.views.iter().any(|v| excluded(&v.name)) || objects.functions.iter().any(|f| excluded(&f.name)) { Some(objects.views.iter().filter(|v| !excluded(&v.name)).map(|v| v.name.clone()).collect()) } else { None },
        marked: true,
    };
    let cancel = AtomicBool::new(false);
    let mut part = PartFile::new(part_path(&dest));
    let file = File::create(&part.path).map_err(io_error)?;
    let mut gz = flate2::write::GzEncoder::new(std::io::BufWriter::new(file), flate2::Compression::default());
    let _ = app.emit("backup-log", LogLine { job_id: &req.job_id, line: &format!("dumping database {database}") });
    let (_, tables, rows) = write_dump(&s, &spec, &mut gz, &cancel, |_| {}).await?;
    gz.finish().map_err(io_error)?;
    fs::rename(&part.path, &dest).map_err(io_error)?;
    part.keep = true;
    let _ = app.emit("backup-log", LogLine { job_id: &req.job_id, line: &format!("{tables} table(s), {rows} row(s)") });
    let external = (folder != store).then(|| dest.to_string_lossy().into_owned());
    let sidecar = serde_json::to_string(&Sidecar { comment: req.comment, path: external }).unwrap_or_default();
    let _ = fs::write(&meta, sidecar);
    Ok(BackupResult { bytes: fs::metadata(&dest).map(|m| m.len()).unwrap_or(0), elapsed_ms: started.elapsed().as_millis() as u64, path: dest.to_string_lossy().into_owned(), tool: "mysql".into(), name })
}

/// Backups guardados do banco (a lista é a mesma dos outros bancos: pastas e registros do app).
#[tauri::command]
pub async fn mysql_backup_list(app: tauri::AppHandle, session_id: String, database: String, schema: String) -> Result<Vec<BackupEntry>, DatabaseError> {
    crate::backup::pg_backup_list(app, session_id, database, schema).await
}

/// Lê o script de um backup `.dump` (gzip) para a memória.
fn read_backup(path: &std::path::Path) -> Result<String, DatabaseError> {
    use std::io::Read;
    let file = File::open(path).map_err(io_error)?;
    let mut text = String::new();
    flate2::read::GzDecoder::new(std::io::BufReader::new(file))
        .read_to_string(&mut text)
        .map_err(|_| DatabaseError::new(ErrorKind::InvalidInput, "o arquivo não é um backup do MySQL feito pelo Atlas"))?;
    if !text.contains(STMT_MARK) {
        return Err(DatabaseError::new(ErrorKind::InvalidInput, "o arquivo não é um backup do MySQL feito pelo Atlas"));
    }
    Ok(text)
}

/// Restaura um backup (os objetos do arquivo substituem os do banco) ou, com `extract_to`, grava o SQL do backup num arquivo.
#[tauri::command]
pub async fn mysql_restore(app: tauri::AppHandle, sessions: State<'_, MysqlSessions>, req: RestoreRequest) -> Result<BackupResult, DatabaseError> {
    let started = Instant::now();
    let source = std::path::PathBuf::from(&req.path);
    if !source.exists() {
        return Err(DatabaseError::new(ErrorKind::InvalidInput, "o arquivo de backup não existe"));
    }
    let text = read_backup(&source)?;
    if let Some(target) = &req.extract_to {
        let dest = validate_path(target)?;
        let plain: String = text.lines().filter(|l| *l != STMT_MARK).collect::<Vec<_>>().join("\n");
        fs::write(&dest, plain).map_err(io_error)?;
        return Ok(BackupResult { bytes: fs::metadata(&dest).map(|m| m.len()).unwrap_or(0), elapsed_ms: started.elapsed().as_millis() as u64, path: dest.to_string_lossy().into_owned(), tool: "mysql".into(), name: String::new() });
    }
    let s = sessions.session(&req.session_id)?;
    let log = |line: String| {
        let _ = app.emit("backup-log", LogLine { job_id: &req.job_id, line: &line });
    };
    log(format!("restoring {}", source.display()));
    let marker = format!("{STMT_MARK}\n");
    let chunks: Vec<&str> = text.split(marker.as_str()).map(str::trim).filter(|c| c.lines().any(|l| !l.trim().is_empty() && !l.trim_start().starts_with("--"))).collect();
    s.run_chunks(&req.database, &chunks).await?;
    log(format!("{} command(s) executed", chunks.len()));
    Ok(BackupResult { bytes: fs::metadata(&source).map(|m| m.len()).unwrap_or(0), elapsed_ms: started.elapsed().as_millis() as u64, path: source.to_string_lossy().into_owned(), tool: "mysql".into(), name: String::new() })
}

/// Console (estilo `mysql`): `input` é um comando completo, SQL ou com `.`.
#[tauri::command]
pub async fn mysql_console_run(sessions: State<'_, MysqlSessions>, session_id: String, database: String, input: String, options: atlas_mysql::ConsoleOptions) -> Result<atlas_mysql::ConsoleOutput, DatabaseError> {
    sessions.session(&session_id)?.console_run(&database, &input, options).await
}

/// O texto do prompt (`atlas_demo=>`).
#[tauri::command]
pub async fn mysql_console_prompt(sessions: State<'_, MysqlSessions>, session_id: String, database: String) -> Result<String, DatabaseError> {
    sessions.session(&session_id)?.console_prompt(&database).await
}

/// Tab no console.
#[tauri::command]
pub async fn mysql_console_complete(sessions: State<'_, MysqlSessions>, session_id: String, database: String, text: String) -> Result<atlas_mysql::ConsoleCompletion, DatabaseError> {
    sessions.session(&session_id)?.console_complete(&database, &text).await
}

/// Conjunto de caracteres e collation do banco, para o diálogo Editar banco.
#[tauri::command]
pub async fn mysql_database_detail(sessions: State<'_, MysqlSessions>, session_id: String, name: String) -> Result<atlas_core::DatabaseDetail, DatabaseError> {
    sessions.session(&session_id)?.database_detail(&name).await
}

/// As collations do servidor, com o conjunto de caracteres de cada uma.
#[tauri::command]
pub async fn mysql_collations(sessions: State<'_, MysqlSessions>, session_id: String) -> Result<Vec<[String; 2]>, DatabaseError> {
    sessions.session(&session_id)?.collations().await
}

/// O texto de uma função ou procedimento, para abrir numa aba de consulta.
#[tauri::command]
pub async fn mysql_routine_definition(sessions: State<'_, MysqlSessions>, session_id: String, database: String, _schema: String, name: String) -> Result<String, DatabaseError> {
    sessions.session(&session_id)?.routine_definition(&database, &name).await
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
