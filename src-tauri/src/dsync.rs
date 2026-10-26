//! Data Synchronization do SQL Server, MySQL e SQLite: compara os dados de duas tabelas do MESMO produto, mostra as diferenças,
//! gera o script e o executa no destino. Os formatos de resposta e os eventos são os do PostgreSQL (`datasync.rs`); o que muda
//! é o catálogo, as aspas, os literais e a gravação de cada banco.
//!
//! A comparação lê as duas tabelas (os campos mapeados) e junta pela chave: chave que não existe no destino é `INSERT`, chave com
//! valores diferentes é `UPDATE` e o que sobrou do destino é `DELETE`. As linhas diferentes ficam na memória, guardadas pelo
//! `compare_id`, para a tela e para o script.

use crate::datasync::{CompareRequest, CompareSummary, DeployCounts, DeployProgress, DeployRequest, LogEvent, ProgressEvent, RowOut, RowsPage, ScriptOut, TableSummary};
use crate::genext::{Dialect, Exec};
use crate::neutral::{NType, Product, canonical, literal as nliteral, parse};
use crate::sessions::Sessions;
use crate::xprod::{open_exec, register_flag, unregister_flag};
use crate::firebird::FirebirdSessions;
use crate::mssql::MssqlSessions;
use crate::mysql::MysqlSessions;
use crate::sqlite::SqliteSessions;
use atlas_core::{DatabaseError, ErrorKind};
use atlas_pg::TableSelection;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tauri::{AppHandle, Emitter, State};

/// Teto de linhas diferentes guardadas na memória (somando todas as tabelas).
const MAX_DIFF_ROWS: usize = 1_000_000;
/// Linhas lidas de cada lado (acima disso a tabela não é comparada).
const MAX_READ_ROWS: usize = 2_000_000;
/// Linhas do script mostradas na tela (o script inteiro é executado do mesmo jeito).
const SCRIPT_PREVIEW: usize = 5000;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Insert,
    Update,
    Delete,
}

impl Kind {
    fn name(self) -> &'static str {
        match self {
            Kind::Insert => "insert",
            Kind::Update => "update",
            Kind::Delete => "delete",
        }
    }
}

struct Row {
    kind: Kind,
    source: Option<Vec<Option<String>>>,
    target: Option<Vec<Option<String>>>,
    changed: Vec<bool>,
}

#[derive(Default)]
struct TableDiff {
    source: String,
    target: String,
    source_fields: Vec<String>,
    target_fields: Vec<String>,
    key_positions: Vec<usize>,
    /// Tipo neutro de cada campo, na origem e no destino.
    src_types: Vec<NType>,
    dst_types: Vec<NType>,
    /// O destino numera sozinho um dos campos (`IDENTITY`): o `INSERT` precisa de `IDENTITY_INSERT`.
    identity: bool,
    insert: u64,
    update: u64,
    delete: u64,
    same: u64,
    message: Option<String>,
    rows: Vec<Row>,
}

struct Comparison {
    dialect: Dialect,
    /// O destino Firebird tem `BOOLEAN` (versão 3 em diante)? Decide como o script escreve os booleanos.
    firebird_v3: bool,
    target_schema: String,
    tables: Vec<TableDiff>,
}

/// Comparações feitas e ainda não descartadas (uma por janela).
#[derive(Default)]
pub struct ExtComparisons {
    inner: Mutex<HashMap<String, Arc<Comparison>>>,
}

impl ExtComparisons {
    fn get(&self, id: &str) -> Result<Arc<Comparison>, DatabaseError> {
        self.inner.lock().unwrap().get(id).cloned().ok_or_else(|| DatabaseError::new(ErrorKind::InvalidInput, "the comparison is no longer available; compare again"))
    }
}

fn canceled() -> DatabaseError {
    DatabaseError::new(ErrorKind::Canceled, "synchronization canceled")
}

