//! Console do SQLite (Tools → Console): um terminal no estilo do `sqlite3`, com os resultados alinhados como no `psql`.
//!
//! A tela só guarda a rolagem e o que se digita; aqui ficam os comandos com `.`, a execução e a formatação do texto.

use super::execute_on;
use atlas_core::{DatabaseError, ResultSet};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Ajustes da sessão do console (`.timing` e `.expanded`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConsoleOptions {
    pub timing: bool,
    pub expanded: bool,
    #[serde(default)]
    pub vars: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConsoleOutput {
    pub text: String,
    pub switch_db: Option<String>,
    pub quit: bool,
    pub watch: Option<f64>,
    pub options: ConsoleOptions,
}

/// O que a tecla Tab completa: onde a palavra começa, as opções e o que acrescentar quando só há uma.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Completion {
    pub start: usize,
    pub candidates: Vec<String>,
    pub append: String,
}

const DOT_COMMANDS: &[&str] = &[".expanded", ".help", ".indexes", ".quit", ".schema", ".tables", ".timing"];

const KEYWORDS: &[&str] = &[
    "ALTER", "AND", "AS", "ASC", "BEGIN", "BETWEEN", "BY", "CASE", "COMMIT", "CREATE", "CROSS", "DELETE", "DESC", "DISTINCT", "DROP", "ELSE", "END", "EXISTS", "EXPLAIN", "FROM", "GROUP",
    "HAVING", "IN", "INDEX", "INNER", "INSERT", "INTO", "IS", "JOIN", "LEFT", "LIKE", "LIMIT", "NOT", "NULL", "OFFSET", "ON", "OR", "ORDER", "OUTER", "PRAGMA", "ROLLBACK", "SELECT", "SET",
    "TABLE", "THEN", "TRIGGER", "UNION", "UNIQUE", "UPDATE", "VALUES", "VIEW", "WHEN", "WHERE", "WITH",
];

/// O texto digitado já forma um comando completo? Termina em `;` (fora de aspas e comentários) ou é um comando com `.` numa linha só.
pub fn is_complete(buffer: &str) -> bool {
    let t = buffer.trim_start();
    if t.starts_with('.') {
        return !t.trim().is_empty();
    }
    let b = buffer.as_bytes();
    let (mut i, mut ended) = (0, false);
    while i < b.len() {
        match b[i] {
            c if c.is_ascii_whitespace() => i += 1,
            b';' => {
                ended = true;
                i += 1;
            }
            b'-' if b.get(i + 1) == Some(&b'-') => {
                while i < b.len() && b[i] != b'\n' {
                    i += 1;
                }
            }
            b'/' if b.get(i + 1) == Some(&b'*') => {
                i += 2;
                while i + 1 < b.len() && !(b[i] == b'*' && b[i + 1] == b'/') {
                    i += 1;
                }
                i += 2;
            }
            q @ (b'\'' | b'"' | b'`') => {
                ended = false;
                i += 1;
                while i < b.len() && b[i] != q {
                    i += 1;
                }
                i += 1;
            }
            _ => {
                ended = false;
                i += 1;
            }
        }
    }
    ended
}

fn is_number(s: &str) -> bool {
    !s.is_empty() && s.parse::<f64>().is_ok()
}

/// A tabela alinhada: cabeçalho centrado, números à direita, `(N rows)` no fim.
fn table(rs: &ResultSet) -> String {
    let n = rs.columns.len();
    let cell = |v: &Option<String>| v.clone().unwrap_or_default().replace('\n', " ");
    let mut widths: Vec<usize> = rs.columns.iter().map(|c| c.name.chars().count()).collect();
    let mut numeric = vec![true; n];
    for r in &rs.rows {
        for (i, v) in r.iter().enumerate().take(n) {
            let s = cell(v);
            widths[i] = widths[i].max(s.chars().count());
            if v.is_some() && !is_number(&s) {
                numeric[i] = false;
            }
        }
    }
    let mut out = String::new();
    let head: Vec<String> = rs.columns.iter().enumerate().map(|(i, c)| {
        let pad = widths[i] - c.name.chars().count();
        let (l, r) = (pad / 2, pad - pad / 2);
        format!("{}{}{}", " ".repeat(l), c.name, " ".repeat(r))
    }).collect();
    out.push_str(&format!(" {} \n", head.join(" | ")));
    out.push_str(&format!("-{}-\n", widths.iter().map(|w| "-".repeat(*w)).collect::<Vec<_>>().join("-+-")));
    for r in &rs.rows {
        let cells: Vec<String> = (0..n)
            .map(|i| {
                let s = r.get(i).map(cell).unwrap_or_default();
                let pad = widths[i].saturating_sub(s.chars().count());
                if numeric[i] { format!("{}{s}", " ".repeat(pad)) } else if i + 1 == n { s } else { format!("{s}{}", " ".repeat(pad)) }
            })
            .collect();
        out.push_str(&format!(" {}\n", cells.join(" | ").trim_end()));
    }
    out.push_str(&format!("({} row{})\n", rs.rows.len(), if rs.rows.len() == 1 { "" } else { "s" }));
    out
}

/// Um registro por bloco (`.expanded`).
fn expanded(rs: &ResultSet) -> String {
    let width = rs.columns.iter().map(|c| c.name.chars().count()).max().unwrap_or(0);
    let mut out = String::new();
    for (k, r) in rs.rows.iter().enumerate() {
        out.push_str(&format!("-[ RECORD {} ]{}\n", k + 1, "-".repeat(width + 4)));
        for (i, c) in rs.columns.iter().enumerate() {
            let v = r.get(i).cloned().flatten().unwrap_or_default().replace('\n', " ");
            out.push_str(&format!("{:<width$} | {v}\n", c.name));
        }
    }
    if rs.rows.is_empty() {
        out.push_str("(0 rows)\n");
    }
    out
}

fn names(conn: &Connection, sql: &str) -> Vec<String> {
    conn.prepare(sql).and_then(|mut s| s.query_map([], |r| r.get::<_, String>(0))?.collect::<Result<Vec<_>, _>>()).unwrap_or_default()
}

fn dot_command(conn: &Connection, input: &str, options: &mut ConsoleOptions, out: &mut ConsoleOutput) -> String {
    let mut parts = input.trim().splitn(2, char::is_whitespace);
    let cmd = parts.next().unwrap_or_default();
    let arg = parts.next().unwrap_or_default().trim();
    let list = |title: &str, items: Vec<String>| {
        if items.is_empty() {
            return "Did not find any relations.\n".to_string();
        }
        format!("{title}\n{}\n", items.iter().map(|n| format!("  {n}")).collect::<Vec<_>>().join("\n"))
    };
    match cmd {
        ".help" | ".?" => ".tables              list the tables and views\n.schema [name]       show the CREATE statements (of one object, or all)\n.indexes [table]     list the indexes\n.timing [on|off]     show the time each command takes\n.expanded [on|off]   one record per block\n.quit                close the console\n".into(),
        ".tables" => list("List of relations", names(conn, "SELECT name FROM sqlite_master WHERE type IN ('table', 'view') AND name NOT LIKE 'sqlite\\_%' ESCAPE '\\' ORDER BY name")),
        ".indexes" => {
            let sql = if arg.is_empty() { "SELECT name FROM sqlite_master WHERE type = 'index' ORDER BY name".to_string() } else { format!("SELECT name FROM sqlite_master WHERE type = 'index' AND tbl_name = '{}' ORDER BY name", arg.replace('\'', "''")) };
            list("List of indexes", names(conn, &sql))
        }
        ".schema" => {
            let sql = if arg.is_empty() { "SELECT sql || ';' FROM sqlite_master WHERE sql IS NOT NULL AND name NOT LIKE 'sqlite\\_%' ESCAPE '\\' ORDER BY rowid".to_string() } else { format!("SELECT sql || ';' FROM sqlite_master WHERE sql IS NOT NULL AND (name = '{0}' OR tbl_name = '{0}') ORDER BY rowid", arg.replace('\'', "''")) };
            let all = names(conn, &sql);
            if all.is_empty() { format!("Did not find any object named \"{arg}\".\n") } else { format!("{}\n", all.join("\n")) }
        }
        ".timing" | ".expanded" | ".x" => {
            let flag = if cmd == ".timing" { &mut options.timing } else { &mut options.expanded };
            *flag = match arg.to_ascii_lowercase().as_str() {
                "on" | "1" | "true" => true,
                "off" | "0" | "false" => false,
                _ => !*flag,
            };
            let label = if cmd == ".timing" { "Timing" } else { "Expanded display" };
            format!("{label} is {}.\n", if *flag { "on" } else { "off" })
        }
        ".quit" | ".q" | ".exit" => {
            out.quit = true;
            String::new()
        }
        other => format!("Unknown command: {other}. Type .help for the list.\n"),
    }
}

/// A etiqueta de um comando que não devolve linhas, como o `psql`.
fn tag(sql: &str, affected: u64) -> String {
    let word = sql.split_whitespace().next().unwrap_or_default().to_ascii_uppercase();
    match word.as_str() {
        "INSERT" => format!("INSERT 0 {affected}"),
        "UPDATE" | "DELETE" => format!("{word} {affected}"),
        "" => String::new(),
        _ => word,
    }
}

pub(crate) fn run_on(conn: &Connection, input: &str, mut options: ConsoleOptions) -> ConsoleOutput {
    let mut out = ConsoleOutput::default();
    let trimmed = input.trim();
    if trimmed.starts_with('.') {
        out.text = dot_command(conn, trimmed, &mut options, &mut out);
        out.options = options;
        return out;
    }
    match execute_on(conn, trimmed, 10_000) {
        Ok(outcome) => {
            for st in &outcome.statements {
                match &st.result {
                    Some(rs) => out.text.push_str(&if options.expanded { expanded(rs) } else { table(rs) }),
                    None => out.text.push_str(&format!("{}\n", tag(&st.sql, st.rows_affected.unwrap_or(0)))),
                }
                if options.timing {
                    out.text.push_str(&format!("Time: {:.3} ms\n", (st.query_time + st.fetch_time) * 1000.0));
                }
            }
        }
        Err(e) => out.text = format!("ERROR:  {}\n", e.message),
    }
    out.options = options;
    out
}

/// Tab: tabelas, views, colunas e palavras-chave que começam com a palavra antes do cursor.
pub(crate) fn complete_on(conn: &Connection, text: &str) -> Completion {
    let chars: Vec<char> = text.chars().collect();
    let mut start = chars.len();
    while start > 0 && (chars[start - 1].is_alphanumeric() || matches!(chars[start - 1], '_' | '.')) {
        start -= 1;
    }
    let word: String = chars[start..].iter().collect();
    let lower = word.to_ascii_lowercase();
    let mut candidates: Vec<String> = Vec::new();
    if word.starts_with('.') && text.trim_start().starts_with(&word) {
        candidates.extend(DOT_COMMANDS.iter().filter(|c| c.starts_with(&lower)).map(|c| c.to_string()));
    } else if !word.is_empty() {
        let objects = names(conn, "SELECT name FROM sqlite_master WHERE type IN ('table', 'view') AND name NOT LIKE 'sqlite\\_%' ESCAPE '\\' ORDER BY name");
        let mut columns = Vec::new();
        for o in &objects {
            columns.extend(names(conn, &format!("SELECT name FROM pragma_table_info({})", format_args!("'{}'", o.replace('\'', "''")))));
        }
        columns.sort();
        columns.dedup();
        candidates.extend(objects.into_iter().chain(columns).filter(|n| n.to_ascii_lowercase().starts_with(&lower)));
        candidates.extend(KEYWORDS.iter().filter(|k| k.to_ascii_lowercase().starts_with(&lower)).map(|k| k.to_string()));
    }
    candidates.dedup();
    let append = if candidates.len() == 1 { " ".to_string() } else { String::new() };
    Completion { start, candidates, append }
}

pub(crate) fn prompt(_conn: &Connection) -> Result<String, DatabaseError> {
    Ok("main=>".to_string())
}
