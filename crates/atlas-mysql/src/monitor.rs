//! Server Monitor do MySQL: processos (`information_schema.processlist`), variáveis e contadores globais.

use super::{MysqlSession, map_err};
use crate::meta::{n, rows_of, s, text};
use atlas_core::{DatabaseError, MonitorProcess, MonitorSnapshot};
use mysql_async::prelude::*;
use mysql_async::Value;

impl MysqlSession {
    /// Um instante do servidor: a lista de processos e as variáveis e contadores globais.
    pub async fn monitor_snapshot(&self) -> Result<MonitorSnapshot, DatabaseError> {
        let client = self.client("").await?;
        let mut conn = client.lock().await;
        let procs = rows_of(&mut conn, "SELECT id, user, host, db, command, state, time, info FROM information_schema.processlist ORDER BY id", Vec::<Value>::new()).await?;
        let processes = procs
            .iter()
            .map(|r| {
                // `host` vem como `ip:porta`.
                let host = text(r, 2);
                let (host, port) = match host.as_deref().and_then(|h| h.rsplit_once(':')) {
                    Some((h, p)) => (Some(h.to_string()), p.parse::<i32>().ok()),
                    None => (host, None),
                };
                MonitorProcess { id: n(r, 0), user: text(r, 1), host, port, db: text(r, 3), command: text(r, 4), state: text(r, 5), time: text(r, 6), info: text(r, 7) }
            })
            .collect();
        let pairs = |rows: Vec<mysql_async::Row>| rows.iter().map(|r| (s(r, 0), s(r, 1))).collect::<Vec<_>>();
        let variables = pairs(rows_of(&mut conn, "SHOW GLOBAL VARIABLES", Vec::<Value>::new()).await?);
        let status = pairs(rows_of(&mut conn, "SHOW GLOBAL STATUS", Vec::<Value>::new()).await?);
        Ok(MonitorSnapshot { processes, variables, status })
    }

    /// Encerra a conexão `id` (`KILL`). Devolve `false` se ela já não existia.
    pub async fn monitor_end_process(&self, id: i64) -> Result<bool, DatabaseError> {
        let mut conn = self.dedicated("").await?;
        match conn.query_drop(format!("KILL {id}")).await {
            Ok(()) => Ok(true),
            // 1094: processo desconhecido.
            Err(mysql_async::Error::Server(e)) if e.code == 1094 => Ok(false),
            Err(e) => Err(map_err(e)),
        }
    }
}
