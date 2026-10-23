//! Data Synchronization: compara os dados de tabelas de dois schemas (do mesmo servidor ou de servidores diferentes) e gera
//! o script que deixa o destino igual à origem.
//!
//! A comparação lê o destino inteiro uma vez (só a chave e um resumo de cada linha) e depois percorre a origem: chave que não
//! existe no destino é `INSERT`, chave com resumo diferente é `UPDATE`, e o que sobrou do destino é `DELETE`. As linhas
//! diferentes ficam guardadas (origem e destino) para a tela mostrar lado a lado e para o script. Os valores são comparados
//! como texto, depois de fixar a sessão (UTC, `ISO`) e de normalizar os números de ponto flutuante (o 9.6 e o 18 escrevem
//! `0.1` de jeitos diferentes).

use crate::connect::PgSession;
use crate::dump::{begin_snapshot, end_snapshot};
use crate::error::map_error;
use crate::exec::quote_ident;
use crate::meta::table_detail;
use crate::transfer::{FieldMap, fetch_batch};
use atlas_core::{DatabaseError, ErrorKind};
use serde::Deserialize;
use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::Hasher;
use std::time::Instant;
use tokio_postgres::SimpleQueryMessage;

const BATCH: usize = 5000;
/// Chaves buscadas de uma vez ao ler as linhas do destino que mudaram ou sobraram.
const KEY_BATCH: usize = 200;
/// Teto de linhas diferentes guardadas na memória (somando todas as tabelas).
const MAX_DIFF_ROWS: usize = 1_000_000;

/// Uma tabela a comparar. `key` e `fields` ausentes valem o padrão: a chave primária da origem e os campos de mesmo nome.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncTable {
    pub source: String,
    pub target: String,
    #[serde(default)]
    pub key: Option<Vec<FieldMap>>,
    #[serde(default)]
    pub fields: Option<Vec<FieldMap>>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncSpec {
    pub source_schema: String,
    pub target_schema: String,
    pub tables: Vec<SyncTable>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffKind {
    Insert,
    Update,
    Delete,
}

/// Linha diferente. O número dela na lista da tabela (`TableDiff::rows`) é o que a tela usa para marcar/desmarcar.
#[derive(Debug, Clone)]
pub struct DiffRow {
    pub kind: DiffKind,
    /// Valores da origem, na ordem de `TableDiff::source_fields` (`None` em exclusões).
    pub source: Option<Vec<Option<String>>>,
    /// Valores do destino, na ordem de `TableDiff::target_fields` (`None` em inclusões).
    pub target: Option<Vec<Option<String>>>,
    /// Campos que mudam (só em atualizações).
    pub changed: Vec<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Text,
    Number,
    Bool,
    F32,
    F64,
}

fn kind_of(formatted_type: &str) -> Kind {
    let t = formatted_type.to_ascii_lowercase();
    if t == "boolean" {
        Kind::Bool
    } else if t == "real" {
        Kind::F32
    } else if t == "double precision" {
        Kind::F64
    } else if ["smallint", "integer", "bigint", "numeric", "decimal"].iter().any(|k| t.starts_with(k)) {
        Kind::Number
    } else {
        Kind::Text
    }
}

#[derive(Debug, Default)]
pub struct TableDiff {
    pub source: String,
    pub target: String,
    pub source_fields: Vec<String>,
    pub target_fields: Vec<String>,
    /// Posições (nas listas de campos) das colunas da chave.
    pub key_positions: Vec<usize>,
    kinds: Vec<Option<Kind>>,
    /// O destino tem identidade `GENERATED ALWAYS` num dos campos: o `INSERT` precisa de `OVERRIDING SYSTEM VALUE`.
    pub identity_always: bool,
    pub insert: u64,
    pub update: u64,
    pub delete: u64,
    pub same: u64,
    /// Por que a tabela não foi comparada (sem chave primária, campo que não existe...).
    pub message: Option<String>,
    pub rows: Vec<DiffRow>,
}

#[derive(Debug, Default)]
pub struct Comparison {
    pub source_schema: String,
    pub target_schema: String,
    pub tables: Vec<TableDiff>,
}

pub struct CompareHooks<'a> {
    /// Texto do que está sendo lido e a fração concluída (0 a 1).
    pub progress: &'a mut (dyn FnMut(&str, f64) + Send),
    pub cancelled: &'a (dyn Fn() -> bool + Sync),
}

