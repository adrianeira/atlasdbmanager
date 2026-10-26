//! Data Profiling e Table Profile do MySQL, no mesmo formato do PostgreSQL.

use super::{MysqlSession, quote_ident};
use mysql_async::Value;
use crate::meta::{n, rows_of, s};
use crate::rows::select_sql;
use atlas_core::{DatabaseError, OrderBy, QueryOutcome, TableFilter, TableProfile};
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

fn val(s: &str) -> Value {
    Value::Bytes(s.as_bytes().to_vec())
}

impl MysqlSession {
    /// Perfil das `columns` dos registros que o filtro, a ordenação e o limite escolhem (sem limite = todos).
    /// Cada medida é uma consulta própria: uma que o tipo não permite (menor valor de `json`, `DISTINCT` de geometria) só deixa
    /// aquela medida vazia. No MySQL o schema é o próprio banco.
    #[allow(clippy::too_many_arguments)]
    pub async fn data_profile(
        &self,
        database: &str,
        table: &str,
        columns: &[String],
        order_by: &[OrderBy],
        filter: Option<&TableFilter>,
        limit: Option<u64>,
    ) -> Result<DataProfile, DatabaseError> {
        let page = limit.map(|l| (u32::try_from(l).unwrap_or(u32::MAX), 0));
        let source = select_sql(database, table, order_by, filter, page);
        let mut out = Vec::with_capacity(columns.len());
        let mut total = 0;
        for name in columns {
            let q = quote_ident(name);
            let src = format!("({source}) AS src");
            let counts = self.execute(database, &format!("SELECT COUNT(*), COUNT({q}) FROM {src}"), 1).await?;
            let rows = num(&counts, 0).unwrap_or(0);
            let not_null = num(&counts, 1).unwrap_or(0);
            total = rows;
            let distinct_sql = format!("SELECT COUNT(DISTINCT {q}), (SELECT COUNT(*) FROM (SELECT {q} AS v FROM {src} WHERE {q} IS NOT NULL GROUP BY {q} HAVING COUNT(*) = 1) u) FROM {src}");
            let (distinct, unique) = match self.execute(database, &distinct_sql, 1).await {
                Ok(o) => (num(&o, 0), num(&o, 1)),
                Err(_) => (None, None),
            };
            let (min, max) = match self.execute(database, &format!("SELECT MIN({q}), MAX({q}) FROM {src}"), 1).await {
                Ok(o) => (cell(&o, 0, 0), cell(&o, 0, 1)),
                Err(_) => (None, None),
            };
            let top_sql = format!("SELECT {q}, COUNT(*) FROM {src} GROUP BY {q} ORDER BY COUNT(*) DESC, {q} LIMIT {TOP_VALUES}");
            let top = match self.execute(database, &top_sql, TOP_VALUES).await {
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

    /// Perfil da tabela: tamanhos (estimados pelo catálogo), contagens da estrutura e total de linhas.
    pub async fn table_profile(&self, database: &str, table: &str) -> Result<TableProfile, DatabaseError> {
        let client = self.client(database).await?;
        let mut conn = client.lock().await;
        let args = || vec![val(database), val(table)];
        let rows = rows_of(
            &mut conn,
            "SELECT COALESCE(table_rows, 0), COALESCE(data_length, 0), COALESCE(index_length, 0), table_comment, COALESCE(engine, '') \
               FROM information_schema.tables WHERE table_schema = ? AND table_name = ?",
            args(),
        )
        .await?;
        let r = rows.first().ok_or_else(|| DatabaseError::new(atlas_core::ErrorKind::InvalidInput, format!("table `{database}`.`{table}` does not exist")))?;
        let (row_count, table_bytes, index_bytes) = (n(r, 0), n(r, 1), n(r, 2));
        let comment = Some(s(r, 3)).filter(|c| !c.is_empty());
        let engine = s(r, 4);
        let counts = rows_of(
            &mut conn,
            "SELECT (SELECT COUNT(*) FROM information_schema.columns WHERE table_schema = ? AND table_name = ?), \
                    (SELECT COUNT(DISTINCT index_name) FROM information_schema.statistics WHERE table_schema = ? AND table_name = ? AND non_unique = 1), \
                    (SELECT COUNT(*) FROM information_schema.table_constraints WHERE table_schema = ? AND table_name = ? AND constraint_type = 'PRIMARY KEY'), \
                    (SELECT COUNT(*) FROM information_schema.table_constraints WHERE table_schema = ? AND table_name = ? AND constraint_type = 'FOREIGN KEY'), \
                    (SELECT COUNT(*) FROM information_schema.table_constraints WHERE table_schema = ? AND table_name = ? AND constraint_type = 'UNIQUE'), \
                    (SELECT COUNT(*) FROM information_schema.table_constraints WHERE table_schema = ? AND table_name = ? AND constraint_type = 'CHECK'), \
                    (SELECT COUNT(*) FROM information_schema.triggers WHERE event_object_schema = ? AND event_object_table = ?)",
            (0..7).flat_map(|_| args()).collect(),
        )
        .await?;
        let c = &counts[0];
        Ok(TableProfile {
            owner: String::new(),
            kind: "r".into(),
            persistence: "permanent".into(),
            tablespace: engine,
            estimated_rows: row_count,
            table_bytes,
            index_bytes,
            total_bytes: table_bytes + index_bytes,
            columns: n(c, 0),
            indexes: n(c, 1),
            primary_keys: n(c, 2),
            foreign_keys: n(c, 3),
            uniques: n(c, 4),
            checks: n(c, 5),
            triggers: n(c, 6),
            row_security: false,
            replica_identity: "d".into(),
            live_tuples: Some(row_count),
            dead_tuples: None,
            seq_scans: None,
            index_scans: None,
            last_vacuum: None,
            last_autovacuum: None,
            last_analyze: None,
            last_autoanalyze: None,
            comment,
        })
    }
}
