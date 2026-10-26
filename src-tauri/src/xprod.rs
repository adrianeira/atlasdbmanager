//! Ferramentas entre produtos diferentes (PostgreSQL, SQL Server, MySQL e SQLite): a conexão de qualquer um deles como `Exec`, o
//! cancelamento comum e o Data Transfer que lê numa linguagem e grava na outra (`neutral.rs` traduz tipos e valores).

use crate::genext::{Dialect, Exec, exec_for};
use crate::mssql::MssqlSessions;
use crate::mysql::MysqlSessions;
use crate::neutral::{NType, Product, create_table, literal, parse};
use crate::sessions::Sessions;
use crate::sqlite::SqliteSessions;
use crate::transfer::{TransferCounts, TransferRequest};
use atlas_core::{DatabaseError, ErrorKind};
use atlas_pg::TransferStats;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tauri::{AppHandle, Emitter, State};

/// Operações longas entre produtos em andamento (id → pedido de parada).
static CANCELS: std::sync::LazyLock<Mutex<HashMap<String, Arc<AtomicBool>>>> = std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));

pub(crate) fn register_flag(id: &str, flag: Arc<AtomicBool>) {
    if let Ok(mut m) = CANCELS.lock() {
        m.insert(id.to_string(), flag);
    }
}

pub(crate) fn unregister_flag(id: &str) {
    if let Ok(mut m) = CANCELS.lock() {
        m.remove(id);
    }
}

/// Botão Parar das operações entre produtos.
#[tauri::command]
pub fn ext_cancel(export_id: String) {
    if let Some(flag) = CANCELS.lock().ok().and_then(|m| m.get(&export_id).cloned()) {
        flag.store(true, Ordering::Relaxed);
    }
}

/// A conexão `conn` (de qualquer produto) como `Exec`, no banco `database` e no schema `schema`.
pub(crate) async fn open_exec(
    conn: &str,
    database: &str,
    schema: &str,
    pg: &Sessions,
    mysql: &MysqlSessions,
    mssql: &MssqlSessions,
    sqlite: &SqliteSessions,
) -> Result<(Product, Exec), DatabaseError> {
    match Product::of_conn(conn) {
        Product::Pg => Ok((Product::Pg, Exec::Pg(Arc::new(pg.dedicated(conn, database).await?)))),
        _ => {
            let (d, e) = exec_for(conn, database, schema, mysql, mssql, sqlite)?;
            let p = match d {
                Dialect::Mysql => Product::Mysql,
                Dialect::Mssql => Product::Mssql,
                _ => Product::Sqlite,
            };
            Ok((p, e))
        }
    }
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

fn head(stmt: &str) -> String {
    let line = stmt.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
    if line.chars().count() > 110 { format!("{}...", line.chars().take(110).collect::<String>()) } else { line.to_string() }
}

#[allow(clippy::too_many_arguments)]
/// A leitura de uma tabela em páginas (ordenadas pela chave), na linguagem do produto de origem.
fn page_sql(p: Product, schema: &str, table: &str, columns: &[String], order: &[String], filter: &str, limit: usize, offset: usize) -> String {
    let d = p.dialect();
    let list = columns.iter().map(|c| d.q(c)).collect::<Vec<_>>().join(", ");
    let from = d.target(schema, table);
    let order_by = if order.is_empty() { String::new() } else { format!(" ORDER BY {}", order.iter().map(|c| d.q(c)).collect::<Vec<_>>().join(", ")) };
    match p {
        Product::Mssql => {
            let ob = if order_by.is_empty() { " ORDER BY (SELECT NULL)".to_string() } else { order_by };
            format!("SELECT {list} FROM {from}{filter}{ob} OFFSET {offset} ROWS FETCH NEXT {limit} ROWS ONLY")
        }
        _ => format!("SELECT {list} FROM {from}{filter}{order_by} LIMIT {limit} OFFSET {offset}"),
    }
}

fn drop_table_sql(p: Product, schema: &str, table: &str) -> String {
    let t = p.dialect().target(schema, table);
    match p {
        Product::Pg => format!("DROP TABLE IF EXISTS {t} CASCADE"),
        Product::Mssql => format!("IF OBJECT_ID({}) IS NOT NULL DROP TABLE {t}", p.dialect().text(&t)),
        _ => format!("DROP TABLE IF EXISTS {t}"),
    }
}

struct Run<'a> {
    stats: TransferStats,
    log: &'a mut (dyn FnMut(String) + Send),
    progress: &'a mut (dyn FnMut(&TransferStats) + Send),
    cancelled: &'a (dyn Fn() -> bool + Sync),
    continue_on_error: bool,
}

