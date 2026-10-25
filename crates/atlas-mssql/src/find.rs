//! "Find in Database" do SQL Server: procura texto na estrutura (nomes de tabelas, campos, índices, gatilhos, views e procedimentos,
//! mais o corpo de views e procedimentos) ou nos dados das tabelas. Os formatos de resposta são os mesmos do PostgreSQL.

use super::{MssqlSession, quote_ident};
use crate::meta::{rows_of, s};
use atlas_core::{DatabaseError, ErrorKind};
use regex::{Regex, RegexBuilder};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Máximo de ocorrências devolvidas (acima disso a busca avisa que foi cortada).
const MAX_OCCURRENCES: usize = 2_000;
/// Ocorrências mostradas por objeto no corpo de uma view.
const MAX_PER_BODY: usize = 100;
/// Janela do trecho mostrado: caracteres antes da ocorrência e tamanho total.
const SNIPPET_BEFORE: usize = 20;
const SNIPPET_LEN: usize = 44;

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


/// Texto entre aspas simples, com `N` na frente (Unicode).
fn lit(s: &str) -> String {
    format!("N'{}'", s.replace('\'', "''"))
}

impl MssqlSession {
    /// "Find in Database/Schema", Look In = Structure.
    pub async fn find(&self, database: &str, o: &FindOptions) -> Result<FindResult, DatabaseError> {
        let empty = FindResult { objects: Vec::new(), occurrences: 0, truncated: false };
        if o.text.trim_end_matches(['\r', '\n']).is_empty() || !(o.tables || o.views || o.functions || o.indexes || o.triggers) {
            return Ok(empty);
        }
        let re = build_regex(o)?;
        let schema = o.schema.clone();
        let client = self.client(database).await?;
        let mut conn = client.lock().await;
        let mut budget = MAX_OCCURRENCES;
        // (tipo na ordem, schema, nome) → objeto
        let mut objects: BTreeMap<(u8, String, String), FindObject> = BTreeMap::new();
        let mut push = |kind: &str, schema: &str, name: &str, matches: Vec<FindMatch>| {
            if matches.is_empty() {
                return;
            }
            let rank = match kind {
                "table" => 0,
                "view" => 1,
                _ => 3,
            };
            objects
                .entry((rank, schema.to_string(), name.to_string()))
                .or_insert_with(|| FindObject { schema: schema.into(), kind: kind.into(), name: name.into(), identity: None, matches: Vec::new() })
                .matches
                .extend(matches);
        };
        let scope = "(@P1 IS NULL OR sc.name = @P1) AND sc.name NOT IN ('sys', 'INFORMATION_SCHEMA')";

        if o.tables || o.indexes || o.triggers {
            let tables = rows_of(&mut conn, &format!("SELECT t.name, sc.name, CAST(t.object_id AS int) FROM sys.tables t JOIN sys.schemas sc ON sc.schema_id = t.schema_id WHERE {scope} AND t.is_ms_shipped = 0 ORDER BY sc.name, t.name"), &[&schema]).await?;
            let mut per: BTreeMap<i32, Vec<FindMatch>> = BTreeMap::new();
            let id = |r: &tiberius::Row, i: usize| r.get::<i32, _>(i).unwrap_or(0);
            if o.tables {
                for r in &tables {
                    if let Some(m) = name_match("tableName", &s(r, 0), &re) {
                        per.entry(id(r, 2)).or_default().push(m);
                    }
                }
                for r in rows_of(&mut conn, &format!("SELECT c.name, CAST(c.object_id AS int) FROM sys.columns c JOIN sys.tables t ON t.object_id = c.object_id JOIN sys.schemas sc ON sc.schema_id = t.schema_id WHERE {scope} AND t.is_ms_shipped = 0 ORDER BY c.object_id, c.column_id"), &[&schema]).await?.iter() {
                    if let Some(m) = name_match("fieldName", &s(r, 0), &re) {
                        per.entry(id(r, 1)).or_default().push(m);
                    }
                }
                for r in rows_of(&mut conn, &format!("SELECT fk.name, CAST(fk.parent_object_id AS int) FROM sys.foreign_keys fk JOIN sys.tables t ON t.object_id = fk.parent_object_id JOIN sys.schemas sc ON sc.schema_id = t.schema_id WHERE {scope} ORDER BY fk.name"), &[&schema]).await?.iter() {
                    if let Some(m) = name_match("foreignKeyName", &s(r, 0), &re) {
                        per.entry(id(r, 1)).or_default().push(m);
                    }
                }
            }
            if o.indexes {
                // Os índices da chave primária e das restrições UNIQUE aparecem como restrições.
                for r in rows_of(&mut conn, &format!("SELECT i.name, CAST(i.object_id AS int) FROM sys.indexes i JOIN sys.tables t ON t.object_id = i.object_id JOIN sys.schemas sc ON sc.schema_id = t.schema_id WHERE {scope} AND i.type > 0 AND i.is_primary_key = 0 AND i.is_unique_constraint = 0 AND t.is_ms_shipped = 0 ORDER BY i.name"), &[&schema]).await?.iter() {
                    if let Some(m) = name_match("indexName", &s(r, 0), &re) {
                        per.entry(id(r, 1)).or_default().push(m);
                    }
                }
            }
            if o.triggers {
                for r in rows_of(&mut conn, &format!("SELECT tr.name, CAST(tr.parent_id AS int) FROM sys.triggers tr JOIN sys.tables t ON t.object_id = tr.parent_id JOIN sys.schemas sc ON sc.schema_id = t.schema_id WHERE {scope} ORDER BY tr.name"), &[&schema]).await?.iter() {
                    if let Some(m) = name_match("triggerName", &s(r, 0), &re) {
                        per.entry(id(r, 1)).or_default().push(m);
                    }
                }
            }
            for r in &tables {
                if let Some(mut found) = per.remove(&id(r, 2)) {
                    let order = |w: &str| ["tableName", "fieldName", "indexName", "foreignKeyName", "triggerName"].iter().position(|x| *x == w).unwrap_or(9);
                    found.sort_by_key(|m| order(&m.what));
                    push("table", &s(r, 1), &s(r, 0), found);
                }
            }
        }

        if o.views {
            let rows = rows_of(&mut conn, &format!("SELECT v.name, sc.name, CAST(OBJECT_DEFINITION(v.object_id) AS nvarchar(max)) FROM sys.views v JOIN sys.schemas sc ON sc.schema_id = v.schema_id WHERE {scope} AND v.is_ms_shipped = 0 ORDER BY sc.name, v.name"), &[&schema]).await?;
            for r in &rows {
                let name = s(r, 0);
                let mut found: Vec<FindMatch> = name_match("viewName", &name, &re).into_iter().collect();
                budget = budget.saturating_sub(found.len());
                if let Some(body) = r.get::<&str, _>(2) {
                    found.extend(body_matches("viewBody", body, &re, &mut budget));
                }
                push("view", &s(r, 1), &name, found);
            }
        }

        if o.functions {
            let rows = rows_of(&mut conn, &format!("SELECT o.name, sc.name, CAST(OBJECT_DEFINITION(o.object_id) AS nvarchar(max)) FROM sys.objects o JOIN sys.schemas sc ON sc.schema_id = o.schema_id WHERE {scope} AND o.type IN ('P', 'FN', 'IF', 'TF') AND o.is_ms_shipped = 0 ORDER BY sc.name, o.name"), &[&schema]).await?;
            for r in &rows {
                let name = s(r, 0);
                let mut found: Vec<FindMatch> = name_match("functionName", &name, &re).into_iter().collect();
                budget = budget.saturating_sub(found.len());
                if let Some(body) = r.get::<&str, _>(2) {
                    found.extend(body_matches("functionBody", body, &re, &mut budget));
                }
                push("function", &s(r, 1), &name, found);
            }
        }

        let truncated = budget == 0;
        let objects: Vec<FindObject> = objects.into_values().collect();
        let occurrences = objects.iter().map(|o| o.matches.len()).sum();
        Ok(FindResult { objects, occurrences, truncated })
    }

