//! Leitura e gravação de linhas: o `SELECT` da grade (ordem, filtro, página), a contagem e os comandos planejados pela interface
//! (`PlannedStatement`), na sessão e na transação manual.

use super::{OracleSession, OracleTx, execute_on, map_err, params_of, quote_ident};
use atlas_core::{DatabaseError, ErrorKind, OrderBy, QueryOutcome, TableFilter};
use oracle_rs::Connection;
use serde::Deserialize;

/// Um comando com parâmetros posicionais (`:1`, `:2`...), como a interface o planeja; os valores vão em texto (ou NULL) e o Oracle
/// converte para o tipo da coluna (a sessão fixa os formatos de data e número).
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlannedStatement {
    pub sql: String,
    #[serde(default)]
    pub params: Vec<Option<String>>,
    /// UPDATE/DELETE de linha com chave: precisa afetar exatamente 1 linha, senão é conflito.
    #[serde(default)]
    pub expect_one_row: bool,
    /// UPDATE/DELETE em tabela sem chave: mexe em exatamente esta quantidade de linhas iguais.
    #[serde(default)]
    pub expect_rows: Option<u64>,
}

fn target(schema: &str, table: &str) -> String {
    if schema.is_empty() { quote_ident(table) } else { format!("{}.{}", quote_ident(schema), quote_ident(table)) }
}

fn where_clause(filter: Option<&TableFilter>) -> String {
    match filter.map(|f| f.expr.trim()) {
        Some(expr) if !expr.is_empty() => format!(" WHERE {expr}"),
        _ => String::new(),
    }
}

/// `SELECT * FROM "SCHEMA"."TABELA" [WHERE ...] [ORDER BY ...] [OFFSET n ROWS FETCH NEXT m ROWS ONLY]`: só identificadores entre aspas
/// entram no texto; o filtro é a expressão do usuário.
pub fn select_sql(schema: &str, table: &str, order_by: &[OrderBy], filter: Option<&TableFilter>, page: Option<(u32, u64)>) -> String {
    let mut sql = format!("SELECT * FROM {}{}", target(schema, table), where_clause(filter));
    if !order_by.is_empty() {
        let cols: Vec<String> = order_by.iter().map(|o| format!("{} {}", quote_ident(&o.column), if o.descending { "DESC" } else { "ASC" })).collect();
        sql.push_str(&format!(" ORDER BY {}", cols.join(", ")));
    }
    if let Some((limit, offset)) = page {
        sql.push_str(&format!(" OFFSET {offset} ROWS FETCH NEXT {limit} ROWS ONLY"));
    }
    sql
}

/// Como `select_sql` (sem página), mas só com as colunas pedidas (vazio = todas).
pub fn select_columns_sql(schema: &str, table: &str, columns: &[String], order_by: &[OrderBy], filter: Option<&TableFilter>) -> String {
    let sql = select_sql(schema, table, order_by, filter, None);
    if columns.is_empty() {
        return sql;
    }
    let list = columns.iter().map(|c| quote_ident(c)).collect::<Vec<_>>().join(", ");
    sql.replacen("SELECT *", &format!("SELECT {list}"), 1)
}

/// A lista de colunas do `SELECT` da grade: o `XMLType` o driver não decodifica, então essas colunas vão como texto (`XMLSERIALIZE`).
/// Sem coluna `XMLType` devolve `None` (o `SELECT *` basta).
async fn select_list(conn: &Connection, schema: &str, table: &str) -> Option<String> {
    let rows = crate::meta::rows_of(conn, "SELECT column_name, data_type FROM all_tab_cols WHERE owner = :1 AND table_name = :2 AND hidden_column = 'NO' ORDER BY column_id", &[oracle_rs::Value::String(schema.to_string()), oracle_rs::Value::String(table.to_string())])
        .await
        .ok()?;
    if !rows.iter().any(|r| crate::meta::s(r, 1) == "XMLTYPE") {
        return None;
    }
    Some(
        rows.iter()
            .map(|r| {
                let q = quote_ident(&crate::meta::s(r, 0));
                if crate::meta::s(r, 1) == "XMLTYPE" { format!("XMLSERIALIZE(CONTENT {q} AS CLOB) AS {q}") } else { q }
            })
            .collect::<Vec<_>>()
            .join(", "),
    )
}

fn with_list(sql: String, list: Option<String>) -> String {
    match list {
        Some(l) => sql.replacen("SELECT *", &format!("SELECT {l}"), 1),
        None => sql,
    }
}

fn count_sql(schema: &str, table: &str, filter: Option<&TableFilter>) -> String {
    format!("SELECT COUNT(*) FROM {}{}", target(schema, table), where_clause(filter))
}

fn first_number(out: &QueryOutcome) -> u64 {
    out.statements.first().and_then(|s| s.result.as_ref()).and_then(|r| r.rows.first()).and_then(|r| r.first().cloned().flatten()).and_then(|v| v.parse().ok()).unwrap_or(0)
}

fn count_problem(st: &PlannedStatement, affected: u64) -> Option<String> {
    if let Some(n) = st.expect_rows {
        return (affected != n).then(|| format!("expected {n} rows"));
    }
    (st.expect_one_row && affected != 1).then(|| "expected exactly 1 row".to_string())
}