fn sql_err(e: tokio_postgres::Error) -> DatabaseError {
    map_error(&e)
}

fn canceled() -> DatabaseError {
    DatabaseError::new(ErrorKind::Canceled, "synchronization canceled")
}

/// Texto estável de um valor para comparar: o ponto flutuante vira o menor texto que o representa.
fn normalized(s: &str, k: Kind) -> String {
    match k {
        Kind::F64 => s.parse::<f64>().map(|f| format!("{f}")).unwrap_or_else(|_| s.to_string()),
        Kind::F32 => s.parse::<f32>().map(|f| format!("{f}")).unwrap_or_else(|_| s.to_string()),
        _ => s.to_string(),
    }
}

fn same_value(a: &Option<String>, b: &Option<String>, k: Kind) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(x), Some(y)) => x == y || normalized(x, k) == normalized(y, k),
        _ => false,
    }
}

fn row_hash(row: &[Option<String>], key: &[usize], kinds: &[Kind]) -> u64 {
    let mut h = DefaultHasher::new();
    for (i, v) in row.iter().enumerate() {
        if key.contains(&i) {
            continue;
        }
        match v {
            None => h.write_u8(0),
            Some(s) => {
                h.write_u8(1);
                h.write(normalized(s, kinds[i]).as_bytes());
                h.write_u8(0xff);
            }
        }
    }
    h.finish()
}

