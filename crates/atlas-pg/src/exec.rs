//! Execução de SQL do editor e leitura paginada de tabelas.

use crate::connect::PgSession;
use crate::error::map_error;
use atlas_core::{DatabaseError, QueryOutcome, ResultColumn, ResultSet, StatementOutcome};
use futures_util::{StreamExt, pin_mut};
use std::time::Instant;
use tokio_postgres::SimpleQueryMessage;

/// Quoting de identificador PostgreSQL: aspas duplas e aspas internas duplicadas (ADR-008).
pub fn quote_ident(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

/// Executa o texto como o usuário escreveu, pelo protocolo simples do PostgreSQL: o próprio
/// servidor separa os comandos (sem split por `;` no cliente — ADR-016) e devolve os valores
/// já formatados em texto, sem perda de precisão.
///
/// Cada result set guarda no máximo `max_rows` linhas; o excedente é lido e descartado
/// (memória limitada) e o result set é marcado como `truncated`.
pub async fn execute(s: &PgSession, sql: &str, max_rows: usize) -> Result<QueryOutcome, DatabaseError> {
    let started = Instant::now();

    // Tipos das colunas: só é possível descrever quando o texto é um único comando.
    // Falha aqui não é erro para o usuário (ex.: lote com vários comandos).
    let described: Option<Vec<String>> = match s.client.prepare(sql).await {
        Ok(stmt) => Some(stmt.columns().iter().map(|c| c.type_().name().to_string()).collect()),
        Err(_) => None,
    };

    let stream = s.client.simple_query_raw(sql).await.map_err(|e| map_error(&e))?;
    pin_mut!(stream);

    let mut statements = Vec::new();
    let mut current: Option<ResultSet> = None;
    while let Some(msg) = stream.next().await {
        match msg.map_err(|e| map_error(&e))? {
            SimpleQueryMessage::RowDescription(cols) => {
                let single = described.as_ref().filter(|d| d.len() == cols.len() && statements.is_empty());
                current = Some(ResultSet {
                    columns: cols
                        .iter()
                        .enumerate()
                        .map(|(i, c)| ResultColumn { name: c.name().to_string(), type_name: single.map(|d| d[i].clone()) })
                        .collect(),
                    rows: Vec::new(),
                    truncated: false,
                });
            }
            SimpleQueryMessage::Row(row) => {
                if let Some(rs) = current.as_mut() {
                    if rs.rows.len() < max_rows {
                        rs.rows.push((0..row.len()).map(|i| row.get(i).map(str::to_string)).collect());
                    } else {
                        rs.truncated = true;
                    }
                }
            }
            SimpleQueryMessage::CommandComplete(n) => {
                statements.push(StatementOutcome { result: current.take(), rows_affected: Some(n) });
            }
            _ => {}
        }
    }
    Ok(QueryOutcome { statements, elapsed_ms: started.elapsed().as_millis() as u64 })
}

/// Página de uma tabela. A ordenação pela chave primária torna a paginação determinística
/// (spec §31.2); sem chave, a ordem do servidor não é garantida.
pub async fn table_rows(
    s: &PgSession,
    schema: &str,
    table: &str,
    order_by: &[String],
    limit: u32,
    offset: u64,
) -> Result<(String, QueryOutcome), DatabaseError> {
    let mut sql = format!("SELECT * FROM {}.{}", quote_ident(schema), quote_ident(table));
    if !order_by.is_empty() {
        let cols: Vec<String> = order_by.iter().map(|c| quote_ident(c)).collect();
        sql.push_str(&format!(" ORDER BY {}", cols.join(", ")));
    }
    sql.push_str(&format!(" LIMIT {limit} OFFSET {offset}"));
    let out = execute(s, &sql, limit as usize).await?;
    Ok((sql, out))
}

#[cfg(test)]
mod tests {
    use super::quote_ident;

    #[test]
    fn quoting_escapes_embedded_quotes() {
        assert_eq!(quote_ident("audit_log"), "\"audit_log\"");
        assert_eq!(quote_ident("Weird.Schema"), "\"Weird.Schema\"");
        assert_eq!(quote_ident("weird\"; DROP TABLE x; --"), "\"weird\"\"; DROP TABLE x; --\"");
    }
}
