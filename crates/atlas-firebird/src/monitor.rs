//! Server Monitor do Firebird: conexões e comandos em andamento (tabelas `MON$`, Firebird 2.1 em diante), mais as configurações e
//! os contadores do banco.

use super::{Conn, FirebirdSession, map_err, meta};
use atlas_core::{DatabaseError, MonitorProcess, MonitorSnapshot};
use rsfbclient::{Execute, SqlType};

fn cell(r: &[Option<String>], i: usize) -> Option<String> {
    r.get(i).cloned().flatten().map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
}

/// `127.0.0.1/50443` ou `TCPv4:127.0.0.1/50443` → (host, porta).
fn split_address(addr: Option<String>) -> (Option<String>, Option<i32>) {
    let Some(a) = addr else { return (None, None) };
    let a = a.rsplit(':').next().map(str::to_string).filter(|_| a.contains(':') && a.contains('/')).unwrap_or(a);
    match a.rsplit_once('/') {
        Some((h, p)) => (Some(h.to_string()), p.parse().ok()),
        None => (Some(a), None),
    }
}

fn snapshot_on(conn: &mut Conn, version: &str) -> Result<MonitorSnapshot, DatabaseError> {
    // A partir do Firebird 3 as conexões do próprio servidor têm `MON$SYSTEM_FLAG`; o 4 devolve a hora com fuso (o cliente não lê esse tipo).
    let system = if meta::is_v3(conn) { "WHERE COALESCE(a.MON$SYSTEM_FLAG, 0) = 0" } else { "" };
    let procs = meta::query(
        conn,
        &format!(
            "SELECT a.MON$ATTACHMENT_ID, a.MON$USER, a.MON$REMOTE_ADDRESS, a.MON$ATTACHMENT_NAME, a.MON$REMOTE_PROCESS, a.MON$STATE, CAST(a.MON$TIMESTAMP AS TIMESTAMP), \
             (SELECT FIRST 1 s.MON$SQL_TEXT FROM MON$STATEMENTS s WHERE s.MON$ATTACHMENT_ID = a.MON$ATTACHMENT_ID AND s.MON$STATE = 1) \
             FROM MON$ATTACHMENTS a {system} ORDER BY 1"
        ),
        vec![],
    )?;
    let processes = procs
        .iter()
        .map(|r| {
            let (host, port) = split_address(cell(r, 2));
            let state = match cell(r, 5).as_deref() {
                Some("1") => "active",
                _ => "idle",
            };
            MonitorProcess {
                id: cell(r, 0).and_then(|v| v.parse().ok()).unwrap_or(0),
                user: cell(r, 1),
                host,
                port,
                db: cell(r, 3),
                command: cell(r, 4),
                state: Some(state.to_string()),
                time: cell(r, 6),
                info: cell(r, 7),
            }
        })
        .collect();

    let mut variables: Vec<(String, String)> = vec![("Firebird version".into(), version.to_string())];
    let mut status: Vec<(String, String)> = Vec::new();
    // `MON$DATABASE` tem uma linha com as configurações e os números do banco.
    let names = meta::query(
        conn,
        "SELECT TRIM(rf.RDB$FIELD_NAME), f.RDB$FIELD_TYPE FROM RDB$RELATION_FIELDS rf JOIN RDB$FIELDS f ON f.RDB$FIELD_NAME = rf.RDB$FIELD_SOURCE WHERE rf.RDB$RELATION_NAME = 'MON$DATABASE' ORDER BY rf.RDB$FIELD_POSITION",
        vec![],
    )?;
    let list: Vec<String> = names
        .iter()
        .filter_map(|n| {
            let name = cell(n, 0)?;
            Some(match cell(n, 1).as_deref() {
                Some("29") => format!("CAST({name} AS TIMESTAMP)"),
                Some("28") => format!("CAST({name} AS TIME)"),
                _ => name,
            })
        })
        .collect();
    let db = meta::query(conn, &format!("SELECT {} FROM MON$DATABASE", list.join(", ")), vec![])?;
    if let Some(row) = db.first() {
        for (i, n) in names.iter().enumerate() {
            let Some(name) = cell(n, 0) else { continue };
            let label = name.trim_start_matches("MON$").to_string();
            let value = row.get(i).cloned().flatten().unwrap_or_default().trim().to_string();
            let counter = matches!(label.as_str(), "OLDEST_TRANSACTION" | "OLDEST_ACTIVE" | "OLDEST_SNAPSHOT" | "NEXT_TRANSACTION" | "PAGES" | "CREATION_DATE" | "STAT_ID");
            if counter { status.push((label, value)) } else { variables.push((label, value)) }
        }
    }
    let count = |conn: &mut Conn, sql: &str| -> String { meta::query(conn, sql, vec![]).ok().and_then(|r| r.first().and_then(|c| cell(c, 0))).unwrap_or_default() };
    status.push(("Attachments".into(), count(conn, "SELECT COUNT(*) FROM MON$ATTACHMENTS WHERE COALESCE(MON$SYSTEM_FLAG, 0) = 0")));
    status.push(("Running statements".into(), count(conn, "SELECT COUNT(*) FROM MON$STATEMENTS WHERE MON$STATE = 1")));
    status.push(("Transactions".into(), count(conn, "SELECT COUNT(*) FROM MON$TRANSACTIONS")));
    status.push(("Tables".into(), count(conn, "SELECT COUNT(*) FROM RDB$RELATIONS WHERE COALESCE(RDB$SYSTEM_FLAG, 0) = 0 AND RDB$VIEW_BLR IS NULL")));
    status.push(("Views".into(), count(conn, "SELECT COUNT(*) FROM RDB$RELATIONS WHERE COALESCE(RDB$SYSTEM_FLAG, 0) = 0 AND RDB$VIEW_BLR IS NOT NULL")));
    status.push(("Procedures".into(), count(conn, "SELECT COUNT(*) FROM RDB$PROCEDURES WHERE COALESCE(RDB$SYSTEM_FLAG, 0) = 0")));
    status.push(("Triggers".into(), count(conn, "SELECT COUNT(*) FROM RDB$TRIGGERS WHERE COALESCE(RDB$SYSTEM_FLAG, 0) = 0")));
    Ok(MonitorSnapshot { processes, variables, status })
}

impl FirebirdSession {
    /// Um instante do servidor: as conexões, as configurações e os contadores do banco.
    pub fn monitor_snapshot(&self) -> Result<MonitorSnapshot, DatabaseError> {
        let version = self.version.clone();
        self.with(|c| snapshot_on(c, &version))
    }

    /// Encerra a conexão `id` (apaga a linha em `MON$ATTACHMENTS`). Devolve `false` se ela já não existia.
    pub fn monitor_end_process(&self, id: i64) -> Result<bool, DatabaseError> {
        let mut conn = self.dedicated()?;
        let n = conn.execute("DELETE FROM MON$ATTACHMENTS WHERE MON$ATTACHMENT_ID = ?", vec![SqlType::Integer(id)]).map_err(map_err)?;
        Ok(n > 0)
    }
}
