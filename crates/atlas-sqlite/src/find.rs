//! "Find in Database" do SQLite: procura texto na estrutura (nomes de tabelas, campos, índices, gatilhos e views, mais o corpo das
//! views) ou nos dados das tabelas. Os formatos de resposta são os mesmos do PostgreSQL.

use super::{map_err, open_connection, quote_ident};
use atlas_core::{DatabaseError, ErrorKind};
use regex::{Regex, RegexBuilder};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

/// Máximo de ocorrências devolvidas (acima disso a busca avisa que foi cortada).
const MAX_OCCURRENCES: usize = 2_000;
/// Ocorrências mostradas por objeto no corpo de uma view.
const MAX_PER_BODY: usize = 100;
/// Janela do trecho mostrado: caracteres antes da ocorrência e tamanho total.
const SNIPPET_BEFORE: usize = 20;
const SNIPPET_LEN: usize = 44;
/// O único schema do SQLite.
const SCHEMA: &str = "main";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FindMode {
    Contains,
    Whole,
    Prefix,
    Regex,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FindOptions {
    pub text: String,
    pub schema: Option<String>,
    pub mode: FindMode,
    pub case_insensitive: bool,
    pub tables: bool,
    pub views: bool,
    #[serde(default)]
    pub matviews: bool,
    #[serde(default)]
    pub functions: bool,
    pub indexes: bool,
    pub triggers: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FindMatch {
    pub what: String,
    pub text: String,
    pub ranges: Vec<[usize; 2]>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FindObject {
    pub schema: String,
    pub kind: String,
    pub name: String,
    pub identity: Option<String>,
    pub matches: Vec<FindMatch>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FindResult {
    pub objects: Vec<FindObject>,
    pub occurrences: usize,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DataMatch {
    pub schema: String,
    pub table: String,
    pub count: i64,
    /// Consulta que lista as linhas achadas, para abrir no editor.
    pub sql: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FindDataResult {
    pub tables: Vec<DataMatch>,
    pub scanned: usize,
    pub total: usize,
    pub skipped: usize,
    pub cancelled: bool,
}

/// O padrão (expressão regular) do modo escolhido, sem a marca de maiúsculas e minúsculas.
fn pattern_text(o: &FindOptions) -> String {
    let text = o.text.trim_end_matches(['\r', '\n']);
    let escaped = regex::escape(text);
    match o.mode {
        FindMode::Contains => escaped,
        FindMode::Whole => format!(r"\b{escaped}\b"),
        FindMode::Prefix => format!(r"\b{escaped}"),
        FindMode::Regex => text.to_string(),
    }
}

fn build_regex(o: &FindOptions) -> Result<Regex, DatabaseError> {
    RegexBuilder::new(&pattern_text(o))
        .case_insensitive(o.case_insensitive)
        .build()
        .map_err(|e| DatabaseError::new(ErrorKind::InvalidInput, format!("invalid regular expression: {e}")))
}

fn char_index(text: &str, byte: usize) -> usize {
    text[..byte].chars().count()
}

fn name_match(what: &str, name: &str, re: &Regex) -> Option<FindMatch> {
    let ranges: Vec<[usize; 2]> = re.find_iter(name).filter(|m| !m.is_empty()).map(|m| [char_index(name, m.start()), char_index(name, m.end())]).collect();
    if ranges.is_empty() { None } else { Some(FindMatch { what: what.to_string(), text: name.to_string(), ranges }) }
}

/// Trecho de `text` ao redor de uma ocorrência, com os espaços em branco achatados; devolve o texto e o destaque.
fn snippet(chars: &[char], match_start: usize, match_end: usize) -> (String, [usize; 2]) {
    let n = chars.len();
    let mut start = match_start.saturating_sub(SNIPPET_BEFORE);
    let end = (start + SNIPPET_LEN).min(n);
    if end - start < SNIPPET_LEN {
        start = end.saturating_sub(SNIPPET_LEN).min(match_start);
    }
    let end = (start + SNIPPET_LEN).min(n).max(match_end.min(n));
    let lead = if start > 0 { "..." } else { "" };
    let mut out = String::from(lead);
    let mut out_len = lead.chars().count();
    let (mut hl_start, mut hl_end) = (out_len, out_len);
    let mut last_space = false;
    for (i, &c) in chars.iter().enumerate().take(end).skip(start) {
        if i == match_start {
            hl_start = out_len;
        }
        if i == match_end {
            hl_end = out_len;
        }
        let space = c.is_whitespace();
        if space && last_space {
            continue;
        }
        out.push(if space { ' ' } else { c });
        out_len += 1;
        last_space = space;
    }
    if match_end >= end {
        hl_end = out_len;
    }
    if end < n {
        out.push_str("...");
    }
    (out, [hl_start, hl_end.max(hl_start)])
}

fn body_matches(what: &str, body: &str, re: &Regex, budget: &mut usize) -> Vec<FindMatch> {
    let chars: Vec<char> = body.chars().collect();
    let mut out = Vec::new();
    for m in re.find_iter(body).filter(|m| !m.is_empty()).take(MAX_PER_BODY) {
        if *budget == 0 {
            break;
        }
        *budget -= 1;
        let (text, range) = snippet(&chars, char_index(body, m.start()), char_index(body, m.end()));
        out.push(FindMatch { what: what.to_string(), text, ranges: vec![range] });
    }
    out
}

fn in_scope(o: &FindOptions) -> bool {
    o.schema.as_deref().is_none_or(|s| s == SCHEMA)
}

/// Lê uma coluna de texto de todas as linhas de uma consulta.
fn strings(conn: &Connection, sql: &str, params: &[&str]) -> Result<Vec<String>, DatabaseError> {
    let mut stmt = conn.prepare(sql).map_err(map_err)?;
    let rows = stmt.query_map(rusqlite::params_from_iter(params.iter()), |r| r.get::<_, String>(0)).map_err(map_err)?.collect::<Result<Vec<_>, _>>().map_err(map_err)?;
    Ok(rows)
}

/// Tabelas do usuário (as internas do SQLite ficam de fora).
fn user_tables(conn: &Connection) -> Result<Vec<String>, DatabaseError> {
    strings(conn, "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite\\_%' ESCAPE '\\' ORDER BY name", &[])
}

pub(crate) fn find_on(conn: &Connection, o: &FindOptions) -> Result<FindResult, DatabaseError> {
    let empty = FindResult { objects: Vec::new(), occurrences: 0, truncated: false };
    if o.text.trim_end_matches(['\r', '\n']).is_empty() || !(o.tables || o.views || o.indexes || o.triggers) || !in_scope(o) {
        return Ok(empty);
    }
    let re = build_regex(o)?;
    let mut budget = MAX_OCCURRENCES;
    // (tabelas antes das views, nome) → objeto
    let mut objects: BTreeMap<(u8, String), FindObject> = BTreeMap::new();
    let mut push = |kind: &str, name: &str, matches: Vec<FindMatch>| {
        if matches.is_empty() {
            return;
        }
        let rank = if kind == "table" { 0 } else { 1 };
        objects
            .entry((rank, name.to_string()))
            .or_insert_with(|| FindObject { schema: SCHEMA.into(), kind: kind.into(), name: name.into(), identity: None, matches: Vec::new() })
            .matches
            .extend(matches);
    };

    if o.tables || o.indexes || o.triggers {
        for table in user_tables(conn)? {
            let mut found: Vec<FindMatch> = Vec::new();
            if o.tables {
                found.extend(name_match("tableName", &table, &re));
                let cols = strings(conn, &format!("SELECT name FROM pragma_table_xinfo({})", quote_literal(&table)), &[])?;
                found.extend(cols.iter().filter_map(|c| name_match("fieldName", c, &re)));
            }
            if o.indexes {
                // Os índices automáticos (chave e UNIQUE) não têm nome de verdade.
                let names = strings(conn, "SELECT name FROM sqlite_master WHERE type = 'index' AND tbl_name = ?1 AND name NOT LIKE 'sqlite\\_autoindex%' ESCAPE '\\' ORDER BY name", &[&table])?;
                found.extend(names.iter().filter_map(|n| name_match("indexName", n, &re)));
            }
            if o.triggers {
                let names = strings(conn, "SELECT name FROM sqlite_master WHERE type = 'trigger' AND tbl_name = ?1 ORDER BY name", &[&table])?;
                found.extend(names.iter().filter_map(|n| name_match("triggerName", n, &re)));
            }
            push("table", &table, found);
        }
    }

    if o.views {
        let mut stmt = conn.prepare("SELECT name, sql FROM sqlite_master WHERE type = 'view' ORDER BY name").map_err(map_err)?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?))).map_err(map_err)?.collect::<Result<Vec<_>, _>>().map_err(map_err)?;
        for (name, body) in rows {
            let mut found: Vec<FindMatch> = name_match("viewName", &name, &re).into_iter().collect();
            budget = budget.saturating_sub(found.len());
            if let Some(body) = body {
                found.extend(body_matches("viewBody", &body, &re, &mut budget));
            }
            push("view", &name, found);
        }
    }

    let truncated = budget == 0;
    let objects: Vec<FindObject> = objects.into_values().collect();
    let occurrences = objects.iter().map(|o| o.matches.len()).sum();
    Ok(FindResult { objects, occurrences, truncated })
}

/// Texto entre aspas simples, para o argumento das funções `pragma_*`.
fn quote_literal(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// Quantas linhas de cada tabela têm o texto em alguma coluna (lida como texto). Só aparecem as tabelas com pelo menos uma.
/// Usa uma conexão própria ao arquivo, para não prender a da sessão durante a leitura.
pub(crate) fn find_data_at(path: &Path, o: &FindOptions, cancelled: &(dyn Fn() -> bool + Sync)) -> Result<FindDataResult, DatabaseError> {
    let mut result = FindDataResult { tables: Vec::new(), scanned: 0, total: 0, skipped: 0, cancelled: false };
    if o.text.trim_end_matches(['\r', '\n']).is_empty() || !in_scope(o) {
        return Ok(result);
    }
    build_regex(o)?;
    let conn = open_connection(path)?;
    let pattern = format!("{}{}", if o.case_insensitive { "(?i)" } else { "" }, pattern_text(o));
    let literal = quote_literal(&pattern);
    let tables = user_tables(&conn)?;
    result.total = tables.len();
    for table in tables {
        if cancelled() {
            result.cancelled = true;
            break;
        }
        // Colunas de blob ficam de fora; as demais são lidas como texto.
        let cols: Vec<String> = {
            let mut stmt = conn.prepare(&format!("SELECT name, type FROM pragma_table_xinfo({})", quote_literal(&table))).map_err(map_err)?;
            let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))).map_err(map_err)?.collect::<Result<Vec<_>, _>>().map_err(map_err)?;
            rows.into_iter().filter(|(_, t)| !t.to_ascii_lowercase().contains("blob")).map(|(n, _)| n).collect()
        };
        if cols.is_empty() {
            result.scanned += 1;
            continue;
        }
        let target = quote_ident(&table);
        let cond = |rhs: &str| cols.iter().map(|c| format!("CAST({target}.{} AS TEXT) REGEXP {rhs}", quote_ident(c))).collect::<Vec<_>>().join(" OR ");
        let sql = format!("SELECT count(*) FROM {target} WHERE {}", cond("?1"));
        match conn.query_row(&sql, [&pattern], |r| r.get::<_, i64>(0)) {
            Ok(count) => {
                result.scanned += 1;
                if count > 0 {
                    let sql = format!("SELECT * FROM {target} WHERE {}", cond(&literal));
                    result.tables.push(DataMatch { schema: SCHEMA.into(), table, count, sql });
                }
            }
            Err(_) => result.skipped += 1,
        }
    }
    Ok(result)
}

/// `X REGEXP Y` do SQLite chama `regexp(Y, X)`; o SQLite não traz essa função, então cada conexão do Atlas a define.
pub(crate) fn register_regexp(conn: &Connection) -> Result<(), DatabaseError> {
    use rusqlite::functions::FunctionFlags;
    conn.create_scalar_function("regexp", 2, FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_DETERMINISTIC, |ctx| {
        let re: std::sync::Arc<Regex> = ctx.get_or_create_aux(0, |vr| -> Result<Regex, Box<dyn std::error::Error + Send + Sync>> { Ok(Regex::new(vr.as_str()?)?) })?;
        let value: Option<String> = ctx.get(1)?;
        Ok(value.is_some_and(|v| re.is_match(&v)))
    })
    .map_err(map_err)?;
    Ok(())
}
