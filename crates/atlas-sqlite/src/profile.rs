//! Data Profiling do SQLite: estatísticas de cada coluna (nulos, distintos, únicos, menor e maior valor, valores mais frequentes),
//! no mesmo formato do PostgreSQL.

use super::{execute_on, quote_ident, select_sql};
use atlas_core::{DatabaseError, OrderBy, QueryOutcome, TableFilter, TableProfile};
use rusqlite::Connection;
use serde::Serialize;

/// Quantos valores mais frequentes cada coluna devolve (para o gráfico de distribuição).
const TOP_VALUES: usize = 100;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ValueCount {
    /// `None` = NULL.
    pub value: Option<String>,
    pub count: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ColumnProfile {
    pub name: String,
    pub rows: u64,
    pub nulls: u64,
    pub distinct: Option<u64>,
    pub unique: Option<u64>,
    pub min: Option<String>,
    pub max: Option<String>,
    pub top: Vec<ValueCount>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DataProfile {
    pub rows: u64,
    pub columns: Vec<ColumnProfile>,
}

fn cell(out: &QueryOutcome, row: usize, col: usize) -> Option<String> {
    out.statements.first()?.result.as_ref()?.rows.get(row)?.get(col)?.clone()
}

fn num(out: &QueryOutcome, col: usize) -> Option<u64> {
    cell(out, 0, col)?.parse().ok()
}

/// Perfil das `columns` dos registros que o filtro, a ordenação e o limite escolhem (sem limite = todos).
/// Cada medida é uma consulta própria: uma que falhar só deixa aquela medida vazia.
pub(crate) fn data_profile_on(
    conn: &Connection,
    table: &str,
    columns: &[String],
    order_by: &[OrderBy],
    filter: Option<&TableFilter>,
    limit: Option<u64>,
) -> Result<DataProfile, DatabaseError> {
    let mut source = select_sql(table, order_by, filter);
    if let Some(n) = limit {
        source.push_str(&format!(" LIMIT {n}"));
    }
    let mut out = Vec::with_capacity(columns.len());
    let mut total = 0;
    for name in columns {
        let q = quote_ident(name);
        let with = format!("WITH src AS ({source})");
        let counts = execute_on(conn, &format!("{with} SELECT count(*), count({q}) FROM src"), 1)?;
        let rows = num(&counts, 0).unwrap_or(0);
        let not_null = num(&counts, 1).unwrap_or(0);
        total = rows;

        let distinct_sql = format!("{with} SELECT count(DISTINCT {q}), (SELECT count(*) FROM (SELECT {q} FROM src WHERE {q} IS NOT NULL GROUP BY {q} HAVING count(*) = 1)) FROM src");
        let (distinct, unique) = match execute_on(conn, &distinct_sql, 1) {
            Ok(o) => (num(&o, 0), num(&o, 1)),
            Err(_) => (None, None),
        };
        let (min, max) = match execute_on(conn, &format!("{with} SELECT min({q}), max({q}) FROM src"), 1) {
            Ok(o) => (cell(&o, 0, 0), cell(&o, 0, 1)),
            Err(_) => (None, None),
        };
        let top_sql = format!("{with} SELECT {q}, count(*) FROM src GROUP BY {q} ORDER BY count(*) DESC, {q} LIMIT {TOP_VALUES}");
        let top = match execute_on(conn, &top_sql, TOP_VALUES) {
            Ok(o) => o
                .statements
                .first()
                .and_then(|st| st.result.as_ref())
                .map(|rs| rs.rows.iter().map(|r| ValueCount { value: r.first().cloned().flatten(), count: r.get(1).and_then(|c| c.as_deref()).and_then(|c| c.parse().ok()).unwrap_or(0) }).collect())
                .unwrap_or_default(),
            Err(_) => Vec::new(),
        };
        out.push(ColumnProfile { name: name.clone(), rows, nulls: rows - not_null.min(rows), distinct, unique, min, max, top });
    }
    Ok(DataProfile { rows: total, columns: out })
}

/// Perfil da tabela: tamanhos (pela tabela virtual `dbstat`, quando o SQLite a tem), contagens da estrutura e total de linhas.
pub(crate) fn table_profile_on(conn: &Connection, table: &str) -> Result<TableProfile, DatabaseError> {
    let detail = super::table_detail_on(conn, table)?;
    let rows: i64 = conn.query_row(&format!("SELECT count(*) FROM {}", quote_ident(table)), [], |r| r.get(0)).map_err(super::map_err)?;
    // Tamanho em páginas da tabela e de cada índice; sem `dbstat` (compilação do SQLite sem ela) fica 0.
    let size_of = |name: &str| -> i64 { conn.query_row("SELECT coalesce(sum(pgsize), 0) FROM dbstat WHERE name = ?1", [name], |r| r.get(0)).unwrap_or(0) };
    let table_bytes = size_of(table);
    let own_indexes: Vec<&str> = detail.indexes.iter().filter(|i| !i.name.starts_with("sqlite_autoindex_")).map(|i| i.name.as_str()).collect();
    let index_bytes: i64 = detail.indexes.iter().map(|i| size_of(&i.name)).sum();
    let count_kind = |k: &str| detail.constraints.iter().filter(|c| c.kind == k).count() as i64;
    Ok(TableProfile {
        owner: String::new(),
        kind: "r".into(),
        persistence: "permanent".into(),
        tablespace: String::new(),
        estimated_rows: rows,
        table_bytes,
        index_bytes,
        total_bytes: table_bytes + index_bytes,
        columns: detail.columns.len() as i64,
        indexes: own_indexes.len() as i64,
        primary_keys: i64::from(detail.columns.iter().any(|c| c.key.is_some())),
        foreign_keys: count_kind("f"),
        uniques: count_kind("u"),
        checks: count_kind("c"),
        triggers: detail.triggers.len() as i64,
        row_security: false,
        replica_identity: "d".into(),
        live_tuples: Some(rows),
        dead_tuples: None,
        seq_scans: None,
        index_scans: None,
        last_vacuum: None,
        last_autovacuum: None,
        last_analyze: None,
        last_autoanalyze: None,
        comment: None,
    })
}
