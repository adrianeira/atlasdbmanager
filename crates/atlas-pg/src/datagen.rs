//! Data Generation: preenche tabelas com dados de teste.
//!
//! `describe` lê as tabelas do schema e sugere um gerador para cada coluna (pelo tipo, pela chave e pelo padrão da coluna);
//! `preview` gera algumas linhas sem gravar; `run` gera e insere, tabela a tabela. Os valores são sempre texto entre aspas
//! (ou `NULL`): o servidor converte para o tipo da coluna, como no Data Synchronization.

use crate::connect::PgSession;
use crate::error::map_error;
use crate::exec::quote_ident;
use atlas_core::{DatabaseError, ErrorKind};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TextStyle {
    /// Frases em latim, como num texto de exemplo.
    Sentence,
    Words,
    Letters,
}

/// Como se gera o valor de uma coluna.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum Generator {
    /// Números em sequência. Sem `start`, continua do maior valor que a coluna já tem (mais 1).
    Sequence { start: Option<i64>, step: i64 },
    Integer { min: i64, max: i64 },
    Decimal { min: f64, max: f64, scale: u32 },
    Text { min_len: u32, max_len: u32, style: TextStyle },
    /// `AAAA-MM-DD hh:mm:ss`.
    DateTime { from: String, to: String },
    /// `AAAA-MM-DD`.
    Date { from: String, to: String },
    Boolean { true_percent: u32 },
    Uuid,
    List { values: Vec<String> },
    Fixed { value: String },
    Null,
    /// Um valor que já existe em outra tabela (chave estrangeira).
    Reference { table: String, column: String },
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ColumnMeta {
    pub name: String,
    pub data_type: String,
    pub not_null: bool,
    pub is_pk: bool,
    /// Coluna gerada pelo servidor (não se insere).
    pub generated: bool,
    pub char_len: Option<u32>,
    pub suggested: Generator,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TableMeta {
    pub name: String,
    pub columns: Vec<ColumnMeta>,
    /// Tabelas das chaves estrangeiras (sem a própria).
    pub depends_on: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ColumnSpec {
    pub name: String,
    pub generator: Generator,
    /// Porcentagem de linhas com `NULL` (só faz sentido em coluna que aceita nulo).
    #[serde(default)]
    pub null_percent: u32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TableSpec {
    pub name: String,
    pub rows: u64,
    pub columns: Vec<ColumnSpec>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GenOptions {
    pub continue_on_error: bool,
    pub empty_first: bool,
    pub transaction: bool,
    pub multi_insert: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GenSpec {
    pub schema: String,
    /// Na ordem em que se geram.
    pub tables: Vec<TableSpec>,
    pub options: GenOptions,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GenStats {
    pub objects: u64,
    pub generated: u64,
    pub inserted: u64,
    pub errors: u64,
}

pub struct GenHooks<'a> {
    pub log: &'a mut (dyn FnMut(String) + Send),
    pub progress: &'a mut (dyn FnMut(&GenStats) + Send),
    pub cancelled: &'a (dyn Fn() -> bool + Sync),
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Preview {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Option<String>>>,
}

fn sql_err(e: tokio_postgres::Error) -> DatabaseError {
    map_error(&e)
}

fn invalid(message: impl Into<String>) -> DatabaseError {
    DatabaseError::new(ErrorKind::InvalidInput, message)
}

// ---------------------------------------------------------------- números aleatórios e datas

/// xorshift64*: sem dependência externa e suficiente para dados de teste.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Rng(seed | 1)
    }

    pub fn from_clock() -> Self {
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(0x9E37_79B9_7F4A_7C15);
        Rng::new(nanos ^ 0x9E37_79B9_7F4A_7C15)
    }

    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// Inteiro em `[min, max]`.
    fn range(&mut self, min: i64, max: i64) -> i64 {
        if max <= min {
            return min;
        }
        let span = (max as i128 - min as i128 + 1) as u128;
        (min as i128 + (self.next() as u128 % span) as i128) as i64
    }

    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
}

/// Dias desde 1970-01-01 para a data civil (algoritmo de Howard Hinnant).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// `AAAA-MM-DD[ hh:mm:ss]` em segundos desde 1970-01-01.
fn parse_stamp(text: &str) -> Option<i64> {
    let (date, time) = text.trim().split_once([' ', 'T']).unwrap_or((text.trim(), "00:00:00"));
    let mut d = date.split('-').map(|p| p.parse::<i64>());
    let (y, m, day) = (d.next()?.ok()?, d.next()?.ok()?, d.next()?.ok()?);
    if !(1..=12).contains(&m) || !(1..=31).contains(&day) {
        return None;
    }
    let mut t = time.split(':').map(|p| p.split('.').next().unwrap_or("0").parse::<i64>());
    let (h, mi, s) = (t.next().unwrap_or(Ok(0)).ok()?, t.next().unwrap_or(Ok(0)).ok()?, t.next().unwrap_or(Ok(0)).ok()?);
    Some(days_from_civil(y, m, day) * 86_400 + h * 3600 + mi * 60 + s)
}

pub fn format_stamp(secs: i64, with_time: bool) -> String {
    let (y, m, d) = civil_from_days(secs.div_euclid(86_400));
    if !with_time {
        return format!("{y:04}-{m:02}-{d:02}");
    }
    let rest = secs.rem_euclid(86_400);
    format!("{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02}", rest / 3600, rest % 3600 / 60, rest % 60)
}

pub fn today() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0).div_euclid(86_400) * 86_400
}

// ---------------------------------------------------------------- geradores

const WORDS: &[&str] = &[
    "lorem", "ipsum", "dolor", "sit", "amet", "consectetur", "adipiscing", "elit", "sed", "do", "eiusmod", "tempor", "incididunt", "ut", "labore", "et",
    "dolore", "magna", "aliqua", "enim", "ad", "minim", "veniam", "quis", "nostrud", "exercitation", "ullamco", "laboris", "nisi", "aliquip", "ex", "ea",
    "commodo", "consequat", "duis", "aute", "irure", "in", "reprehenderit", "voluptate", "velit", "esse", "cillum", "eu", "fugiat", "nulla", "pariatur",
    "excepteur", "sint", "occaecat", "cupidatat", "non", "proident", "sunt", "culpa", "qui", "officia", "deserunt", "mollit", "anim", "id", "est", "laborum",
];

/// Texto com tamanho (em caracteres) entre `min` e `max`.
fn text_value(rng: &mut Rng, min: u32, max: u32, style: TextStyle) -> String {
    let max = max.max(1) as usize;
    let min = (min as usize).min(max);
    let target = rng.range(min as i64, max as i64) as usize;
    let mut out = String::new();
    match style {
        TextStyle::Letters => {
            for _ in 0..target {
                out.push((b'a' + rng.range(0, 25) as u8) as char);
            }
        }
        TextStyle::Words | TextStyle::Sentence => {
            let mut start_of_sentence = true;
            while out.len() < target {
                let word = WORDS[rng.range(0, WORDS.len() as i64 - 1) as usize];
                let next = if out.is_empty() { word.to_string() } else { format!(" {word}") };
                if out.len() + next.len() > target && !out.is_empty() {
                    break;
                }
                if style == TextStyle::Sentence && start_of_sentence {
                    let mut c = next.trim_start().chars();
                    let cap: String = c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default();
                    out.push_str(&next[..next.len() - next.trim_start().len()]);
                    out.push_str(&cap);
                    start_of_sentence = false;
                } else {
                    out.push_str(&next);
                }
                if style == TextStyle::Sentence && rng.range(0, 7) == 0 && out.len() + 1 < target {
                    out.push('.');
                    start_of_sentence = true;
                }
            }
            if style == TextStyle::Sentence && out.len() < target && !out.ends_with('.') {
                out.push('.');
            }
        }
    }
    out.chars().take(max).collect()
}

fn uuid_value(rng: &mut Rng) -> String {
    let (a, b) = (rng.next(), rng.next());
    let hex = format!("{a:016x}{b:016x}");
    // Versão 4 e variante RFC 4122.
    format!("{}-{}-4{}-{:x}{}-{}", &hex[0..8], &hex[8..12], &hex[13..16], 8 + (u8::from_str_radix(&hex[16..17], 16).unwrap_or(0) & 3), &hex[17..20], &hex[20..32])
}

/// Estado de uma coluna durante a geração: o próximo número da sequência e os valores que a referência pode escolher.
#[derive(Default)]
pub struct ColumnState {
    pub next: i64,
    pub pool: Vec<String>,
}

pub fn value_for(g: &Generator, rng: &mut Rng, st: &mut ColumnState) -> Option<String> {
    match g {
        Generator::Sequence { step, .. } => {
            let v = st.next;
            st.next += step;
            Some(v.to_string())
        }
        Generator::Integer { min, max } => Some(rng.range(*min, *max).to_string()),
        Generator::Decimal { min, max, scale } => {
            let v = min + rng.unit() * (max - min);
            Some(format!("{v:.*}", *scale as usize))
        }
        Generator::Text { min_len, max_len, style } => Some(text_value(rng, *min_len, *max_len, *style)),
        Generator::DateTime { from, to } | Generator::Date { from, to } => {
            let (a, b) = (parse_stamp(from)?, parse_stamp(to)?);
            let with_time = matches!(g, Generator::DateTime { .. });
            let secs = rng.range(a.min(b), a.max(b));
            Some(format_stamp(secs, with_time))
        }
        Generator::Boolean { true_percent } => Some(if rng.range(1, 100) <= *true_percent as i64 { "true" } else { "false" }.to_string()),
        Generator::Uuid => Some(uuid_value(rng)),
        Generator::List { values } => (!values.is_empty()).then(|| values[rng.range(0, values.len() as i64 - 1) as usize].clone()),
        Generator::Fixed { value } => Some(value.clone()),
        Generator::Null => None,
        Generator::Reference { .. } => (!st.pool.is_empty()).then(|| st.pool[rng.range(0, st.pool.len() as i64 - 1) as usize].clone()),
    }
}

pub fn check(table: &TableSpec) -> Result<(), DatabaseError> {
    for c in &table.columns {
        match &c.generator {
            Generator::Integer { min, max } if min > max => return Err(invalid(format!("{}.{}: the minimum is greater than the maximum", table.name, c.name))),
            Generator::Decimal { min, max, .. } if min > max => return Err(invalid(format!("{}.{}: the minimum is greater than the maximum", table.name, c.name))),
            Generator::DateTime { from, to } | Generator::Date { from, to } if parse_stamp(from).is_none() || parse_stamp(to).is_none() => {
                return Err(invalid(format!("{}.{}: invalid date (use YYYY-MM-DD)", table.name, c.name)));
            }
            Generator::List { values } if values.is_empty() => return Err(invalid(format!("{}.{}: the list has no values", table.name, c.name))),
            _ => {}
        }
    }
    Ok(())
}

/// Lê o que os geradores precisam do banco: onde a sequência continua e os valores que as referências sorteiam.
async fn prepare(s: &PgSession, schema: &str, table: &TableSpec) -> Result<Vec<ColumnState>, DatabaseError> {
    check(table)?;
    let mut out = Vec::new();
    for c in &table.columns {
        let mut st = ColumnState::default();
        match &c.generator {
            Generator::Sequence { start: Some(v), .. } => st.next = *v,
            Generator::Sequence { start: None, .. } => {
                let sql = format!("SELECT COALESCE(MAX({})::bigint, 0) + 1 FROM {}.{}", quote_ident(&c.name), quote_ident(schema), quote_ident(&table.name));
                let row = s.client.query_one(&sql, &[]).await.map_err(sql_err)?;
                st.next = row.get::<_, i64>(0);
            }
            Generator::Reference { table: t, column } => {
                let sql = format!("SELECT DISTINCT {col}::text FROM {}.{} WHERE {col} IS NOT NULL LIMIT 10000", quote_ident(schema), quote_ident(t), col = quote_ident(column));
                st.pool = s.client.query(&sql, &[]).await.map_err(sql_err)?.iter().map(|r| r.get::<_, String>(0)).collect();
            }
            _ => {}
        }
        out.push(st);
    }
    Ok(out)
}

pub fn make_row(table: &TableSpec, states: &mut [ColumnState], rng: &mut Rng) -> Vec<Option<String>> {
    table
        .columns
        .iter()
        .zip(states.iter_mut())
        .map(|(c, st)| {
            if c.null_percent > 0 && rng.range(1, 100) <= c.null_percent as i64 {
                // A sequência avança mesmo assim, para os números não repetirem.
                if matches!(c.generator, Generator::Sequence { .. }) {
                    return value_for(&c.generator, rng, st);
                }
                return None;
            }
            value_for(&c.generator, rng, st)
        })
        .collect()
}

fn literal(v: &Option<String>) -> String {
    match v {
        Some(t) => format!("'{}'", t.replace('\'', "''")),
        None => "NULL".to_string(),
    }
}

pub async fn preview(s: &PgSession, schema: &str, table: &TableSpec, rows: u64) -> Result<Preview, DatabaseError> {
    let mut states = prepare(s, schema, table).await?;
    let mut rng = Rng::from_clock();
    Ok(Preview { columns: table.columns.iter().map(|c| c.name.clone()).collect(), rows: (0..rows).map(|_| make_row(table, &mut states, &mut rng)).collect() })
}

// ---------------------------------------------------------------- descrição das tabelas

#[allow(clippy::too_many_arguments)]
fn suggest(c: &ColumnMeta, kind: &str, default_expr: Option<&str>, identity: &str, reference: Option<(String, String)>, precision: Option<u32>, scale: Option<u32>, labels: &[String]) -> Generator {
    if !identity.is_empty() || default_expr.is_some_and(|d| d.starts_with("nextval(")) {
        return Generator::Sequence { start: None, step: 1 };
    }
    if let Some((table, column)) = reference {
        return Generator::Reference { table, column };
    }
    let now = format_stamp(today(), false);
    match kind {
        "int" => {
            if c.is_pk {
                return Generator::Sequence { start: None, step: 1 };
            }
            let max = if c.data_type == "smallint" { 32_767 } else { 1_000_000 };
            Generator::Integer { min: 0, max }
        }
        "numeric" => {
            let scale = scale.unwrap_or(2).min(6);
            let digits = precision.map(|p| p.saturating_sub(scale.min(p))).unwrap_or(6).clamp(1, 6);
            Generator::Decimal { min: 0.0, max: 10f64.powi(digits as i32) - 1.0, scale }
        }
        "text" => {
            let max = c.char_len.map(|n| n.min(100)).unwrap_or(100);
            Generator::Text { min_len: max.min(10), max_len: max, style: if max >= 20 { TextStyle::Sentence } else { TextStyle::Words } }
        }
        "bool" => Generator::Boolean { true_percent: 50 },
        "date" => Generator::Date { from: "2000-01-01".into(), to: now },
        "timestamp" => Generator::DateTime { from: "2000-01-01 00:00:00".into(), to: format!("{now} 23:59:59") },
        "uuid" => Generator::Uuid,
        "json" => Generator::Fixed { value: "{}".into() },
        "enum" if !labels.is_empty() => Generator::List { values: labels.to_vec() },
        _ if !c.not_null => Generator::Null,
        _ => Generator::Fixed { value: String::new() },
    }
}

/// Tabelas do schema com as colunas e um gerador sugerido para cada uma; as tabelas vêm com as referenciadas antes.
pub async fn describe(s: &PgSession, schema: &str) -> Result<Vec<TableMeta>, DatabaseError> {
    let v = s.server.version_num;
    let identity = if v >= 100000 { "a.attidentity::text" } else { "''::text" };
    let generated = if v >= 120000 { "a.attgenerated <> ''" } else { "false" };
    let kinds = if v >= 100000 { "'r','p'" } else { "'r'" };
    let rows = s
        .client
        .query(
            &format!(
                "SELECT c.oid, c.relname::text, a.attnum::int, a.attname::text, format_type(a.atttypid, a.atttypmod), a.attnotnull, pg_get_expr(d.adbin, d.adrelid), \
                        {identity}, {generated}, \
                        CASE WHEN bt.typtype = 'e' THEN 'enum' \
                             WHEN bt.typname IN ('int2','int4','int8') THEN 'int' \
                             WHEN bt.typname IN ('numeric','float4','float8','money') THEN 'numeric' \
                             WHEN bt.typname IN ('text','varchar','bpchar','name','citext') THEN 'text' \
                             WHEN bt.typname = 'bool' THEN 'bool' \
                             WHEN bt.typname = 'date' THEN 'date' \
                             WHEN bt.typname IN ('timestamp','timestamptz') THEN 'timestamp' \
                             WHEN bt.typname = 'uuid' THEN 'uuid' \
                             WHEN bt.typname IN ('json','jsonb') THEN 'json' \
                             ELSE 'other' END, \
                        CASE WHEN bt.typname IN ('varchar','bpchar') AND a.atttypmod > 4 THEN a.atttypmod - 4 END, \
                        CASE WHEN bt.typname = 'numeric' AND a.atttypmod > 4 THEN ((a.atttypmod - 4) >> 16) & 65535 END, \
                        CASE WHEN bt.typname = 'numeric' AND a.atttypmod > 4 THEN (a.atttypmod - 4) & 65535 END, \
                        ARRAY(SELECT e.enumlabel::text FROM pg_catalog.pg_enum e WHERE e.enumtypid = bt.oid ORDER BY e.enumsortorder), \
                        EXISTS (SELECT 1 FROM pg_catalog.pg_index i WHERE i.indrelid = c.oid AND i.indisprimary AND a.attnum = ANY(i.indkey)) \
                   FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
                   JOIN pg_catalog.pg_attribute a ON a.attrelid = c.oid AND a.attnum > 0 AND NOT a.attisdropped \
                   JOIN pg_catalog.pg_type t ON t.oid = a.atttypid \
                   JOIN pg_catalog.pg_type bt ON bt.oid = CASE WHEN t.typtype = 'd' THEN t.typbasetype ELSE t.oid END \
                   LEFT JOIN pg_catalog.pg_attrdef d ON d.adrelid = c.oid AND d.adnum = a.attnum \
                  WHERE n.nspname = $1 AND c.relkind IN ({kinds}) \
                    AND NOT EXISTS (SELECT 1 FROM pg_catalog.pg_depend ext WHERE ext.classid = 'pg_catalog.pg_class'::regclass AND ext.objid = c.oid AND ext.deptype = 'e') \
                  ORDER BY c.relname, a.attnum"
            ),
            &[&schema],
        )
        .await
        .map_err(sql_err)?;
    // Chaves estrangeiras: tabela e coluna referenciadas, por (tabela, coluna).
    let fks = s
        .client
        .query(
            "SELECT c.relname::text, a.attname::text, rc.relname::text, ra.attname::text \
               FROM pg_catalog.pg_constraint con \
               JOIN pg_catalog.pg_class c ON c.oid = con.conrelid JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
               JOIN pg_catalog.pg_class rc ON rc.oid = con.confrelid \
               JOIN pg_catalog.pg_namespace rn ON rn.oid = rc.relnamespace \
               CROSS JOIN LATERAL unnest(con.conkey, con.confkey) AS k(attnum, refattnum) \
               JOIN pg_catalog.pg_attribute a ON a.attrelid = con.conrelid AND a.attnum = k.attnum \
               JOIN pg_catalog.pg_attribute ra ON ra.attrelid = con.confrelid AND ra.attnum = k.refattnum \
              WHERE con.contype = 'f' AND n.nspname = $1 AND rn.nspname = $1",
            &[&schema],
        )
        .await
        .map_err(sql_err)?;
    let mut refs: HashMap<(String, String), (String, String)> = HashMap::new();
    let mut deps: HashMap<String, HashSet<String>> = HashMap::new();
    for r in fks {
        let (t, col, rt, rc): (String, String, String, String) = (r.get(0), r.get(1), r.get(2), r.get(3));
        if t != rt {
            deps.entry(t.clone()).or_default().insert(rt.clone());
        }
        refs.insert((t, col), (rt, rc));
    }
    let mut tables: Vec<TableMeta> = Vec::new();
    for r in rows {
        let table: String = r.get(1);
        if tables.last().is_none_or(|t| t.name != table) {
            let mut depends_on: Vec<String> = deps.get(&table).map(|d| d.iter().cloned().collect()).unwrap_or_default();
            depends_on.sort();
            tables.push(TableMeta { name: table.clone(), columns: Vec::new(), depends_on });
        }
        let name: String = r.get(3);
        let default_expr: Option<String> = r.get(6);
        let identity: String = r.get(7);
        let kind: String = r.get(9);
        let (precision, scale): (Option<i32>, Option<i32>) = (r.get(11), r.get(12));
        let labels: Vec<String> = r.get(13);
        let mut c = ColumnMeta {
            name: name.clone(),
            data_type: r.get(4),
            not_null: r.get(5),
            is_pk: r.get(14),
            generated: r.get(8),
            char_len: r.get::<_, Option<i32>>(10).map(|n| n as u32),
            suggested: Generator::Null,
        };
        c.suggested = suggest(&c, &kind, default_expr.as_deref(), &identity, refs.get(&(table, name)).cloned(), precision.map(|p| p as u32), scale.map(|p| p as u32), &labels);
        tables.last_mut().expect("table").columns.push(c);
    }
    Ok(order_by_dependency(tables))
}

/// Tabelas referenciadas antes das que referenciam (ciclos ficam no fim, na ordem alfabética).
pub fn order_by_dependency(tables: Vec<TableMeta>) -> Vec<TableMeta> {
    let known: HashSet<String> = tables.iter().map(|t| t.name.clone()).collect();
    let mut placed: HashSet<String> = HashSet::new();
    let mut out: Vec<TableMeta> = Vec::new();
    let mut rest = tables;
    while !rest.is_empty() {
        let (ready, waiting): (Vec<TableMeta>, Vec<TableMeta>) = rest.into_iter().partition(|t| t.depends_on.iter().all(|d| placed.contains(d) || !known.contains(d)));
        if ready.is_empty() {
            out.extend(waiting);
            break;
        }
        placed.extend(ready.iter().map(|t| t.name.clone()));
        out.extend(ready);
        rest = waiting;
    }
    out
}

// ---------------------------------------------------------------- execução

const BATCH: u64 = 500;

pub async fn run(s: &PgSession, spec: &GenSpec, hooks: GenHooks<'_>) -> Result<GenStats, DatabaseError> {
    let GenHooks { log, progress, cancelled } = hooks;
    let opts = &spec.options;
    let mut stats = GenStats::default();
    let mut rng = Rng::from_clock();
    log("[DGR] Data Generation started".into());
    s.client.batch_execute("SET TimeZone = 'UTC'").await.map_err(sql_err)?;
    let abort = |stats: &GenStats, log: &mut (dyn FnMut(String) + Send)| {
        log("[DGR] Finished with error".into());
        Ok(stats.clone())
    };

    if opts.empty_first {
        for (i, t) in spec.tables.iter().enumerate().rev() {
            log(format!("[DGR] {}> {}: Emptying table", i + 1, t.name));
            if let Err(e) = s.client.batch_execute(&format!("DELETE FROM {}.{}", quote_ident(&spec.schema), quote_ident(&t.name))).await {
                let err = map_error(&e);
                log(format!("[DGR] {}> {}: ERROR: {}", i + 1, t.name, err.message));
                stats.errors += 1;
                progress(&stats);
                return abort(&stats, log);
            }
        }
    }

    for (i, t) in spec.tables.iter().enumerate() {
        let tag = format!("[DGR] {}> {}", i + 1, t.name);
        if cancelled() {
            return Err(DatabaseError::new(ErrorKind::Canceled, "data generation canceled"));
        }
        if opts.transaction {
            s.client.batch_execute("BEGIN").await.map_err(sql_err)?;
            log(format!("{tag}: --Transaction - Begin--"));
        }
        log(format!("{tag}: Preparing field"));
        let mut states = match prepare(s, &spec.schema, t).await {
            Ok(v) => v,
            Err(e) => {
                log(format!("{tag}: ERROR: {}", e.message));
                stats.errors += 1;
                progress(&stats);
                if opts.transaction {
                    let _ = s.client.batch_execute("ROLLBACK").await;
                    log(format!("{tag}: --Transaction - Rollback--"));
                }
                if opts.continue_on_error {
                    continue;
                }
                return abort(&stats, log);
            }
        };
        let override_identity = if s.server.version_num >= 100000 {
            let sql = "SELECT 1 FROM pg_catalog.pg_attribute a JOIN pg_catalog.pg_class c ON c.oid = a.attrelid JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
                        WHERE n.nspname = $1 AND c.relname = $2 AND a.attidentity = 'a' AND a.attname = ANY($3)";
            let names: Vec<&str> = t.columns.iter().map(|c| c.name.as_str()).collect();
            !s.client.query(sql, &[&spec.schema, &t.name, &names]).await.map_err(sql_err)?.is_empty()
        } else {
            false
        };
        log(format!("{tag}: Start records generation"));
        let target = format!("{}.{}", quote_ident(&spec.schema), quote_ident(&t.name));
        let columns = t.columns.iter().map(|c| quote_ident(&c.name)).collect::<Vec<_>>().join(", ");
        let overriding = if override_identity { " OVERRIDING SYSTEM VALUE" } else { "" };
        let per_batch = if opts.multi_insert { BATCH } else { 1 };
        let mut done: u64 = 0;
        let mut failed = false;
        while done < t.rows {
            if cancelled() {
                if opts.transaction {
                    let _ = s.client.batch_execute("ROLLBACK").await;
                }
                return Err(DatabaseError::new(ErrorKind::Canceled, "data generation canceled"));
            }
            let n = per_batch.min(t.rows - done);
            let rows: Vec<String> = (0..n).map(|_| format!("({})", make_row(t, &mut states, &mut rng).iter().map(literal).collect::<Vec<_>>().join(", "))).collect();
            stats.generated += n;
            let sql = if t.columns.is_empty() {
                format!("INSERT INTO {target} DEFAULT VALUES")
            } else {
                format!("INSERT INTO {target} ({columns}){overriding} VALUES {}", rows.join(", "))
            };
            let savepoint = opts.transaction && opts.continue_on_error;
            if savepoint {
                s.client.batch_execute("SAVEPOINT atlas_dg").await.map_err(sql_err)?;
            }
            match s.client.batch_execute(&sql).await {
                Ok(()) => {
                    stats.inserted += n;
                    if savepoint {
                        s.client.batch_execute("RELEASE SAVEPOINT atlas_dg").await.map_err(sql_err)?;
                    }
                }
                Err(e) => {
                    let err = map_error(&e);
                    log(format!("{tag}: ERROR: {}{}", err.message, err.detail.as_ref().map(|d| format!("DETAIL: {d}")).unwrap_or_default()));
                    stats.errors += 1;
                    if savepoint {
                        s.client.batch_execute("ROLLBACK TO SAVEPOINT atlas_dg").await.map_err(sql_err)?;
                    } else if !opts.continue_on_error {
                        failed = true;
                    }
                }
            }
            done += n;
            progress(&stats);
            if failed {
                break;
            }
        }
        if !failed {
            log(format!("{tag}: Records generated"));
            // As colunas serial seguem de onde a geração parou.
            for c in t.columns.iter().filter(|c| matches!(c.generator, Generator::Sequence { .. })) {
                let seq = s
                    .client
                    .query_one("SELECT pg_get_serial_sequence($1, $2)", &[&target, &c.name])
                    .await
                    .ok()
                    .and_then(|r| r.get::<_, Option<String>>(0));
                if let Some(seq) = seq {
                    let sql = format!("SELECT setval('{}', GREATEST(COALESCE((SELECT MAX({}) FROM {target}), 1), 1))", seq.replace('\'', "''"), quote_ident(&c.name));
                    if let Err(e) = s.client.batch_execute(&sql).await {
                        log(format!("{tag}: ERROR: {}", map_error(&e).message));
                    }
                }
            }
        }
        if opts.transaction {
            if failed {
                let _ = s.client.batch_execute("ROLLBACK").await;
                log(format!("{tag}: --Transaction - Rollback--"));
            } else {
                match s.client.batch_execute("COMMIT").await {
                    Ok(()) => log(format!("{tag}: --Transaction - Commit--")),
                    Err(e) => {
                        log(format!("{tag}: ERROR: {}", map_error(&e).message));
                        stats.errors += 1;
                        failed = true;
                    }
                }
            }
        }
        if failed {
            progress(&stats);
            return abort(&stats, log);
        }
        stats.objects += 1;
        progress(&stats);
    }
    log(if stats.errors > 0 { "[DGR] Finished with errors" } else { "[DGR] Finished successfully" }.into());
    Ok(stats)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_dates_round_trip() {
        for secs in [0, 86_399, 946_684_800, 1_700_000_000, -86_400, 4_102_444_800] {
            assert_eq!(parse_stamp(&format_stamp(secs, true)), Some(secs));
        }
        assert_eq!(format_stamp(946_684_800, true), "2000-01-01 00:00:00");
        assert_eq!(parse_stamp("2000-02-29"), Some(951_782_400));
        assert_eq!(parse_stamp("2000-13-01"), None);
        assert_eq!(parse_stamp("nope"), None);
    }

    #[test]
    fn generators_respect_their_ranges() {
        let mut rng = Rng::new(42);
        let mut st = ColumnState::default();
        for _ in 0..200 {
            let n: i64 = value_for(&Generator::Integer { min: 5, max: 9 }, &mut rng, &mut st).unwrap().parse().unwrap();
            assert!((5..=9).contains(&n));
            let d: f64 = value_for(&Generator::Decimal { min: 1.0, max: 2.0, scale: 3 }, &mut rng, &mut st).unwrap().parse().unwrap();
            assert!((1.0..=2.0).contains(&d));
            let t = value_for(&Generator::Text { min_len: 3, max_len: 12, style: TextStyle::Sentence }, &mut rng, &mut st).unwrap();
            assert!(t.chars().count() <= 12 && !t.is_empty(), "{t}");
            let when = value_for(&Generator::DateTime { from: "2020-01-01 00:00:00".into(), to: "2020-12-31 23:59:59".into() }, &mut rng, &mut st).unwrap();
            assert!(when.starts_with("2020-"), "{when}");
        }
        let u = value_for(&Generator::Uuid, &mut rng, &mut st).unwrap();
        assert_eq!(u.len(), 36);
        assert_eq!(u.as_bytes()[14], b'4');
    }

    #[test]
    fn sequences_advance_and_lists_and_nulls_work() {
        let mut rng = Rng::new(7);
        let mut st = ColumnState { next: 10, pool: vec![] };
        let seq = Generator::Sequence { start: None, step: 5 };
        assert_eq!(value_for(&seq, &mut rng, &mut st).as_deref(), Some("10"));
        assert_eq!(value_for(&seq, &mut rng, &mut st).as_deref(), Some("15"));
        assert!(value_for(&Generator::Null, &mut rng, &mut st).is_none());
        let list = Generator::List { values: vec!["a".into(), "b".into()] };
        for _ in 0..20 {
            assert!(matches!(value_for(&list, &mut rng, &mut st).as_deref(), Some("a" | "b")));
        }
        // Referência sem valores não gera nada.
        assert!(value_for(&Generator::Reference { table: "t".into(), column: "c".into() }, &mut rng, &mut st).is_none());
        assert_eq!(literal(&Some("it's".into())), "'it''s'");
        assert_eq!(literal(&None), "NULL");
    }

    #[test]
    fn tables_are_ordered_with_referenced_ones_first() {
        let t = |name: &str, deps: &[&str]| TableMeta { name: name.into(), columns: vec![], depends_on: deps.iter().map(|d| d.to_string()).collect() };
        let ordered = order_by_dependency(vec![t("items", &["orders"]), t("orders", &["clients"]), t("clients", &[]), t("loose", &[])]);
        let names: Vec<&str> = ordered.iter().map(|t| t.name.as_str()).collect();
        let pos = |n: &str| names.iter().position(|x| *x == n).unwrap();
        assert!(pos("clients") < pos("orders") && pos("orders") < pos("items"), "{names:?}");
        // Ciclo: ninguém fica de fora.
        assert_eq!(order_by_dependency(vec![t("a", &["b"]), t("b", &["a"])]).len(), 2);
    }
}
