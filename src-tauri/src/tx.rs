//! Transações manuais da grade de dados: cada uma tem uma conexão física dedicada, que fica com a transação
//! aberta até COMMIT/ROLLBACK. Se o app fechar no meio, a conexão cai e o servidor desfaz tudo.

use crate::commands::{TableRows, TABLE_ROWS_CAP};
use crate::sessions::Sessions;
use atlas_core::{DatabaseError, ErrorKind, OrderBy, TableFilter};
use atlas_pg::PgSession;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tauri::State;
use tokio::sync::Mutex;

#[derive(Default)]
pub struct Transactions {
    next: AtomicU64,
    open: Mutex<HashMap<String, Arc<PgSession>>>,
}

impl Transactions {
    async fn get(&self, id: &str) -> Result<Arc<PgSession>, DatabaseError> {
        self.open.lock().await.get(id).cloned().ok_or_else(|| DatabaseError::new(ErrorKind::SessionNotFound, "transaction is not open"))
    }
}

/// Abre uma conexão dedicada com `BEGIN` e devolve o id da transação.
#[tauri::command]
pub async fn pg_tx_begin(
    sessions: State<'_, Sessions>,
    txs: State<'_, Transactions>,
    session_id: String,
    database: String,
) -> Result<String, DatabaseError> {
    let conn = sessions.dedicated(&session_id, &database).await?;
    atlas_pg::begin_manual_transaction(&conn).await?;
    let id = format!("tx{}", txs.next.fetch_add(1, Ordering::Relaxed) + 1);
    txs.open.lock().await.insert(id.clone(), Arc::new(conn));
    Ok(id)
}

/// Aplica alterações da grade dentro da transação aberta (sem commit).
#[tauri::command]
pub async fn pg_tx_apply(
    txs: State<'_, Transactions>,
    tx_id: String,
    statements: Vec<atlas_pg::PlannedStatement>,
) -> Result<Vec<u64>, DatabaseError> {
    let conn = txs.get(&tx_id).await?;
    atlas_pg::apply_in_open_transaction(&conn, &statements).await
}

/// Executa SQL do editor dentro da transação aberta (os resultados enxergam as alterações ainda não gravadas).
#[tauri::command]
pub async fn pg_tx_execute(
    txs: State<'_, Transactions>,
    tx_id: String,
    sql: String,
    max_rows: usize,
    schema: Option<String>,
) -> Result<atlas_core::QueryOutcome, DatabaseError> {
    let conn = txs.get(&tx_id).await?;
    atlas_pg::execute_in_schema(&conn, &sql, max_rows.clamp(1, 100_000), schema.as_deref()).await
}

/// Lê uma página da tabela enxergando as alterações ainda não gravadas da transação.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn pg_tx_table_rows(
    txs: State<'_, Transactions>,
    tx_id: String,
    schema: String,
    table: String,
    order_by: Vec<OrderBy>,
    filter: Option<TableFilter>,
    limit: Option<u32>,
    offset: u64,
) -> Result<TableRows, DatabaseError> {
    let s = txs.get(&tx_id).await?;
    let limit = limit.map(|l| l.clamp(1, TABLE_ROWS_CAP as u32));
    let (sql, outcome) = atlas_pg::table_rows(&s, &schema, &table, &order_by, filter.as_ref(), limit, offset, TABLE_ROWS_CAP).await?;
    Ok(TableRows { sql, outcome })
}

#[tauri::command]
pub async fn pg_tx_table_count(
    txs: State<'_, Transactions>,
    tx_id: String,
    schema: String,
    table: String,
    filter: Option<TableFilter>,
) -> Result<u64, DatabaseError> {
    let s = txs.get(&tx_id).await?;
    atlas_pg::table_count(&s, &schema, &table, filter.as_ref()).await
}

/// Encerra a transação (`commit` grava; senão desfaz) e fecha a conexão dedicada.
#[tauri::command]
pub async fn pg_tx_end(txs: State<'_, Transactions>, tx_id: String, commit: bool) -> Result<(), DatabaseError> {
    let conn = txs.get(&tx_id).await?;
    let res = atlas_pg::end_manual_transaction(&conn, commit).await;
    // Em falha de COMMIT o servidor já terminou a transação; de qualquer forma a conexão não serve mais.
    txs.open.lock().await.remove(&tx_id);
    res
}
