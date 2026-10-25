//! Assistente de importação: lista as tabelas de um arquivo, mostra os campos e as primeiras linhas, e importa.
//!
//! A leitura (TXT/CSV, JSON, XML, Excel, dBase) está em `atlas-import`; a gravação nos seis modos, em
//! `atlas_pg::run_import`. O progresso e o registro seguem por eventos e o cancelamento usa `pg_export_cancel`.

use crate::export::Exports;
use crate::sessions::Sessions;
use atlas_core::{DatabaseError, ErrorKind};
use atlas_import::convert::{ConvertOptions, TargetKind, convert, kind_of};
use atlas_import::{ImportFormat, SourceSpec};
use atlas_pg::{Feed, ImportHooks, ImportMode, ImportSpec, TargetColumn};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Instant;
use tauri::{AppHandle, Emitter, State};

/// Linhas lidas para escolher o tipo das colunas de uma tabela nova.
const INFER_SAMPLE: usize = 1000;

fn io_error(e: std::io::Error) -> DatabaseError {
    DatabaseError::new(ErrorKind::InvalidInput, e.to_string())
}

#[tauri::command]
pub async fn import_list_tables(path: String, format: ImportFormat) -> Result<Vec<String>, DatabaseError> {
    let path = PathBuf::from(path);
    tauri::async_runtime::spawn_blocking(move || atlas_import::list_tables(&path, &format).map_err(io_error))
        .await
        .map_err(|e| DatabaseError::new(ErrorKind::Internal, e.to_string()))?
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportPreview {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Option<String>>>,
    /// Tipo PostgreSQL sugerido para cada campo (para criar uma tabela nova).
    pub types: Vec<String>,
}

#[tauri::command]
pub async fn import_preview(spec: SourceSpec, convert: ConvertOptions, limit: usize) -> Result<ImportPreview, DatabaseError> {
    tauri::async_runtime::spawn_blocking(move || {
        // Uma leitura só: as primeiras `limit` linhas para mostrar e até 1000 para deduzir os tipos.
        let mut src = atlas_import::open(&spec).map_err(io_error)?;
        let mut sample: Vec<Vec<Option<String>>> = Vec::new();
        while sample.len() < INFER_SAMPLE.max(limit) {
            match src.next_row().map_err(io_error)? {
                Some(r) => sample.push(r),
                None => break,
            }
        }
        let columns = src.columns().to_vec();
        let types = (0..columns.len()).map(|i| atlas_import::infer::infer_type(sample.iter().map(|r| r[i].as_deref()), &convert)).collect();
        sample.truncate(limit);
        Ok(ImportPreview { columns, rows: sample, types })
    })
    .await
    .map_err(|e| DatabaseError::new(ErrorKind::Internal, e.to_string()))?
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Mapping {
    /// Campo da origem.
    pub source: String,
    /// Campo do destino (vazio = não importar).
    pub target: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewColumn {
    pub name: String,
    #[serde(rename = "type")]
    pub type_name: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportJob {
    pub source: SourceSpec,
    /// Tabela de destino (existente, ou o nome da nova).
    pub target: String,
    /// Tabela nova: as colunas a criar.
    #[serde(default)]
    pub create: Option<Vec<NewColumn>>,
    pub mappings: Vec<Mapping>,
    /// Campos do destino que são a chave (modos que casam origem e destino).
    #[serde(default)]
    pub keys: Vec<String>,
    pub mode: ImportMode,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportAdvanced {
    pub extended_insert: bool,
    pub max_statement_kb: u32,
    pub empty_as_null: bool,
    pub continue_on_error: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportRequest {
    pub import_id: String,
    pub session_id: String,
    pub database: String,
    pub schema: String,
    pub jobs: Vec<ImportJob>,
    pub advanced: ImportAdvanced,
    #[serde(default)]
    pub convert: ConvertOptions,
}

#[derive(Serialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct JobResult {
    pub table: String,
    pub processed: u64,
    pub added: u64,
    pub updated: u64,
    pub deleted: u64,
    pub errors: u64,
    pub rolled_back: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportSummary {
    pub jobs: Vec<JobResult>,
    pub elapsed_ms: u64,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProgressEvent<'a> {
    pub(crate) import_id: &'a str,
    pub(crate) job: usize,
    #[serde(flatten)]
    pub(crate) counts: JobResult,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LogEvent<'a> {
    pub(crate) import_id: &'a str,
    pub(crate) line: String,
}

pub(crate) fn failed_job(table: &str, message: String, app: &AppHandle, id: &str) -> JobResult {
    let _ = app.emit("import-log", LogEvent { import_id: id, line: format!("[ERR] {message}") });
    JobResult { table: table.to_string(), errors: 1, rolled_back: true, ..Default::default() }
}

#[tauri::command]
pub async fn pg_import(
    app: AppHandle,
    sessions: State<'_, Sessions>,
    exports: State<'_, Exports>,
    req: ImportRequest,
) -> Result<ImportSummary, DatabaseError> {
    let started = Instant::now();
    let session = Arc::new(sessions.dedicated(&req.session_id, &req.database).await?);
    let (cancel, _registered) = exports.register(&req.import_id, session.clone());
    let mut results = Vec::new();

    for (job_no, job) in req.jobs.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            return Err(DatabaseError::new(ErrorKind::Canceled, "import canceled"));
        }
        let _ = app.emit("import-log", LogEvent { import_id: &req.import_id, line: format!("[IMP] {}", job.target) });
        // Colunas do destino: as da tabela (lidas do servidor) ou as que serão criadas.
        let target_columns: Vec<(String, String, bool, bool)> = match &job.create {
            Some(cols) => cols.iter().map(|c| (c.name.clone(), c.type_name.clone(), false, false)).collect(),
            None => match atlas_pg::table_detail(&session, &req.schema, &job.target).await {
                Ok(d) => d.columns.iter().map(|c| (c.name.clone(), c.formatted_type.clone(), c.identity.as_deref() == Some("a"), c.generated.as_deref() == Some("s"))).collect(),
                Err(e) => {
                    results.push(failed_job(&job.target, e.message, &app, &req.import_id));
                    continue;
                }
            },
        };
        let mut source = match atlas_import::open(&job.source) {
            Ok(s) => s,
            Err(e) => {
                results.push(failed_job(&job.target, format!("{}: {e}", job.source.path.display()), &app, &req.import_id));
                continue;
            }
        };
        // Mapeamento: posição do campo na origem, coluna do destino e a categoria do tipo para converter.
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
            let Some((name, ty, identity_always, generated)) = target_columns.iter().find(|(n, ..)| *n == m.target) else {
                problem = Some(format!("destination field \"{}\" does not exist in {}", m.target, job.target));
                break;
            };
            if *generated {
                problem = Some(format!("destination field \"{name}\" is a generated column and cannot receive data"));
                break;
            }
            columns.push(TargetColumn { name: name.clone(), cast_type: ty.clone(), identity_always: *identity_always });
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

        let spec = ImportSpec {
            schema: req.schema.clone(),
            table: job.target.clone(),
            create: job.create.as_ref().map(|c| c.iter().map(|x| (x.name.clone(), x.type_name.clone())).collect()),
            columns,
            key_columns,
            mode: job.mode,
            extended_insert: req.advanced.extended_insert,
            max_statement_bytes: req.advanced.max_statement_kb.max(1) as usize * 1024,
            continue_on_error: req.advanced.continue_on_error,
            manage_transaction: true,
        };
        let mut convert_opts = req.convert.clone();
        convert_opts.empty_as_null = req.advanced.empty_as_null;
        let mut feed = || -> Feed {
            match source.next_row() {
                Err(e) => Feed::Bad(e.to_string()),
                Ok(None) => Feed::End,
                Ok(Some(row)) => {
                    let mut out = Vec::with_capacity(spec.columns.len());
                    for (n, &si) in source_index.iter().enumerate() {
                        let value = row[si].as_deref();
                        let converted = match value {
                            None => Ok(None),
                            Some(v) => convert(v, kinds[n], &convert_opts),
                        };
                        match converted {
                            Ok(v) => out.push(v),
                            Err(e) => return Feed::Bad(format!("{} ({})", e, spec.columns[n].name)),
                        }
                    }
                    Feed::Row(out)
                }
            }
        };
        let import_id = req.import_id.clone();
        let app_log = app.clone();
        let app_progress = app.clone();
        let target = job.target.clone();
        let mut on_log = move |line: String| {
            let _ = app_log.emit("import-log", LogEvent { import_id: &import_id, line });
        };
        let progress_id = req.import_id.clone();
        let progress_target = target.clone();
        let mut on_progress = move |s: &atlas_pg::ImportStats| {
            let counts = JobResult {
                table: progress_target.clone(),
                processed: s.processed,
                added: s.added,
                updated: s.updated,
                deleted: s.deleted,
                errors: s.errors,
                rolled_back: false,
            };
            let _ = app_progress.emit("import-progress", ProgressEvent { import_id: &progress_id, job: job_no, counts });
        };
        let cancelled = || cancel.load(Ordering::Relaxed);
        let stats = atlas_pg::run_import(&session, &spec, &mut feed, ImportHooks { log: &mut on_log, progress: &mut on_progress, cancelled: &cancelled }).await?;
        let result = JobResult {
            table: target,
            processed: stats.processed,
            added: stats.added,
            updated: stats.updated,
            deleted: stats.deleted,
            errors: stats.errors,
            rolled_back: stats.rolled_back,
        };
        let _ = app.emit("import-progress", ProgressEvent { import_id: &req.import_id, job: job_no, counts: result.clone() });
        results.push(result);
    }
    Ok(ImportSummary { jobs: results, elapsed_ms: started.elapsed().as_millis() as u64 })
}
