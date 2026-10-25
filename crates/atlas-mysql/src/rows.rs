//! Leitura e gravação de linhas: o `SELECT` da grade (ordem, filtro, página), a contagem, os comandos planejados pela interface
//! (`PlannedStatement`) e a transação manual.

use super::{MysqlSession, map_err, quote_ident, run_text};
use atlas_core::{DatabaseError, ErrorKind, OrderBy, QueryOutcome, TableFilter};
use mysql_async::prelude::*;
use mysql_async::{Conn, Params, Value};
use serde::Deserialize;
use tokio::sync::Mutex;

/// Um comando com parâmetros posicionais (`?`), como a interface o planeja; os valores vão em texto (ou NULL) e o MySQL converte
/// para o tipo da coluna.
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

/// `SELECT * FROM `banco`.`tabela` [WHERE ...] [ORDER BY ...] [LIMIT n OFFSET m]`: só identificadores entre crases entram no texto;
/// o filtro é a expressão do usuário.
pub fn select_sql(database: &str, table: &str, order_by: &[OrderBy], filter: Option<&TableFilter>, page: Option<(u32, u64)>) -> String {
    let mut sql = format!("SELECT * FROM {}.{}", quote_ident(database), quote_ident(table));
    if let Some(f) = filter {
        let expr = f.expr.trim();
        if !expr.is_empty() {
            sql.push_str(&format!(" WHERE {expr}"));
        }
    }
    if !order_by.is_empty() {
        let cols: Vec<String> = order_by.iter().map(|o| format!("{} {}", quote_ident(&o.column), if o.descending { "DESC" } else { "ASC" })).collect();
        sql.push_str(&format!(" ORDER BY {}", cols.join(", ")));
    }
    if let Some((limit, offset)) = page {
        sql.push_str(&format!(" LIMIT {limit} OFFSET {offset}"));
    }
    sql
}

/// Como `select_sql` (sem página), mas só com as colunas pedidas (vazio = todas).
pub fn select_columns_sql(database: &str, table: &str, columns: &[String], order_by: &[OrderBy], filter: Option<&TableFilter>) -> String {
    let sql = select_sql(database, table, order_by, filter, None);
    if columns.is_empty() {
        return sql;
    }
    let list = columns.iter().map(|c| quote_ident(c)).collect::<Vec<_>>().join(", ");
    sql.replacen("SELECT *", &format!("SELECT {list}"), 1)
}

fn count_problem(st: &PlannedStatement, affected: u64) -> Option<String> {
    if let Some(n) = st.expect_rows {
        return (affected != n).then(|| format!("expected {n} rows"));
    }
    (st.expect_one_row && affected != 1).then(|| "expected exactly 1 row".to_string())
}

fn count_sql(database: &str, table: &str, filter: Option<&TableFilter>) -> String {
    let mut sql = format!("SELECT COUNT(*) FROM {}.{}", quote_ident(database), quote_ident(table));
    if let Some(f) = filter {
        let expr = f.expr.trim();
        if !expr.is_empty() {
            sql.push_str(&format!(" WHERE {expr}"));
        }
    }
    sql
}

fn first_number(out: &QueryOutcome) -> u64 {
    out.statements.first().and_then(|s| s.result.as_ref()).and_then(|r| r.rows.first()).and_then(|r| r.first().cloned().flatten()).and_then(|v| v.parse().ok()).unwrap_or(0)
}

/// Roda os comandos planejados, todos ou nenhum. Numa transação própria (`own`) abre e fecha ela; numa transação manual do
/// usuário só marca um ponto de retorno e desfaz até ele em caso de erro. (DDL fecha a transação sozinho no MySQL.)
async fn apply_on(conn: &mut Conn, statements: &[PlannedStatement], own: bool) -> Result<Vec<u64>, DatabaseError> {
    conn.query_drop(if own { "START TRANSACTION" } else { "SAVEPOINT atlas_batch" }).await.map_err(map_err)?;
    let mut affected = Vec::with_capacity(statements.len());
    let mut failure: Option<DatabaseError> = None;
    for (i, st) in statements.iter().enumerate() {
        let params: Vec<Value> = st.params.iter().map(|p| p.as_ref().map_or(Value::NULL, |s| Value::Bytes(s.clone().into_bytes()))).collect();
        let run = if params.is_empty() { conn.query_drop(st.sql.as_str()).await } else { conn.exec_drop(st.sql.as_str(), Params::Positional(params)).await };
        match run {
            Ok(()) => {
                let n = conn.affected_rows();
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
                conn.query_drop("COMMIT").await.map_err(map_err)?;
            } else {
                conn.query_drop("RELEASE SAVEPOINT atlas_batch").await.map_err(map_err)?;
            }
            Ok(affected)
        }
        Some(err) => {
            let _ = conn.query_drop(if own { "ROLLBACK" } else { "ROLLBACK TO SAVEPOINT atlas_batch" }).await;
            if !own {
                let _ = conn.query_drop("RELEASE SAVEPOINT atlas_batch").await;
            }
            Err(err)
        }
    }
}

