//! Server Monitor do SQLite: o arquivo não tem servidor nem processos; a tela mostra as configurações (`PRAGMA`) e os números do arquivo.

use super::{SqliteSession, map_err};
use atlas_core::{DatabaseError, MonitorSnapshot};
use rusqlite::Connection;

/// Os `PRAGMA` de leitura que descrevem o banco.
const SETTINGS: &[&str] = &[
    "application_id", "auto_vacuum", "busy_timeout", "cache_size", "case_sensitive_like", "encoding", "foreign_keys", "journal_mode", "journal_size_limit",
    "locking_mode", "max_page_count", "mmap_size", "page_size", "query_only", "recursive_triggers", "secure_delete", "synchronous", "temp_store", "user_version", "wal_autocheckpoint",
];

fn pragma(c: &Connection, name: &str) -> Option<String> {
    c.query_row(&format!("PRAGMA {name}"), [], |r| {
        r.get_ref(0).map(|v| match v {
            rusqlite::types::ValueRef::Null => String::new(),
            rusqlite::types::ValueRef::Integer(i) => i.to_string(),
            rusqlite::types::ValueRef::Real(f) => f.to_string(),
            rusqlite::types::ValueRef::Text(t) => String::from_utf8_lossy(t).into_owned(),
            rusqlite::types::ValueRef::Blob(b) => format!("{} bytes", b.len()),
        })
    })
    .ok()
}

impl SqliteSession {
    pub fn monitor_snapshot(&self) -> Result<MonitorSnapshot, DatabaseError> {
        let path = self.path.display().to_string();
        let bytes = std::fs::metadata(&self.path).map(|m| m.len()).unwrap_or(0);
        self.with(|c| {
            let variables = SETTINGS.iter().filter_map(|n| pragma(c, n).map(|v| ((*n).to_string(), v))).collect();
            let count = |kind: &str| c.query_row("SELECT count(*) FROM sqlite_master WHERE type = ?1", [kind], |r| r.get::<_, i64>(0)).map_err(map_err);
            let pages = pragma(c, "page_count").unwrap_or_default();
            let free = pragma(c, "freelist_count").unwrap_or_default();
            let status = vec![
                ("File".to_string(), path.clone()),
                ("Size (bytes)".to_string(), bytes.to_string()),
                ("Pages".to_string(), pages),
                ("Free pages".to_string(), free),
                ("SQLite version".to_string(), self.version.clone()),
                ("Tables".to_string(), count("table")?.to_string()),
                ("Views".to_string(), count("view")?.to_string()),
                ("Indexes".to_string(), count("index")?.to_string()),
                ("Triggers".to_string(), count("trigger")?.to_string()),
            ];
            Ok(MonitorSnapshot { processes: Vec::new(), variables, status })
        })
    }
}
