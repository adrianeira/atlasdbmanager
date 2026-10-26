//! Comandos do SQL Server. Espelham os `pg_*` que as telas usam (mesmos argumentos e mesmos formatos de resposta), com o prefixo
//! `mssql_`; o roteador da interface (`providerRoute.ts`) manda a chamada para cá quando o id da conexão começa por `mssql_`.
//! Os comandos do PostgreSQL não são tocados.

use crate::commands::{TABLE_ROWS_CAP, TableRows};
use crate::local::Local;
use crate::import::{ImportRequest, ImportSummary, JobResult, LogEvent, ProgressEvent, failed_job};
use atlas_import::convert::{TargetKind, convert, kind_of};
use atlas_mssql::{ImportFeed, ImportHooks, ImportMode, ImportSpec, TargetColumn};
use crate::backup::{BackupEntry, BackupRequest, BackupResult, RestoreRequest};
use crate::dump::{DumpRequest, DumpResult};
use crate::export::{ExportRequest, ExportResult, PartFile, Progress, io_error, part_path, validate_path};
use atlas_export::ExportColumn;
use std::fs::{self, File};
use std::time::Duration;
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
    /// Túneis SSH das conexões abertas (vivem enquanto a conexão existir).
    tunnels: Mutex<HashMap<String, atlas_ssh::Tunnel>>,
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
pub async fn mssql_test_connection(local: State<'_, Local>, params: ConnectionParams, profile_id: Option<String>) -> Result<ServerInfo, DatabaseError> {
    let started = Instant::now();
    let mut params = local.resolve_password(params, profile_id.as_deref())?;
    // O túnel só vive durante o teste.
    let _tunnel = crate::tunnel::start(&local, &mut params).await?;
    params.ssh = None;
    let s = MssqlSession::open(params).await?;
    Ok(s.server_info(started.elapsed().as_millis() as u64))
}

