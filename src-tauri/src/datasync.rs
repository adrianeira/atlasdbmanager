//! Data Synchronization: compara os dados de dois schemas, mostra as diferenças, gera o script e o executa.
//!
//! O resultado da comparação (com todas as linhas diferentes) fica na memória do app, guardado pelo `compare_id`; a tela
//! só recebe o resumo e páginas de linhas. A implantação usa a mesma comparação, com o que o usuário marcou.

use crate::export::Exports;
use crate::sessions::Sessions;
use atlas_core::{DatabaseError, ErrorKind};
use atlas_pg::{CompareHooks, Comparison, DeployHooks, DiffKind, SyncSpec, TableSelection};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tauri::{AppHandle, Emitter, State};

/// Comparações feitas e ainda não descartadas (uma por janela).
#[derive(Default)]
pub struct Comparisons {
    inner: Mutex<HashMap<String, Arc<Comparison>>>,
}

impl Comparisons {
    fn get(&self, id: &str) -> Result<Arc<Comparison>, DatabaseError> {
        self.inner
            .lock()
            .unwrap()
            .get(id)
            .cloned()
            .ok_or_else(|| DatabaseError::new(ErrorKind::InvalidInput, "the comparison is no longer available; compare again"))
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompareRequest {
    pub compare_id: String,
    pub source_conn: String,
    pub source_database: String,
    pub target_conn: String,
    pub target_database: String,
    pub spec: SyncSpec,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TableSummary {
    pub source: String,
    pub target: String,
    pub insert: u64,
    pub update: u64,
    pub delete: u64,
    pub same: u64,
    pub message: Option<String>,
    pub source_fields: Vec<String>,
    pub target_fields: Vec<String>,
    pub key_positions: Vec<usize>,
    /// Linhas diferentes guardadas (inserir + atualizar + excluir).
    pub rows: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompareSummary {
    pub tables: Vec<TableSummary>,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct ProgressEvent<'a> {
    id: &'a str,
    text: String,
    fraction: f64,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct LogEvent<'a> {
    id: &'a str,
    line: String,
}

#[tauri::command]
pub async fn datasync_compare(
    app: AppHandle,
    sessions: State<'_, Sessions>,
    exports: State<'_, Exports>,
    comparisons: State<'_, Comparisons>,
    req: CompareRequest,
) -> Result<CompareSummary, DatabaseError> {
    comparisons.inner.lock().unwrap().remove(&req.compare_id);
    let src = sessions.dedicated(&req.source_conn, &req.source_database).await?;
    let dst = Arc::new(sessions.dedicated(&req.target_conn, &req.target_database).await?);
    let (cancel, _registered) = exports.register(&req.compare_id, dst.clone());
    let (progress_app, id) = (app.clone(), req.compare_id.clone());
    let mut on_progress = move |text: &str, fraction: f64| {
        let _ = progress_app.emit("datasync-progress", ProgressEvent { id: &id, text: text.to_string(), fraction });
    };
    let cancelled = || cancel.load(Ordering::Relaxed);
    let comparison = atlas_pg::compare_data(&src, &dst, &req.spec, CompareHooks { progress: &mut on_progress, cancelled: &cancelled }).await?;
    let summary = CompareSummary {
        tables: comparison
            .tables
            .iter()
            .map(|t| TableSummary {
                source: t.source.clone(),
                target: t.target.clone(),
                insert: t.insert,
                update: t.update,
                delete: t.delete,
                same: t.same,
                message: t.message.clone(),
                source_fields: t.source_fields.clone(),
                target_fields: t.target_fields.clone(),
                key_positions: t.key_positions.clone(),
                rows: t.rows.len(),
            })
            .collect(),
    };
    comparisons.inner.lock().unwrap().insert(req.compare_id, Arc::new(comparison));
    Ok(summary)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RowOut {
    /// Número da linha na lista da tabela (o que a tela marca e desmarca).
    pub id: u32,
    pub kind: &'static str,
    pub source: Option<Vec<Option<String>>>,
    pub target: Option<Vec<Option<String>>>,
    pub changed: Vec<bool>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RowsPage {
    /// Linhas que passam pelo filtro (para a paginação).
    pub total: usize,
    pub rows: Vec<RowOut>,
}

fn kind_name(k: DiffKind) -> &'static str {
    match k {
        DiffKind::Insert => "insert",
        DiffKind::Update => "update",
        DiffKind::Delete => "delete",
    }
}

#[tauri::command]
pub fn datasync_rows(
    comparisons: State<'_, Comparisons>,
    compare_id: String,
    table: String,
    kind: Option<String>,
    offset: usize,
    limit: usize,
) -> Result<RowsPage, DatabaseError> {
    let c = comparisons.get(&compare_id)?;
    let t = c.tables.iter().find(|t| t.source == table).ok_or_else(|| DatabaseError::new(ErrorKind::InvalidInput, "table not in the comparison"))?;
    let matching: Vec<(usize, &atlas_pg::DiffRow)> = t.rows.iter().enumerate().filter(|(_, r)| kind.as_deref().is_none_or(|k| k == kind_name(r.kind))).collect();
    let rows = matching
        .iter()
        .skip(offset)
        .take(limit)
        .map(|(i, r)| RowOut { id: *i as u32, kind: kind_name(r.kind), source: r.source.clone(), target: r.target.clone(), changed: r.changed.clone() })
        .collect();
    Ok(RowsPage { total: matching.len(), rows })
}

/// Linhas do script mostradas na tela (o script inteiro é executado do mesmo jeito).
const SCRIPT_PREVIEW: usize = 5000;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScriptOut {
    pub statements: usize,
    pub text: String,
    pub truncated: bool,
}

#[tauri::command]
pub fn datasync_script(comparisons: State<'_, Comparisons>, compare_id: String, selection: Vec<TableSelection>) -> Result<ScriptOut, DatabaseError> {
    let c = comparisons.get(&compare_id)?;
    let statements = atlas_pg::data_script(&c, &selection);
    let truncated = statements.len() > SCRIPT_PREVIEW;
    let text = statements.iter().take(SCRIPT_PREVIEW).cloned().collect::<Vec<_>>().join("\n\n");
    Ok(ScriptOut { statements: statements.len(), text, truncated })
}

/// Script completo, para "Copy Script".
#[tauri::command]
pub fn datasync_script_full(comparisons: State<'_, Comparisons>, compare_id: String, selection: Vec<TableSelection>) -> Result<String, DatabaseError> {
    let c = comparisons.get(&compare_id)?;
    Ok(atlas_pg::data_script(&c, &selection).join("\n\n"))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeployRequest {
    pub deploy_id: String,
    pub compare_id: String,
    pub target_conn: String,
    pub target_database: String,
    pub selection: Vec<TableSelection>,
    pub transaction: bool,
    pub continue_on_error: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeployCounts {
    pub total: u64,
    pub done: u64,
    pub errors: u64,
    pub rolled_back: bool,
    pub elapsed_ms: u64,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct DeployProgress<'a> {
    id: &'a str,
    done: u64,
    total: u64,
}

#[tauri::command]
pub async fn datasync_deploy(
    app: AppHandle,
    sessions: State<'_, Sessions>,
    exports: State<'_, Exports>,
    comparisons: State<'_, Comparisons>,
    req: DeployRequest,
) -> Result<DeployCounts, DatabaseError> {
    let started = Instant::now();
    let c = comparisons.get(&req.compare_id)?;
    let statements = atlas_pg::data_script(&c, &req.selection);
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
    let stats = atlas_pg::deploy_data(&dst, &statements, req.transaction, req.continue_on_error, DeployHooks { log: &mut on_log, progress: &mut on_progress, cancelled: &cancelled, echo: false }).await?;
    Ok(DeployCounts { total: stats.total, done: stats.done, errors: stats.errors, rolled_back: stats.rolled_back, elapsed_ms: started.elapsed().as_millis() as u64 })
}

/// Descarta uma comparação (a janela fechou ou vai comparar de novo).
#[tauri::command]
pub fn datasync_forget(comparisons: State<'_, Comparisons>, compare_id: String) {
    comparisons.inner.lock().unwrap().remove(&compare_id);
}
