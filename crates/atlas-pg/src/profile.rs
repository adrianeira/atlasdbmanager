//! Data Profiling: estatísticas de cada coluna de uma tabela (nulos, distintos, únicos, menor e maior valor, valores mais frequentes).

use crate::connect::PgSession;
use crate::exec::{execute, quote_ident, select_sql};
use atlas_core::{DatabaseError, OrderBy, TableFilter};
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
    /// Valores diferentes (sem contar NULL) e valores que aparecem uma única vez; `None` em tipos sem igualdade (ex.: `json`).
    pub distinct: Option<u64>,
    pub unique: Option<u64>,
    /// Menor e maior valor; `None` em tipos sem ordem (ex.: `boolean`, `json`).
    pub min: Option<String>,
    pub max: Option<String>,
    /// Os valores mais frequentes (o NULL entra como um valor), do mais ao menos frequente.
    pub top: Vec<ValueCount>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DataProfile {
    pub rows: u64,
    pub columns: Vec<ColumnProfile>,
}

fn cell(out: &atlas_core::QueryOutcome, row: usize, col: usize) -> Option<String> {
    out.statements.first()?.result.as_ref()?.rows.get(row)?.get(col)?.clone()
}

fn num(out: &atlas_core::QueryOutcome, col: usize) -> Option<u64> {
    cell(out, 0, col)?.parse().ok()
}

/// Perfil das `columns` dos registros que o filtro, a ordenação e o limite escolhem (sem limite = todos).
/// Cada medida de cada coluna é uma consulta própria: uma que o tipo não permite (menor valor de um `boolean`, distintos de um `json`)
/// só deixa aquela medida vazia.
pub async fn data_profile(
    s: &PgSession,
    schema: &str,
    table: &str,
    columns: &[String],
    order_by: &[OrderBy],
    filter: Option<&TableFilter>,
    limit: Option<u64>,
) -> Result<DataProfile, DatabaseError> {
    let mut source = select_sql(schema, table, &[], order_by, filter);
    if let Some(n) = limit {
        source.push_str(&format!(" LIMIT {n}"));
    }
    let mut out = Vec::with_capacity(columns.len());
    let mut total = 0;
    for name in columns {
        let q = quote_ident(name);
        // Tudo sobre a mesma fonte (um CTE é calculado uma vez no PostgreSQL 9.6). Os valores voltam como o servidor os escreve (como a grade os mostra: um boolean é `t`, não `true`).
        let with = format!("WITH src AS ({source})");
        let counts = execute(s, &format!("{with} SELECT count(*), count({q}) FROM src"), 1).await?;
        let rows = num(&counts, 0).unwrap_or(0);
        let not_null = num(&counts, 1).unwrap_or(0);
        total = rows;

        let distinct_sql = format!(
            "{with} SELECT count(DISTINCT {q}), (SELECT count(*) FROM (SELECT {q} FROM src WHERE {q} IS NOT NULL GROUP BY {q} HAVING count(*) = 1) u) FROM src"
        );
        let (distinct, unique) = match execute(s, &distinct_sql, 1).await {
            Ok(o) => (num(&o, 0), num(&o, 1)),
            Err(_) => (None, None),
        };
        let (min, max) = match execute(s, &format!("{with} SELECT min({q}), max({q}) FROM src"), 1).await {
            Ok(o) => (cell(&o, 0, 0), cell(&o, 0, 1)),
            Err(_) => (None, None),
        };
        let top_sql = format!("{with} SELECT {q}, count(*) FROM src GROUP BY {q} ORDER BY count(*) DESC, {q} LIMIT {TOP_VALUES}");
        let top = match execute(s, &top_sql, TOP_VALUES).await {
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
