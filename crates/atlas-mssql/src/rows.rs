//! Leitura e gravação de linhas: o `SELECT` da grade (ordem, filtro, página), a contagem, os comandos planejados pela interface
//! (`PlannedStatement`) e a transação manual.

use super::{Conn, MssqlSession, map_err, quote_ident, run_text};
use atlas_core::{DatabaseError, ErrorKind, OrderBy, QueryOutcome, TableFilter};
use serde::Deserialize;
use tokio::sync::Mutex;

/// Um comando com parâmetros (`@P1`, `@P2`...), como a interface o planeja; os valores vão em texto (ou NULL) e o SQL Server
/// converte para o tipo da coluna.
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

/// `SELECT * FROM [schema].[tabela] [WHERE ...] [ORDER BY ...] [OFFSET ... FETCH ...]`: só identificadores entre colchetes entram
/// no texto; o filtro é a expressão do usuário. A página exige `ORDER BY`: sem ordem escolhida vai `(SELECT NULL)`.
pub fn select_sql(schema: &str, table: &str, order_by: &[OrderBy], filter: Option<&TableFilter>, page: Option<(u32, u64)>) -> String {
    let mut sql = format!("SELECT * FROM {}.{}", quote_ident(schema), quote_ident(table));
    if let Some(f) = filter {
        let expr = f.expr.trim();
        if !expr.is_empty() {
            sql.push_str(&format!(" WHERE {expr}"));
        }
    }
    if !order_by.is_empty() {
        let cols: Vec<String> = order_by.iter().map(|o| format!("{} {}", quote_ident(&o.column), if o.descending { "DESC" } else { "ASC" })).collect();
        sql.push_str(&format!(" ORDER BY {}", cols.join(", ")));
    } else if page.is_some() {
        sql.push_str(" ORDER BY (SELECT NULL)");
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

fn count_problem(st: &PlannedStatement, affected: u64) -> Option<String> {
    if let Some(n) = st.expect_rows {
        return (affected != n).then(|| format!("expected {n} rows"));
    }
    (st.expect_one_row && affected != 1).then(|| "expected exactly 1 row".to_string())
}

/// Roda os comandos planejados, todos ou nenhum. Numa transação própria (`own`) abre e fecha ela; numa transação manual do
/// usuário só marca um ponto de retorno e desfaz até ele em caso de erro.
async fn apply_on(conn: &mut Conn, statements: &[PlannedStatement], own: bool) -> Result<Vec<u64>, DatabaseError> {
    let begin = if own { "BEGIN TRANSACTION" } else { "SAVE TRANSACTION atlas_batch" };
    conn.simple_query(begin).await.map_err(map_err)?.into_results().await.map_err(map_err)?;
    let mut affected = Vec::with_capacity(statements.len());
    let mut failure: Option<DatabaseError> = None;
    for (i, st) in statements.iter().enumerate() {
        let mut query = tiberius::Query::new(st.sql.clone());
        for p in &st.params {
            query.bind(p.clone());
        }
        match query.execute(conn).await {
            Ok(res) => {
                let n = res.total();
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
                conn.simple_query("COMMIT TRANSACTION").await.map_err(map_err)?.into_results().await.map_err(map_err)?;
            }
            Ok(affected)
        }
        Some(err) => {
            let undo = if own { "ROLLBACK TRANSACTION" } else { "ROLLBACK TRANSACTION atlas_batch" };
            if let Ok(s) = conn.simple_query(undo).await {
                let _ = s.into_results().await;
            }
            Err(err)
        }
    }
}

impl MssqlSession {
    pub async fn execute(&self, database: &str, sql: &str, max_rows: usize) -> Result<QueryOutcome, DatabaseError> {
        let client = self.client(database).await?;
        let mut conn = client.lock().await;
        run_text(&mut conn, sql, max_rows).await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn table_rows(
        &self,
        database: &str,
        schema: &str,
        table: &str,
        order_by: &[OrderBy],
        filter: Option<&TableFilter>,
        limit: Option<u32>,
        offset: u64,
        max_rows: usize,
    ) -> Result<(String, QueryOutcome), DatabaseError> {
        let page = limit.map(|l| (l, offset));
        let sql = select_sql(schema, table, order_by, filter, page);
        let keep = limit.map_or(max_rows, |l| (l as usize).min(max_rows));
        let out = self.execute(database, &sql, keep).await?;
        Ok((sql, out))
    }

    pub async fn table_count(&self, database: &str, schema: &str, table: &str, filter: Option<&TableFilter>) -> Result<u64, DatabaseError> {
        let mut sql = format!("SELECT COUNT_BIG(*) FROM {}.{}", quote_ident(schema), quote_ident(table));
        if let Some(f) = filter {
            let expr = f.expr.trim();
            if !expr.is_empty() {
                sql.push_str(&format!(" WHERE {expr}"));
            }
        }
        let out = self.execute(database, &sql, 1).await?;
        Ok(out.statements.first().and_then(|s| s.result.as_ref()).and_then(|r| r.rows.first()).and_then(|r| r.first().cloned().flatten()).and_then(|v| v.parse().ok()).unwrap_or(0))
    }

    pub async fn apply(&self, database: &str, statements: &[PlannedStatement]) -> Result<Vec<u64>, DatabaseError> {
        let client = self.client(database).await?;
        let mut conn = client.lock().await;
        apply_on(&mut conn, statements, true).await
    }

    /// Lê o resultado de um `SELECT` linha a linha, sem guardar nada na memória, numa conexão só desta leitura. `on_begin` recebe as
    /// colunas e `on_row` cada linha; devolver `Ok(false)` encerra a leitura. Devolve quantas linhas foram entregues.
    pub async fn stream_rows(
        &self,
        database: &str,
        sql: &str,
        on_begin: impl FnOnce(&[atlas_core::ResultColumn]) -> Result<(), DatabaseError>,
        mut on_row: impl FnMut(&[Option<&str>]) -> Result<bool, DatabaseError>,
    ) -> Result<u64, DatabaseError> {
        use futures_util::TryStreamExt;
        use tiberius::QueryItem;
        let mut conn = self.dedicated(database).await?;
        let mut stream = conn.simple_query(sql).await.map_err(map_err)?;
        let mut on_begin = Some(on_begin);
        let mut count = 0u64;
        while let Some(item) = stream.try_next().await.map_err(map_err)? {
            match item {
                QueryItem::Metadata(meta) => {
                    if let Some(begin) = on_begin.take() {
                        let cols: Vec<atlas_core::ResultColumn> = meta.columns().iter().map(|c| atlas_core::ResultColumn { name: c.name().to_string(), type_name: Some(crate::values::type_name(c.column_type())) }).collect();
                        begin(&cols)?;
                    }
                }
                QueryItem::Row(row) => {
                    let cells = crate::values::row_text(&row);
                    let refs: Vec<Option<&str>> = cells.iter().map(|c| c.as_deref()).collect();
                    if !on_row(&refs)? {
                        break;
                    }
                    count += 1;
                }
            }
        }
        Ok(count)
    }

    /// Abre uma transação manual: uma conexão só dela, com `BEGIN TRANSACTION`.
    pub async fn begin(&self, database: &str) -> Result<MssqlTx, DatabaseError> {
        let mut conn = self.dedicated(database).await?;
        conn.simple_query("BEGIN TRANSACTION").await.map_err(map_err)?.into_results().await.map_err(map_err)?;
        Ok(MssqlTx { conn: Mutex::new(conn) })
    }
}

/// Transação manual aberta: tudo que roda nela fica pendente até `finish`.
pub struct MssqlTx {
    conn: Mutex<Conn>,
}

impl MssqlTx {
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
        schema: &str,
        table: &str,
        order_by: &[OrderBy],
        filter: Option<&TableFilter>,
        limit: Option<u32>,
        offset: u64,
        max_rows: usize,
    ) -> Result<(String, QueryOutcome), DatabaseError> {
        let sql = select_sql(schema, table, order_by, filter, limit.map(|l| (l, offset)));
        let keep = limit.map_or(max_rows, |l| (l as usize).min(max_rows));
        Ok((sql.clone(), self.execute(&sql, keep).await?))
    }

    pub async fn table_count(&self, schema: &str, table: &str, filter: Option<&TableFilter>) -> Result<u64, DatabaseError> {
        let mut sql = format!("SELECT COUNT_BIG(*) FROM {}.{}", quote_ident(schema), quote_ident(table));
        if let Some(f) = filter {
            let expr = f.expr.trim();
            if !expr.is_empty() {
                sql.push_str(&format!(" WHERE {expr}"));
            }
        }
        let out = self.execute(&sql, 1).await?;
        Ok(out.statements.first().and_then(|s| s.result.as_ref()).and_then(|r| r.rows.first()).and_then(|r| r.first().cloned().flatten()).and_then(|v| v.parse().ok()).unwrap_or(0))
    }

    pub async fn finish(&self, commit: bool) -> Result<(), DatabaseError> {
        let mut conn = self.conn.lock().await;
        let sql = if commit { "COMMIT TRANSACTION" } else { "ROLLBACK TRANSACTION" };
        conn.simple_query(sql).await.map_err(map_err)?.into_results().await.map_err(map_err)?;
        Ok(())
    }
}
