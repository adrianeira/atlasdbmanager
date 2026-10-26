//! Data Dictionary: lê os objetos marcados (servidores, bancos e schemas), monta o modelo do documento e grava o PDF.
//! A pré-visualização das abas de layout sai do mesmo desenho, em SVG.

use crate::dictsrc::DictSource;
use crate::mssql::MssqlSessions;
use crate::mysql::MysqlSessions;
use crate::sessions::Sessions;
use crate::sqlite::SqliteSessions;
use atlas_core::{DatabaseError, ErrorKind};
use atlas_docs::dictionary as doc;
use atlas_docs::pdf::write_pdf;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tauri::{AppHandle, Emitter, State};

/// Gerações em andamento (botão Parar).
#[derive(Default)]
pub struct DictRuns {
    active: Mutex<HashMap<String, Arc<AtomicBool>>>,
}

#[derive(Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct DictSchema {
    pub name: String,
    pub info: bool,
    /// `None` = categoria desmarcada; a lista traz o que foi marcado (funções como `nome(argumentos)`).
    pub tables: Option<Vec<String>>,
    pub views: Option<Vec<String>>,
    pub functions: Option<Vec<String>>,
}

#[derive(Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct DictDatabase {
    pub name: String,
    pub info: bool,
    pub schemas: Vec<DictSchema>,
}

#[derive(Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct DictServer {
    pub conn_id: String,
    pub name: String,
    pub product: String,
    /// "Server Information": o que a tela mostra no painel (versão, host, porta...). `None` = desmarcado.
    pub info: Option<Vec<(String, String)>>,
    pub databases: Vec<DictDatabase>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GenerateRequest {
    pub run_id: String,
    pub servers: Vec<DictServer>,
    pub settings: doc::Settings,
    /// Caminho escolhido (com `.pdf`).
    pub path: String,
    pub append_timestamp: bool,
    pub overwrite: bool,
    pub continue_on_error: bool,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct LogEvent<'a> {
    id: &'a str,
    line: String,
}

#[derive(Serialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct Counts {
    pub tables: u64,
    pub views: u64,
    pub functions: u64,
    pub errors: u64,
    pub fraction: f64,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct ProgressEvent<'a> {
    id: &'a str,
    #[serde(flatten)]
    counts: Counts,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GenerateResult {
    #[serde(flatten)]
    pub counts: Counts,
    pub path: Option<String>,
    pub elapsed_ms: u64,
}

fn convert_table(name: &str, d: atlas_core::TableDetail, own_schema: &str) -> doc::Table {
    let columns = d
        .columns
        .iter()
        .map(|c| {
            let serial = c.default.as_deref().is_some_and(|x| x.starts_with("nextval("));
            doc::Column {
                name: c.name.clone(),
                comment: c.comment.clone(),
                data_type: c.formatted_type.clone(),
                not_null: c.not_null,
                key: c.key.is_some(),
                auto_increment: serial || c.identity.as_deref().is_some_and(|i| !i.is_empty()),
                default: c.default.clone().filter(|_| !serial),
            }
        })
        .collect();
    let indexes = d
        .indexes
        .iter()
        .map(|i| doc::Index {
            name: i.name.clone(),
            comment: i.comment.clone(),
            kind: if i.primary { "PRIMARY" } else if i.unique { "UNIQUE" } else { "NORMAL" }.into(),
            method: i.method.to_uppercase(),
            fields: i.fields.clone(),
        })
        .collect();
    let foreign_keys = d
        .constraints
        .iter()
        .filter(|c| c.kind == "f")
        .map(|c| {
            let table = c.ref_table.clone().unwrap_or_default();
            doc::ForeignKey {
                name: c.name.clone(),
                comment: c.comment.clone(),
                fields: c.fields.clone(),
                ref_table: match &c.ref_schema {
                    Some(s) if s != own_schema => format!("{s}.{table}"),
                    _ => table,
                },
                ref_fields: c.ref_fields.clone().unwrap_or_default(),
                on_delete: c.on_delete.clone().unwrap_or_default().to_uppercase(),
                on_update: c.on_update.clone().unwrap_or_default().to_uppercase(),
            }
        })
        .collect();
    doc::Table {
        name: name.to_string(),
        comment: d.comment,
        columns,
        indexes,
        foreign_keys,
        checks: d.constraints.iter().filter(|c| c.kind == "c").map(|c| (c.name.clone(), c.definition.clone())).collect(),
        triggers: d.triggers.iter().map(|t| (t.name.clone(), t.definition.clone())).collect(),
    }
}

