//! Console do SQL Server (Tools → Console): um terminal no estilo do `sqlcmd`, com os resultados alinhados como no `psql`.
//!
//! A tela só guarda a rolagem e o que se digita; aqui ficam os comandos com `.`, a execução e a formatação do texto.

use super::MssqlSession;
use crate::meta::{rows_of, s};
use atlas_core::{DatabaseError, ResultSet};
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

const DOT_COMMANDS: &[&str] = &[".databases", ".expanded", ".help", ".quit", ".schemas", ".tables", ".timing"];

const KEYWORDS: &[&str] = &[
    "ALTER", "AND", "AS", "ASC", "BEGIN", "BETWEEN", "BY", "CASE", "COMMIT", "CREATE", "CROSS", "DELETE", "DESC", "DISTINCT", "DROP", "ELSE", "END", "EXEC", "EXISTS", "FROM", "GO", "GROUP",
    "HAVING", "IN", "INDEX", "INNER", "INSERT", "INTO", "IS", "JOIN", "LEFT", "LIKE", "NOT", "NULL", "OFFSET", "ON", "OR", "ORDER", "OUTER", "ROLLBACK", "SELECT", "SET", "TOP",
    "TABLE", "THEN", "TRIGGER", "UNION", "UNIQUE", "UPDATE", "VALUES", "VIEW", "WHEN", "WHERE", "WITH",
];

/// O texto digitado já forma um comando completo? Termina em `;` (fora de aspas e comentários) ou é um comando com `.` numa linha só.
pub fn is_complete(buffer: &str) -> bool {
    let t = buffer.trim_start();
    if t.starts_with('.') {
        return !t.trim().is_empty();
    }
    if buffer.lines().last().is_some_and(|l| l.trim().eq_ignore_ascii_case("go")) {
        return true;
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

/// Divide o texto nos lotes separados por uma linha `GO`.
fn batches(input: &str) -> Vec<String> {
    let mut out = vec![String::new()];
    for line in input.lines() {
        if line.trim().eq_ignore_ascii_case("go") {
            out.push(String::new());
        } else if let Some(cur) = out.last_mut() {
            cur.push_str(line);
            cur.push('\n');
        }
    }
    out.into_iter().filter(|b| !b.trim().is_empty()).collect()
}

impl MssqlSession {
    async fn names(&self, database: &str, sql: &str) -> Vec<String> {
        let Ok(client) = self.client(database).await else { return Vec::new() };
        let mut conn = client.lock().await;
        rows_of(&mut conn, sql, &[]).await.map(|r| r.iter().map(|x| s(x, 0)).collect()).unwrap_or_default()
    }

    async fn dot_command(&self, database: &str, input: &str, options: &mut ConsoleOptions, out: &mut ConsoleOutput) -> String {
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
            ".help" | ".?" => ".tables              list the tables and views of the database\n.schemas             list the schemas\n.databases           list the databases\n.timing [on|off]     show the time each command takes\n.expanded [on|off]   one record per block\n.quit                close the console\nA line with GO ends a batch.\n".into(),
            ".tables" => list("List of relations", self.names(database, "SELECT sc.name + '.' + o.name FROM sys.objects o JOIN sys.schemas sc ON sc.schema_id = o.schema_id WHERE o.type IN ('U', 'V') AND o.is_ms_shipped = 0 ORDER BY 1").await),
            ".schemas" => list("List of schemas", self.names(database, "SELECT name FROM sys.schemas WHERE name NOT IN ('sys', 'INFORMATION_SCHEMA', 'guest') AND name NOT LIKE 'db[_]%' ORDER BY name").await),
            ".databases" => list("List of databases", self.names("", "SELECT name FROM sys.databases WHERE state = 0 ORDER BY name").await),
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

    pub async fn console_run(&self, database: &str, input: &str, mut options: ConsoleOptions) -> Result<ConsoleOutput, DatabaseError> {
        let mut out = ConsoleOutput::default();
        let trimmed = input.trim();
        if trimmed.starts_with('.') {
            out.text = self.dot_command(database, trimmed, &mut options, &mut out).await;
            out.options = options;
            return Ok(out);
        }
        for batch in batches(trimmed) {
            match self.execute(database, batch.trim().trim_end_matches(';'), 10_000).await {
                Ok(outcome) => {
                    for st in &outcome.statements {
                        match &st.result {
                            Some(rs) => out.text.push_str(&if options.expanded { expanded(rs) } else { table(rs) }),
                            None => out.text.push_str("Command(s) completed.\n"),
                        }
                        if options.timing {
                            out.text.push_str(&format!("Time: {:.3} ms\n", (st.query_time + st.fetch_time) * 1000.0));
                        }
                    }
                }
                Err(e) => {
                    out.text.push_str(&format!("ERROR:  {}\n", e.message));
                    break;
                }
            }
        }
        out.options = options;
        Ok(out)
    }

    /// Tab: tabelas, views, colunas e palavras-chave que começam com a palavra antes do cursor.
    pub async fn console_complete(&self, database: &str, text: &str) -> Result<Completion, DatabaseError> {
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
            let objects = self.names(database, "SELECT o.name FROM sys.objects o WHERE o.type IN ('U', 'V') AND o.is_ms_shipped = 0 ORDER BY o.name").await;
            let mut columns = self.names(database, "SELECT DISTINCT c.name FROM sys.columns c JOIN sys.objects o ON o.object_id = c.object_id WHERE o.type IN ('U', 'V') AND o.is_ms_shipped = 0 ORDER BY c.name").await;
            columns.dedup();
            candidates.extend(objects.into_iter().chain(columns).filter(|n| n.to_ascii_lowercase().starts_with(&lower)));
            candidates.extend(KEYWORDS.iter().filter(|k| k.to_ascii_lowercase().starts_with(&lower)).map(|k| k.to_string()));
        }
        candidates.dedup();
        let append = if candidates.len() == 1 { " ".to_string() } else { String::new() };
        Ok(Completion { start, candidates, append })
    }

    /// O texto do prompt (`atlas_demo=>`).
    pub async fn console_prompt(&self, database: &str) -> Result<String, DatabaseError> {
        Ok(format!("{}=>", if database.is_empty() { self.initial_database() } else { database }))
    }
}