/// A chave e os valores da linha em forma canônica (para juntar e comparar linhas de produtos diferentes).
fn canon_row(row: &[Option<String>], types: &[NType]) -> Vec<Option<String>> {
    row.iter().zip(types).map(|(v, t)| v.as_ref().map(|x| canonical(t, x))).collect()
}

fn where_keys(d: Dialect, fields: &[String], positions: &[usize], from: &[NType], to: &[NType], row: &[Option<String>]) -> String {
    positions
        .iter()
        .map(|&p| match &row[p] {
            None => format!("{} IS NULL", d.q(&fields[p])),
            v => format!("{} = {}", d.q(&fields[p]), nliteral(d, &to[p], v, &from[p])),
        })
        .collect::<Vec<_>>()
        .join(" AND ")
}

fn select_sql(d: Dialect, schema: &str, table: &str, fields: &[String]) -> String {
    format!("SELECT {} FROM {}", fields.iter().map(|f| d.q(f)).collect::<Vec<_>>().join(", "), d.target(schema, table))
}

/// Compara uma tabela. Mensagens em `message` explicam por que ela não foi comparada.
#[allow(clippy::too_many_arguments)]
async fn compare_table(sp: Product, dp: Product, src: &Exec, dst: &Exec, spec: &atlas_pg::SyncSpec, t: &atlas_pg::SyncTable, budget: &mut usize) -> Result<TableDiff, DatabaseError> {
    let mut out = TableDiff { source: t.source.clone(), target: t.target.clone(), ..Default::default() };
    let stable = match src.table_detail(&spec.source_schema, &t.source).await {
        Ok(x) if !x.columns.is_empty() => x,
        _ => {
            out.message = Some(format!("source table {} was not found", t.source));
            return Ok(out);
        }
    };
    let ttable = match dst.table_detail(&spec.target_schema, &t.target).await {
        Ok(x) if !x.columns.is_empty() => x,
        _ => {
            out.message = Some(format!("target table {} was not found", t.target));
            return Ok(out);
        }
    };
    // Campos: os mapeados, ou os de mesmo nome (sem os calculados).
    let pairs: Vec<(String, String)> = match t.fields.as_ref().filter(|f| !f.is_empty()) {
        Some(f) => f.iter().map(|m| (m.source.clone(), if m.target.is_empty() { m.source.clone() } else { m.target.clone() })).collect(),
        None => stable.columns.iter().filter(|c| c.generated.is_none() && ttable.columns.iter().any(|x| x.name == c.name)).map(|c| (c.name.clone(), c.name.clone())).collect(),
    };
    if pairs.is_empty() {
        out.message = Some("no field in common between the two tables".into());
        return Ok(out);
    }
    for (s, g) in &pairs {
        if !stable.columns.iter().any(|c| c.name == *s) {
            out.message = Some(format!("source field \"{s}\" does not exist in {}", t.source));
            return Ok(out);
        }
        if !ttable.columns.iter().any(|c| c.name == *g) {
            out.message = Some(format!("target field \"{g}\" does not exist in {}", t.target));
            return Ok(out);
        }
    }
    // Chave: a escolhida, ou a chave primária da origem.
    let key_sources: Vec<String> = match t.key.as_ref().filter(|k| !k.is_empty()) {
        Some(k) => k.iter().map(|m| m.source.clone()).collect(),
        None => {
            let mut keys: Vec<(i32, String)> = stable.columns.iter().filter_map(|c| c.key.map(|k| (k, c.name.clone()))).collect();
            keys.sort();
            keys.into_iter().map(|(_, n)| n).collect()
        }
    };
    let positions: Vec<usize> = key_sources.iter().filter_map(|k| pairs.iter().position(|(s, _)| s == k)).collect();
    if positions.is_empty() || positions.len() != key_sources.len() {
        out.message = Some(if key_sources.is_empty() { "the source table has no primary key".into() } else { "a key field is not among the mapped fields".into() });
        return Ok(out);
    }
    out.source_fields = pairs.iter().map(|(s, _)| s.clone()).collect();
    out.target_fields = pairs.iter().map(|(_, g)| g.clone()).collect();
    out.key_positions = positions.clone();
    out.src_types = out.source_fields.iter().map(|f| parse(sp, &stable.columns.iter().find(|c| c.name == *f).map(|c| c.formatted_type.clone()).unwrap_or_default())).collect();
    out.dst_types = out.target_fields.iter().map(|f| parse(dp, &ttable.columns.iter().find(|c| c.name == *f).map(|c| c.formatted_type.clone()).unwrap_or_default())).collect();
    out.identity = ttable.columns.iter().any(|c| c.identity.is_some() && out.target_fields.contains(&c.name));

    let src_rows = src.rows(&select_sql(sp.dialect(), &spec.source_schema, &t.source, &out.source_fields), MAX_READ_ROWS).await?;
    let dst_rows = dst.rows(&select_sql(dp.dialect(), &spec.target_schema, &t.target, &out.target_fields), MAX_READ_ROWS).await?;
    if src_rows.len() >= MAX_READ_ROWS || dst_rows.len() >= MAX_READ_ROWS {
        out.message = Some("the table is too large to compare".into());
        return Ok(out);
    }
    let (src_types, dst_types) = (out.src_types.clone(), out.dst_types.clone());
    let key_src = |row: &[Option<String>]| -> Vec<Option<String>> { positions.iter().map(|&p| row[p].as_ref().map(|x| canonical(&src_types[p], x))).collect() };
    let key_dst = |row: &[Option<String>]| -> Vec<Option<String>> { positions.iter().map(|&p| row[p].as_ref().map(|x| canonical(&dst_types[p], x))).collect() };
    let mut target_by_key: HashMap<Vec<Option<String>>, usize> = HashMap::with_capacity(dst_rows.len());
    for (i, r) in dst_rows.iter().enumerate() {
        target_by_key.insert(key_dst(r), i);
    }
    let mut matched = vec![false; dst_rows.len()];
    for r in &src_rows {
        match target_by_key.get(&key_src(r)) {
            None => {
                out.insert += 1;
                if *budget > 0 {
                    *budget -= 1;
                    out.rows.push(Row { kind: Kind::Insert, source: Some(r.clone()), target: None, changed: Vec::new() });
                }
            }
            Some(&i) => {
                matched[i] = true;
                let (cs, cd) = (canon_row(r, &src_types), canon_row(&dst_rows[i], &dst_types));
                let changed: Vec<bool> = (0..r.len()).map(|k| !positions.contains(&k) && cs[k] != cd[k]).collect();
                if changed.iter().any(|c| *c) {
                    out.update += 1;
                    if *budget > 0 {
                        *budget -= 1;
                        out.rows.push(Row { kind: Kind::Update, source: Some(r.clone()), target: Some(dst_rows[i].clone()), changed });
                    }
                } else {
                    out.same += 1;
                }
            }
        }
    }
    for (i, r) in dst_rows.iter().enumerate() {
        if !matched[i] {
            out.delete += 1;
            if *budget > 0 {
                *budget -= 1;
                out.rows.push(Row { kind: Kind::Delete, source: None, target: Some(r.clone()), changed: Vec::new() });
            }
        }
    }
    Ok(out)
}