/// `YYYYMMDDhhmmss` com a data local que a tela mandou.
fn stamp(now: &[i32; 6]) -> String {
    format!("{:04}{:02}{:02}{:02}{:02}{:02}", now[0], now[1], now[2], now[3], now[4], now[5])
}

fn target_path(req: &GenerateRequest) -> Result<PathBuf, DatabaseError> {
    let path = Path::new(&req.path);
    let mut path = if path.extension().is_some() { path.to_path_buf() } else { path.with_extension("pdf") };
    if req.append_timestamp {
        let stem = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "DataDictionary".into());
        let ext = path.extension().map(|e| e.to_string_lossy().to_string()).unwrap_or_else(|| "pdf".into());
        path.set_file_name(format!("{stem}_{}.{ext}", stamp(&req.settings.now)));
    }
    if path.exists() && !req.overwrite {
        return Err(DatabaseError::new(ErrorKind::InvalidInput, format!("the file already exists: {}", path.display())));
    }
    Ok(path)
}

#[tauri::command]
pub async fn dict_generate(
    app: AppHandle,
    sessions: State<'_, Sessions>,
    sqlite: State<'_, SqliteSessions>,
    mssql: State<'_, MssqlSessions>,
    mysql: State<'_, MysqlSessions>,
    runs: State<'_, DictRuns>,
    req: GenerateRequest,
) -> Result<GenerateResult, DatabaseError> {
    let started = Instant::now();
    let cancel = Arc::new(AtomicBool::new(false));
    runs.active.lock().unwrap().insert(req.run_id.clone(), cancel.clone());
    let result = run(&app, (&sessions, &sqlite, &mssql, &mysql), &req, &cancel, started).await;
    runs.active.lock().unwrap().remove(&req.run_id);
    result
}

