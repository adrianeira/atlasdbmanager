//! Exportação de tabelas/views para arquivo (CSV, JSON, SQL, Excel), em streaming.
//!
//! O arquivo é escrito em `<destino>.part` e só vira o destino ao terminar com sucesso; em erro ou
//! cancelamento o `.part` é apagado e o arquivo anterior (se houver) fica intocado.

use crate::sessions::Sessions;
use atlas_core::{DatabaseError, ErrorKind, OrderBy, TableFilter};
use atlas_export::{ExportColumn, ExportFormat};
use atlas_pg::PgSession;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, State};

/// Exportações em andamento (para o botão Cancelar).
#[derive(Default)]
pub struct Exports {
    active: Mutex<HashMap<String, Active>>,
}

/// Remove o registro da exportação/dump ao sair, de qualquer forma.
pub(crate) struct Registration<'a>(&'a Exports, String);

impl Drop for Registration<'_> {
    fn drop(&mut self) {
        self.0.active.lock().unwrap().remove(&self.1);
    }
}

impl Exports {
    /// Registra uma operação longa para o botão Cancelar; devolve a bandeira de cancelamento e o registro.
    pub(crate) fn register(&self, id: &str, session: Arc<PgSession>) -> (Arc<AtomicBool>, Registration<'_>) {
        let cancel = Arc::new(AtomicBool::new(false));
        self.active.lock().unwrap().insert(id.to_string(), Active { cancel: cancel.clone(), session });
        (cancel, Registration(self, id.to_string()))
    }
}

pub(crate) struct Active {
    cancel: Arc<AtomicBool>,
    /// Conexão exclusiva da exportação: o cancelamento vai direto ao servidor, sem esperar uma linha chegar.
    session: Arc<PgSession>,
}

