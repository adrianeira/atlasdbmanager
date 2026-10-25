//! Aplicação transacional de alterações geradas pela UI (edição de dados e DDL do designer).

use crate::connect::PgSession;
use crate::error::map_error;
use atlas_core::{DatabaseError, ErrorKind};
use serde::Deserialize;
use tokio_postgres::types::ToSql;

/// Um comando com parâmetros posicionais em texto. O SQL faz o cast para o tipo da coluna
/// (`$1::text::int4`), então os valores nunca são concatenados no texto (ADR-008).
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlannedStatement {
    pub sql: String,
    #[serde(default)]
    pub params: Vec<Option<String>>,
    /// UPDATE/DELETE de linha: precisa afetar exatamente 1 linha, senão é conflito (spec §31.4).
    #[serde(default)]
    pub expect_one_row: bool,
    /// UPDATE/DELETE em tabela sem chave: o comando mexe em exatamente esta quantidade de linhas iguais (as marcadas na tela); outra contagem é conflito.
    #[serde(default)]
    pub expect_rows: Option<u64>,
}

/// O que há de errado com a contagem de linhas afetadas, se houver.
fn count_problem(st: &PlannedStatement, n: u64) -> Option<String> {
    if st.expect_one_row && n != 1 {
        Some("expected exactly 1".to_string())
    } else if let Some(k) = st.expect_rows
        && n != k
    {
        Some(format!("expected exactly {k}"))
    } else {
        None
    }
}

/// Executa tudo em uma transação em conexão dedicada; qualquer falha ou contagem
/// inesperada desfaz o conjunto inteiro. Devolve as linhas afetadas por comando.
pub async fn apply_in_transaction(conn: PgSession, statements: &[PlannedStatement]) -> Result<Vec<u64>, DatabaseError> {
    let client = conn.client;
    client.batch_execute("BEGIN").await.map_err(|e| map_error(&e))?;
    let mut affected = Vec::with_capacity(statements.len());
    for (i, st) in statements.iter().enumerate() {
        let params: Vec<&(dyn ToSql + Sync)> = st.params.iter().map(|p| p as &(dyn ToSql + Sync)).collect();
        let n = match client.execute(st.sql.as_str(), &params).await {
            Ok(n) => n,
            Err(e) => {
                let _ = client.batch_execute("ROLLBACK").await;
                return Err(map_error(&e));
            }
        };
        if let Some(expected) = count_problem(st, n) {
            let _ = client.batch_execute("ROLLBACK").await;
            let mut err = DatabaseError::new(
                ErrorKind::ConcurrencyConflict,
                format!("statement {} affected {n} rows ({expected}); all changes were rolled back", i + 1),
            );
            err.detail = Some(st.sql.clone());
            return Err(err);
        }
        affected.push(n);
    }
    if let Err(e) = client.batch_execute("COMMIT").await {
        let _ = client.batch_execute("ROLLBACK").await;
        return Err(map_error(&e));
    }
    Ok(affected)
}

/// Abre uma transação manual na conexão (fica aberta até `end_manual_transaction`).
pub async fn begin_manual_transaction(conn: &PgSession) -> Result<(), DatabaseError> {
    conn.client.batch_execute("BEGIN").await.map_err(|e| map_error(&e))
}

/// Encerra a transação manual: `commit` grava, senão desfaz.
pub async fn end_manual_transaction(conn: &PgSession, commit: bool) -> Result<(), DatabaseError> {
    conn.client.batch_execute(if commit { "COMMIT" } else { "ROLLBACK" }).await.map_err(|e| map_error(&e))
}

/// Aplica os comandos dentro da transação manual já aberta. Cada lote roda sob um SAVEPOINT: se algo falhar, só o
/// lote é desfeito e a transação continua utilizável (o que já foi feito antes permanece até o COMMIT/ROLLBACK).
pub async fn apply_in_open_transaction(conn: &PgSession, statements: &[PlannedStatement]) -> Result<Vec<u64>, DatabaseError> {
    let _guard = conn.exec_lock.lock().await;
    conn.client.batch_execute("SAVEPOINT atlas_batch").await.map_err(|e| map_error(&e))?;
    let mut affected = Vec::with_capacity(statements.len());
    for (i, st) in statements.iter().enumerate() {
        let params: Vec<&(dyn ToSql + Sync)> = st.params.iter().map(|p| p as &(dyn ToSql + Sync)).collect();
        let res = conn.client.execute(st.sql.as_str(), &params).await.map_err(|e| map_error(&e));
        let failure = match res {
            Ok(n) if count_problem(st, n).is_some() => {
                let mut err = DatabaseError::new(
                    ErrorKind::ConcurrencyConflict,
                    format!("statement {} affected {n} rows ({}); this batch was rolled back", i + 1, count_problem(st, n).unwrap_or_default()),
                );
                err.detail = Some(st.sql.clone());
                Some(err)
            }
            Ok(n) => {
                affected.push(n);
                None
            }
            Err(e) => Some(e),
        };
        if let Some(err) = failure {
            let _ = conn.client.batch_execute("ROLLBACK TO SAVEPOINT atlas_batch").await;
            return Err(err);
        }
    }
    conn.client.batch_execute("RELEASE SAVEPOINT atlas_batch").await.map_err(|e| map_error(&e))?;
    Ok(affected)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn row_count_rules() {
        let one = PlannedStatement { expect_one_row: true, ..Default::default() };
        assert!(count_problem(&one, 1).is_none());
        assert!(count_problem(&one, 0).is_some() && count_problem(&one, 2).is_some());
        // Tabela sem chave: o comando mexe só nas linhas marcadas (aqui 2 de várias iguais); outra contagem é conflito.
        let keyless = PlannedStatement { expect_rows: Some(2), ..Default::default() };
        assert!(count_problem(&keyless, 2).is_none());
        assert!(count_problem(&keyless, 0).is_some() && count_problem(&keyless, 3).is_some());
        // INSERT e DDL não conferem a contagem.
        assert!(count_problem(&PlannedStatement::default(), 0).is_none());
    }
}