#[tauri::command]
pub async fn ext_datasync_compare(
    app: AppHandle,
    pg: State<'_, Sessions>,
    mysql: State<'_, MysqlSessions>,
    mssql: State<'_, MssqlSessions>,
    sqlite: State<'_, SqliteSessions>,
    comparisons: State<'_, ExtComparisons>,
    req: CompareRequest,
) -> Result<CompareSummary, DatabaseError> {
    comparisons.inner.lock().unwrap().remove(&req.compare_id);
    let (sp, src) = open_exec(&req.source_conn, &req.source_database, &req.spec.source_schema, &pg, &mysql, &mssql, &sqlite).await?;
    let (dp, dst) = open_exec(&req.target_conn, &req.target_database, &req.spec.target_schema, &pg, &mysql, &mssql, &sqlite).await?;
    let d = dp.dialect();
    let cancel = Arc::new(AtomicBool::new(false));
    register_cancel(&req.compare_id, &req.source_conn, &cancel, &mysql, &mssql, &sqlite);
    let total = req.spec.tables.len().max(1);
    let mut tables = Vec::new();
    let mut budget = MAX_DIFF_ROWS;
    let mut outcome = Ok(());
    for (n, t) in req.spec.tables.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            outcome = Err(canceled());
            break;
        }
        let _ = app.emit("datasync-progress", ProgressEvent { id: &req.compare_id, text: t.source.clone(), fraction: n as f64 / total as f64 });
        match compare_table(sp, dp, &src, &dst, &req.spec, t, &mut budget).await {
            Ok(diff) => tables.push(diff),
            Err(e) => {
                outcome = Err(e);
                break;
            }
        }
    }
    unregister_cancel(&req.compare_id, &req.source_conn, &mysql, &mssql, &sqlite);
    outcome?;
    let _ = app.emit("datasync-progress", ProgressEvent { id: &req.compare_id, text: String::new(), fraction: 1.0 });
    let summary = CompareSummary {
        tables: tables
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
    comparisons.inner.lock().unwrap().insert(req.compare_id, Arc::new(Comparison { dialect: d, firebird_v3: crate::neutral::firebird_v3(), target_schema: req.spec.target_schema.clone(), tables }));
    Ok(summary)
}

