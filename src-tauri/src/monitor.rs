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