async fn run(app: &AppHandle, all: (&Sessions, &SqliteSessions, &MssqlSessions, &MysqlSessions), req: &GenerateRequest, cancel: &AtomicBool, started: Instant) -> Result<GenerateResult, DatabaseError> {
    let log = |line: String| {
        let _ = app.emit("dict-log", LogEvent { id: &req.run_id, line: format!("[DDI] {line}") });
    };
    let path = target_path(req)?;
    let total: usize = req
        .servers
        .iter()
        .flat_map(|s| &s.databases)
        .flat_map(|d| &d.schemas)
        .map(|s| s.tables.as_ref().map_or(0, Vec::len) + s.views.as_ref().map_or(0, Vec::len) + s.functions.as_ref().map_or(0, Vec::len))
        .sum::<usize>()
        .max(1);
    let mut counts = Counts::default();
    let mut done = 0usize;
    let progress = |counts: &Counts| {
        let _ = app.emit("dict-progress", ProgressEvent { id: &req.run_id, counts: counts.clone() });
    };
    let canceled = || DatabaseError::new(ErrorKind::Canceled, "data dictionary canceled");
    let mut dict = doc::Dictionary::default();
    log("Data Dictionary started".into());
    for server in &req.servers {
        let mut out_server = doc::Server { name: server.name.clone(), product: server.product.clone(), info: server.info.clone(), databases: Vec::new() };
        log(format!("Start processing server - {}", server.name));
        for db in &server.databases {
            let session = DictSource::open(&server.conn_id, &db.name, all.0, all.1, all.2, all.3).await?;
            log(format!("Start processing catalog - {}", db.name));
            let mut out_db = doc::Database { name: db.name.clone(), info: None, schemas: Vec::new() };
            if db.info {
                out_db.info = Some(session.database_info(&db.name).await?);
            }
            for schema in &db.schemas {
                if cancel.load(Ordering::Relaxed) {
                    return Err(canceled());
                }
                log(format!("Start processing schema - {}", schema.name));
                let mut out = doc::Schema { name: schema.name.clone(), ..Default::default() };
                if schema.info {
                    out.info = Some(session.schema_info(&schema.name).await?);
                }
                if let Some(names) = &schema.tables {
                    log(format!("Fetching tables - {}", schema.name));
                    log(format!("{} - fetched {} tables", schema.name, names.len()));
                    let mut tables = Vec::new();
                    for name in names {
                        if cancel.load(Ordering::Relaxed) {
                            return Err(canceled());
                        }
                        done += 1;
                        log(format!("{done}> Generating table - {name}"));
                        match session.table(&db.name, &schema.name, name).await {
                            Ok(d) => {
                                counts.tables += 1;
                                tables.push(convert_table(name, d, &schema.name));
                            }
                            Err(e) => {
                                counts.errors += 1;
                                log(format!("ERROR: {name}: {}", e.message));
                                if !req.continue_on_error {
                                    progress(&counts);
                                    return Err(e);
                                }
                            }
                        }
                        counts.fraction = done as f64 / total as f64 * 0.95;
                        progress(&counts);
                    }
                    out.tables = Some(tables);
                }
                if let Some(names) = &schema.views {
                    log(format!("Fetching views - {}", schema.name));
                    log(format!("{} - fetched {} views", schema.name, names.len()));
                    let mut views = Vec::new();
                    for name in names {
                        if cancel.load(Ordering::Relaxed) {
                            return Err(canceled());
                        }
                        done += 1;
                        log(format!("{done}> Generating view - {name}"));
                        match session.view(&db.name, &schema.name, name).await {
                            Ok((comment, definition)) => {
                                counts.views += 1;
                                views.push(doc::View { name: name.clone(), comment, definition });
                            }
                            Err(e) => {
                                counts.errors += 1;
                                log(format!("ERROR: {name}: {}", e.message));
                                if !req.continue_on_error {
                                    progress(&counts);
                                    return Err(e);
                                }
                            }
                        }
                        counts.fraction = done as f64 / total as f64 * 0.95;
                        progress(&counts);
                    }
                    out.views = Some(views);
                }
                if let Some(names) = &schema.functions {
                    log(format!("Fetching functions - {}", schema.name));
                    log(format!("{} - fetched {} functions", schema.name, names.len()));
                    let mut functions = Vec::new();
                    for full in names {
                        if cancel.load(Ordering::Relaxed) {
                            return Err(canceled());
                        }
                        done += 1;
                        let (name, identity) = match full.split_once('(') {
                            Some((n, rest)) => (n.to_string(), rest.strip_suffix(')').unwrap_or(rest).to_string()),
                            None => (full.clone(), String::new()),
                        };
                        log(format!("{done}> Generating function - {name}"));
                        match session.function(&db.name, &schema.name, &name, &identity).await {
                            Ok(d) => {
                                counts.functions += 1;
                                functions.push(doc::Function { name: full.clone(), arguments: d.arguments, returns: d.returns, language: d.language, comment: d.comment, body: d.body });
                            }
                            Err(e) => {
                                counts.errors += 1;
                                log(format!("ERROR: {name}: {}", e.message));
                                if !req.continue_on_error {
                                    progress(&counts);
                                    return Err(e);
                                }
                            }
                        }
                        counts.fraction = done as f64 / total as f64 * 0.95;
                        progress(&counts);
                    }
                    out.functions = Some(functions);
                }
                log(format!("End processing schema - {}", schema.name));
                out_db.schemas.push(out);
            }
            log(format!("End processing catalog - {}", db.name));
            out_server.databases.push(out_db);
        }
        log(format!("End processing server - {}", server.name));
        dict.servers.push(out_server);
    }
    log("Generating table of content".into());
    let rendered = doc::render(&dict, &req.settings);
    let title = if req.settings.cover_title.is_empty() { "Data Dictionary" } else { req.settings.cover_title.as_str() };
    let bytes = write_pdf(&rendered.pages, title);
    std::fs::write(&path, bytes).map_err(|e| DatabaseError::new(ErrorKind::Internal, format!("cannot write {}: {e}", path.display())))?;
    log(format!("File created: {}", path.display()));
    log(if counts.errors > 0 { "Finished with errors".into() } else { "Finished successfully".to_string() });
    counts.fraction = 1.0;
    progress(&counts);
    Ok(GenerateResult { counts, path: Some(path.display().to_string()), elapsed_ms: started.elapsed().as_millis() as u64 })
}

