//! Data Transfer do SQL Server, MySQL e SQLite: copia objetos de um schema para outro banco ou servidor do MESMO produto.
//!
//! A estrutura vem do planejador do Dump SQL de cada banco (o DDL na linguagem dele); os dados são lidos da origem em lotes e
//! gravados no destino como `INSERT` (SQLite: `INSERT ... SELECT` de um arquivo anexado). Eventos e contadores são os do
//! PostgreSQL (`transfer-log`, `transfer-progress`, cancelamento por `export_cancel`).

use crate::genext::{Dialect, Exec, mssql_schema_rewrite};
use crate::mssql::MssqlSessions;
use crate::mysql::MysqlSessions;
use crate::sqlite::SqliteSessions;
use crate::transfer::{TransferCounts, TransferRequest};
use atlas_core::{DatabaseError, ErrorKind};
use atlas_pg::{TableOverride, TransferSpec, TransferStats};
use serde::Serialize;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;
use tauri::{AppHandle, Emitter, State};

const DEFAULT_BATCH: usize = 500;

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

/// Uma tabela cujos dados entram: nome, colunas (nome e tipo) e se tem identidade (SQL Server).
struct DataTable {
    name: String,
    columns: Vec<(String, String)>,
    has_identity: bool,
}

struct Plan {
    pre: Vec<String>,
    post: Vec<String>,
    tables: Vec<DataTable>,
    objects: usize,
}

/// As duas pontas da transferência.
enum Pair {
    Mysql(Arc<atlas_mysql::MysqlSession>, Arc<atlas_mysql::MysqlSession>),
    Mssql(Arc<atlas_mssql::MssqlSession>, Arc<atlas_mssql::MssqlSession>),
    Sqlite(Arc<atlas_sqlite::SqliteSession>, Arc<atlas_sqlite::SqliteSession>),
}

fn head(stmt: &str) -> String {
    let line = stmt.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
    if line.chars().count() > 110 { format!("{}...", line.chars().take(110).collect::<String>()) } else { line.to_string() }
}

/// Os lotes de um comando do SQL Server: linhas `GO` separam (as views e rotinas vêm assim).
fn batches(stmt: &str) -> Vec<String> {
    let mut out = vec![String::new()];
    for line in stmt.lines() {
        if line.trim().eq_ignore_ascii_case("go") {
            out.push(String::new());
        } else if let Some(cur) = out.last_mut() {
            cur.push_str(line);
            cur.push('\n');
        }
    }
    out.into_iter().filter(|b| !b.trim().is_empty()).collect()
}

impl Pair {
    fn dialect(&self) -> Dialect {
        match self {
            Pair::Mysql(..) => Dialect::Mysql,
            Pair::Mssql(..) => Dialect::Mssql,
            Pair::Sqlite(..) => Dialect::Sqlite,
        }
    }

    /// A conexão (sem transação) do destino.
    fn target(&self, database: &str, schema: &str) -> Exec {
        match self {
            Pair::Mysql(_, d) => Exec::Mysql(d.clone(), if schema.is_empty() { database.to_string() } else { schema.to_string() }),
            Pair::Mssql(_, d) => Exec::Mssql(d.clone(), database.to_string()),
            Pair::Sqlite(_, d) => Exec::Sqlite(d.clone()),
        }
    }

    async fn plan(&self, source_database: &str, spec: &TransferSpec) -> Result<Plan, DatabaseError> {
        let tables = Some(spec.tables.as_slice());
        let views = Some(spec.views.as_slice());
        let functions = Some(spec.functions.as_slice());
        Ok(match self {
            Pair::Mysql(s, _) => {
                let p = s.dump_plan(&spec.source_schema, tables, views, functions, spec.drop_before_create).await?;
                Plan {
                    pre: p.pre_data,
                    post: p.post_data,
                    tables: p.data_tables.into_iter().map(|t| DataTable { name: t.name, columns: t.columns, has_identity: false }).collect(),
                    objects: p.objects,
                }
            }
            Pair::Mssql(s, _) => {
                let p = s.dump_plan(source_database, Some(&spec.source_schema), tables, views, functions, spec.drop_before_create).await?;
                Plan {
                    pre: p.pre_data,
                    post: p.post_data,
                    tables: p.data_tables.into_iter().map(|t| DataTable { name: t.name, columns: t.columns, has_identity: t.has_identity }).collect(),
                    objects: p.objects,
                }
            }
            Pair::Sqlite(s, _) => {
                let p = s.dump_plan(tables, views, spec.drop_before_create)?;
                Plan { pre: p.pre_data, post: p.post_data, tables: p.data_tables.into_iter().map(|name| DataTable { name, columns: Vec::new(), has_identity: false }).collect(), objects: p.objects }
            }
        })
    }

