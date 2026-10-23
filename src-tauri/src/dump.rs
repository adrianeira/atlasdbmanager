//! Dump SQL de uma tabela, de um schema ou do banco inteiro (estrutura e/ou dados), em streaming.
//!
//! O arquivo é escrito em `<destino>.part` e só vira o destino ao terminar bem. O progresso e o cancelamento usam os
//! mesmos canais da exportação (`export-progress` e `pg_export_cancel`).

use crate::export::{io_error, part_path, validate_path, Exports, PartFile};
use crate::sessions::Sessions;
use atlas_core::{DatabaseError, ErrorKind};
use atlas_export::{ExportColumn, ExportFormat};
use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::io::{self, BufWriter, Write};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, State};

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum DataFormat {
    /// `COPY ... FROM stdin;` (rápido, precisa do `psql`).
    Copy,
    /// Comandos `INSERT` (qualquer cliente SQL).
    Insert,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DumpRequest {
    /// Identifica o dump para o progresso e o cancelamento (`pg_export_cancel`).
    pub dump_id: String,
    pub session_id: String,
    pub database: String,
    pub options: atlas_pg::DumpOptions,
    pub data_format: DataFormat,
    /// Linhas por `INSERT` (só no formato `insert`).
    #[serde(default = "one")]
    pub rows_per_insert: u32,
    pub path: String,
}

fn one() -> u32 {
    1
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DumpResult {
    pub objects: usize,
    pub tables: usize,
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

/// Destino compartilhado pelo dump e pelo gravador de `INSERT` (cada um escreve sem o outro estar no meio de uma linha).
#[derive(Clone)]
struct Sink(Arc<Mutex<BufWriter<File>>>);

impl Write for Sink {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().write(data)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.0.lock().unwrap().flush()
    }
}

#[tauri::command]
pub async fn pg_dump_sql(
    app: AppHandle,
    sessions: State<'_, Sessions>,
    exports: State<'_, Exports>,
    req: DumpRequest,
) -> Result<DumpResult, DatabaseError> {
    let started = Instant::now();
    let dest = validate_path(&req.path)?;
    if !req.options.structure && !req.options.data {
        return Err(DatabaseError::new(ErrorKind::InvalidInput, "escolha ao menos estrutura ou dados"));
    }
    let session = Arc::new(sessions.dedicated(&req.session_id, &req.database).await?);
    let (cancel, _registered) = exports.register(&req.dump_id, session.clone());

    let mut part = PartFile::new(part_path(&dest));
    let out = Sink(Arc::new(Mutex::new(BufWriter::with_capacity(256 * 1024, File::create(&part.path).map_err(io_error)?))));
    let write = |text: &str| -> Result<(), DatabaseError> { out.clone().write_all(text.as_bytes()).map_err(io_error) };

    let canceled = || DatabaseError::new(ErrorKind::Canceled, "dump canceled");
    atlas_pg::begin_snapshot(&session).await?;
    // O snapshot termina de qualquer forma (leitura apenas, o ROLLBACK é só para liberar a transação).
    let result = async {
        let plan = atlas_pg::dump_plan(&session, &req.options).await?;
        write(&atlas_pg::dump_header(&session.server.version, &req.database))?;
        for stmt in &plan.pre_data {
            write(&format!("\n{stmt}\n"))?;
        }

        let mut rows = 0u64;
        let mut last_emit = Instant::now();
        for table in &plan.data_tables {
            if cancel.load(Ordering::Relaxed) {
                return Err(canceled());
            }
            if table.columns.is_empty() {
                continue;
            }
            match req.data_format {
                DataFormat::Copy => {
                    write(&format!("\n{}", atlas_pg::copy_header(table)))?;
                    let base = rows;
                    let copied = atlas_pg::copy_table(&session, table, |chunk, table_rows| {
                        if cancel.load(Ordering::Relaxed) {
                            return Ok(false);
                        }
                        out.clone().write_all(chunk).map_err(io_error)?;
                        if last_emit.elapsed() >= Duration::from_millis(200) {
                            last_emit = Instant::now();
                            let _ = app.emit("export-progress", Progress { export_id: &req.dump_id, rows: base + table_rows });
                        }
                        Ok(true)
                    })
                    .await?;
                    rows += copied;
                    write("\\.\n")?;
                }
                DataFormat::Insert => {
                    write(&format!("\n-- Dados de {}\n", table.quoted))?;
                    let format = ExportFormat::Sql { rows_per_insert: req.rows_per_insert.clamp(1, 1000) };
                    let writer = Mutex::new(
                        atlas_export::create_stream(&format, &atlas_export::WriterTarget { sql: &table.quoted, title: &table.name }, Box::new(out.clone()))
                            .map_err(io_error)?,
                    );
                    let mut table_rows = 0u64;
                    atlas_pg::stream_rows(
                        &session,
                        &atlas_pg::data_select(table),
                        |cols| {
                            let cols: Vec<ExportColumn> = cols.iter().map(|c| ExportColumn { name: c.name.clone(), type_name: c.type_name.clone() }).collect();
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
                    )
                    .await?;
                    if cancel.load(Ordering::Relaxed) {
                        return Err(canceled());
                    }
                    writer.into_inner().unwrap().finish().map_err(io_error)?;
                    rows += table_rows;
                }
            }
        }
        if cancel.load(Ordering::Relaxed) {
            return Err(canceled());
        }
        for stmt in &plan.post_data {
            write(&format!("\n{stmt}\n"))?;
        }
        Ok::<_, DatabaseError>((plan.objects, plan.data_tables.len(), rows))
    }
    .await;
    atlas_pg::end_snapshot(&session).await;

    // Cancelado: qualquer erro do servidor (57014) é consequência do cancelamento.
    if cancel.load(Ordering::Relaxed) {
        return Err(canceled());
    }
    let (objects, tables, rows) = result?;
    out.clone().flush().map_err(io_error)?;
    fs::rename(&part.path, &dest).map_err(io_error)?;
    part.keep = true;
    let _ = app.emit("export-progress", Progress { export_id: &req.dump_id, rows });

    Ok(DumpResult {
        objects,
        tables,
        rows,
        bytes: fs::metadata(&dest).map(|m| m.len()).unwrap_or(0),
        elapsed_ms: started.elapsed().as_millis() as u64,
        path: dest.to_string_lossy().into_owned(),
    })
}
