//! Server Monitor: lista de processos (com os bloqueios de cada um), variáveis e contadores do servidor.

use crate::connect::PgSession;
use crate::error::map_error;
use atlas_core::DatabaseError;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessRow {
    pub id: i32,
    pub user: Option<String>,
    pub host: Option<String>,
    pub port: Option<i32>,
    pub db: Option<String>,
    pub command: Option<String>,
    /// Modo do bloqueio que o processo segura (`AccessShareLock`...); vazio se não segura nenhum.
    pub state: Option<String>,
    pub time: Option<String>,
    /// `Locked Object: tabela` ou `Locked Transaction: x/y`.
    pub info: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub processes: Vec<ProcessRow>,
    pub variables: Vec<(String, String)>,
    pub status: Vec<(String, String)>,
}

fn sql_err(e: tokio_postgres::Error) -> DatabaseError {
    map_error(&e)
}

/// Um instante do servidor: processos, `pg_settings` e os contadores do banco atual. As consultas do próprio monitor ficam
/// na lista, como na referência.
pub async fn snapshot(s: &PgSession) -> Result<Snapshot, DatabaseError> {
    let rows = s
        .client
        .query(
            "SELECT a.pid, a.usename::text, pg_catalog.host(a.client_addr), NULLIF(a.client_port, -1), a.datname::text, a.query, l.mode::text, \
                    to_char(COALESCE(a.query_start, a.backend_start), 'YYYY-MM-DD HH24:MI:SS.US'), \
                    CASE WHEN l.relation IS NOT NULL THEN 'Locked Object: ' || (SELECT c.relname::text FROM pg_catalog.pg_class c WHERE c.oid = l.relation) \
                         WHEN l.transactionid IS NOT NULL THEN 'Locked Transaction: ' || l.transactionid::text \
                         WHEN l.virtualxid IS NOT NULL THEN 'Locked Transaction: ' || l.virtualxid END \
               FROM pg_catalog.pg_stat_activity a \
               LEFT JOIN pg_catalog.pg_locks l ON l.pid = a.pid AND (l.relation IS NOT NULL OR l.transactionid IS NOT NULL) \
              WHERE a.pid IS NOT NULL \
              ORDER BY a.pid, l.mode, 9",
            &[],
        )
        .await
        .map_err(sql_err)?;
    let processes = rows
        .iter()
        .map(|r| ProcessRow {
            id: r.get(0),
            user: r.get(1),
            host: r.get(2),
            port: r.get(3),
            db: r.get(4),
            command: r.get(5),
            state: r.get(6),
            time: r.get(7),
            info: r.get(8),
        })
        .collect();
    // DateStyle, IntervalStyle e TimeZone primeiro (como na referência); o resto em ordem alfabética de bytes.
    let variables = s
        .client
        .query(
            "SELECT name::text, pg_catalog.current_setting(name) FROM pg_catalog.pg_settings \
              ORDER BY (name NOT IN ('DateStyle', 'IntervalStyle', 'TimeZone')), name COLLATE \"C\"",
            &[],
        )
        .await
        .map_err(sql_err)?
        .iter()
        .map(|r| (r.get(0), r.get(1)))
        .collect();
    let status = s
        .client
        .query_one(
            "SELECT pg_catalog.pg_stat_get_db_blocks_fetched(d.oid)::text, pg_catalog.pg_stat_get_db_blocks_hit(d.oid)::text, \
                    pg_catalog.pg_stat_get_db_numbackends(d.oid)::text, pg_catalog.pg_stat_get_db_xact_commit(d.oid)::text, \
                    pg_catalog.pg_stat_get_db_xact_rollback(d.oid)::text \
               FROM pg_catalog.pg_database d WHERE d.datname = pg_catalog.current_database()",
            &[],
        )
        .await
        .map_err(sql_err)?;
    let names = ["db_blocks_fetched", "db_blocks_hit", "db_numbackends", "db_xact_commit", "db_xact_rollback"];
    let status = names.iter().enumerate().map(|(i, n)| (n.to_string(), status.get::<_, String>(i))).collect();
    Ok(Snapshot { processes, variables, status })
}

/// Encerra o processo (`pg_terminate_backend`). Devolve se o servidor aceitou.
pub async fn end_process(s: &PgSession, pid: i32) -> Result<bool, DatabaseError> {
    let row = s.client.query_one("SELECT pg_catalog.pg_terminate_backend($1)", &[&pid]).await.map_err(sql_err)?;
    Ok(row.get(0))
}
