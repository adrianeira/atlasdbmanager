//! Server Monitor do SQL Server: sessões (`sys.dm_exec_sessions`), configurações e contadores.

use super::{MssqlSession, map_err};
use crate::meta::{n, os, rows_of, s};
use atlas_core::{DatabaseError, MonitorProcess, MonitorSnapshot};

impl MssqlSession {
    /// Um instante do servidor: as sessões de usuário, as configurações e os contadores principais.
    pub async fn monitor_snapshot(&self) -> Result<MonitorSnapshot, DatabaseError> {
        let client = self.client("master").await?;
        let mut conn = client.lock().await;
        let procs = rows_of(
            &mut conn,
            "SELECT CAST(s.session_id AS int), s.login_name, s.host_name, DB_NAME(COALESCE(r.database_id, s.database_id)), r.command, COALESCE(r.status, s.status), \
                    CAST(CAST(COALESCE(r.total_elapsed_time, DATEDIFF(MILLISECOND, s.last_request_start_time, SYSDATETIME())) / 1000.0 AS decimal(18,3)) AS nvarchar(30)), \
                    CAST(t.text AS nvarchar(max)) \
               FROM sys.dm_exec_sessions s \
               LEFT JOIN sys.dm_exec_requests r ON r.session_id = s.session_id \
               OUTER APPLY sys.dm_exec_sql_text(r.sql_handle) t \
              WHERE s.is_user_process = 1 ORDER BY s.session_id",
            &[],
        )
        .await?;
        let processes = procs
            .iter()
            .map(|r| MonitorProcess { id: i64::from(n(r, 0)), user: os(r, 1), host: os(r, 2), port: None, db: os(r, 3), command: os(r, 4), state: os(r, 5), time: os(r, 6), info: os(r, 7) })
            .collect();
        let variables = rows_of(&mut conn, "SELECT name, CAST(value_in_use AS nvarchar(100)) FROM sys.configurations ORDER BY name", &[]).await?.iter().map(|r| (s(r, 0), s(r, 1))).collect();
        let status = rows_of(
            &mut conn,
            "SELECT 'Uptime (seconds)', CAST(DATEDIFF(SECOND, sqlserver_start_time, SYSDATETIME()) AS nvarchar(30)) FROM sys.dm_os_sys_info \
             UNION ALL SELECT 'Sessions (user)', CAST(COUNT(*) AS nvarchar(30)) FROM sys.dm_exec_sessions WHERE is_user_process = 1 \
             UNION ALL SELECT 'Running requests', CAST(COUNT(*) AS nvarchar(30)) FROM sys.dm_exec_requests WHERE session_id > 50 \
             UNION ALL SELECT 'Databases', CAST(COUNT(*) AS nvarchar(30)) FROM sys.databases \
             UNION ALL SELECT RTRIM(counter_name), CAST(cntr_value AS nvarchar(30)) FROM sys.dm_os_performance_counters WHERE object_name LIKE '%:General Statistics%' OR object_name LIKE '%:SQL Statistics%' OR object_name LIKE '%:Buffer Manager%'",
            &[],
        )
        .await?
        .iter()
        .map(|r| (s(r, 0), s(r, 1)))
        .collect();
        Ok(MonitorSnapshot { processes, variables, status })
    }

    /// Encerra a sessão `id` (`KILL`). Devolve `false` se ela já não existia.
    pub async fn monitor_end_process(&self, id: i64) -> Result<bool, DatabaseError> {
        let mut conn = self.dedicated("master").await?;
        match conn.simple_query(format!("KILL {id}")).await.map_err(map_err)?.into_results().await {
            Ok(_) => Ok(true),
            Err(e) => {
                let err = map_err(e);
                // 6106: o processo já não existe.
                if err.code.as_deref() == Some("6106") { Ok(false) } else { Err(err) }
            }
        }
    }
}