    /// O comando como o destino o recebe: schema e nome da tabela trocados quando a transferência pede.
    fn adapt(&self, stmt: &str, spec: &TransferSpec, renames: &[(String, String)]) -> String {
        let mut out = stmt.to_string();
        match self {
            Pair::Mssql(..) => {
                if spec.source_schema != spec.target_schema {
                    out = mssql_schema_rewrite(&out, &spec.source_schema, &spec.target_schema);
                }
                for (from, to) in renames {
                    out = out.replacen(&format!("[{}].[{}]", spec.target_schema, from), &format!("[{}].[{}]", spec.target_schema, to), 1);
                }
            }
            Pair::Mysql(..) => {
                for (from, to) in renames {
                    out = out.replacen(&format!("`{from}`"), &format!("`{to}`"), 1);
                }
            }
            Pair::Sqlite(..) => {
                for (from, to) in renames {
                    out = out.replacen(&format!("\"{from}\""), &format!("\"{to}\""), 1);
                }
            }
        }
        out
    }
}

struct Ctx<'a> {
    spec: &'a TransferSpec,
    stats: TransferStats,
    log: &'a mut (dyn FnMut(String) + Send),
    progress: &'a mut (dyn FnMut(&TransferStats) + Send),
    cancelled: &'a (dyn Fn() -> bool + Sync),
}

impl Ctx<'_> {
    fn check(&self) -> Result<(), DatabaseError> {
        if (self.cancelled)() { Err(DatabaseError::new(ErrorKind::Canceled, "transfer canceled")) } else { Ok(()) }
    }

    fn error(&mut self, what: &str, message: &str) {
        (self.log)(format!("[ERR] {what}: {message}"));
        self.stats.errors += 1;
        (self.progress)(&self.stats);
    }
}

/// Executa uma lista de comandos no destino. Devolve `false` se a transferência deve parar.
async fn run_statements(pair: &Pair, exec: &Exec, list: &[String], cx: &mut Ctx<'_>) -> Result<bool, DatabaseError> {
    for stmt in list {
        cx.check()?;
        let parts = if pair.dialect() == Dialect::Mssql { batches(stmt) } else { vec![stmt.clone()] };
        for part in parts {
            match exec.run(&part, 1).await {
                Ok(_) => {
                    cx.stats.processed += 1;
                    (cx.log)(format!("[DTF] {}", head(&part)));
                }
                Err(e) => {
                    cx.error(&head(&part), &e.message);
                    if !cx.spec.continue_on_error {
                        return Ok(false);
                    }
                }
            }
        }
        (cx.progress)(&cx.stats);
    }
    Ok(true)
}

/// A origem e os filtros de uma tabela: pares (campo da origem, campo do destino) e os "recordsets".
fn field_pairs(table: &DataTable, ov: Option<&TableOverride>) -> Vec<(String, String)> {
    match ov.and_then(|o| o.fields.as_ref()).filter(|f| !f.is_empty()) {
        Some(f) => f.iter().map(|m| (m.source.clone(), if m.target.is_empty() { m.source.clone() } else { m.target.clone() })).collect(),
        None => table.columns.iter().map(|(n, _)| (n.clone(), n.clone())).collect(),
    }
}

fn recordsets(ov: Option<&TableOverride>) -> Vec<Option<String>> {
    match ov.and_then(|o| o.recordsets.as_ref()).filter(|r| r.iter().any(|x| !x.trim().is_empty())) {
        Some(r) => r.iter().filter(|x| !x.trim().is_empty()).map(|x| Some(x.trim().to_string())).collect(),
        None => vec![None],
    }
}

