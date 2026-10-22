//! Aplicação transacional de alterações geradas pela UI (edição de dados e DDL do designer).

use crate::connect::PgSession;
use crate::error::map_error;
use atlas_core::{DatabaseError, ErrorKind};
use serde::Deserialize;
use tokio_postgres::types::ToSql;

/// Um comando com parâmetros posicionais em texto. O SQL faz o cast para o tipo da coluna
/// (`$1::text::int4`), então os valores nunca são concatenados no texto (ADR-008).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlannedStatement {
    pub sql: String,
    #[serde(default)]
    pub params: Vec<Option<String>>,
    /// UPDATE/DELETE de linha: precisa afetar exatamente 1 linha, senão é conflito (spec §31.4).
    #[serde(default)]
    pub expect_one_row: bool,
}

/// Executa tudo em uma transação em conexão dedicada; qualquer falha ou contagem
/// inesperada desfaz o conjunto inteiro. Devolve as linhas afetadas por comando.
pub async fn apply_in_transaction(conn: PgSession, statements: &[PlannedStatement]) -> Result<Vec<u64>, DatabaseError> {
    let mut client = conn.client;
    let tx = client.transaction().await.map_err(|e| map_error(&e))?;
    let mut affected = Vec::with_capacity(statements.len());
    for (i, st) in statements.iter().enumerate() {
        let params: Vec<&(dyn ToSql + Sync)> = st.params.iter().map(|p| p as &(dyn ToSql + Sync)).collect();
        let n = tx.execute(st.sql.as_str(), &params).await.map_err(|e| map_error(&e))?;
        if st.expect_one_row && n != 1 {
            // `tx` é descartado sem commit: o driver envia ROLLBACK.
            let mut err = DatabaseError::new(
                ErrorKind::ConcurrencyConflict,
                format!("statement {} affected {n} rows (expected exactly 1); all changes were rolled back", i + 1),
            );
            err.detail = Some(st.sql.clone());
            return Err(err);
        }
        affected.push(n);
    }
    tx.commit().await.map_err(|e| map_error(&e))?;
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
            Ok(n) if st.expect_one_row && n != 1 => {
                let mut err = DatabaseError::new(
                    ErrorKind::ConcurrencyConflict,
                    format!("statement {} affected {n} rows (expected exactly 1); this batch was rolled back", i + 1),
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