#[tauri::command]
pub async fn mssql_open(
    app: tauri::AppHandle,
    local: State<'_, Local>,
    sessions: State<'_, MssqlSessions>,
    session_id: String,
    params: ConnectionParams,
    profile_id: Option<String>,
) -> Result<ServerInfo, DatabaseError> {
    let started = Instant::now();
    // A senha guardada no cofre do sistema e o túnel SSH, como no PostgreSQL.
    let mut params = local.resolve_password(params, profile_id.as_deref())?;
    let tunnel = crate::tunnel::start(&local, &mut params).await?;
    params.ssh = None;
    let session = Arc::new(MssqlSession::open(params).await?);
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
pub async fn mssql_close(sessions: State<'_, MssqlSessions>, session_id: String) -> Result<(), DatabaseError> {
    if let Ok(mut map) = sessions.sessions.lock() {
        map.remove(&session_id);
    }
    if let Ok(mut map) = sessions.tunnels.lock() {
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

/// Botão Parar: encerra a sessão que está executando a consulta (o SQL Server não cancela só o comando por este driver).
#[tauri::command]
pub async fn mssql_cancel(sessions: State<'_, MssqlSessions>, session_id: String, database: String) -> Result<(), DatabaseError> {
    sessions.session(&session_id)?.cancel(&database).await
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

/// O tipo do SQL Server com o nome que os gravadores de exportação conhecem (os do PostgreSQL).
fn export_type(t: &str) -> String {
    match t.to_ascii_lowercase().as_str() {
        "bit" => "bool",
        "tinyint" | "smallint" | "int" | "bigint" => "int8",
        "decimal" | "numeric" | "money" | "smallmoney" => "numeric",
        "float" | "real" => "float8",
        "date" => "date",
        "datetime" | "datetime2" | "smalldatetime" | "datetimeoffset" => "timestamp",
        "time" => "time",
        "uniqueidentifier" => "uuid",
        "varbinary" | "binary" | "image" | "timestamp" => "bytea",
        _ => "text",
    }
    .to_string()
}

/// Exporta uma tabela, view ou consulta para arquivo (CSV, JSON, SQL, Excel...), em streaming e numa conexão só da exportação.
#[tauri::command]
pub async fn mssql_export(app: tauri::AppHandle, sessions: State<'_, MssqlSessions>, req: ExportRequest) -> Result<ExportResult, DatabaseError> {
    let started = Instant::now();
    let dest = validate_path(&req.path)?;
    let sql = match &req.query {
        Some(q) => {
            if q.sql.trim().is_empty() || !atlas_pg::is_single_statement(&q.sql) {
                return Err(DatabaseError::new(ErrorKind::InvalidInput, "a exportação aceita uma única consulta (um SELECT): selecione só o comando desejado"));
            }
            let body = q.sql.trim().trim_end_matches(';').trim_end().to_string();
            if req.columns.is_empty() {
                body
            } else {
                format!("SELECT {} FROM (\n{body}\n) AS atlas_export", req.columns.iter().map(|c| atlas_mssql::quote_ident(c)).collect::<Vec<_>>().join(", "))
            }
        }
        None => atlas_mssql::select_columns_sql(&req.schema, &req.table, &req.columns, &req.order_by, req.filter.as_ref()),
    };
    let s = sessions.session(&req.session_id)?;
    let cancel = Arc::new(AtomicBool::new(false));
    if let Ok(mut m) = sessions.finds.lock() {
        m.insert(req.export_id.clone(), cancel.clone());
    }
    let out = async {
        let mut part = PartFile::new(part_path(&dest));
        let file = File::create(&part.path).map_err(io_error)?;
        let target = format!("{}.{}", atlas_mssql::quote_ident(&req.schema), atlas_mssql::quote_ident(&req.table));
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

/// A categoria de conversão do assistente para um tipo do SQL Server.
fn kind_for(ty: &str) -> TargetKind {
    match ty.to_ascii_lowercase().as_str() {
        "bit" => kind_of("boolean"),
        "tinyint" => kind_of("smallint"),
        "money" | "smallmoney" => kind_of("numeric"),
        "datetime" | "datetime2" | "smalldatetime" | "datetimeoffset" => kind_of("timestamp"),
        other => kind_of(other),
    }
}

/// Importa arquivos (TXT/CSV, JSON, XML, Excel, dBase) para tabelas do SQL Server, nos seis modos do assistente.
#[tauri::command]
pub async fn mssql_import(app: tauri::AppHandle, sessions: State<'_, MssqlSessions>, req: ImportRequest) -> Result<ImportSummary, DatabaseError> {
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
            // Colunas do destino: as da tabela (lidas do servidor) ou as que serão criadas. Colunas IDENTITY e calculadas não recebem dados.
            let target_columns: Vec<(String, String, bool)> = match &job.create {
                Some(cols) => cols.iter().map(|c| (c.name.clone(), atlas_mssql::declared_type(&c.type_name), false)).collect(),
                None => match s.table_detail(&req.database, &req.schema, &job.target).await {
                    Ok(d) => d.columns.iter().map(|c| (c.name.clone(), c.data_type.clone(), c.generated.is_some() || c.identity.is_some())).collect(),
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
                    problem = Some(format!("destination field \"{name}\" is an identity or computed column and cannot receive data"));
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
                schema: req.schema.clone(),
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
            let mut on_progress = move |st: &atlas_mssql::ImportStats| {
                let counts = JobResult { table: progress_target.clone(), processed: st.processed, added: st.added, updated: st.updated, deleted: st.deleted, errors: st.errors, rolled_back: false };
                let _ = progress_app.emit("import-progress", ProgressEvent { import_id: &progress_id, job: job_no, counts });
            };
            let cancelled = || cancel.load(Ordering::Relaxed);
            let stats = s.import(&req.database, &spec, &mut feed, ImportHooks { log: &mut on_log, progress: &mut on_progress, cancelled: &cancelled }).await?;
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

/// Definição de uma view para o designer.
#[tauri::command]
pub async fn mssql_view_detail(sessions: State<'_, MssqlSessions>, session_id: String, database: String, schema: String, view: String) -> Result<atlas_core::ViewDetail, DatabaseError> {
    sessions.session(&session_id)?.view_detail(&database, &schema, &view).await
}

/// Data Profiling: estatísticas de cada coluna sobre os registros que o filtro, a ordenação e o limite escolhem.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn mssql_data_profile(
    sessions: State<'_, MssqlSessions>,
    session_id: String,
    database: String,
    schema: String,
    table: String,
    columns: Vec<String>,
    order_by: Vec<OrderBy>,
    filter: Option<TableFilter>,
    limit: Option<u64>,
) -> Result<atlas_mssql::DataProfile, DatabaseError> {
    sessions.session(&session_id)?.data_profile(&database, &schema, &table, &columns, &order_by, filter.as_ref(), limit).await
}

/// Perfil da tabela: tamanhos, contagens da estrutura e total de linhas.
#[tauri::command]
pub async fn mssql_table_profile(sessions: State<'_, MssqlSessions>, session_id: String, database: String, schema: String, table: String) -> Result<atlas_core::TableProfile, DatabaseError> {
    sessions.session(&session_id)?.table_profile(&database, &schema, &table).await
}

/// Console (estilo `sqlcmd`): `input` é um comando completo, SQL (com `GO` entre lotes) ou com `.`.
#[tauri::command]
pub async fn mssql_console_run(sessions: State<'_, MssqlSessions>, session_id: String, database: String, input: String, options: atlas_mssql::ConsoleOptions) -> Result<atlas_mssql::ConsoleOutput, DatabaseError> {
    sessions.session(&session_id)?.console_run(&database, &input, options).await
}

/// O texto do prompt (`atlas_demo=>`).
#[tauri::command]
pub async fn mssql_console_prompt(sessions: State<'_, MssqlSessions>, session_id: String, database: String) -> Result<String, DatabaseError> {
    sessions.session(&session_id)?.console_prompt(&database).await
}

/// Tab no console.
#[tauri::command]
pub async fn mssql_console_complete(sessions: State<'_, MssqlSessions>, session_id: String, database: String, text: String) -> Result<atlas_mssql::ConsoleCompletion, DatabaseError> {
    sessions.session(&session_id)?.console_complete(&database, &text).await
}

/// O texto já forma um comando completo?
#[tauri::command]
pub fn mssql_console_is_complete(input: String) -> bool {
    atlas_mssql::console_is_complete(&input)
}

/// O texto de um procedimento ou função, para abrir numa aba de consulta.
#[tauri::command]
pub async fn mssql_routine_definition(sessions: State<'_, MssqlSessions>, session_id: String, database: String, schema: String, name: String) -> Result<String, DatabaseError> {
    sessions.session(&session_id)?.routine_definition(&database, &schema, &name).await
}

/// Destino do dump.
type DumpOut = std::io::BufWriter<File>;

/// Dump SQL: o DDL lido do catálogo e os dados em `INSERT`, num script com lotes separados por `GO`.
#[tauri::command]
pub async fn mssql_dump_sql(app: tauri::AppHandle, sessions: State<'_, MssqlSessions>, req: DumpRequest) -> Result<DumpResult, DatabaseError> {
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
    let out = async {
        let (schema, tables, views): (Option<String>, Option<Vec<String>>, Option<Vec<String>>) = match &req.options.scope {
            atlas_pg::DumpScope::Database => (None, None, None),
            atlas_pg::DumpScope::Schema { schema } => (Some(schema.clone()), None, None),
            atlas_pg::DumpScope::Tables { schema, tables } => (Some(schema.clone()), Some(tables.clone()), Some(Vec::new())),
            atlas_pg::DumpScope::Selection { schema, tables, views, .. } => (Some(schema.clone()), Some(tables.clone()), Some(views.clone())),
        };
        let plan = s.dump_plan(&req.database, schema.as_deref(), tables.as_deref(), views.as_deref(), req.options.clean).await?;
        let mut part = PartFile::new(part_path(&dest));
        let mut out: DumpOut = std::io::BufWriter::with_capacity(256 * 1024, File::create(&part.path).map_err(io_error)?);
        let canceled = || DatabaseError::new(ErrorKind::Canceled, "dump canceled");
        let write = |text: &str, out: &mut DumpOut| -> Result<(), DatabaseError> { out.write_all(text.as_bytes()).map_err(io_error) };
        write(&format!("-- Atlas DB Manager: SQL dump\n-- SQL Server {}; banco: {}\n\nSET QUOTED_IDENTIFIER ON;\nGO\n", s.version, req.database), &mut out)?;
        if req.options.structure {
            for stmt in &plan.pre_data {
                write(&format!("\n{stmt}\n"), &mut out)?;
            }
            write("\nGO\n", &mut out)?;
        }
        let mut rows = 0u64;
        let mut last_emit = Instant::now();
        let per = req.rows_per_insert.clamp(1, 1000) as usize;
        if req.options.data {
            for table in &plan.data_tables {
                if cancel.load(Ordering::Relaxed) {
                    return Err(canceled());
                }
                let target = format!("{}.{}", atlas_mssql::quote_ident(&table.schema), atlas_mssql::quote_ident(&table.name));
                if table.columns.is_empty() {
                    continue;
                }
                let list = table.columns.iter().map(|(n, _)| atlas_mssql::quote_ident(n)).collect::<Vec<_>>().join(", ");
                write(&format!("\n-- Dados de {target}\n"), &mut out)?;
                if table.has_identity {
                    write(&format!("SET IDENTITY_INSERT {target} ON;\n"), &mut out)?;
                }
                let mut batch: Vec<String> = Vec::new();
                let mut table_rows = 0u64;
                let flush = |batch: &mut Vec<String>, out: &mut DumpOut| -> Result<(), DatabaseError> {
                    if batch.is_empty() {
                        return Ok(());
                    }
                    let text = format!("INSERT INTO {target} ({list}) VALUES\n{};\n", batch.join(",\n"));
                    batch.clear();
                    out.write_all(text.as_bytes()).map_err(io_error)
                };
                s.stream_rows(
                    &req.database,
                    &format!("SELECT {list} FROM {target}"),
                    |_| Ok(()),
                    |values| {
                        if cancel.load(Ordering::Relaxed) {
                            return Ok(false);
                        }
                        let cells: Vec<String> = values.iter().zip(&table.columns).map(|(v, (_, ty))| atlas_mssql::literal(*v, ty)).collect();
                        batch.push(format!("({})", cells.join(", ")));
                        table_rows += 1;
                        if batch.len() >= per {
                            flush(&mut batch, &mut out)?;
                        }
                        if last_emit.elapsed() >= Duration::from_millis(200) {
                            last_emit = Instant::now();
                            let _ = app.emit("export-progress", Progress { export_id: &req.dump_id, rows: rows + table_rows });
                        }
                        Ok(true)
                    },
                )
                .await?;
                if cancel.load(Ordering::Relaxed) {
                    return Err(canceled());
                }
                flush(&mut batch, &mut out)?;
                if table.has_identity {
                    write(&format!("SET IDENTITY_INSERT {target} OFF;\n"), &mut out)?;
                }
                write("GO\n", &mut out)?;
                rows += table_rows;
            }
        }
        if req.options.structure {
            for stmt in &plan.post_data {
                write(&format!("\n{stmt}\n"), &mut out)?;
            }
            write("\nGO\n", &mut out)?;
        }
        out.flush().map_err(io_error)?;
        drop(out);
        fs::rename(&part.path, &dest).map_err(io_error)?;
        part.keep = true;
        let _ = app.emit("export-progress", Progress { export_id: &req.dump_id, rows });
        Ok(DumpResult { objects: plan.objects, tables: plan.data_tables.len(), rows, bytes: fs::metadata(&dest).map(|m| m.len()).unwrap_or(0), elapsed_ms: started.elapsed().as_millis() as u64, path: dest.to_string_lossy().into_owned() })
    }
    .await;
    if let Ok(mut m) = sessions.finds.lock() {
        m.remove(&req.dump_id);
    }
    out
}

/// Backup completo do banco: o arquivo `.bak` é gravado pelo servidor, no disco dele (`folder` é uma pasta do servidor; sem ela vale
/// a pasta padrão de backups da instância). O SQL Server não faz backup de um schema só.
#[tauri::command]
pub async fn mssql_backup(sessions: State<'_, MssqlSessions>, req: BackupRequest) -> Result<BackupResult, DatabaseError> {
    let started = Instant::now();
    let s = sessions.session(&req.session_id)?;
    let name: String = req.file_name.chars().map(|c| if c.is_alphanumeric() || matches!(c, '.' | '-' | '_') { c } else { '_' }).collect();
    let folder = match req.folder.as_deref().map(str::trim).filter(|f| !f.is_empty()) {
        Some(f) => f.to_string(),
        None => s.default_backup_dir().await?,
    };
    let sep = if folder.contains('\\') { '\\' } else { '/' };
    let path = format!("{}{sep}{name}.bak", folder.trim_end_matches(['/', '\\']));
    s.backup(&req.database, &path, &name, &req.comment).await?;
    let bytes = s.backup_list(&req.database).await?.into_iter().find(|b| b.path == path).map_or(0, |b| b.bytes);
    Ok(BackupResult { bytes, elapsed_ms: started.elapsed().as_millis() as u64, path, tool: "sqlserver".into(), name })
}

/// Backups completos do banco, lidos do histórico do servidor.
#[tauri::command]
pub async fn mssql_backup_list(sessions: State<'_, MssqlSessions>, session_id: String, database: String, _schema: String) -> Result<Vec<BackupEntry>, DatabaseError> {
    let rows = sessions.session(&session_id)?.backup_list(&database).await?;
    Ok(rows
        .into_iter()
        .map(|b| {
            let file = b.path.rsplit(['/', '\\']).next().unwrap_or(&b.path).to_string();
            BackupEntry { name: file.strip_suffix(".bak").unwrap_or(&file).to_string(), path: b.path, meta_path: String::new(), bytes: b.bytes, modified_ms: b.finished_ms, comment: b.description, scope: "database".into() }
        })
        .collect())
}

/// Exclui um backup: o arquivo no disco do servidor e o registro no histórico.
#[tauri::command]
pub async fn mssql_backup_delete(sessions: State<'_, MssqlSessions>, session_id: String, path: String) -> Result<(), DatabaseError> {
    sessions.session(&session_id)?.backup_delete(&path).await
}

/// Restaura um backup (caminho no servidor): o banco passa a ter o conteúdo do arquivo.
#[tauri::command]
pub async fn mssql_restore(sessions: State<'_, MssqlSessions>, req: RestoreRequest) -> Result<BackupResult, DatabaseError> {
    let started = Instant::now();
    if req.extract_to.is_some() {
        return Err(DatabaseError::new(ErrorKind::UnsupportedFeature, "extracting SQL from a backup is not available for SQL Server"));
    }
    sessions.session(&req.session_id)?.restore(&req.database, &req.path).await?;
    Ok(BackupResult { bytes: 0, elapsed_ms: started.elapsed().as_millis() as u64, path: req.path, tool: "sqlserver".into(), name: String::new() })
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
