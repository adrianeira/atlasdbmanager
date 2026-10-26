//! "Find in Database" do Firebird: procura texto na estrutura (nomes de tabelas, campos, índices, gatilhos, views e rotinas, mais o
//! corpo de views, gatilhos e rotinas) ou nos dados das tabelas. Os formatos de resposta são os mesmos do PostgreSQL.
//! Nos dados, "contém" usa `CONTAINING`/`LIKE`, "palavra inteira" a igualdade, "começa com" `STARTING WITH` e "expressão regular"
//! o `SIMILAR TO` do Firebird (a sintaxe de expressão regular do SQL, não a do Rust).

use super::{Conn, Target, connect, map_err, meta, quote_ident};
use atlas_core::{DatabaseError, ErrorKind};
use regex::{Regex, RegexBuilder};
use rsfbclient::SqlType;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Máximo de ocorrências devolvidas (acima disso a busca avisa que foi cortada).
const MAX_OCCURRENCES: usize = 2_000;
/// Ocorrências mostradas por objeto no corpo de uma view.
const MAX_PER_BODY: usize = 100;
/// Janela do trecho mostrado: caracteres antes da ocorrência e tamanho total.
const SNIPPET_BEFORE: usize = 20;
const SNIPPET_LEN: usize = 44;
/// O único schema do Firebird (até a versão 4).
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

fn strings(conn: &mut Conn, sql: &str, params: Vec<SqlType>) -> Result<Vec<String>, DatabaseError> {
    Ok(meta::query(conn, sql, params)?.into_iter().map(|r| r.into_iter().next().flatten().unwrap_or_default().trim().to_string()).collect())
}

fn pairs(conn: &mut Conn, sql: &str) -> Result<Vec<(String, String)>, DatabaseError> {
    Ok(meta::query(conn, sql, vec![])?
        .into_iter()
        .map(|r| (r.first().cloned().flatten().unwrap_or_default().trim().to_string(), r.get(1).cloned().flatten().unwrap_or_default()))
        .collect())
}

/// Tabelas do usuário (as de sistema ficam de fora).
fn user_tables(conn: &mut Conn) -> Result<Vec<String>, DatabaseError> {
    strings(conn, "SELECT TRIM(RDB$RELATION_NAME) FROM RDB$RELATIONS WHERE COALESCE(RDB$SYSTEM_FLAG, 0) = 0 AND RDB$VIEW_BLR IS NULL ORDER BY 1", vec![])
}

pub(crate) fn find_on(conn: &mut Conn, o: &FindOptions) -> Result<FindResult, DatabaseError> {
    let empty = FindResult { objects: Vec::new(), occurrences: 0, truncated: false };
    if o.text.trim_end_matches(['\r', '\n']).is_empty() || !(o.tables || o.views || o.indexes || o.triggers || o.functions) || !in_scope(o) {
        return Ok(empty);
    }
    let re = build_regex(o)?;
    let mut budget = MAX_OCCURRENCES;
    // (tabelas, views, rotinas; nome) → objeto
    let mut objects: BTreeMap<(u8, String), FindObject> = BTreeMap::new();
    let mut push = |kind: &str, name: &str, matches: Vec<FindMatch>| {
        if matches.is_empty() {
            return;
        }
        let rank = match kind {
            "table" => 0,
            "view" => 1,
            _ => 2,
        };
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
                let cols = strings(conn, "SELECT TRIM(RDB$FIELD_NAME) FROM RDB$RELATION_FIELDS WHERE RDB$RELATION_NAME = ? ORDER BY RDB$FIELD_POSITION", meta::text(&table))?;
                found.extend(cols.iter().filter_map(|c| name_match("fieldName", c, &re)));
            }
            if o.indexes {
                // Os índices que sustentam chaves e restrições têm nome gerado: só entram os criados pelo usuário.
                let names = strings(
                    conn,
                    "SELECT TRIM(RDB$INDEX_NAME) FROM RDB$INDICES WHERE RDB$RELATION_NAME = ? AND COALESCE(RDB$SYSTEM_FLAG, 0) = 0 AND RDB$INDEX_NAME NOT STARTING WITH 'RDB$' ORDER BY 1",
                    meta::text(&table),
                )?;
                found.extend(names.iter().filter_map(|n| name_match("indexName", n, &re)));
            }
            if o.triggers {
                let rows = meta::query(conn, "SELECT TRIM(RDB$TRIGGER_NAME), RDB$TRIGGER_SOURCE FROM RDB$TRIGGERS WHERE RDB$RELATION_NAME = ? AND COALESCE(RDB$SYSTEM_FLAG, 0) = 0 ORDER BY 1", meta::text(&table))?;
                for r in rows {
                    let name = r.first().cloned().flatten().unwrap_or_default().trim().to_string();
                    found.extend(name_match("triggerName", &name, &re));
                }
            }
            push("table", &table, found);
        }
    }

    if o.views {
        for (name, body) in pairs(conn, "SELECT TRIM(RDB$RELATION_NAME), RDB$VIEW_SOURCE FROM RDB$RELATIONS WHERE COALESCE(RDB$SYSTEM_FLAG, 0) = 0 AND RDB$VIEW_BLR IS NOT NULL ORDER BY 1")? {
            let mut found: Vec<FindMatch> = name_match("viewName", &name, &re).into_iter().collect();
            budget = budget.saturating_sub(found.len());
            found.extend(body_matches("viewBody", &body, &re, &mut budget));
            push("view", &name, found);
        }
    }

    if o.functions {
        let mut routines = pairs(conn, "SELECT TRIM(RDB$PROCEDURE_NAME), RDB$PROCEDURE_SOURCE FROM RDB$PROCEDURES WHERE COALESCE(RDB$SYSTEM_FLAG, 0) = 0 ORDER BY 1")?;
        if meta::is_v3(conn) {
            routines.extend(pairs(conn, "SELECT TRIM(RDB$FUNCTION_NAME), RDB$FUNCTION_SOURCE FROM RDB$FUNCTIONS WHERE COALESCE(RDB$SYSTEM_FLAG, 0) = 0 AND RDB$PACKAGE_NAME IS NULL AND RDB$MODULE_NAME IS NULL ORDER BY 1")?);
        }
        for (name, body) in routines {
            let mut found: Vec<FindMatch> = name_match("functionName", &name, &re).into_iter().collect();
            budget = budget.saturating_sub(found.len());
            found.extend(body_matches("functionBody", &body, &re, &mut budget));
            push("function", &name, found);
        }
    }

    let truncated = budget == 0;
    let objects: Vec<FindObject> = objects.into_values().collect();
    let occurrences = objects.iter().map(|o| o.matches.len()).sum();
    Ok(FindResult { objects, occurrences, truncated })
}