/// Copia uma tabela. Devolve `false` se a transferência deve parar.
async fn copy_table(pair: &Pair, exec: &Exec, table: &DataTable, source_database: &str, cx: &mut Ctx<'_>) -> Result<bool, DatabaseError> {
    let started = Instant::now();
    let ov = cx.spec.overrides.iter().find(|o| o.table == table.name);
    let target_name = ov.and_then(|o| o.target_name.clone()).filter(|n| !n.is_empty()).unwrap_or_else(|| table.name.clone());
    let batch = ov.and_then(|o| o.rows_per_batch).filter(|n| *n > 0).unwrap_or(DEFAULT_BATCH);
    let dialect = pair.dialect();
    let (src_schema, dst_schema) = (cx.spec.source_schema.clone(), cx.spec.target_schema.clone());
    let pairs = field_pairs(table, ov);
    // No MySQL o banco é o próprio schema.
    let src_db = if matches!(pair, Pair::Mysql(..)) { src_schema.clone() } else { source_database.to_string() };
    (cx.log)(format!("[DTF] {}: transferring to {}", table.name, target_name));
    let mut table_rows = 0u64;
    for rs in recordsets(ov) {
        let filter = rs.as_ref().map(|w| format!(" WHERE {w}")).unwrap_or_default();
        cx.check()?;
        match pair {
            Pair::Sqlite(..) => {
                let list = if pairs.is_empty() {
                    String::new()
                } else {
                    format!(" ({})", pairs.iter().map(|(_, t)| dialect.q(t)).collect::<Vec<_>>().join(", "))
                };
                let select = if pairs.is_empty() { "*".to_string() } else { pairs.iter().map(|(s, _)| dialect.q(s)).collect::<Vec<_>>().join(", ") };
                let sql = format!("INSERT INTO main.{}{list} SELECT {select} FROM atlas_src.{}{filter}", dialect.q(&target_name), dialect.q(&table.name));
                match exec.run(&sql, 1).await {
                    Ok(out) => {
                        let n = out.statements.first().and_then(|s| s.rows_affected).unwrap_or(0);
                        table_rows += n;
                        cx.stats.transferred += n;
                        cx.stats.processed += n;
                    }
                    Err(e) => {
                        cx.error(&table.name, &e.message);
                        if !cx.spec.continue_on_error {
                            return Ok(false);
                        }
                    }
                }
            }
            Pair::Mysql(..) | Pair::Mssql(..) => {
                let type_of = |name: &str| table.columns.iter().find(|(n, _)| n == name).map(|(_, t)| t.clone()).unwrap_or_default();
                let source_list = pairs.iter().map(|(s, _)| dialect.q(s)).collect::<Vec<_>>().join(", ");
                let target_list = pairs.iter().map(|(_, t)| dialect.q(t)).collect::<Vec<_>>().join(", ");
                let from = match pair {
                    Pair::Mysql(..) => format!("{}.{}", dialect.q(&src_schema), dialect.q(&table.name)),
                    _ => dialect.target(&src_schema, &table.name),
                };
                let select = format!("SELECT {source_list} FROM {from}{filter}");
                let target = match pair {
                    Pair::Mysql(..) => format!("{}.{}", dialect.q(&dst_schema), dialect.q(&target_name)),
                    _ => dialect.target(&dst_schema, &target_name),
                };
                let types: Vec<String> = pairs.iter().map(|(s, _)| type_of(s)).collect();
                let identity = table.has_identity;
                let cancelled = cx.cancelled;
                let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Vec<String>>();
                let producer = async {
                    let mut buf: Vec<String> = Vec::new();
                    let push = |values: &[Option<&str>], buf: &mut Vec<String>| {
                        let cells: Vec<String> = values
                            .iter()
                            .zip(&types)
                            .map(|(v, ty)| match pair {
                                Pair::Mysql(..) => atlas_mysql::literal(*v, ty),
                                _ => atlas_mssql::literal(*v, ty),
                            })
                            .collect();
                        buf.push(format!("({})", cells.join(", ")));
                    };
                    let result = match pair {
                        Pair::Mysql(s, _) => {
                            s.stream_rows(
                                &src_db,
                                &select,
                                |_| Ok(()),
                                |values| {
                                    if cancelled() {
                                        return Ok(false);
                                    }
                                    push(values, &mut buf);
                                    if buf.len() >= batch {
                                        let _ = tx.send(std::mem::take(&mut buf));
                                    }
                                    Ok(true)
                                },
                            )
                            .await
                        }
                        Pair::Mssql(s, _) => {
                            s.stream_rows(
                                &src_db,
                                &select,
                                |_| Ok(()),
                                |values| {
                                    if cancelled() {
                                        return Ok(false);
                                    }
                                    push(values, &mut buf);
                                    if buf.len() >= batch {
                                        let _ = tx.send(std::mem::take(&mut buf));
                                    }
                                    Ok(true)
                                },
                            )
                            .await
                        }
                        Pair::Sqlite(..) => Ok(0),
                    };
                    if !buf.is_empty() {
                        let _ = tx.send(buf);
                    }
                    drop(tx);
                    result
                };
                let mut failed: Option<DatabaseError> = None;
                let mut inserted = 0u64;
                let consumer = async {
                    while let Some(rows) = rx.recv().await {
                        if failed.is_some() {
                            continue;
                        }
                        let n = rows.len() as u64;
                        let insert = format!("INSERT INTO {target} ({target_list}) VALUES {}", rows.join(", "));
                        let sql = if identity { format!("SET IDENTITY_INSERT {target} ON; {insert}; SET IDENTITY_INSERT {target} OFF") } else { insert };
                        match exec.run(&sql, 1).await {
                            Ok(_) => inserted += n,
                            Err(e) => failed = Some(e),
                        }
                    }
                };
                let (read, ()) = tokio::join!(producer, consumer);
                cx.stats.transferred += inserted;
                cx.stats.processed += inserted;
                table_rows += inserted;
                if let Some(e) = failed {
                    cx.error(&table.name, &e.message);
                    if !cx.spec.continue_on_error {
                        return Ok(false);
                    }
                }
                if let Err(e) = read {
                    cx.error(&table.name, &e.message);
                    if !cx.spec.continue_on_error {
                        return Ok(false);
                    }
                }
            }
        }
        (cx.progress)(&cx.stats);
    }
    (cx.log)(format!("[DTF] {}: {table_rows} record(s) transferred ({:.2} s)", table.name, started.elapsed().as_secs_f64()));
    Ok(true)
}