fn text_literal(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// Valor como literal SQL: número e booleano sem aspas quando o tipo do destino é esse; o resto como texto (o servidor converte).
fn literal(v: &Option<String>, k: Kind) -> String {
    let Some(s) = v else { return "NULL".into() };
    let plain_number = {
        let body = s.strip_prefix('-').unwrap_or(s);
        let mut parts = body.splitn(2, '.');
        let int = parts.next().unwrap_or("");
        !int.is_empty() && int.bytes().all(|b| b.is_ascii_digit()) && parts.next().is_none_or(|f| !f.is_empty() && f.bytes().all(|b| b.is_ascii_digit()))
    };
    match k {
        Kind::Number if plain_number => s.clone(),
        Kind::Bool if s == "t" || s == "true" => "true".into(),
        Kind::Bool if s == "f" || s == "false" => "false".into(),
        _ => text_literal(s),
    }
}

fn where_keys(fields: &[String], positions: &[usize], kinds: &[Option<Kind>], row: &[Option<String>]) -> String {
    positions
        .iter()
        .map(|&p| format!("{} = {}", quote_ident(&fields[p]), literal(&row[p], kinds[p].unwrap_or(Kind::Text))))
        .collect::<Vec<_>>()
        .join(" AND ")
}

fn select_sql(schema: &str, table: &str, columns: &[String]) -> String {
    format!("SELECT {} FROM {}.{}", columns.iter().map(|c| quote_ident(c)).collect::<Vec<_>>().join(", "), quote_ident(schema), quote_ident(table))
}

type Key = Vec<Option<String>>;

/// Lê do destino as linhas das chaves dadas (as que mudaram ou sobraram), em lotes.
async fn fetch_rows(dst: &PgSession, schema: &str, table: &str, fields: &[String], positions: &[usize], keys: &[Key]) -> Result<HashMap<Key, Vec<Option<String>>>, DatabaseError> {
    let mut out = HashMap::new();
    for chunk in keys.chunks(KEY_BATCH) {
        let key_cols: Vec<String> = positions.iter().map(|&p| quote_ident(&fields[p])).collect();
        let tuples: Vec<String> = chunk
            .iter()
            .map(|k| {
                let lits: Vec<String> = k.iter().map(|v| v.as_deref().map(text_literal).unwrap_or_else(|| "NULL".into())).collect();
                if lits.len() == 1 { lits[0].clone() } else { format!("({})", lits.join(", ")) }
            })
            .collect();
        let lhs = if key_cols.len() == 1 { key_cols[0].clone() } else { format!("({})", key_cols.join(", ")) };
        let sql = format!("{} WHERE {lhs} IN ({})", select_sql(schema, table, fields), tuples.join(", "));
        for m in dst.client.simple_query(&sql).await.map_err(sql_err)? {
            if let SimpleQueryMessage::Row(r) = m {
                let row: Vec<Option<String>> = (0..fields.len()).map(|i| r.get(i).map(str::to_string)).collect();
                out.insert(positions.iter().map(|&p| row[p].clone()).collect(), row);
            }
        }
    }
    Ok(out)
}

/// Resolve o mapeamento de uma tabela; `Err` é o motivo (mensagem da tela) de ela não poder ser comparada.
async fn plan_table(src: &PgSession, dst: &PgSession, spec: &SyncSpec, t: &SyncTable, diff: &mut TableDiff) -> Result<(), String> {
    let sd = table_detail(src, &spec.source_schema, &t.source).await.map_err(|e| e.message)?;
    if sd.columns.is_empty() {
        return Err(format!("Source table {} was not found", t.source));
    }
    let td = table_detail(dst, &spec.target_schema, &t.target).await.map_err(|e| e.message)?;
    if td.columns.is_empty() {
        return Err(format!("Target table {} was not found", t.target));
    }
    let pairs = |list: &[FieldMap]| -> Vec<(String, String)> { list.iter().map(|m| (m.source.clone(), if m.target.is_empty() { m.source.clone() } else { m.target.clone() })).collect() };
    let mut fields: Vec<(String, String)> = match t.fields.as_ref().filter(|f| !f.is_empty()) {
        Some(f) => pairs(f),
        None => sd
            .columns
            .iter()
            .filter(|c| c.generated.as_deref() != Some("s"))
            .filter(|c| td.columns.iter().any(|x| x.name == c.name && x.generated.as_deref() != Some("s")))
            .map(|c| (c.name.clone(), c.name.clone()))
            .collect(),
    };
    let key: Vec<(String, String)> = match t.key.as_ref().filter(|k| !k.is_empty()) {
        Some(k) => pairs(k),
        None => {
            let mut pk: Vec<_> = sd.columns.iter().filter_map(|c| c.key.map(|n| (n, c.name.clone()))).collect();
            pk.sort();
            pk.into_iter().map(|(_, n)| (n.clone(), n)).collect()
        }
    };
    if key.is_empty() {
        return Err("Source table do not have primary key".into());
    }
    // Os campos da chave também são lidos.
    for k in &key {
        if !fields.iter().any(|f| f.0 == k.0) {
            fields.insert(0, k.clone());
        }
    }
    for (s, d) in &fields {
        if !sd.columns.iter().any(|c| c.name == *s) {
            return Err(format!("Source table do not have field {s}"));
        }
        if !td.columns.iter().any(|c| c.name == *d) {
            return Err(format!("Target table do not have field {d}"));
        }
    }
    diff.key_positions = key.iter().map(|k| fields.iter().position(|f| f.0 == k.0).unwrap_or(0)).collect();
    diff.kinds = fields.iter().map(|(_, d)| td.columns.iter().find(|c| c.name == *d).map(|c| kind_of(&c.formatted_type))).collect();
    diff.identity_always = fields.iter().any(|(_, d)| td.columns.iter().any(|c| c.name == *d && c.identity.as_deref() == Some("a")));
    diff.source_fields = fields.iter().map(|f| f.0.clone()).collect();
    diff.target_fields = fields.iter().map(|f| f.1.clone()).collect();
    Ok(())
}

/// Compara as tabelas pedidas. Cancelamento e perda de conexão voltam como `Err`.
pub async fn compare(src: &PgSession, dst: &PgSession, spec: &SyncSpec, hooks: CompareHooks<'_>) -> Result<Comparison, DatabaseError> {
    let CompareHooks { progress, cancelled } = hooks;
    for s in [src, dst] {
        s.client
            .batch_execute("SET TimeZone = 'UTC'; SET datestyle = 'ISO, MDY'; SET intervalstyle = 'postgres'; SET bytea_output = 'hex'; SET extra_float_digits = 3")
            .await
            .map_err(sql_err)?;
    }
    begin_snapshot(src).await?;
    dst.client.batch_execute("BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY").await.map_err(sql_err)?;
    let outcome = async {
        let mut result = Comparison { source_schema: spec.source_schema.clone(), target_schema: spec.target_schema.clone(), tables: Vec::new() };
        let mut stored = 0usize;
        for (n, t) in spec.tables.iter().enumerate() {
            let step = |inside: f64| (n as f64 + inside) / spec.tables.len().max(1) as f64;
            if cancelled() {
                return Err(canceled());
            }
            let mut diff = TableDiff { source: t.source.clone(), target: t.target.clone(), ..Default::default() };
            progress(&format!("Getting \"{}\" info (Source)...", t.source), step(0.0));
            if let Err(message) = plan_table(src, dst, spec, t, &mut diff).await {
                diff.message = Some(message);
                result.tables.push(diff);
                continue;
            }
            let kinds: Vec<Kind> = diff.kinds.iter().map(|k| k.unwrap_or(Kind::Text)).collect();
            let ncols = diff.source_fields.len();
            // 1) Destino: chave e resumo de cada linha.
            progress(&format!("Getting \"{}\" info (Target)...", t.target), step(0.2));
            let mut target_map: HashMap<Key, (u64, bool)> = HashMap::new();
            dst.client
                .batch_execute(&format!("DECLARE atlas_ds_t NO SCROLL CURSOR FOR {}", select_sql(&spec.target_schema, &t.target, &diff.target_fields)))
                .await
                .map_err(sql_err)?;
            loop {
                if cancelled() {
                    return Err(canceled());
                }
                let rows = fetch_batch(dst, "atlas_ds_t", BATCH, ncols).await?;
                if rows.is_empty() {
                    break;
                }
                for r in rows {
                    let key: Key = diff.key_positions.iter().map(|&p| r[p].clone()).collect();
                    target_map.insert(key, (row_hash(&r, &diff.key_positions, &kinds), false));
                }
            }
            dst.client.batch_execute("CLOSE atlas_ds_t").await.map_err(sql_err)?;
            // 2) Origem: classifica cada linha.
            progress(&format!("Comparing \"{}\" data...", t.source), step(0.5));
            src.client
                .batch_execute(&format!("DECLARE atlas_ds_s NO SCROLL CURSOR FOR {}", select_sql(&spec.source_schema, &t.source, &diff.source_fields)))
                .await
                .map_err(sql_err)?;
            let mut updates: Vec<(usize, Key)> = Vec::new();
            loop {
                if cancelled() {
                    return Err(canceled());
                }
                let rows = fetch_batch(src, "atlas_ds_s", BATCH, ncols).await?;
                if rows.is_empty() {
                    break;
                }
                for r in rows {
                    let key: Key = diff.key_positions.iter().map(|&p| r[p].clone()).collect();
                    let h = row_hash(&r, &diff.key_positions, &kinds);
                    match target_map.get_mut(&key) {
                        Some(entry) => {
                            entry.1 = true;
                            if entry.0 == h {
                                diff.same += 1;
                            } else {
                                diff.update += 1;
                                updates.push((diff.rows.len(), key));
                                diff.rows.push(DiffRow { kind: DiffKind::Update, source: Some(r), target: None, changed: Vec::new() });
                            }
                        }
                        None => {
                            diff.insert += 1;
                            diff.rows.push(DiffRow { kind: DiffKind::Insert, source: Some(r), target: None, changed: Vec::new() });
                        }
                    }
                    if stored + diff.rows.len() > MAX_DIFF_ROWS {
                        return Err(DatabaseError::new(ErrorKind::InvalidInput, format!("too many differences (more than {MAX_DIFF_ROWS} rows); synchronize in smaller parts")));
                    }
                }
            }
            src.client.batch_execute("CLOSE atlas_ds_s").await.map_err(sql_err)?;
            // 3) Linhas do destino que mudaram ou sobraram.
            progress(&format!("Getting \"{}\" differences...", t.target), step(0.8));
            let mut leftovers: Vec<Key> = target_map.into_iter().filter(|(_, v)| !v.1).map(|(k, _)| k).collect();
            leftovers.sort();
            let wanted: Vec<Key> = updates.iter().map(|(_, k)| k.clone()).chain(leftovers.iter().cloned()).collect();
            let found = fetch_rows(dst, &spec.target_schema, &t.target, &diff.target_fields, &diff.key_positions, &wanted).await?;
            for (idx, key) in &updates {
                let target = found.get(key).cloned();
                let row = &mut diff.rows[*idx];
                if let (Some(src_row), Some(tgt)) = (&row.source, &target) {
                    row.changed = (0..ncols).map(|i| !diff.key_positions.contains(&i) && !same_value(&src_row[i], &tgt[i], kinds[i])).collect();
                }
                row.target = target;
            }
            for key in leftovers {
                if let Some(target) = found.get(&key) {
                    diff.delete += 1;
                    diff.rows.push(DiffRow { kind: DiffKind::Delete, source: None, target: Some(target.clone()), changed: Vec::new() });
                }
            }
            stored += diff.rows.len();
            result.tables.push(diff);
        }
        progress("Done", 1.0);
        Ok::<Comparison, DatabaseError>(result)
    }
    .await;
    end_snapshot(src).await;
    let _ = dst.client.batch_execute("ROLLBACK").await;
    outcome
}

/// O que implantar de uma tabela: os tipos de diferença marcados e as linhas desmarcadas (pelo número na lista).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TableSelection {
    /// Nome da tabela de origem (como em `SyncTable::source`).
    pub table: String,
    pub insert: bool,
    pub update: bool,
    pub delete: bool,
    #[serde(default)]
    pub excluded: Vec<u32>,
}