/// Texto entre aspas simples, como literal do SQL.
fn literal(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// A condição de uma coluna (já convertida em texto) para o modo da busca, com o texto escrito no comando.
fn condition(column: &str, o: &FindOptions) -> String {
    let text = o.text.trim_end_matches(['\r', '\n']);
    let value = literal(text);
    let upper = |s: &str| format!("UPPER({s})");
    match (o.mode, o.case_insensitive) {
        (FindMode::Contains, true) => format!("{column} CONTAINING {value}"),
        (FindMode::Contains, false) => {
            let like = text.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_");
            format!("{column} LIKE {} ESCAPE '\\'", literal(&format!("%{like}%")))
        }
        (FindMode::Whole, true) => format!("{} = {}", upper(column), upper(&value)),
        (FindMode::Whole, false) => format!("{column} = {value}"),
        (FindMode::Prefix, true) => format!("{} STARTING WITH {}", upper(column), upper(&value)),
        (FindMode::Prefix, false) => format!("{column} STARTING WITH {value}"),
        (FindMode::Regex, true) => format!("{} SIMILAR TO {}", upper(column), upper(&value)),
        (FindMode::Regex, false) => format!("{column} SIMILAR TO {value}"),
    }
}

/// Quantas linhas de cada tabela têm o texto em alguma coluna (lida como texto). Só aparecem as tabelas com pelo menos uma.
/// Usa uma conexão própria, para não prender a da sessão durante a leitura.
pub(crate) fn find_data_at(target: &Target, o: &FindOptions, cancelled: &(dyn Fn() -> bool + Sync)) -> Result<FindDataResult, DatabaseError> {
    let mut result = FindDataResult { tables: Vec::new(), scanned: 0, total: 0, skipped: 0, cancelled: false };
    if o.text.trim_end_matches(['\r', '\n']).is_empty() || !in_scope(o) {
        return Ok(result);
    }
    if o.mode != FindMode::Regex {
        build_regex(o)?;
    }
    let mut conn = connect(target)?;
    let tables = user_tables(&mut conn)?;
    result.total = tables.len();
    for table in tables {
        if cancelled() {
            result.cancelled = true;
            break;
        }
        // Colunas de blob e de tipos que não viram texto ficam de fora.
        let cols: Vec<String> = meta::query(
            &mut conn,
            "SELECT TRIM(rf.RDB$FIELD_NAME), f.RDB$FIELD_TYPE FROM RDB$RELATION_FIELDS rf JOIN RDB$FIELDS f ON f.RDB$FIELD_NAME = rf.RDB$FIELD_SOURCE WHERE rf.RDB$RELATION_NAME = ? ORDER BY rf.RDB$FIELD_POSITION",
            meta::text(&table),
        )?
        .into_iter()
        .filter(|r| r.get(1).cloned().flatten().and_then(|t| t.parse::<i32>().ok()) != Some(261))
        .map(|r| r.first().cloned().flatten().unwrap_or_default().trim().to_string())
        .collect();
        if cols.is_empty() {
            result.scanned += 1;
            continue;
        }
        let quoted = quote_ident(&table);
        let cond = cols.iter().map(|c| condition(&format!("CAST({quoted}.{} AS VARCHAR(8191))", quote_ident(c)), o)).collect::<Vec<_>>().join(" OR ");
        let sql = format!("SELECT COUNT(*) FROM {quoted} WHERE {cond}");
        match meta::query(&mut conn, &sql, vec![]) {
            Ok(rows) => {
                result.scanned += 1;
                let count = rows.first().and_then(|r| r.first().cloned().flatten()).and_then(|c| c.parse::<i64>().ok()).unwrap_or(0);
                if count > 0 {
                    result.tables.push(DataMatch { schema: SCHEMA.into(), table, count, sql: format!("SELECT * FROM {quoted} WHERE {cond}") });
                }
            }
            Err(_) => result.skipped += 1,
        }
    }
    let _ = map_err;
    Ok(result)
}