async fn run(pair: &Pair, req: &TransferRequest, mut cx: Ctx<'_>) -> Result<TransferStats, DatabaseError> {
    let spec = &req.spec;
    let dialect = pair.dialect();
    // Copiar um objeto sobre ele mesmo apagaria os dados antes de lê-los.
    let same_place = req.source_conn == req.target_conn && (matches!(pair, Pair::Sqlite(..)) || (req.source_database == req.target_database && spec.source_schema == spec.target_schema));
    let renames_all = !spec.overrides.is_empty() && spec.overrides.iter().all(|o| o.target_name.as_ref().is_some_and(|n| !n.is_empty() && *n != o.table));
    if same_place && !(renames_all && spec.tables.len() == spec.overrides.len()) {
        return Err(DatabaseError::new(ErrorKind::InvalidInput, "the source and the target are the same: choose another database, schema or connection"));
    }
    let plan = pair.plan(&req.source_database, spec).await?;
    cx.stats.objects = plan.objects as u64;
    (cx.progress)(&cx.stats);
    let exec = pair.target(&req.target_database, &spec.target_schema);

    // O destino: o banco (MySQL) ou o schema (SQL Server) precisa existir.
    match pair {
        Pair::Mysql(_, d) => {
            d.execute("", &format!("CREATE DATABASE IF NOT EXISTS {}", dialect.q(&spec.target_schema)), 1).await?;
            exec.run("SET FOREIGN_KEY_CHECKS = 0", 1).await?;
        }
        Pair::Mssql(..) if spec.target_schema != "dbo" => {
            exec.run(&format!("IF SCHEMA_ID({}) IS NULL EXEC('CREATE SCHEMA {}')", dialect.text(&spec.target_schema), dialect.q(&spec.target_schema).replace('\'', "''")), 1).await?;
        }
        Pair::Sqlite(..) => {
            exec.run("PRAGMA foreign_keys = OFF", 1).await?;
        }
        _ => {}
    }

    let renames: Vec<(String, String)> = spec
        .overrides
        .iter()
        .filter_map(|o| o.target_name.clone().filter(|n| !n.is_empty() && *n != o.table).map(|n| (o.table.clone(), n)))
        .collect();
    let adapt = |list: &[String]| -> Vec<String> { list.iter().map(|s| pair.adapt(s, spec, &renames)).collect() };

    let outcome = async {
        if spec.structure && !run_statements(pair, &exec, &adapt(&plan.pre), &mut cx).await? {
            return Ok(());
        }
        if spec.data {
            // O arquivo de origem fica anexado só enquanto os dados correm: um nome sem banco (DROP, CREATE INDEX...) também
            // procura nos anexados, e não pode alcançar a origem.
            if let Pair::Sqlite(src, _) = pair {
                exec.run(&format!("ATTACH DATABASE {} AS atlas_src", dialect.text(&src.path.display().to_string())), 1).await?;
            }
            let copied = async {
                for t in &plan.tables {
                    cx.check()?;
                    if !copy_table(pair, &exec, t, &req.source_database, &mut cx).await? {
                        return Ok(false);
                    }
                }
                Ok::<bool, DatabaseError>(true)
            }
            .await;
            if matches!(pair, Pair::Sqlite(..)) {
                let _ = exec.run("DETACH DATABASE atlas_src", 1).await;
            }
            if !copied? {
                return Ok(());
            }
        }
        if spec.structure {
            run_statements(pair, &exec, &adapt(&plan.post), &mut cx).await?;
        }
        Ok::<(), DatabaseError>(())
    }
    .await;

    // Deixa a conexão do destino como estava.
    match pair {
        Pair::Mysql(..) => {
            let _ = exec.run("SET FOREIGN_KEY_CHECKS = 1", 1).await;
        }
        Pair::Sqlite(..) => {
            let _ = exec.run("PRAGMA foreign_keys = ON", 1).await;
        }
        _ => {}
    }
    outcome?;
    Ok(cx.stats)
}