impl Run<'_> {
    fn check(&self) -> Result<(), DatabaseError> {
        if (self.cancelled)() { Err(DatabaseError::new(ErrorKind::Canceled, "transfer canceled")) } else { Ok(()) }
    }

    fn error(&mut self, what: &str, message: &str) {
        (self.log)(format!("[ERR] {what}: {message}"));
        self.stats.errors += 1;
        (self.progress)(&self.stats);
    }

    /// Executa um comando de estrutura. Devolve `false` se a transferência deve parar.
    async fn ddl(&mut self, exec: &Exec, sql: &str) -> bool {
        match exec.run(sql, 1).await {
            Ok(_) => {
                self.stats.processed += 1;
                (self.log)(format!("[DTF] {}", head(sql)));
                true
            }
            Err(e) => {
                self.error(&head(sql), &e.message);
                self.continue_on_error
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn copy_table(
    sp: Product,
    dp: Product,
    src: &Exec,
    dst: &Exec,
    req: &TransferRequest,
    table: &str,
    target_name: &str,
    detail: &atlas_core::TableDetail,
    run: &mut Run<'_>,
) -> Result<bool, DatabaseError> {
    let started = Instant::now();
    let spec = &req.spec;
    let ov = spec.overrides.iter().find(|o| o.table == table);
    let pairs: Vec<(String, String)> = match ov.and_then(|o| o.fields.as_ref()).filter(|f| !f.is_empty()) {
        Some(f) => f.iter().map(|m| (m.source.clone(), if m.target.is_empty() { m.source.clone() } else { m.target.clone() })).collect(),
        None => detail.columns.iter().filter(|c| c.generated.is_none()).map(|c| (c.name.clone(), c.name.clone())).collect(),
    };
    if pairs.is_empty() {
        return Ok(true);
    }
    let batch = ov.and_then(|o| o.rows_per_batch).filter(|n| *n > 0).unwrap_or(500);
    let types: Vec<(NType, NType)> = pairs
        .iter()
        .map(|(s, _)| {
            let f = detail.columns.iter().find(|c| c.name == *s).map(|c| c.formatted_type.clone()).unwrap_or_default();
            let from = parse(sp, &f);
            // O MySQL e o SQLite não têm tipo com fuso: o valor vai sem ele.
            let to = match (&from, dp) {
                (NType::Timestamp(true), Product::Mysql | Product::Sqlite) => NType::Timestamp(false),
                _ => from.clone(),
            };
            (from, to)
        })
        .collect();
    let mut keys: Vec<(i32, String)> = detail.columns.iter().filter_map(|c| c.key.map(|k| (k, c.name.clone()))).collect();
    keys.sort();
    let order: Vec<String> = keys.into_iter().map(|(_, n)| n).filter(|n| pairs.iter().any(|(s, _)| s == n)).collect();
    let dd = dp.dialect();
    let target = dd.target(&spec.target_schema, target_name);
    let target_cols = pairs.iter().map(|(_, t)| dd.q(t)).collect::<Vec<_>>().join(", ");
    let identity_insert = dp == Product::Mssql && detail.columns.iter().any(|c| c.identity.is_some() && pairs.iter().any(|(s, _)| *s == c.name));
    let source_cols: Vec<String> = pairs.iter().map(|(s, _)| s.clone()).collect();
    let recordsets: Vec<Option<String>> = match ov.and_then(|o| o.recordsets.as_ref()).filter(|r| r.iter().any(|x| !x.trim().is_empty())) {
        Some(r) => r.iter().filter(|x| !x.trim().is_empty()).map(|x| Some(x.trim().to_string())).collect(),
        None => vec![None],
    };
    (run.log)(format!("[DTF] {table}: transferring to {target_name}"));
    let mut table_rows = 0u64;
    for rs in recordsets {
        let filter = rs.map(|w| format!(" WHERE {w}")).unwrap_or_default();
        let mut offset = 0usize;
        loop {
            run.check()?;
            let sql = page_sql(sp, &req.spec.source_schema, table, &source_cols, &order, &filter, batch, offset);
            let rows = match src.rows(&sql, batch + 1).await {
                Ok(r) => r,
                Err(e) => {
                    run.error(table, &e.message);
                    return Ok(run.continue_on_error);
                }
            };
            if rows.is_empty() {
                break;
            }
            let values: Vec<String> = rows
                .iter()
                .map(|r| {
                    let cells: Vec<String> = r.iter().zip(&types).map(|(v, (from, to))| literal(dd, to, v, from)).collect();
                    format!("({})", cells.join(", "))
                })
                .collect();
            let insert = format!("INSERT INTO {target} ({target_cols}) VALUES {}", values.join(", "));
            let sql = if identity_insert { format!("SET IDENTITY_INSERT {target} ON; {insert}; SET IDENTITY_INSERT {target} OFF") } else { insert };
            let n = rows.len() as u64;
            match dst.run(&sql, 1).await {
                Ok(_) => {
                    run.stats.transferred += n;
                    run.stats.processed += n;
                    table_rows += n;
                }
                Err(e) => {
                    run.error(table, &e.message);
                    if !run.continue_on_error {
                        return Ok(false);
                    }
                }
            }
            (run.progress)(&run.stats);
            if rows.len() < batch {
                break;
            }
            offset += batch;
        }
    }
    // PostgreSQL: a numeração automática segue de onde os dados pararam.
    if dp == Product::Pg {
        for c in detail.columns.iter().filter(|c| c.identity.is_some() || c.default.as_deref().is_some_and(|d| d.starts_with("nextval("))) {
            if let Some((_, t)) = pairs.iter().find(|(s, _)| *s == c.name) {
                let col = dd.q(t);
                if let Ok(rows) = dst.rows(&format!("SELECT COALESCE(MAX({col}), 0) + 1 FROM {target}"), 1).await
                    && let Some(next) = rows.first().and_then(|r| r.first().cloned().flatten())
                {
                    let _ = dst.run(&format!("ALTER TABLE {target} ALTER COLUMN {col} RESTART WITH {next}"), 1).await;
                }
            }
        }
    }
    (run.log)(format!("[DTF] {table}: {table_rows} record(s) transferred ({:.2} s)", started.elapsed().as_secs_f64()));
    Ok(true)
}

fn counts(s: &TransferStats, started: Instant) -> TransferCounts {
    TransferCounts { objects: s.objects, processed: s.processed, transferred: s.transferred, errors: s.errors, elapsed_ms: started.elapsed().as_millis() as u64 }
}

/// Data Transfer entre produtos: a estrutura das tabelas escrita na linguagem do destino e os dados em lotes.
#[tauri::command]
pub async fn x_transfer(
    app: AppHandle,
    pg: State<'_, Sessions>,
    mysql: State<'_, MysqlSessions>,
    mssql: State<'_, MssqlSessions>,
    sqlite: State<'_, SqliteSessions>,
    req: TransferRequest,
) -> Result<TransferCounts, DatabaseError> {
    let started = Instant::now();
    let spec = &req.spec;
    let (sp, src) = open_exec(&req.source_conn, &req.source_database, &spec.source_schema, &pg, &mysql, &mssql, &sqlite).await?;
    let (dp, dst) = open_exec(&req.target_conn, &req.target_database, &spec.target_schema, &pg, &mysql, &mssql, &sqlite).await?;
    if req.source_conn == req.target_conn && req.source_database == req.target_database && spec.source_schema == spec.target_schema {
        return Err(DatabaseError::new(ErrorKind::InvalidInput, "the source and the target are the same: choose another database, schema or connection"));
    }
    let cancel = Arc::new(AtomicBool::new(false));
    register_flag(&req.transfer_id, cancel.clone());
    let (log_app, log_id) = (app.clone(), req.transfer_id.clone());
    let mut on_log = move |line: String| {
        let _ = log_app.emit("transfer-log", LogEvent { transfer_id: &log_id, line });
    };
    let (progress_app, progress_id) = (app.clone(), req.transfer_id.clone());
    let mut on_progress = move |s: &TransferStats| {
        let _ = progress_app.emit("transfer-progress", ProgressEvent { transfer_id: &progress_id, counts: counts(s, started) });
    };
    let cancelled = || cancel.load(Ordering::Relaxed);
    let mut run = Run { stats: TransferStats::default(), log: &mut on_log, progress: &mut on_progress, cancelled: &cancelled, continue_on_error: spec.continue_on_error };
    let outcome = transfer(sp, dp, &src, &dst, &req, &mut run).await;
    unregister_flag(&req.transfer_id);
    outcome?;
    Ok(counts(&run.stats, started))
}

async fn transfer(sp: Product, dp: Product, src: &Exec, dst: &Exec, req: &TransferRequest, run: &mut Run<'_>) -> Result<(), DatabaseError> {
    let spec = &req.spec;
    let dd = dp.dialect();
    run.stats.objects = spec.tables.len() as u64;
    (run.progress)(&run.stats);
    if !spec.functions.is_empty() {
        (run.log)(format!("[DTF] {} function(s)/procedure(s) were not transferred: their text is code in the source product's language", spec.functions.len()));
    }
    // O destino: banco (MySQL) ou schema (PostgreSQL, SQL Server) precisa existir.
    match dp {
        Product::Pg => {
            dst.run(&format!("CREATE SCHEMA IF NOT EXISTS {}", dd.q(&spec.target_schema)), 1).await?;
        }
        Product::Mysql => {
            // O banco ainda pode não existir: o comando roda pela conexão sem banco.
            let admin = match dst {
                Exec::Mysql(s, _) => Exec::Mysql(s.clone(), String::new()),
                other => return Err(DatabaseError::new(ErrorKind::Internal, format!("unexpected target connection {}", matches!(other, Exec::Pg(_))))),
            };
            admin.run(&format!("CREATE DATABASE IF NOT EXISTS {}", dd.q(&spec.target_schema)), 1).await?;
        }
        Product::Mssql if spec.target_schema != "dbo" => {
            dst.run(&format!("IF SCHEMA_ID({}) IS NULL EXEC('CREATE SCHEMA {}')", dd.text(&spec.target_schema), dd.q(&spec.target_schema).replace('\'', "''")), 1).await?;
        }
        _ => {}
    }
    match dp {
        Product::Mysql => {
            dst.run("SET FOREIGN_KEY_CHECKS = 0", 1).await?;
        }
        Product::Sqlite => {
            dst.run("PRAGMA foreign_keys = OFF", 1).await?;
        }
        _ => {}
    }
    let mut post: Vec<String> = Vec::new();
    let mut details: Vec<(String, String, atlas_core::TableDetail)> = Vec::new();
    let outcome = async {
        if spec.structure {
            for t in &spec.tables {
                run.check()?;
                let detail = match src.table_detail(&spec.source_schema, t).await {
                    Ok(d) if !d.columns.is_empty() => d,
                    _ => {
                        run.error(t, "table not found in the source");
                        if !spec.continue_on_error {
                            return Ok(());
                        }
                        continue;
                    }
                };
                let target_name = spec.overrides.iter().find(|o| o.table == *t).and_then(|o| o.target_name.clone()).filter(|n| !n.is_empty()).unwrap_or_else(|| t.clone());
                let (pre, after, warnings) = create_table(sp, dp, &spec.target_schema, &target_name, &detail, &|_, _| None);
                for w in warnings {
                    (run.log)(format!("[DTF] {w}"));
                }
                if spec.drop_before_create && !run.ddl(dst, &drop_table_sql(dp, &spec.target_schema, &target_name)).await {
                    return Ok(());
                }
                for sql in &pre {
                    if !run.ddl(dst, sql).await {
                        return Ok(());
                    }
                }
                post.extend(after);
                details.push((t.clone(), target_name, detail));
            }
        } else {
            for t in &spec.tables {
                if let Ok(d) = src.table_detail(&spec.source_schema, t).await {
                    let target_name = spec.overrides.iter().find(|o| o.table == *t).and_then(|o| o.target_name.clone()).filter(|n| !n.is_empty()).unwrap_or_else(|| t.clone());
                    details.push((t.clone(), target_name, d));
                }
            }
        }
        if spec.data {
            for (t, target_name, detail) in &details {
                run.check()?;
                if !copy_table(sp, dp, src, dst, req, t, target_name, detail, run).await? {
                    return Ok(());
                }
            }
        }
        for sql in &post {
            run.check()?;
            if !run.ddl(dst, sql).await {
                return Ok(());
            }
        }
        // Views: o SELECT é traduzido quando só usa o que os dois produtos têm em comum.
        if spec.structure {
            for v in &spec.views {
                run.check()?;
                let def = match src.view_definition(&spec.source_schema, v).await {
                    Ok(d) => d,
                    Err(e) => {
                        run.error(v, &e.message);
                        if !spec.continue_on_error {
                            return Ok(());
                        }
                        continue;
                    }
                };
                let cx = crate::sqlx::Ctx { dst: dp, src_schema: &spec.source_schema, dst_schema: &spec.target_schema, select: true };
                let Some(select) = crate::sqlx::translate(&def, &cx) else {
                    (run.log)(format!("[DTF] view {v} was not transferred: its SELECT uses something only the source product has"));
                    continue;
                };
                let name = spec.overrides.iter().find(|o| o.table == *v).and_then(|o| o.target_name.clone()).filter(|n| !n.is_empty()).unwrap_or_else(|| v.clone());
                let target = dd.target(&spec.target_schema, &name);
                if spec.drop_before_create {
                    let drop = match dp {
                        Product::Mssql => format!("IF OBJECT_ID({}) IS NOT NULL DROP VIEW {target}", dd.text(&target)),
                        _ => format!("DROP VIEW IF EXISTS {target}"),
                    };
                    if !run.ddl(dst, &drop).await {
                        return Ok(());
                    }
                }
                if !run.ddl(dst, &format!("CREATE VIEW {target} AS {select}")).await {
                    return Ok(());
                }
                run.stats.objects += 1;
            }
        }
        Ok::<(), DatabaseError>(())
    }
    .await;
    match dp {
        Product::Mysql => {
            let _ = dst.run("SET FOREIGN_KEY_CHECKS = 1", 1).await;
        }
        Product::Sqlite => {
            let _ = dst.run("PRAGMA foreign_keys = ON", 1).await;
        }
        _ => {}
    }
    outcome
}
