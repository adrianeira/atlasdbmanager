//! Console do MySQL (Tools → Console): um terminal no estilo do `mysql`, com os resultados alinhados como no `psql`.
//!
//! A tela só guarda a rolagem e o que se digita; aqui ficam os comandos com `.`, a execução e a formatação do texto.

use super::MysqlSession;
use crate::meta::{rows_of, s};
use mysql_async::Value;
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

const DOT_COMMANDS: &[&str] = &[".databases", ".expanded", ".help", ".quit", ".tables", ".timing"];

const KEYWORDS: &[&str] = &[
    "ALTER", "AND", "AS", "ASC", "BEGIN", "BETWEEN", "BY", "CASE", "COMMIT", "CREATE", "CROSS", "DELETE", "DESC", "DESCRIBE", "DISTINCT", "DROP", "ELSE", "END", "EXISTS", "FROM", "GROUP",
    "HAVING", "IN", "INDEX", "INNER", "INSERT", "INTO", "IS", "JOIN", "LEFT", "LIKE", "LIMIT", "NOT", "NULL", "OFFSET", "ON", "OR", "ORDER", "OUTER", "ROLLBACK", "SELECT", "SET", "SHOW",
    "TABLE", "THEN", "TRIGGER", "UNION", "UNIQUE", "UPDATE", "USE", "VALUES", "VIEW", "WHEN", "WHERE", "WITH",
];

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

impl MysqlSession {
    async fn names(&self, database: &str, sql: &str) -> Vec<String> {
        let Ok(client) = self.client(database).await else { return Vec::new() };
        let mut conn = client.lock().await;
        rows_of(&mut conn, sql, Vec::<Value>::new()).await.map(|r| r.iter().map(|x| s(x, 0)).collect()).unwrap_or_default()
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
            ".help" | ".?" => ".tables              list the tables and views of the database\n.databases           list the databases\n.timing [on|off]     show the time each command takes\n.expanded [on|off]   one record per block\n.quit                close the console\nA command ends with a semicolon.\n".into(),
            ".tables" => list("List of relations", self.names(database, "SELECT table_name FROM information_schema.tables WHERE table_schema = DATABASE() ORDER BY 1").await),
            ".databases" => list("List of databases", self.names("", "SELECT schema_name FROM information_schema.schemata ORDER BY 1").await),
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
        match self.execute(database, trimmed.trim_end_matches(';'), 10_000).await {
            Ok(outcome) => {
                for st in &outcome.statements {
                    match &st.result {
                        Some(rs) => out.text.push_str(&if options.expanded { expanded(rs) } else { table(rs) }),
                        None => out.text.push_str(&format!("Query OK, {} row(s) affected\n", st.rows_affected.unwrap_or(0))),
                    }
                    if options.timing {
                        out.text.push_str(&format!("Time: {:.3} ms\n", (st.query_time + st.fetch_time) * 1000.0));
                    }
                }
            }
            Err(e) => out.text.push_str(&format!("ERROR {}:  {}\n", e.code.as_deref().unwrap_or(""), e.message)),
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
            let objects = self.names(database, "SELECT table_name FROM information_schema.tables WHERE table_schema = DATABASE() ORDER BY table_name").await;
            let mut columns = self.names(database, "SELECT DISTINCT column_name FROM information_schema.columns WHERE table_schema = DATABASE() ORDER BY column_name").await;
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
        Ok(format!("{}=>", if database.is_empty() { self.initial_db.as_str() } else { database }))
    }
}
