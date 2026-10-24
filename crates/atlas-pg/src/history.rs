//! Registro dos comandos que o Atlas manda ao servidor (a aba History Log): hora, servidor, processo, comando, tempo e erro.
//!
//! Fica só na memória, num buffer circular com limite de entradas e de tamanho. Senhas em `PASSWORD '...'` saem mascaradas.

use serde::Serialize;
use std::collections::VecDeque;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_ENTRIES: usize = 4000;
const MAX_BYTES: usize = 8 * 1024 * 1024;
/// Um comando maior que isso (cargas de `INSERT` em lote) é cortado no registro.
const MAX_TEXT: usize = 16 * 1024;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub seq: u64,
    /// Milissegundos desde 1970-01-01 (a tela formata na hora local).
    pub at_ms: u64,
    /// `PostgreSQL 9.6`.
    pub server: String,
    pub pid: i32,
    /// O comando, ou a mensagem de erro quando `error` é verdadeiro.
    pub text: String,
    /// Segundos que o comando levou; ausente nas mensagens de erro.
    pub secs: Option<f64>,
    pub error: bool,
    /// O comando que falhou (a tela o mostra junto do erro em "Show Errors Only").
    pub failed: bool,
}

static LOG: Mutex<VecDeque<Entry>> = Mutex::new(VecDeque::new());
static SEQ: AtomicU64 = AtomicU64::new(0);
static PAUSED: AtomicBool = AtomicBool::new(false);

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// `PASSWORD 'segredo'` vira `PASSWORD '********'` (sem diferenciar maiúsculas).
fn mask_passwords(sql: &str) -> String {
    let lower = sql.to_ascii_lowercase();
    let mut out = String::with_capacity(sql.len());
    let mut i = 0;
    while let Some(found) = lower[i..].find("password") {
        let at = i + found + "password".len();
        out.push_str(&sql[i..at]);
        let rest = &sql[at..];
        let trimmed = rest.trim_start();
        if let Some(body) = trimmed.strip_prefix('\'') {
            let gap = rest.len() - trimmed.len();
            // Fim do literal: aspas simples que não são dobradas.
            let b = body.as_bytes();
            let mut k = 0;
            let mut end = None;
            while k < b.len() {
                if b[k] == b'\'' {
                    if b.get(k + 1) == Some(&b'\'') {
                        k += 2;
                        continue;
                    }
                    end = Some(k);
                    break;
                }
                k += 1;
            }
            if let Some(e) = end {
                out.push_str(&rest[..gap]);
                out.push_str("'********'");
                i = at + gap + 1 + e + 1;
                continue;
            }
        }
        i = at;
    }
    out.push_str(&sql[i..]);
    out
}

fn push(server: &str, pid: i32, text: String, secs: Option<f64>, error: bool, failed: bool) {
    if PAUSED.load(Ordering::Relaxed) {
        return;
    }
    let mut text = if error { text } else { mask_passwords(&text) };
    if text.len() > MAX_TEXT {
        let mut cut = MAX_TEXT;
        while !text.is_char_boundary(cut) {
            cut -= 1;
        }
        text.truncate(cut);
        text.push_str("...");
    }
    let entry = Entry { seq: SEQ.fetch_add(1, Ordering::Relaxed) + 1, at_ms: now_ms(), server: server.to_string(), pid, text, secs, error, failed };
    let mut log = LOG.lock().unwrap();
    log.push_back(entry);
    let mut bytes: usize = log.iter().map(|e| e.text.len()).sum();
    while log.len() > MAX_ENTRIES || bytes > MAX_BYTES {
        match log.pop_front() {
            Some(e) => bytes -= e.text.len(),
            None => break,
        }
    }
}

/// Registra um comando e, se falhou, a mensagem de erro logo depois.
pub fn record(server: &str, pid: i32, sql: &str, secs: f64, error: Option<String>) {
    push(server, pid, sql.to_string(), Some(secs), false, error.is_some());
    if let Some(message) = error {
        push(server, pid, message, None, true, true);
    }
}

/// Entradas depois da `after` (0 = todas) e o número da última.
pub fn read(after: u64) -> (Vec<Entry>, u64) {
    let log = LOG.lock().unwrap();
    (log.iter().filter(|e| e.seq > after).cloned().collect(), SEQ.load(Ordering::Relaxed))
}

pub fn clear() {
    LOG.lock().unwrap().clear();
}

pub fn set_paused(paused: bool) {
    PAUSED.store(paused, Ordering::Relaxed);
}

pub fn is_paused() -> bool {
    PAUSED.load(Ordering::Relaxed)
}