fn counts(s: &TransferStats, started: Instant) -> TransferCounts {
    TransferCounts { objects: s.objects, processed: s.processed, transferred: s.transferred, errors: s.errors, elapsed_ms: started.elapsed().as_millis() as u64 }
}

async fn finish(app: AppHandle, pair: Pair, req: TransferRequest, cancel: Arc<AtomicBool>) -> Result<TransferCounts, DatabaseError> {
    let started = Instant::now();
    let (log_app, log_id) = (app.clone(), req.transfer_id.clone());
    let mut on_log = move |line: String| {
        let _ = log_app.emit("transfer-log", LogEvent { transfer_id: &log_id, line });
    };
    let (progress_app, progress_id) = (app.clone(), req.transfer_id.clone());
    let mut on_progress = move |s: &TransferStats| {
        let _ = progress_app.emit("transfer-progress", ProgressEvent { transfer_id: &progress_id, counts: counts(s, started) });
    };
    let cancelled = || cancel.load(Ordering::Relaxed);
    let spec = &req.spec;
    let stats = run(&pair, &req, Ctx { spec, stats: TransferStats::default(), log: &mut on_log, progress: &mut on_progress, cancelled: &cancelled }).await?;
    Ok(counts(&stats, started))
}

macro_rules! transfer_command {
    ($name:ident, $sessions:ty, $variant:ident) => {
        #[tauri::command]
        pub async fn $name(app: AppHandle, sessions: State<'_, $sessions>, req: TransferRequest) -> Result<TransferCounts, DatabaseError> {
            let src = sessions.session(&req.source_conn)?;
            let dst = sessions.session(&req.target_conn).map_err(|_| DatabaseError::new(ErrorKind::InvalidInput, "the target connection must be a connection of the same database product, and open"))?;
            let cancel = Arc::new(AtomicBool::new(false));
            sessions.register_cancel(&req.transfer_id, cancel.clone());
            let id = req.transfer_id.clone();
            let out = finish(app, Pair::$variant(src, dst), req, cancel).await;
            sessions.unregister_cancel(&id);
            out
        }
    };
}

transfer_command!(mysql_transfer, MysqlSessions, Mysql);
transfer_command!(mssql_transfer, MssqlSessions, Mssql);
transfer_command!(sqlite_transfer, SqliteSessions, Sqlite);
