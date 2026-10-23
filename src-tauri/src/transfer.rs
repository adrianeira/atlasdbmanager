//! Data Transfer: copia objetos de um schema para outro schema/servidor.
//!
//! O trabalho está em `atlas_pg::run_transfer`; aqui ficam as conexões exclusivas da origem e do destino, o progresso e
//! o registro por eventos (`transfer-progress`, `transfer-log`) e o cancelamento (`pg_export_cancel` com o mesmo id).

use crate::export::Exports;
use crate::sessions::Sessions;
use atlas_core::DatabaseError;
use atlas_pg::{TransferHooks, TransferSpec, TransferStats};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Instant;
use tauri::{AppHandle, Emitter, State};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransferRequest {
    /// Identifica a transferência para o progresso e o cancelamento.
    pub transfer_id: String,
    pub source_conn: String,
    pub source_database: String,
    pub target_conn: String,
    pub target_database: String,
    pub spec: TransferSpec,
}

#[derive(Serialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct TransferCounts {
    pub objects: u64,
    pub processed: u64,
    pub transferred: u64,
    pub errors: u64,
    pub elapsed_ms: u64,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct ProgressEvent<'a> {
    transfer_id: &'a str,
    #[serde(flatten)]
    counts: TransferCounts,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct LogEvent<'a> {
    transfer_id: &'a str,
    line: String,
}

fn counts(s: &TransferStats, started: Instant) -> TransferCounts {
    TransferCounts { objects: s.objects, processed: s.processed, transferred: s.transferred, errors: s.errors, elapsed_ms: started.elapsed().as_millis() as u64 }
}

#[tauri::command]
pub async fn pg_transfer(
    app: AppHandle,
    sessions: State<'_, Sessions>,
    exports: State<'_, Exports>,
    req: TransferRequest,
) -> Result<TransferCounts, DatabaseError> {
    let started = Instant::now();
    let src = sessions.dedicated(&req.source_conn, &req.source_database).await?;
    let dst = Arc::new(sessions.dedicated(&req.target_conn, &req.target_database).await?);
    let (cancel, _registered) = exports.register(&req.transfer_id, dst.clone());

    let (log_app, log_id) = (app.clone(), req.transfer_id.clone());
    let mut on_log = move |line: String| {
        let _ = log_app.emit("transfer-log", LogEvent { transfer_id: &log_id, line });
    };
    let (progress_app, progress_id) = (app.clone(), req.transfer_id.clone());
    let mut on_progress = move |s: &TransferStats| {
        let _ = progress_app.emit("transfer-progress", ProgressEvent { transfer_id: &progress_id, counts: counts(s, started) });
    };
    let cancelled = || cancel.load(Ordering::Relaxed);
    let stats = atlas_pg::run_transfer(&src, &dst, &req.spec, TransferHooks { log: &mut on_log, progress: &mut on_progress, cancelled: &cancelled }).await?;
    Ok(counts(&stats, started))
}