/// Consulta do editor SQL usada como origem (em vez de uma tabela).
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuerySource {
    pub sql: String,
    /// Schema escolhido no editor: onde os nomes sem schema da consulta são procurados.
    pub schema: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportRequest {
    pub export_id: String,
    pub session_id: String,
    pub database: String,
    pub schema: String,
    pub table: String,
    /// Vazio = todas as colunas.
    #[serde(default)]
    pub columns: Vec<String>,
    #[serde(default)]
    pub order_by: Vec<OrderBy>,
    pub filter: Option<TableFilter>,
    /// Com consulta, `table` é só o nome usado no XML/HTML e no destino do `INSERT`.
    #[serde(default)]
    pub query: Option<QuerySource>,
    pub format: ExportFormat,
    pub path: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportResult {
    pub rows: u64,
    pub bytes: u64,
    pub elapsed_ms: u64,
    pub path: String,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct Progress<'a> {
    export_id: &'a str,
    rows: u64,
}

pub(crate) fn io_error(e: std::io::Error) -> DatabaseError {
    DatabaseError::new(ErrorKind::Internal, e.to_string())
}

/// Apaga o arquivo temporário ao sair, a menos que a exportação tenha terminado bem.
pub(crate) struct PartFile {
    pub(crate) path: PathBuf,
    pub(crate) keep: bool,
}

impl PartFile {
    pub(crate) fn new(path: PathBuf) -> Self {
        Self { path, keep: false }
    }
}

impl Drop for PartFile {
    fn drop(&mut self) {
        if !self.keep {
            let _ = fs::remove_file(&self.path);
        }
    }
}

/// Garante que o destino é um caminho de arquivo utilizável: absoluto, com pasta existente e
/// que não seja uma pasta.
pub(crate) fn validate_path(path: &str) -> Result<PathBuf, DatabaseError> {
    let p = PathBuf::from(path);
    let invalid = |msg: &str| DatabaseError::new(ErrorKind::InvalidInput, msg.to_string());
    if path.trim().is_empty() || !p.is_absolute() {
        return Err(invalid("o destino precisa ser um caminho de arquivo completo"));
    }
    if p.is_dir() || p.file_name().is_none() {
        return Err(invalid("o destino é uma pasta; informe o nome do arquivo"));
    }
    match p.parent() {
        Some(dir) if dir.is_dir() => Ok(p),
        _ => Err(invalid("a pasta de destino não existe")),
    }
}

pub(crate) fn part_path(dest: &Path) -> PathBuf {
    let mut name = dest.file_name().map(|n| n.to_os_string()).unwrap_or_default();
    name.push(".part");
    dest.with_file_name(name)
}

#[tauri::command]
pub async fn pg_export(
    app: AppHandle,
    sessions: State<'_, Sessions>,
    exports: State<'_, Exports>,
    req: ExportRequest,
) -> Result<ExportResult, DatabaseError> {
    let started = Instant::now();
    let dest = validate_path(&req.path)?;
    // O texto a executar. Consulta do editor: precisa ser um único comando (não separamos comandos no cliente).
    let sql = match &req.query {
        Some(q) => {
            if q.sql.trim().is_empty() || !atlas_pg::is_single_statement(&q.sql) {
                return Err(DatabaseError::new(ErrorKind::InvalidInput, "a exportação aceita uma única consulta (um SELECT): selecione só o comando desejado"));
            }
            atlas_pg::wrap_query(&q.sql, &req.columns)
        }
        None => atlas_pg::select_sql(&req.schema, &req.table, &req.columns, &req.order_by, req.filter.as_ref()),
    };
    // Uma conexão exclusiva: a consulta longa não trava as abas abertas, e cancelar não as atinge.
    let session = Arc::new(sessions.dedicated(&req.session_id, &req.database).await?);
    // Nomes sem schema da consulta valem no schema escolhido no editor (a conexão é só da exportação).
    if let Some(schema) = req.query.as_ref().and_then(|q| q.schema.as_deref()).filter(|x| !x.is_empty()) {
        let _ = session.client.simple_query(&format!("SET search_path TO {}", atlas_pg::search_path_for(schema))).await;
    }

    let (cancel, _registered) = exports.register(&req.export_id, session.clone());

    let mut part = PartFile { path: part_path(&dest), keep: false };
    let file = File::create(&part.path).map_err(io_error)?;
    let target = format!("{}.{}", atlas_pg::quote_ident(&req.schema), atlas_pg::quote_ident(&req.table));
    // Os dois callbacks (início e cada linha) usam o mesmo gravador, nunca ao mesmo tempo (Mutex só para ser `Send`).
    let writer = Mutex::new(atlas_export::create(&req.format, &atlas_export::WriterTarget { sql: &target, title: &req.table }, file).map_err(io_error)?);

    let began = AtomicBool::new(false);
    let mut last_emit = Instant::now();
    let mut rows = 0u64;
    let export_id = req.export_id.clone();

    let streamed = atlas_pg::stream_rows(
        &session,
        &sql,
        |cols| {
            let cols: Vec<ExportColumn> = cols.iter().map(|c| ExportColumn { name: c.name.clone(), type_name: c.type_name.clone() }).collect();
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
                let _ = app.emit("export-progress", Progress { export_id: &export_id, rows });
            }
            Ok(true)
        },
    )
    .await;

    // Cancelado: o erro do servidor (57014) ou a parada na próxima linha não importam; o `.part` é apagado.
    if cancel.load(Ordering::Relaxed) {
        return Err(DatabaseError::new(ErrorKind::Canceled, "export canceled"));
    }
    streamed?;
    // Um comando que não devolve linhas (UPDATE, DDL...) não tem o que exportar.
    if !began.load(Ordering::Relaxed) {
        return Err(DatabaseError::new(ErrorKind::InvalidInput, "o comando não retorna linhas para exportar"));
    }
    writer.into_inner().unwrap().finish().map_err(io_error)?;
    fs::rename(&part.path, &dest).map_err(io_error)?;
    part.keep = true;
    let _ = app.emit("export-progress", Progress { export_id: &req.export_id, rows });

    Ok(ExportResult {
        rows,
        bytes: fs::metadata(&dest).map(|m| m.len()).unwrap_or(0),
        elapsed_ms: started.elapsed().as_millis() as u64,
        path: dest.to_string_lossy().into_owned(),
    })
}

/// Pede o cancelamento: a gravação para na próxima linha e o servidor é avisado para abortar a consulta.
#[tauri::command]
pub async fn pg_export_cancel(exports: State<'_, Exports>, export_id: String) -> Result<(), DatabaseError> {
    let found = exports.active.lock().unwrap().get(&export_id).map(|a| (a.cancel.clone(), a.session.clone()));
    if let Some((cancel, session)) = found {
        cancel.store(true, Ordering::Relaxed);
        // Sem efeito se a consulta já terminou; a conexão é exclusiva, então nada mais é atingido.
        let _ = session.cancel().await;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_relative_missing_dir_and_directory_targets() {
        assert!(validate_path("").is_err());
        assert!(validate_path("saida.csv").is_err());
        let dir = std::env::temp_dir();
        assert!(validate_path(dir.to_str().unwrap()).is_err());
        assert!(validate_path(dir.join("nao-existe-xyz").join("a.csv").to_str().unwrap()).is_err());
        assert!(validate_path(dir.join("ok-atlas.csv").to_str().unwrap()).is_ok());
    }

    #[test]
    fn part_file_sits_next_to_the_destination() {
        let p = part_path(Path::new("C:/x/dados.csv"));
        assert_eq!(p.file_name().unwrap(), "dados.csv.part");
    }
}