#[tauri::command]
pub fn dict_cancel(runs: State<'_, DictRuns>, run_id: String) {
    if let Some(flag) = runs.active.lock().unwrap().get(&run_id) {
        flag.store(true, Ordering::Relaxed);
    }
}

/// Estrutura só com os nomes (sem ler nada do banco): serve à capa, ao sumário e à introdução da pré-visualização.
fn names_only(servers: &[DictServer]) -> doc::Dictionary {
    doc::Dictionary {
        servers: servers
            .iter()
            .map(|s| doc::Server {
                name: s.name.clone(),
                product: s.product.clone(),
                info: s.info.clone(),
                databases: s
                    .databases
                    .iter()
                    .map(|d| doc::Database {
                        name: d.name.clone(),
                        info: None,
                        schemas: d
                            .schemas
                            .iter()
                            .map(|x| doc::Schema {
                                name: x.name.clone(),
                                info: None,
                                tables: x.tables.as_ref().map(|t| t.iter().map(|n| doc::Table { name: n.clone(), ..Default::default() }).collect()),
                                views: x.views.as_ref().map(|t| t.iter().map(|n| doc::View { name: n.clone(), ..Default::default() }).collect()),
                                functions: x.functions.as_ref().map(|t| t.iter().map(|n| doc::Function { name: n.clone(), ..Default::default() }).collect()),
                            })
                            .collect(),
                    })
                    .collect(),
            })
            .collect(),
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewPage {
    pub svg: String,
    pub width: f32,
    pub height: f32,
}

/// Uma página para a pré-visualização: `cover`, `toc`, `intro` (do que está marcado) ou `sample` (uma tabela de exemplo).
#[tauri::command]
pub fn dict_preview(servers: Vec<DictServer>, settings: doc::Settings, which: String) -> Result<Option<PreviewPage>, DatabaseError> {
    let (rendered, index) = if which == "sample" {
        let mut s = settings.clone();
        s.include_cover = false;
        s.include_toc = false;
        s.include_intro = false;
        let r = doc::render(&doc::sample_dictionary(), &s);
        (r, Some(0))
    } else {
        let r = doc::render(&names_only(&servers), &settings);
        let index = match which.as_str() {
            "cover" => r.cover,
            "toc" => r.toc.first().copied(),
            "intro" => r.intro,
            _ => Some(r.content),
        };
        (r, index)
    };
    Ok(index.and_then(|i| rendered.pages.get(i)).map(|p| PreviewPage { svg: p.to_svg(), width: p.width, height: p.height }))
}

/// Os ajustes de layout de fábrica (a tela os usa ao abrir e nos "Restore Defaults").
#[tauri::command]
pub fn dict_defaults(title: String) -> doc::Settings {
    doc::Settings::defaults(&title)
}

/// Abre o arquivo (ou a pasta que o contém) no aplicativo padrão do sistema.
#[tauri::command]
pub fn dict_open(path: String, reveal: bool) -> Result<(), DatabaseError> {
    let p = Path::new(&path);
    if !p.exists() {
        return Err(DatabaseError::new(ErrorKind::InvalidInput, format!("the file does not exist: {path}")));
    }
    let io = |e: std::io::Error| DatabaseError::new(ErrorKind::Internal, e.to_string());
    #[cfg(target_os = "windows")]
    {
        use std::process::Command;
        if reveal {
            Command::new("explorer").arg(format!("/select,{path}")).spawn().map_err(io)?;
        } else {
            Command::new("cmd").args(["/C", "start", "", &path]).spawn().map_err(io)?;
        }
    }
    #[cfg(target_os = "macos")]
    {
        let mut c = std::process::Command::new("open");
        if reveal {
            c.arg("-R");
        }
        c.arg(&path).spawn().map_err(io)?;
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let target = if reveal { p.parent().map(|x| x.to_path_buf()).unwrap_or_else(|| p.to_path_buf()) } else { p.to_path_buf() };
        std::process::Command::new("xdg-open").arg(target).spawn().map_err(io)?;
    }
    Ok(())
}