/// Registra o pedido de parada da operação: no registro comum (ids `xp_`, entre produtos) ou no gerenciador do produto da conexão.
pub(crate) fn register_cancel(id: &str, conn: &str, flag: &Arc<AtomicBool>, mysql: &MysqlSessions, mssql: &MssqlSessions, sqlite: &SqliteSessions) {
    if id.starts_with("xp_") || !matches!(Product::of_conn(conn), Product::Mysql | Product::Mssql | Product::Sqlite | Product::Firebird) {
        register_flag(id, flag.clone());
    } else if conn.starts_with("mysql_") {
        mysql.register_cancel(id, flag.clone());
    } else if conn.starts_with("mssql_") {
        mssql.register_cancel(id, flag.clone());
    } else if conn.starts_with("firebird_") {
        FirebirdSessions::default().register_cancel(id, flag.clone());
    } else {
        sqlite.register_cancel(id, flag.clone());
    }
}

pub(crate) fn unregister_cancel(id: &str, conn: &str, mysql: &MysqlSessions, mssql: &MssqlSessions, sqlite: &SqliteSessions) {
    unregister_flag(id);
    if conn.starts_with("mysql_") {
        mysql.unregister_cancel(id);
    } else if conn.starts_with("mssql_") {
        mssql.unregister_cancel(id);
    } else if conn.starts_with("sqlite_") {
        sqlite.unregister_cancel(id);
    } else if conn.starts_with("firebird_") {
        FirebirdSessions::default().unregister_cancel(id);
    }
}

#[tauri::command]
pub fn ext_datasync_rows(comparisons: State<'_, ExtComparisons>, compare_id: String, table: String, kind: Option<String>, offset: usize, limit: usize) -> Result<RowsPage, DatabaseError> {
    let c = comparisons.get(&compare_id)?;
    let t = c.tables.iter().find(|t| t.source == table).ok_or_else(|| DatabaseError::new(ErrorKind::InvalidInput, "table not in the comparison"))?;
    let matching: Vec<(usize, &Row)> = t.rows.iter().enumerate().filter(|(_, r)| kind.as_deref().is_none_or(|k| k == r.kind.name())).collect();
    let rows = matching
        .iter()
        .skip(offset)
        .take(limit)
        .map(|(i, r)| RowOut { id: *i as u32, kind: r.kind.name(), source: r.source.clone(), target: r.target.clone(), changed: r.changed.clone() })
        .collect();
    Ok(RowsPage { total: matching.len(), rows })
}