    /// "Find in Database/Schema", Look In = Data: quantas linhas de cada tabela têm o texto. Contém e prefixo usam `LIKE`; palavra
    /// inteira, igualdade com o valor; expressão regular o T-SQL não tem.
    pub async fn find_data(&self, database: &str, o: &FindOptions, cancelled: &(dyn Fn() -> bool + Sync)) -> Result<FindDataResult, DatabaseError> {
        let mut result = FindDataResult { tables: Vec::new(), scanned: 0, total: 0, skipped: 0, cancelled: false };
        let text = o.text.trim_end_matches(['\r', '\n']);
        if text.is_empty() {
            return Ok(result);
        }
        if o.mode == FindMode::Regex {
            return Err(DatabaseError::new(ErrorKind::UnsupportedFeature, "regular expressions are not available for searching data in SQL Server"));
        }
        let like = |t: &str| t.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_").replace('[', "\\[");
        let (op, pattern) = match o.mode {
            FindMode::Contains => ("LIKE", format!("%{}%", like(text))),
            FindMode::Prefix => ("LIKE", format!("{}%", like(text))),
            _ => ("=", text.to_string()),
        };
        let collate = if o.case_insensitive { " COLLATE Latin1_General_CI_AS" } else { " COLLATE Latin1_General_CS_AS" };
        let escape = if op == "LIKE" { " ESCAPE '\\'" } else { "" };
        let schema = o.schema.clone();
        let dedicated = self.dedicated(database).await?;
        let mut conn = dedicated;
        let tables = rows_of(&mut conn, "SELECT sc.name, t.name FROM sys.tables t JOIN sys.schemas sc ON sc.schema_id = t.schema_id WHERE (@P1 IS NULL OR sc.name = @P1) AND sc.name NOT IN ('sys', 'INFORMATION_SCHEMA') AND t.is_ms_shipped = 0 ORDER BY sc.name, t.name", &[&schema]).await?;
        result.total = tables.len();
        for tr in &tables {
            if cancelled() {
                result.cancelled = true;
                break;
            }
            let (sc, tb) = (s(tr, 0), s(tr, 1));
            let target = format!("{}.{}", quote_ident(&sc), quote_ident(&tb));
            // Colunas de texto, número, data e identificador; binário e XML ficam de fora.
            let cols = rows_of(&mut conn, "SELECT c.name FROM sys.columns c JOIN sys.types ty ON ty.user_type_id = c.user_type_id WHERE c.object_id = OBJECT_ID(@P1) AND ty.name NOT IN ('image', 'varbinary', 'binary', 'timestamp', 'xml', 'geography', 'geometry', 'hierarchyid', 'sql_variant') ORDER BY c.column_id", &[&target]).await?;
            if cols.is_empty() {
                result.scanned += 1;
                continue;
            }
            let cond = |rhs: &str| cols.iter().map(|c| format!("CAST({} AS nvarchar(max)){collate} {op} {rhs}{escape}", quote_ident(&s(c, 0)))).collect::<Vec<_>>().join(" OR ");
            let sql = format!("SELECT COUNT_BIG(*) FROM {target} WHERE {}", cond("@P1"));
            let mut q = tiberius::Query::new(sql);
            q.bind(pattern.clone());
            match q.query(&mut conn).await {
                Ok(stream) => match stream.into_first_result().await {
                    Ok(rows) => {
                        result.scanned += 1;
                        let count = rows.first().and_then(|r| r.get::<i64, _>(0)).unwrap_or(0);
                        if count > 0 {
                            let sql = format!("SELECT * FROM {target} WHERE {}", cond(&lit(&pattern)));
                            result.tables.push(DataMatch { schema: sc, table: tb, count, sql });
                        }
                    }
                    Err(_) => result.skipped += 1,
                },
                Err(_) => result.skipped += 1,
            }
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts(text: &str, mode: FindMode) -> FindOptions {
        FindOptions { text: text.into(), schema: None, mode, case_insensitive: true, tables: true, views: true, matviews: false, functions: true, indexes: true, triggers: true }
    }

    #[test]
    fn name_match_highlights_every_occurrence() {
        let re = build_regex(&opts("client", FindMode::Contains)).unwrap();
        assert_eq!(name_match("fieldName", "Client_notes_client_id", &re).unwrap().ranges, vec![[0, 6], [13, 19]]);
        assert!(name_match("fieldName", "orders", &re).is_none());
    }
}