/// Comandos que levam o destino ao estado da origem: por tabela os `INSERT` e `UPDATE`; no fim os `DELETE`, na ordem inversa.
pub fn script(c: &Comparison, selection: &[TableSelection]) -> Vec<String> {
    let mut out = Vec::new();
    let mut deletes: Vec<Vec<String>> = Vec::new();
    for sel in selection {
        let Some(t) = c.tables.iter().find(|t| t.source == sel.table) else { continue };
        let kinds: Vec<Kind> = t.kinds.iter().map(|k| k.unwrap_or(Kind::Text)).collect();
        let target = format!("{}.{}", quote_ident(&c.target_schema), quote_ident(&t.target));
        let columns = t.target_fields.iter().map(|f| quote_ident(f)).collect::<Vec<_>>().join(", ");
        let mut del = Vec::new();
        for (i, row) in t.rows.iter().enumerate() {
            if sel.excluded.contains(&(i as u32)) {
                continue;
            }
            match row.kind {
                DiffKind::Insert if sel.insert => {
                    let src = row.source.as_ref().expect("insert has a source row");
                    let values = src.iter().enumerate().map(|(k, v)| literal(v, kinds[k])).collect::<Vec<_>>().join(", ");
                    let over = if t.identity_always { " OVERRIDING SYSTEM VALUE" } else { "" };
                    out.push(format!("INSERT INTO {target} ({columns}){over} VALUES ({values});"));
                }
                DiffKind::Update if sel.update => {
                    let src = row.source.as_ref().expect("update has a source row");
                    let sets: Vec<String> = (0..src.len())
                        .filter(|k| !t.key_positions.contains(k))
                        .map(|k| format!("{} = {}", quote_ident(&t.target_fields[k]), literal(&src[k], kinds[k])))
                        .collect();
                    if !sets.is_empty() {
                        out.push(format!("UPDATE {target} SET {} WHERE {};", sets.join(", "), where_keys(&t.target_fields, &t.key_positions, &t.kinds, src)));
                    }
                }
                DiffKind::Delete if sel.delete => {
                    let tgt = row.target.as_ref().expect("delete has a target row");
                    del.push(format!("DELETE FROM {target} WHERE {};", where_keys(&t.target_fields, &t.key_positions, &t.kinds, tgt)));
                }
                _ => {}
            }
        }
        deletes.push(del);
    }
    for d in deletes.into_iter().rev() {
        out.extend(d);
    }
    out
}