/// Comandos que levam o destino ao estado da origem: por tabela os `INSERT` e `UPDATE`; no fim os `DELETE`, na ordem inversa.
fn script(c: &Comparison, selection: &[TableSelection]) -> Vec<String> {
    crate::neutral::set_firebird_v3(c.firebird_v3);
    let d = c.dialect;
    let mut out = Vec::new();
    let mut deletes: Vec<Vec<String>> = Vec::new();
    for sel in selection {
        let Some(t) = c.tables.iter().find(|t| t.source == sel.table) else { continue };
        let target = d.target(&c.target_schema, &t.target);
        let columns = t.target_fields.iter().map(|f| d.q(f)).collect::<Vec<_>>().join(", ");
        let mut del = Vec::new();
        for (i, row) in t.rows.iter().enumerate() {
            if sel.excluded.contains(&(i as u32)) {
                continue;
            }
            match row.kind {
                Kind::Insert if sel.insert => {
                    let src = row.source.as_ref().expect("insert has a source row");
                    let values = src.iter().enumerate().map(|(k, v)| nliteral(d, &t.dst_types[k], v, &t.src_types[k])).collect::<Vec<_>>().join(", ");
                    let insert = format!("INSERT INTO {target} ({columns}) VALUES ({values})");
                    if t.identity && d == Dialect::Mssql {
                        out.push(format!("SET IDENTITY_INSERT {target} ON; {insert}; SET IDENTITY_INSERT {target} OFF;"));
                    } else {
                        out.push(format!("{insert};"));
                    }
                }
                Kind::Update if sel.update => {
                    let src = row.source.as_ref().expect("update has a source row");
                    let sets: Vec<String> = (0..src.len()).filter(|k| !t.key_positions.contains(k)).map(|k| format!("{} = {}", d.q(&t.target_fields[k]), nliteral(d, &t.dst_types[k], &src[k], &t.src_types[k]))).collect();
                    if !sets.is_empty() {
                        out.push(format!("UPDATE {target} SET {} WHERE {};", sets.join(", "), where_keys(d, &t.target_fields, &t.key_positions, &t.src_types, &t.dst_types, src)));
                    }
                }
                Kind::Delete if sel.delete => {
                    let tgt = row.target.as_ref().expect("delete has a target row");
                    del.push(format!("DELETE FROM {target} WHERE {};", where_keys(d, &t.target_fields, &t.key_positions, &t.dst_types, &t.dst_types, tgt)));
                }
                _ => {}
            }
        }
        deletes.push(del);
    }
    for dl in deletes.into_iter().rev() {
        out.extend(dl);
    }
    out
}

#[tauri::command]
pub fn ext_datasync_script(comparisons: State<'_, ExtComparisons>, compare_id: String, selection: Vec<TableSelection>) -> Result<ScriptOut, DatabaseError> {
    let c = comparisons.get(&compare_id)?;
    let statements = script(&c, &selection);
    let truncated = statements.len() > SCRIPT_PREVIEW;
    let text = statements.iter().take(SCRIPT_PREVIEW).cloned().collect::<Vec<_>>().join("\n\n");
    Ok(ScriptOut { statements: statements.len(), text, truncated })
}

#[tauri::command]
pub fn ext_datasync_script_full(comparisons: State<'_, ExtComparisons>, compare_id: String, selection: Vec<TableSelection>) -> Result<String, DatabaseError> {
    let c = comparisons.get(&compare_id)?;
    Ok(script(&c, &selection).join("\n\n"))
}