impl MysqlSession {
    pub async fn execute(&self, database: &str, sql: &str, max_rows: usize) -> Result<QueryOutcome, DatabaseError> {
        let client = self.client(database).await?;
        let mut conn = client.lock().await;
        run_text(&mut conn, sql, max_rows).await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn table_rows(
        &self,
        database: &str,
        table: &str,
        order_by: &[OrderBy],
        filter: Option<&TableFilter>,
        limit: Option<u32>,
        offset: u64,
        max_rows: usize,
    ) -> Result<(String, QueryOutcome), DatabaseError> {
        let sql = select_sql(database, table, order_by, filter, limit.map(|l| (l, offset)));
        let keep = limit.map_or(max_rows, |l| (l as usize).min(max_rows));
        let out = self.execute(database, &sql, keep).await?;
        Ok((sql, out))
    }

    pub async fn table_count(&self, database: &str, table: &str, filter: Option<&TableFilter>) -> Result<u64, DatabaseError> {
        Ok(first_number(&self.execute(database, &count_sql(database, table, filter), 1).await?))
    }

    pub async fn apply(&self, database: &str, statements: &[PlannedStatement]) -> Result<Vec<u64>, DatabaseError> {
        let client = self.client(database).await?;
        let mut conn = client.lock().await;
        apply_on(&mut conn, statements, true).await
    }

    /// Lê o resultado de um `SELECT` linha a linha numa conexão só desta leitura. `on_begin` recebe as colunas e `on_row` cada linha;
    /// devolver `Ok(false)` encerra a leitura. Devolve quantas linhas foram entregues.
    pub async fn stream_rows(
        &self,
        database: &str,
        sql: &str,
        on_begin: impl FnOnce(&[atlas_core::ResultColumn]) -> Result<(), DatabaseError>,
        mut on_row: impl FnMut(&[Option<&str>]) -> Result<bool, DatabaseError>,
    ) -> Result<u64, DatabaseError> {
        let mut conn = self.dedicated(database).await?;
        let mut result = conn.query_iter(sql).await.map_err(map_err)?;
        let cols = result.columns();
        let cols_ref = cols.as_deref().unwrap_or(&[]);
        let described: Vec<atlas_core::ResultColumn> = cols_ref.iter().map(|c| atlas_core::ResultColumn { name: c.name_str().to_string(), type_name: Some(crate::values::type_name(c)) }).collect();
        on_begin(&described)?;
        let mut count = 0u64;
        while let Some(row) = result.next().await.map_err(map_err)? {
            let cells = crate::values::row_text(&row, cols_ref);
            let refs: Vec<Option<&str>> = cells.iter().map(|c| c.as_deref()).collect();
            if !on_row(&refs)? {
                break;
            }
            count += 1;
        }
        Ok(count)
    }

    /// Abre uma transação manual: uma conexão só dela, com `START TRANSACTION`.
    pub async fn begin(&self, database: &str) -> Result<MysqlTx, DatabaseError> {
        let mut conn = self.dedicated(database).await?;
        conn.query_drop("START TRANSACTION").await.map_err(map_err)?;
        Ok(MysqlTx { conn: Mutex::new(conn) })
    }
}

/// Transação manual aberta: tudo que roda nela fica pendente até `finish`.
pub struct MysqlTx {
    conn: Mutex<Conn>,
}

impl MysqlTx {
    pub async fn execute(&self, sql: &str, max_rows: usize) -> Result<QueryOutcome, DatabaseError> {
        let mut conn = self.conn.lock().await;
        run_text(&mut conn, sql, max_rows).await
    }

    pub async fn apply(&self, statements: &[PlannedStatement]) -> Result<Vec<u64>, DatabaseError> {
        let mut conn = self.conn.lock().await;
        apply_on(&mut conn, statements, false).await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn table_rows(
        &self,
        database: &str,
        table: &str,
        order_by: &[OrderBy],
        filter: Option<&TableFilter>,
        limit: Option<u32>,
        offset: u64,
        max_rows: usize,
    ) -> Result<(String, QueryOutcome), DatabaseError> {
        let sql = select_sql(database, table, order_by, filter, limit.map(|l| (l, offset)));
        let keep = limit.map_or(max_rows, |l| (l as usize).min(max_rows));
        Ok((sql.clone(), self.execute(&sql, keep).await?))
    }

    pub async fn table_count(&self, database: &str, table: &str, filter: Option<&TableFilter>) -> Result<u64, DatabaseError> {
        Ok(first_number(&self.execute(&count_sql(database, table, filter), 1).await?))
    }

    pub async fn finish(&self, commit: bool) -> Result<(), DatabaseError> {
        let mut conn = self.conn.lock().await;
        conn.query_drop(if commit { "COMMIT" } else { "ROLLBACK" }).await.map_err(map_err)
    }
}
