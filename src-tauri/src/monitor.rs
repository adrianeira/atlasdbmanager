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

/// Console (estilo psql): `input` é um comando completo, SQL ou com `\`. `\c banco` troca o banco se ele existir.
#[tauri::command]
pub async fn console_run(
    sessions: State<'_, Sessions>,
    session_id: String,
    database: String,
    input: String,
    options: atlas_pg::ConsoleOptions,
) -> Result<atlas_pg::ConsoleOutput, DatabaseError> {
    let s = sessions.get(&session_id, Some(&database)).await?;
    let mut out = atlas_pg::console_run(&s, &input, options).await;
    if let Some(target) = out.switch_db.clone() {
        match sessions.get(&session_id, Some(&target)).await {
            Ok(other) => match other.client.simple_query("SELECT 1").await {
                Ok(_) => out.text.push_str(&format!("You are now connected to database \"{target}\" as user \"{}\".\n", other.server.current_user)),
                Err(e) => {
                    out.switch_db = None;
                    out.text.push_str(&format!("connection to database \"{target}\" failed: {e}\n"));
                }
            },
            Err(e) => {
                out.switch_db = None;
                out.text.push_str(&format!("connection to database \"{target}\" failed: {}\n", e.message));
            }
        }
    }
    Ok(out)
}

/// O texto do prompt (`atlas_demo=#`): `#` para superusuário, `>` para os demais.
#[tauri::command]
pub async fn console_prompt(sessions: State<'_, Sessions>, session_id: String, database: String) -> Result<String, DatabaseError> {
    let s = sessions.get(&session_id, Some(&database)).await?;
    let row = s.client.query_one("SELECT current_database()::text, (SELECT rolsuper FROM pg_catalog.pg_roles WHERE rolname = current_user)", &[]).await.map_err(|e| atlas_pg::map_error(&e))?;
    let (db, superuser): (String, Option<bool>) = (row.get(0), row.get(1));
    Ok(format!("{db}={}", if superuser.unwrap_or(false) { "#" } else { ">" }))
}

/// `true` se o texto já forma um comando completo (termina em `;` ou é um `\comando`).
#[tauri::command]
pub fn console_is_complete(input: String) -> bool {
    atlas_pg::console_is_complete(&input)
}
