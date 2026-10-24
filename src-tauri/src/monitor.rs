//! Server Monitor: um instante do servidor (processos, variáveis e contadores) e o botão End Process.

use crate::sessions::Sessions;
use atlas_core::DatabaseError;
use atlas_pg::MonitorSnapshot;
use tauri::State;

#[tauri::command]
pub async fn monitor_snapshot(sessions: State<'_, Sessions>, session_id: String) -> Result<MonitorSnapshot, DatabaseError> {
    let s = sessions.get(&session_id, None).await?;
    atlas_pg::monitor_snapshot(&s).await
}

#[tauri::command]
pub async fn monitor_end_process(sessions: State<'_, Sessions>, session_id: String, pid: i32) -> Result<bool, DatabaseError> {
    let s = sessions.get(&session_id, None).await?;
    atlas_pg::monitor_end_process(&s, pid).await
}

/// Comandos registrados desde `after` (0 = todos), o número do último e se o registro está em pausa.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryPage {
    pub entries: Vec<atlas_pg::HistoryEntry>,
    pub last: u64,
    pub paused: bool,
}

#[tauri::command]
pub fn sqllog_read(after: u64) -> HistoryPage {
    let (entries, last) = atlas_pg::history_read(after);
    HistoryPage { entries, last, paused: atlas_pg::history_is_paused() }
}

#[tauri::command]
pub fn sqllog_clear() {
    atlas_pg::history_clear();
}

#[tauri::command]
pub fn sqllog_set_paused(paused: bool) {
    atlas_pg::history_set_paused(paused);
}
