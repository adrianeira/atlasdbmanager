//! Data Generation: descreve as tabelas de um schema, pré-visualiza dados de teste e os insere.

use crate::export::Exports;
use crate::sessions::Sessions;
use atlas_core::DatabaseError;
use atlas_pg::{GenHooks, GenPreview, GenSpec, GenStats, GenTableMeta, GenTableSpec};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Instant;
use tauri::{AppHandle, Emitter, State};

#[tauri::command]
pub async fn datagen_describe(sessions: State<'_, Sessions>, conn: String, database: String, schema: String) -> Result<Vec<GenTableMeta>, DatabaseError> {
    let s = sessions.dedicated(&conn, &database).await?;
    atlas_pg::describe_generation(&s, &schema).await
}

#[tauri::command]
pub async fn datagen_preview(
    sessions: State<'_, Sessions>,
    conn: String,
    database: String,
    schema: String,
    table: GenTableSpec,
    rows: u64,
) -> Result<GenPreview, DatabaseError> {
    let s = sessions.dedicated(&conn, &database).await?;
    atlas_pg::preview_generation(&s, &schema, &table, rows.min(1000)).await
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunRequest {
    pub run_id: String,
    pub conn: String,
    pub database: String,
    pub spec: GenSpec,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunResult {
    #[serde(flatten)]
    pub stats: GenStats,
    pub elapsed_ms: u64,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct LogEvent<'a> {
    id: &'a str,
    line: String,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct ProgressEvent<'a> {
    id: &'a str,
    #[serde(flatten)]
    stats: GenStats,
}

#[tauri::command]
pub async fn datagen_run(app: AppHandle, sessions: State<'_, Sessions>, exports: State<'_, Exports>, req: RunRequest) -> Result<RunResult, DatabaseError> {
    let started = Instant::now();
    let s = Arc::new(sessions.dedicated(&req.conn, &req.database).await?);
    let (cancel, _registered) = exports.register(&req.run_id, s.clone());
    let (log_app, log_id) = (app.clone(), req.run_id.clone());
    let mut on_log = move |line: String| {
        let _ = log_app.emit("datagen-log", LogEvent { id: &log_id, line });
    };
    let (progress_app, progress_id) = (app.clone(), req.run_id.clone());
    let mut on_progress = move |stats: &GenStats| {
        let _ = progress_app.emit("datagen-progress", ProgressEvent { id: &progress_id, stats: stats.clone() });
    };
    let cancelled = || cancel.load(Ordering::Relaxed);
    let stats = atlas_pg::run_generation(&s, &req.spec, GenHooks { log: &mut on_log, progress: &mut on_progress, cancelled: &cancelled }).await?;
    Ok(RunResult { stats, elapsed_ms: started.elapsed().as_millis() as u64 })
}