/// Roda os comandos planejados, todos ou nenhum. Numa transação própria (`own`) grava no fim e desfaz tudo se algo falhar; numa
/// transação manual do usuário só marca um ponto de retorno e desfaz até ele em caso de erro.
async fn apply_on(conn: &Connection, statements: &[PlannedStatement], own: bool) -> Result<Vec<u64>, DatabaseError> {
    if !own {
        conn.savepoint("atlas_batch").await.map_err(map_err)?;
    }
    let mut affected = Vec::with_capacity(statements.len());
    let mut failure: Option<DatabaseError> = None;
    for (i, st) in statements.iter().enumerate() {
        match conn.execute(&st.sql, &params_of(&st.params)).await {
            Ok(res) => {
                let n = res.rows_affected;
                if let Some(expected) = count_problem(st, n) {
                    let mut err = DatabaseError::new(ErrorKind::ConcurrencyConflict, format!("statement {} affected {n} rows ({expected}); all changes were rolled back", i + 1));
                    err.detail = Some(st.sql.clone());
                    failure = Some(err);
                    break;
                }
                affected.push(n);
            }
            Err(e) => {
                let mut err = map_err(e);
                err.detail = Some(st.sql.clone());
                failure = Some(err);
                break;
            }
        }
    }
    match failure {
        None => {
            if own {
                conn.commit().await.map_err(map_err)?;
            }
            Ok(affected)
        }
        Some(err) => {
            let _ = if own { conn.rollback().await } else { conn.rollback_to_savepoint("atlas_batch").await };
            Err(err)
        }
    }
}

impl OracleSession {
    #[allow(clippy::too_many_arguments)]
    pub async fn table_rows(&self, schema: &str, table: &str, order_by: &[OrderBy], filter: Option<&TableFilter>, limit: Option<u32>, offset: u64, max_rows: usize) -> Result<(String, QueryOutcome), DatabaseError> {
        let list = select_list(self.conn(), schema, table).await;
        let sql = with_list(select_sql(schema, table, order_by, filter, limit.map(|l| (l, offset))), list);
        let keep = limit.map_or(max_rows, |l| (l as usize).min(max_rows));
        let out = self.execute("", &sql, keep).await?;
        Ok((sql, out))
    }

    pub async fn table_count(&self, schema: &str, table: &str, filter: Option<&TableFilter>) -> Result<u64, DatabaseError> {
        Ok(first_number(&self.execute("", &count_sql(schema, table, filter), 1).await?))
    }

    pub async fn apply(&self, statements: &[PlannedStatement]) -> Result<Vec<u64>, DatabaseError> {
        apply_on(self.conn(), statements, true).await
    }

    /// Lê o resultado de um `SELECT` linha a linha numa conexão só desta leitura. `on_begin` recebe as colunas e `on_row` cada linha;
    /// devolver `Ok(false)` encerra a leitura. Devolve quantas linhas foram entregues.
    pub async fn stream_rows(
        &self,
        sql: &str,
        on_begin: impl FnOnce(&[atlas_core::ResultColumn]) -> Result<(), DatabaseError>,
        mut on_row: impl FnMut(&[Option<&str>]) -> Result<bool, DatabaseError>,
    ) -> Result<u64, DatabaseError> {
        let conn = self.dedicated().await?;
        let mut res = conn.query(sql, &[]).await.map_err(map_err)?;
        let columns = res.columns.clone();
        let described: Vec<atlas_core::ResultColumn> = columns.iter().map(|c| atlas_core::ResultColumn { name: c.name.clone(), type_name: Some(crate::values::type_name(c)) }).collect();
        on_begin(&described)?;
        let mut count = 0u64;
        loop {
            for row in std::mem::take(&mut res.rows) {
                let mut cells = Vec::with_capacity(columns.len());
                for (i, col) in columns.iter().enumerate() {
                    cells.push(match row.get(i) {
                        Some(v) => crate::values::cell(&conn, col, v).await,
                        None => None,
                    });
                }
                let refs: Vec<Option<&str>> = cells.iter().map(|c| c.as_deref()).collect();
                if !on_row(&refs)? {
                    let _ = conn.close().await;
                    return Ok(count);
                }
                count += 1;
            }
            if !res.has_more_rows {
                break;
            }
            res = conn.fetch_more(res.cursor_id, &columns, 1000).await.map_err(map_err)?;
        }
        let _ = conn.close().await;
        Ok(count)
    }
}

impl OracleTx {
    pub async fn apply(&self, statements: &[PlannedStatement]) -> Result<Vec<u64>, DatabaseError> {
        apply_on(&self.conn, statements, false).await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn table_rows(&self, schema: &str, table: &str, order_by: &[OrderBy], filter: Option<&TableFilter>, limit: Option<u32>, offset: u64, max_rows: usize) -> Result<(String, QueryOutcome), DatabaseError> {
        let list = select_list(&self.conn, schema, table).await;
        let sql = with_list(select_sql(schema, table, order_by, filter, limit.map(|l| (l, offset))), list);
        let keep = limit.map_or(max_rows, |l| (l as usize).min(max_rows));
        let out = execute_on(&self.conn, &sql, keep, false).await?;
        Ok((sql, out))
    }

    pub async fn table_count(&self, schema: &str, table: &str, filter: Option<&TableFilter>) -> Result<u64, DatabaseError> {
        Ok(first_number(&execute_on(&self.conn, &count_sql(schema, table, filter), 1, false).await?))
    }
}
