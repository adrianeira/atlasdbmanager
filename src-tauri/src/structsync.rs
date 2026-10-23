//! Structure Synchronization: compara a estrutura de dois schemas, mostra as diferenças e executa o script no destino.
//!
//! A comparação fica na memória, guardada pelo `compare_id`; a tela recebe o resultado (objetos, partes e o DDL dos dois
//! lados) e depois pede o script do que está marcado. A implantação executa a lista de comandos que a tela mandar (ela
//! pode ter sido reordenada), sem transação: alguns comandos, como `ALTER TYPE ... ADD VALUE`, não rodam dentro de uma.

use crate::datasync::{DeployCounts, DeployProgress, LogEvent, ProgressEvent};
use crate::export::Exports;
use crate::sessions::Sessions;
use atlas_core::{DatabaseError, ErrorKind};
use atlas_pg::{CompareHooks, DeployHooks, StructComparison, StructOp, StructPart, StructSelection, StructSpec};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tauri::{AppHandle, Emitter, State};

#[derive(Default)]
pub struct StructComparisons {
    inner: Mutex<HashMap<String, Arc<StructComparison>>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StructCompareRequest {
    pub compare_id: String,
    pub source_conn: String,
    pub source_database: String,
    pub target_conn: String,
    pub target_database: String,
    pub spec: StructSpec,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ObjectOut {
    pub kind: &'static str,
    pub name: String,
    pub op: StructOp,
    pub parts: Vec<StructPart>,
    pub source_ddl: String,
    pub target_ddl: String,
}

#[tauri::command]
pub async fn structsync_compare(
    app: AppHandle,
    sessions: State<'_, Sessions>,
    exports: State<'_, Exports>,
    comparisons: State<'_, StructComparisons>,
    req: StructCompareRequest,
) -> Result<Vec<ObjectOut>, DatabaseError> {
    comparisons.inner.lock().unwrap().remove(&req.compare_id);
    let src = sessions.dedicated(&req.source_conn, &req.source_database).await?;
    let dst = Arc::new(sessions.dedicated(&req.target_conn, &req.target_database).await?);
    let (cancel, _registered) = exports.register(&req.compare_id, dst.clone());
    let (progress_app, id) = (app.clone(), req.compare_id.clone());
    let mut on_progress = move |text: &str, fraction: f64| {
        let _ = progress_app.emit("datasync-progress", ProgressEvent { id: &id, text: text.to_string(), fraction });
    };
    let cancelled = || cancel.load(Ordering::Relaxed);
    let comparison = atlas_pg::compare_structure(&src, &dst, &req.spec, CompareHooks { progress: &mut on_progress, cancelled: &cancelled }).await?;
    let out = comparison
        .objects
        .iter()
        .map(|o| ObjectOut { kind: o.kind, name: o.name.clone(), op: o.op, parts: o.parts.clone(), source_ddl: o.source_ddl.clone(), target_ddl: o.target_ddl.clone() })
        .collect();
    comparisons.inner.lock().unwrap().insert(req.compare_id, Arc::new(comparison));
    Ok(out)
}

/// Os comandos do que está marcado, na ordem em que devem rodar.
#[tauri::command]
pub fn structsync_script(comparisons: State<'_, StructComparisons>, compare_id: String, selection: Vec<StructSelection>) -> Result<Vec<String>, DatabaseError> {
    let c = comparisons
        .inner
        .lock()
        .unwrap()
        .get(&compare_id)
        .cloned()
        .ok_or_else(|| DatabaseError::new(ErrorKind::InvalidInput, "the comparison is no longer available; compare again"))?;
    Ok(atlas_pg::structure_script(&c, &selection))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StructDeployRequest {
    pub deploy_id: String,
    pub target_conn: String,
    pub target_database: String,
    pub statements: Vec<String>,
    pub continue_on_error: bool,
    /// Registra cada comando no log.
    pub include_query: bool,
}

#[tauri::command]
pub async fn structsync_deploy(app: AppHandle, sessions: State<'_, Sessions>, exports: State<'_, Exports>, req: StructDeployRequest) -> Result<DeployCounts, DatabaseError> {
    let started = Instant::now();
    let dst = Arc::new(sessions.dedicated(&req.target_conn, &req.target_database).await?);
    let (cancel, _registered) = exports.register(&req.deploy_id, dst.clone());
    let (log_app, log_id) = (app.clone(), req.deploy_id.clone());
    let mut on_log = move |line: String| {
        let _ = log_app.emit("datasync-log", LogEvent { id: &log_id, line });
    };
    let (progress_app, progress_id) = (app.clone(), req.deploy_id.clone());
    let mut on_progress = move |done: u64, total: u64| {
        let _ = progress_app.emit("datasync-deploy-progress", DeployProgress { id: &progress_id, done, total });
    };
    let cancelled = || cancel.load(Ordering::Relaxed);
    let stats = atlas_pg::deploy_data(
        &dst,
        &req.statements,
        false,
        req.continue_on_error,
        DeployHooks { log: &mut on_log, progress: &mut on_progress, cancelled: &cancelled, echo: req.include_query },
    )
    .await?;
    Ok(DeployCounts { total: stats.total, done: stats.done, errors: stats.errors, rolled_back: stats.rolled_back, elapsed_ms: started.elapsed().as_millis() as u64 })
}

#[tauri::command]
pub fn structsync_forget(comparisons: State<'_, StructComparisons>, compare_id: String) {
    comparisons.inner.lock().unwrap().remove(&compare_id);
}
