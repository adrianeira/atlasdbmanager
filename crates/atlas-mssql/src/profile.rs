//! Data Profiling e Table Profile do SQL Server, no mesmo formato do PostgreSQL.

use super::{MssqlSession, quote_ident};
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

impl MssqlSession {
    /// Perfil das `columns` dos registros que o filtro, a ordenação e o limite escolhem (sem limite = todos).
    /// Cada medida é uma consulta própria: uma que o tipo não permite (`DISTINCT` de `xml`, menor valor de `bit`) só deixa
    /// aquela medida vazia.
    #[allow(clippy::too_many_arguments)]
    pub async fn data_profile(
        &self,
        database: &str,
        schema: &str,
        table: &str,
        columns: &[String],
        order_by: &[OrderBy],
        filter: Option<&TableFilter>,
        limit: Option<u64>,
    ) -> Result<DataProfile, DatabaseError> {
        // A origem: com ordem escolhida o `TOP` é obrigatório (o SQL Server recusa `ORDER BY` sem ele numa tabela derivada).
        let mut source = select_sql(schema, table, order_by, filter, None);
        let top = limit.map_or("9223372036854775807".to_string(), |l| l.to_string());
        source = source.replacen("SELECT *", &format!("SELECT TOP ({top}) *"), 1);
        let mut out = Vec::with_capacity(columns.len());
        let mut total = 0;
        for name in columns {
            let q = quote_ident(name);
            let with = format!("WITH src AS ({source})");
            let counts = self.execute(database, &format!("{with} SELECT COUNT_BIG(*), COUNT_BIG({q}) FROM src"), 1).await?;
            let rows = num(&counts, 0).unwrap_or(0);
            let not_null = num(&counts, 1).unwrap_or(0);
            total = rows;
            let distinct_sql = format!("{with} SELECT COUNT_BIG(DISTINCT {q}), (SELECT COUNT_BIG(*) FROM (SELECT {q} AS v FROM src WHERE {q} IS NOT NULL GROUP BY {q} HAVING COUNT_BIG(*) = 1) u) FROM src");
            let (distinct, unique) = match self.execute(database, &distinct_sql, 1).await {
                Ok(o) => (num(&o, 0), num(&o, 1)),
                Err(_) => (None, None),
            };
            let (min, max) = match self.execute(database, &format!("{with} SELECT MIN({q}), MAX({q}) FROM src"), 1).await {
                Ok(o) => (cell(&o, 0, 0), cell(&o, 0, 1)),
                Err(_) => (None, None),
            };
            let top_sql = format!("{with} SELECT TOP ({TOP_VALUES}) {q}, COUNT_BIG(*) FROM src GROUP BY {q} ORDER BY COUNT_BIG(*) DESC, {q}");
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

    /// Perfil da tabela: tamanhos, contagens da estrutura e total de linhas.
    pub async fn table_profile(&self, database: &str, schema: &str, table: &str) -> Result<TableProfile, DatabaseError> {
        let client = self.client(database).await?;
        let mut conn = client.lock().await;
        let object = format!("{}.{}", quote_ident(schema), quote_ident(table));
        let rows = rows_of(
            &mut conn,
            "SELECT CAST(COALESCE(SUM(CASE WHEN ps.index_id IN (0, 1) THEN ps.row_count END), 0) AS bigint), \
                    CAST(COALESCE(SUM(CASE WHEN ps.index_id IN (0, 1) THEN ps.used_page_count END), 0) * 8192 AS bigint), \
                    CAST(COALESCE(SUM(CASE WHEN ps.index_id > 1 THEN ps.used_page_count END), 0) * 8192 AS bigint), \
                    CAST(COALESCE(SUM(ps.reserved_page_count), 0) * 8192 AS bigint) \
               FROM sys.dm_db_partition_stats ps WHERE ps.object_id = OBJECT_ID(@P1)",
            &[&object],
        )
        .await?;
        let r = rows.first().ok_or_else(|| DatabaseError::new(atlas_core::ErrorKind::InvalidInput, format!("table {object} does not exist")))?;
        let (row_count, table_bytes, index_bytes, total_bytes) = (r.get::<i64, _>(0).unwrap_or(0), r.get::<i64, _>(1).unwrap_or(0), r.get::<i64, _>(2).unwrap_or(0), r.get::<i64, _>(3).unwrap_or(0));
        let counts = rows_of(
            &mut conn,
            "SELECT (SELECT COUNT(*) FROM sys.columns WHERE object_id = OBJECT_ID(@P1)), \
                    (SELECT COUNT(*) FROM sys.indexes WHERE object_id = OBJECT_ID(@P1) AND type > 0 AND is_primary_key = 0 AND is_unique_constraint = 0), \
                    (SELECT COUNT(*) FROM sys.indexes WHERE object_id = OBJECT_ID(@P1) AND is_primary_key = 1), \
                    (SELECT COUNT(*) FROM sys.foreign_keys WHERE parent_object_id = OBJECT_ID(@P1)), \
                    (SELECT COUNT(*) FROM sys.indexes WHERE object_id = OBJECT_ID(@P1) AND is_unique_constraint = 1), \
                    (SELECT COUNT(*) FROM sys.check_constraints WHERE parent_object_id = OBJECT_ID(@P1)), \
                    (SELECT COUNT(*) FROM sys.triggers WHERE parent_id = OBJECT_ID(@P1)), \
                    (SELECT SCHEMA_NAME(schema_id) FROM sys.tables WHERE object_id = OBJECT_ID(@P1))",
            &[&object],
        )
        .await?;
        let c = &counts[0];
        let _ = s(c, 7);
        Ok(TableProfile {
            owner: String::new(),
            kind: "r".into(),
            persistence: "permanent".into(),
            tablespace: String::new(),
            estimated_rows: row_count,
            table_bytes,
            index_bytes,
            total_bytes,
            columns: i64::from(n(c, 0)),
            indexes: i64::from(n(c, 1)),
            primary_keys: i64::from(n(c, 2)),
            foreign_keys: i64::from(n(c, 3)),
            uniques: i64::from(n(c, 4)),
            checks: i64::from(n(c, 5)),
            triggers: i64::from(n(c, 6)),
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
            comment: None,
        })
    }
}