pub struct DeployHooks<'a> {
    pub log: &'a mut (dyn FnMut(String) + Send),
    /// Comandos executados e total.
    pub progress: &'a mut (dyn FnMut(u64, u64) + Send),
    pub cancelled: &'a (dyn Fn() -> bool + Sync),
}

#[derive(Debug, Clone, Default)]
pub struct DeployStats {
    pub total: u64,
    pub done: u64,
    pub errors: u64,
    pub rolled_back: bool,
}

/// Executa o script no destino. Em transação (padrão) um erro desfaz tudo, a menos que se continue em erro (cada comando
/// roda sob um `SAVEPOINT` e o que falha é só registrado).
pub async fn deploy(dst: &PgSession, statements: &[String], transaction: bool, continue_on_error: bool, hooks: DeployHooks<'_>) -> Result<DeployStats, DatabaseError> {
    let DeployHooks { log, progress, cancelled } = hooks;
    let started = Instant::now();
    let mut stats = DeployStats { total: statements.len() as u64, ..Default::default() };
    log("--Start--".into());
    dst.client.batch_execute("SET TimeZone = 'UTC'").await.map_err(sql_err)?;
    if transaction {
        dst.client.batch_execute("BEGIN").await.map_err(sql_err)?;
        log("--Transaction - Begin--".into());
    }
    let mut failed = false;
    if continue_on_error || !transaction {
        for (i, stmt) in statements.iter().enumerate() {
            if cancelled() {
                if transaction {
                    let _ = dst.client.batch_execute("ROLLBACK").await;
                }
                return Err(canceled());
            }
            if continue_on_error && transaction {
                dst.client.batch_execute("SAVEPOINT atlas_ds").await.map_err(sql_err)?;
            }
            match dst.client.batch_execute(stmt).await {
                Ok(()) => {
                    stats.done += 1;
                    if continue_on_error && transaction {
                        dst.client.batch_execute("RELEASE SAVEPOINT atlas_ds").await.map_err(sql_err)?;
                    }
                }
                Err(e) => {
                    let err = map_error(&e);
                    log(format!("[ERR] {}", err.message));
                    stats.errors += 1;
                    if continue_on_error && transaction {
                        dst.client.batch_execute("ROLLBACK TO SAVEPOINT atlas_ds").await.map_err(sql_err)?;
                    } else if !continue_on_error {
                        failed = true;
                        break;
                    }
                }
            }
            if i % 100 == 0 {
                progress(stats.done + stats.errors, stats.total);
            }
        }
    } else {
        for chunk in statements.chunks(200) {
            if cancelled() {
                let _ = dst.client.batch_execute("ROLLBACK").await;
                return Err(canceled());
            }
            if let Err(e) = dst.client.batch_execute(&chunk.join("\n")).await {
                log(format!("[ERR] {}", map_error(&e).message));
                stats.errors += 1;
                failed = true;
                break;
            }
            stats.done += chunk.len() as u64;
            progress(stats.done, stats.total);
        }
    }
    if transaction {
        if failed {
            let _ = dst.client.batch_execute("ROLLBACK").await;
            stats.rolled_back = true;
            stats.done = 0;
            log("--Transaction - Rollback--".into());
        } else {
            dst.client.batch_execute("COMMIT").await.map_err(sql_err)?;
        }
    }
    log(format!("Executed Query: {}", stats.done));
    if transaction && !failed {
        log("--Transaction - Commit--".into());
    }
    progress(stats.done + stats.errors, stats.total);
    log(format!("--End-- ({:.2} s)", started.elapsed().as_secs_f64()));
    Ok(stats)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn floats_compare_by_value_not_by_text() {
        assert!(same_value(&Some("0.10000000000000001".into()), &Some("0.1".into()), Kind::F64));
        assert!(same_value(&Some("0.100000001".into()), &Some("0.1".into()), Kind::F32));
        assert!(!same_value(&Some("0.1".into()), &Some("0.2".into()), Kind::F64));
        assert!(!same_value(&Some("1.50".into()), &Some("1.5".into()), Kind::Number));
        assert!(same_value(&None, &None, Kind::Text) && !same_value(&None, &Some(String::new()), Kind::Text));
    }

    #[test]
    fn literals_follow_the_target_type() {
        assert_eq!(literal(&Some("12".into()), Kind::Number), "12");
        assert_eq!(literal(&Some("-1.50".into()), Kind::Number), "-1.50");
        assert_eq!(literal(&Some("1e5".into()), Kind::Number), "'1e5'");
        assert_eq!(literal(&Some("t".into()), Kind::Bool), "true");
        assert_eq!(literal(&Some("O'Brien".into()), Kind::Text), "'O''Brien'");
        assert_eq!(literal(&Some("NaN".into()), Kind::F64), "'NaN'");
        assert_eq!(literal(&None, Kind::Text), "NULL");
    }
}