#[allow(clippy::too_many_arguments)]
/// Executa o script no destino (também usado pela Structure Synchronization). Em transação um erro desfaz tudo, a menos que se continue em erro.
pub(crate) async fn deploy(
    d: Dialect,
    dst: &Exec,
    statements: &[String],
    transaction: bool,
    continue_on_error: bool,
    echo: bool,
    log: &mut (dyn FnMut(String) + Send),
    progress: &mut (dyn FnMut(u64, u64) + Send),
    cancelled: &(dyn Fn() -> bool + Sync),
) -> Result<DeployCounts, DatabaseError> {
    let started = Instant::now();
    let total = statements.len() as u64;
    let (mut done, mut errors, mut rolled_back) = (0u64, 0u64, false);
    log("--Start--".into());
    let tx = if transaction { dst.begin().await? } else { None };
    let conn: &Exec = tx.as_ref().unwrap_or(dst);
    if tx.is_some() {
        log("--Transaction - Begin--".into());
    }
    let mut failed = false;
    for (i, stmt) in statements.iter().enumerate() {
        if cancelled() {
            if let Some(tx) = &tx {
                let _ = tx.finish(false).await;
            }
            return Err(canceled());
        }
        let savepoint = tx.is_some() && continue_on_error;
        if savepoint {
            conn.run(d.savepoint(), 1).await?;
        }
        if echo {
            log(stmt.clone());
        }
        match conn.run(stmt, 1).await {
            Ok(_) => {
                done += 1;
                if savepoint && let Some(r) = d.release() {
                    conn.run(r, 1).await?;
                }
            }
            Err(e) => {
                log(format!("[ERR] {}", e.message));
                errors += 1;
                if savepoint {
                    conn.run(d.rollback_to(), 1).await?;
                } else if !continue_on_error {
                    failed = true;
                    break;
                }
            }
        }
        if i % 100 == 0 {
            progress(done + errors, total);
        }
    }
    if let Some(tx) = &tx {
        if failed {
            let _ = tx.finish(false).await;
            rolled_back = true;
            done = 0;
            log("--Transaction - Rollback--".into());
        } else {
            tx.finish(true).await?;
        }
    }
    log(format!("Executed Query: {done}"));
    if tx.is_some() && !failed {
        log("--Transaction - Commit--".into());
    }
    progress(done + errors, total);
    log(format!("--End-- ({:.2} s)", started.elapsed().as_secs_f64()));
    Ok(DeployCounts { total, done, errors, rolled_back, elapsed_ms: started.elapsed().as_millis() as u64 })
}

#[tauri::command]
pub async fn ext_datasync_deploy(
    app: AppHandle,
    pg: State<'_, Sessions>,
    mysql: State<'_, MysqlSessions>,
    mssql: State<'_, MssqlSessions>,
    sqlite: State<'_, SqliteSessions>,
    comparisons: State<'_, ExtComparisons>,
    req: DeployRequest,
) -> Result<DeployCounts, DatabaseError> {
    let c = comparisons.get(&req.compare_id)?;
    let statements = script(&c, &req.selection);
    let (dp, dst) = open_exec(&req.target_conn, &req.target_database, &c.target_schema, &pg, &mysql, &mssql, &sqlite).await?;
    let d = dp.dialect();
    let cancel = Arc::new(AtomicBool::new(false));
    register_cancel(&req.deploy_id, &req.target_conn, &cancel, &mysql, &mssql, &sqlite);
    let (log_app, log_id) = (app.clone(), req.deploy_id.clone());
    let mut on_log = move |line: String| {
        let _ = log_app.emit("datasync-log", LogEvent { id: &log_id, line });
    };
    let (progress_app, progress_id) = (app.clone(), req.deploy_id.clone());
    let mut on_progress = move |done: u64, total: u64| {
        let _ = progress_app.emit("datasync-deploy-progress", DeployProgress { id: &progress_id, done, total });
    };
    let cancelled = || cancel.load(Ordering::Relaxed);
    let out = deploy(d, &dst, &statements, req.transaction, req.continue_on_error, false, &mut on_log, &mut on_progress, &cancelled).await;
    unregister_cancel(&req.deploy_id, &req.target_conn, &mysql, &mssql, &sqlite);
    out
}

#[tauri::command]
pub fn ext_datasync_forget(comparisons: State<'_, ExtComparisons>, compare_id: String) {
    comparisons.inner.lock().unwrap().remove(&compare_id);
}