/// A mensagem de erro como o `psql` a imprime: `ERROR:  mensagem`, `LINE n: trecho` com o cursor, `DETAIL:` e `HINT:`.
pub fn format_error(severity: &str, message: &str, position: Option<u32>, detail: Option<&str>, hint: Option<&str>, sql: &str) -> String {
    let mut out = format!("{severity}:  {message}");
    if let Some(pos) = position {
        // `pos` conta caracteres a partir de 1.
        let upto: String = sql.chars().take((pos as usize).saturating_sub(1)).collect();
        let line_no = upto.matches('\n').count() + 1;
        let line_start = upto.rfind('\n').map(|i| i + 1).unwrap_or(0);
        let col = upto[line_start..].chars().count();
        let chars: Vec<char> = sql.lines().nth(line_no - 1).unwrap_or("").chars().collect();
        // Mostra um trecho de até 40 caracteres ao redor do ponto do erro.
        let from = col.saturating_sub(20).min(chars.len());
        let to = (from + 40).min(chars.len());
        let snippet: String = chars[from..to].iter().collect();
        let (pre, post) = (if from > 0 { "..." } else { "" }, if to < chars.len() { "..." } else { "" });
        let prefix = format!("LINE {line_no}: {pre}");
        out.push_str(&format!("\n{prefix}{snippet}{post}\n{}^", " ".repeat(prefix.chars().count() + col - from)));
    }
    if let Some(d) = detail {
        out.push_str(&format!("\nDETAIL:  {d}"));
    }
    if let Some(h) = hint {
        out.push_str(&format!("\nHINT:  {h}"));
    }
    out
}

/// O erro do driver, como o servidor o devolveu.
pub fn format_driver_error(e: &tokio_postgres::Error, sql: &str) -> String {
    let Some(db) = e.as_db_error() else { return format!("ERROR:  {e}") };
    let position = match db.position() {
        Some(tokio_postgres::error::ErrorPosition::Original(p)) => Some(*p),
        _ => None,
    };
    format_error(db.severity(), db.message(), position, db.detail(), db.hint(), sql)
}

/// O erro já convertido para o Atlas (o `detail` traz DETAIL e HINT juntos).
pub fn format_atlas_error(e: &atlas_core::DatabaseError, sql: &str) -> String {
    // O `detail` do Atlas junta DETAIL e HINT (DETAIL, quebra de linha e HINT): separa de novo.
    let (detail, hint) = match e.detail.as_deref() {
        Some(d) => match d.strip_prefix("HINT: ").map(|h| (None, Some(h))).or_else(|| d.split_once("\nHINT: ").map(|(a, b)| (Some(a), Some(b)))) {
            Some(parts) => parts,
            None => (Some(d), None),
        },
        None => (None, None),
    };
    format_error("ERROR", &e.message, e.position, detail, hint, sql)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passwords_are_masked_and_other_text_is_kept() {
        assert_eq!(mask_passwords("CREATE ROLE a LOGIN PASSWORD 's3''cret' VALID UNTIL 'x'"), "CREATE ROLE a LOGIN PASSWORD '********' VALID UNTIL 'x'");
        assert_eq!(mask_passwords("alter role a with password   'x'; select 'password'"), "alter role a with password   '********'; select 'password'");
        assert_eq!(mask_passwords("SELECT 1"), "SELECT 1");
    }

    #[test]
    fn errors_split_detail_and_hint_like_psql() {
        let mut e = atlas_core::DatabaseError::new(atlas_core::ErrorKind::Server, "boom");
        e.detail = Some("Key (id)=(1) already exists.
HINT: try again".into());
        assert_eq!(format_atlas_error(&e, "SELECT 1"), "ERROR:  boom
DETAIL:  Key (id)=(1) already exists.
HINT:  try again");
        e.detail = Some("HINT: only a hint".into());
        e.position = Some(8);
        assert_eq!(format_atlas_error(&e, "SELECT x"), "ERROR:  boom
LINE 1: SELECT x
               ^
HINT:  only a hint");
    }

    #[test]
    fn the_log_is_ordered_paused_cleared_and_marks_failures() {
        clear();
        set_paused(false);
        let (_, base) = read(0);
        record("PostgreSQL 18", 7, "SELECT 1", 0.001, None);
        record("PostgreSQL 18", 7, "SELECT x", 0.002, Some("ERROR:  column \"x\" does not exist".into()));
        let (entries, last) = read(base);
        assert_eq!(entries.len(), 3);
        assert_eq!(last, base + 3);
        assert!(!entries[0].failed && entries[1].failed && entries[2].error && entries[2].secs.is_none());
        assert!(read(last).0.is_empty());
        set_paused(true);
        record("PostgreSQL 18", 7, "SELECT 2", 0.0, None);
        assert!(read(last).0.is_empty());
        set_paused(false);
        clear();
        assert!(read(0).0.is_empty());
    }
}
