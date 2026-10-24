//! "Find in Database/Schema": procura texto na estrutura (nomes de tabelas, campos, chaves estrangeiras, índices, gatilhos,
//! views, views materializadas e funções, mais o corpo de views e funções) ou nos dados das tabelas. Só lê o catálogo e as
//! tabelas; o texto procurado entra sempre como parâmetro (dados) ou como expressão regular local (estrutura).

use crate::connect::PgSession;
use crate::error::map_error;
use crate::exec::quote_ident;
use atlas_core::{DatabaseError, ErrorKind};
use regex::{Regex, RegexBuilder};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

/// Máximo de ocorrências devolvidas (acima disso a busca avisa que foi cortada).
const MAX_OCCURRENCES: usize = 2_000;
/// Ocorrências mostradas por objeto no corpo de uma view ou função.
const MAX_PER_BODY: usize = 100;
/// Quanto de cada definição vem do servidor.
const MAX_DEFINITION_CHARS: i32 = 200_000;
/// Janela do trecho mostrado: caracteres antes da ocorrência e tamanho total.
const SNIPPET_BEFORE: usize = 20;
const SNIPPET_LEN: usize = 44;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FindMode {
    Contains,
    /// Palavra inteira.
    Whole,
    /// Começa com o texto (no início de uma palavra).
    Prefix,
    Regex,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FindOptions {
    pub text: String,
    /// Só este schema; `None` = todos os schemas do usuário.
    pub schema: Option<String>,
    pub mode: FindMode,
    pub case_insensitive: bool,
    pub tables: bool,
    pub views: bool,
    pub matviews: bool,
    pub functions: bool,
    pub indexes: bool,
    pub triggers: bool,
}

/// Um achado dentro de um objeto: o que casou (`what`), o texto mostrado e onde destacar (posições em caracteres).
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FindMatch {
    /// "tableName" | "fieldName" | "foreignKeyName" | "indexName" | "triggerName" | "viewName" | "viewBody" |
    /// "matviewName" | "matviewBody" | "functionName" | "functionBody".
    pub what: String,
    pub text: String,
    pub ranges: Vec<[usize; 2]>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FindObject {
    pub schema: String,
    /// "table" | "view" | "matview" | "function".
    pub kind: String,
    pub name: String,
    /// Funções: argumentos de identidade (a chave da função com sobrecarga).
    pub identity: Option<String>,
    pub matches: Vec<FindMatch>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FindResult {
    pub objects: Vec<FindObject>,
    /// Total de ocorrências (um nome conta uma; o corpo conta cada vez que o texto aparece).
    pub occurrences: usize,
    /// Havia mais ocorrências que o limite.
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DataMatch {
    pub schema: String,
    pub table: String,
    pub count: i64,
    /// Consulta que lista as linhas achadas (o texto procurado já dentro dela), para abrir no editor.
    pub sql: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FindDataResult {
    pub tables: Vec<DataMatch>,
    /// Tabelas lidas até o fim e total de tabelas do escopo.
    pub scanned: usize,
    pub total: usize,
    /// Tabelas que o usuário não pôde ler (permissão, por exemplo).
    pub skipped: usize,
    pub cancelled: bool,
}

fn is_transient_catalog_error(e: &tokio_postgres::Error) -> bool {
    e.as_db_error().is_some_and(|d| d.message().contains("could not open relation") || d.message().contains("cache lookup failed"))
}

/// A expressão regular do modo escolhido (a busca na estrutura é toda feita aqui, em Rust).
fn build_regex(o: &FindOptions) -> Result<Regex, DatabaseError> {
    let text = o.text.trim_end_matches(['\r', '\n']);
    let escaped = regex::escape(text);
    let pattern = match o.mode {
        FindMode::Contains => escaped,
        FindMode::Whole => format!(r"\b{escaped}\b"),
        FindMode::Prefix => format!(r"\b{escaped}"),
        FindMode::Regex => text.to_string(),
    };
    RegexBuilder::new(&pattern)
        .case_insensitive(o.case_insensitive)
        .build()
        .map_err(|e| DatabaseError::new(ErrorKind::InvalidInput, format!("invalid regular expression: {e}")))
}

/// Posição em caracteres de um deslocamento em bytes.
fn char_index(text: &str, byte: usize) -> usize {
    text[..byte].chars().count()
}

/// O nome inteiro com todas as ocorrências destacadas; `None` se não casa.
fn name_match(what: &str, name: &str, re: &Regex) -> Option<FindMatch> {
    let ranges: Vec<[usize; 2]> = re.find_iter(name).filter(|m| !m.is_empty()).map(|m| [char_index(name, m.start()), char_index(name, m.end())]).collect();
    if ranges.is_empty() { None } else { Some(FindMatch { what: what.to_string(), text: name.to_string(), ranges }) }
}

/// Trecho de `text` ao redor de uma ocorrência (posições em caracteres), com os espaços em branco achatados.
/// Devolve o texto e o destaque só dessa ocorrência.
fn snippet(chars: &[char], match_start: usize, match_end: usize) -> (String, [usize; 2]) {
    let n = chars.len();
    let mut start = match_start.saturating_sub(SNIPPET_BEFORE);
    let end = (start + SNIPPET_LEN).min(n);
    // Perto do fim do texto a janela volta para trás, para manter o tamanho.
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

/// Uma ocorrência por vez que o texto aparece no corpo.
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

const SCHEMA_OK: &str = "n.nspname !~ '^pg_' AND n.nspname <> 'information_schema' AND ($1::text IS NULL OR n.nspname = $1)";

type ObjectKey = (u8, String, String, String);

fn rank(kind: &str) -> u8 {
    match kind {
        "table" => 0,
        "view" => 1,
        "matview" => 2,
        _ => 3,
    }
}

pub async fn find(s: &PgSession, o: &FindOptions) -> Result<FindResult, DatabaseError> {
    let empty = FindResult { objects: Vec::new(), occurrences: 0, truncated: false };
    if o.text.trim_end_matches(['\r', '\n']).is_empty() || !(o.tables || o.views || o.matviews || o.functions || o.indexes || o.triggers) {
        return Ok(empty);
    }
    let re = build_regex(o)?;
    let v = s.server.version_num;
    let mut budget = MAX_OCCURRENCES;
    let mut objects: BTreeMap<ObjectKey, FindObject> = BTreeMap::new();
    let mut push = |kind: &str, schema: &str, name: &str, identity: Option<&str>, matches: Vec<FindMatch>| {
        if matches.is_empty() {
            return;
        }
        let key = (rank(kind), schema.to_string(), name.to_string(), identity.unwrap_or("").to_string());
        objects
            .entry(key)
            .or_insert_with(|| FindObject { schema: schema.into(), kind: kind.into(), name: name.into(), identity: identity.map(String::from), matches: Vec::new() })
            .matches
            .extend(matches);
    };
    let query = |sql: String| async move {
        // Outra sessão pode apagar um objeto entre a leitura do catálogo e `pg_get_viewdef`: o servidor responde
        // "could not open relation" / "cache lookup failed". É passageiro, então tenta mais uma ou duas vezes.
        let mut attempt = 0;
        loop {
            match s.client.query(&sql, &[&o.schema]).await {
                Ok(rows) => return Ok(rows),
                Err(e) if attempt < 2 && is_transient_catalog_error(&e) => attempt += 1,
                Err(e) => return Err(map_error(&e)),
            }
        }
    };

    if o.tables || o.indexes || o.triggers {
        let tables = query(format!(
            "SELECT c.oid::bigint, n.nspname::text, c.relname::text FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
              WHERE c.relkind IN ('r','p','f') AND {SCHEMA_OK} ORDER BY 2, 3"
        ))
        .await?;
        // Por tabela: o que casou em cada categoria, na ordem em que aparece (nome, campos, índices, chaves, gatilhos).
        let mut per: HashMap<i64, Vec<FindMatch>> = HashMap::new();
        if o.tables {
            for r in &tables {
                let (oid, name): (i64, String) = (r.get(0), r.get(2));
                if let Some(m) = name_match("tableName", &name, &re) {
                    per.entry(oid).or_default().push(m);
                }
            }
            let cols = query(format!(
                "SELECT c.oid::bigint, a.attname::text FROM pg_catalog.pg_attribute a JOIN pg_catalog.pg_class c ON c.oid = a.attrelid \
                   JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
                  WHERE c.relkind IN ('r','p','f') AND a.attnum > 0 AND NOT a.attisdropped AND {SCHEMA_OK} ORDER BY c.oid, a.attnum"
            ))
            .await?;
            for r in &cols {
                let (oid, name): (i64, String) = (r.get(0), r.get(1));
                if let Some(m) = name_match("fieldName", &name, &re) {
                    per.entry(oid).or_default().push(m);
                }
            }
        }
        if o.indexes {
            // Os índices que sustentam chave primária e restrições aparecem como restrições, não como índices.
            let rows = query(format!(
                "SELECT i.indrelid::bigint, ic.relname::text FROM pg_catalog.pg_index i JOIN pg_catalog.pg_class ic ON ic.oid = i.indexrelid \
                   JOIN pg_catalog.pg_class c ON c.oid = i.indrelid JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
                  WHERE NOT i.indisprimary AND NOT EXISTS (SELECT 1 FROM pg_catalog.pg_constraint k WHERE k.conindid = i.indexrelid) AND {SCHEMA_OK} \
                  ORDER BY ic.relname"
            ))
            .await?;
            for r in &rows {
                let (oid, name): (i64, String) = (r.get(0), r.get(1));
                if let Some(m) = name_match("indexName", &name, &re) {
                    per.entry(oid).or_default().push(m);
                }
            }
        }
        if o.tables {
            let rows = query(format!(
                "SELECT k.conrelid::bigint, k.conname::text FROM pg_catalog.pg_constraint k JOIN pg_catalog.pg_class c ON c.oid = k.conrelid \
                   JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace WHERE k.contype = 'f' AND {SCHEMA_OK} ORDER BY k.conname"
            ))
            .await?;
            for r in &rows {
                let (oid, name): (i64, String) = (r.get(0), r.get(1));
                if let Some(m) = name_match("foreignKeyName", &name, &re) {
                    per.entry(oid).or_default().push(m);
                }
            }
        }
        if o.triggers {
            let rows = query(format!(
                "SELECT t.tgrelid::bigint, t.tgname::text FROM pg_catalog.pg_trigger t JOIN pg_catalog.pg_class c ON c.oid = t.tgrelid \
                   JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace WHERE NOT t.tgisinternal AND {SCHEMA_OK} ORDER BY t.tgname"
            ))
            .await?;
            for r in &rows {
                let (oid, name): (i64, String) = (r.get(0), r.get(1));
                if let Some(m) = name_match("triggerName", &name, &re) {
                    per.entry(oid).or_default().push(m);
                }
            }
        }
        for r in &tables {
            let (oid, schema, name): (i64, String, String) = (r.get(0), r.get(1), r.get(2));
            if let Some(mut found) = per.remove(&oid) {
                // Mantém a ordem das categorias mesmo que as consultas tenham vindo separadas.
                let order = |w: &str| ["tableName", "fieldName", "indexName", "foreignKeyName", "triggerName"].iter().position(|x| *x == w).unwrap_or(9);
                found.sort_by_key(|m| order(&m.what));
                push("table", &schema, &name, None, found);
            }
        }
    }

    if o.views || o.matviews {
        let kinds = match (o.views, o.matviews && v >= 90300) {
            (true, true) => "'v','m'",
            (true, false) => "'v'",
            (false, true) => "'m'",
            (false, false) => "'v'",
        };
        if o.views || v >= 90300 {
            let rows = query(format!(
                "SELECT n.nspname::text, c.relname::text, c.relkind::text, left(pg_get_viewdef(c.oid), {MAX_DEFINITION_CHARS}) \
                   FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
                  WHERE c.relkind IN ({kinds}) AND {SCHEMA_OK} ORDER BY 1, 2"
            ))
            .await?;
            for r in &rows {
                let (schema, name, relkind): (String, String, String) = (r.get(0), r.get(1), r.get(2));
                let body: Option<String> = r.get(3);
                let (kind, name_what, body_what) = if relkind == "m" { ("matview", "matviewName", "matviewBody") } else { ("view", "viewName", "viewBody") };
                let mut found: Vec<FindMatch> = name_match(name_what, &name, &re).into_iter().collect();
                budget = budget.saturating_sub(found.len());
                if let Some(body) = body {
                    found.extend(body_matches(body_what, &body, &re, &mut budget));
                }
                push(kind, &schema, &name, None, found);
            }
        }
    }

    if o.functions {
        // Funções (sem agregações/janelas): `prokind` é do PostgreSQL 11; antes eram flags.
        let fn_filter = if v >= 110000 { "p.prokind IN ('f','p')" } else { "NOT p.proisagg AND NOT p.proiswindow" };
        let rows = query(format!(
            "SELECT n.nspname::text, p.proname::text, pg_get_function_identity_arguments(p.oid), \
                    CASE WHEN p.prolang IN (SELECT oid FROM pg_catalog.pg_language WHERE lanname IN ('internal', 'c')) THEN NULL ELSE left(p.prosrc, {MAX_DEFINITION_CHARS}) END \
               FROM pg_catalog.pg_proc p JOIN pg_catalog.pg_namespace n ON n.oid = p.pronamespace \
              WHERE {fn_filter} AND {SCHEMA_OK} ORDER BY 1, 2, 3"
        ))
        .await?;
        for r in &rows {
            let (schema, name, identity): (String, String, String) = (r.get(0), r.get(1), r.get(2));
            let body: Option<String> = r.get(3);
            let mut found: Vec<FindMatch> = name_match("functionName", &name, &re).into_iter().collect();
            budget = budget.saturating_sub(found.len());
            if let Some(body) = body {
                found.extend(body_matches("functionBody", &body, &re, &mut budget));
            }
            push("function", &schema, &name, Some(&identity), found);
        }
    }

    // Os nomes (uma ocorrência cada) consomem o limite junto com o corpo; o excesso só corta o corpo.
    let truncated = budget == 0;
    let objects: Vec<FindObject> = objects.into_values().collect();
    let occurrences = objects.iter().map(|o| o.matches.len()).sum();
    Ok(FindResult { objects, occurrences, truncated })
}

/// `LIKE`/`ILIKE` (contém e prefixo, com `%` e `_` e `\` como escape) ou expressão regular do servidor.
fn data_condition(o: &FindOptions) -> (&'static str, String) {
    let text = o.text.trim_end_matches(['\r', '\n']);
    let ci = o.case_insensitive;
    match o.mode {
        FindMode::Contains => (if ci { "ILIKE" } else { "LIKE" }, format!("%{text}%")),
        FindMode::Prefix => (if ci { "ILIKE" } else { "LIKE" }, format!("{text}%")),
        FindMode::Whole => (if ci { "~*" } else { "~" }, format!(r"\m{}\M", regex::escape(text))),
        FindMode::Regex => (if ci { "~*" } else { "~" }, text.to_string()),
    }
}

/// Quantas linhas de cada tabela têm o texto em alguma coluna (lida como texto). Só aparecem as tabelas com pelo menos uma.
pub async fn find_data(s: &PgSession, o: &FindOptions, cancelled: &(dyn Fn() -> bool + Sync)) -> Result<FindDataResult, DatabaseError> {
    let mut result = FindDataResult { tables: Vec::new(), scanned: 0, total: 0, skipped: 0, cancelled: false };
    if o.text.trim_end_matches(['\r', '\n']).is_empty() {
        return Ok(result);
    }
    if o.mode == FindMode::Regex {
        build_regex(o)?;
    }
    let (op, pattern) = data_condition(o);
    // Partições filhas ficam de fora (a tabela pai já as inclui).
    let not_partition = if s.server.version_num >= 100000 { "AND NOT c.relispartition" } else { "" };
    let tables = s
        .client
        .query(
            &format!(
                "SELECT c.oid::bigint, n.nspname::text, c.relname::text FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
                  WHERE c.relkind IN ('r','p') {not_partition} AND {SCHEMA_OK} ORDER BY 2, 3"
            ),
            &[&o.schema],
        )
        .await
        .map_err(|e| map_error(&e))?;
    let columns = s
        .client
        .query(
            &format!(
                "SELECT c.oid::bigint, a.attname::text, t.typname::text FROM pg_catalog.pg_attribute a JOIN pg_catalog.pg_class c ON c.oid = a.attrelid \
                   JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace JOIN pg_catalog.pg_type t ON t.oid = a.atttypid \
                  WHERE c.relkind IN ('r','p') AND a.attnum > 0 AND NOT a.attisdropped AND t.typname <> 'bytea' AND {SCHEMA_OK} ORDER BY c.oid, a.attnum"
            ),
            &[&o.schema],
        )
        .await
        .map_err(|e| map_error(&e))?;
    let mut by_table: HashMap<i64, Vec<(String, bool)>> = HashMap::new();
    for r in &columns {
        let typ: String = r.get(2);
        // Colunas de texto entram direto no filtro; as demais são lidas como texto.
        let textual = matches!(typ.as_str(), "text" | "varchar" | "bpchar" | "name" | "citext");
        by_table.entry(r.get(0)).or_default().push((r.get(1), textual));
    }
    let literal = pattern.replace('\'', "''");
    result.total = tables.len();
    for r in &tables {
        if cancelled() {
            result.cancelled = true;
            break;
        }
        let (oid, schema, table): (i64, String, String) = (r.get(0), r.get(1), r.get(2));
        let Some(cols) = by_table.get(&oid) else {
            result.scanned += 1;
            continue;
        };
        let target = format!("{}.{}", quote_ident(&schema), quote_ident(&table));
        let column = |c: &str, textual: bool| if textual { format!("{target}.{}", quote_ident(c)) } else { format!("{target}.{}::text", quote_ident(c)) };
        let cond = |rhs: &str| cols.iter().map(|(c, textual)| format!("{} {op} {rhs}", column(c, *textual))).collect::<Vec<_>>().join(" OR ");
        let sql = format!("SELECT count(*) FROM {target} WHERE {}", cond("$1"));
        match s.client.query_one(&sql, &[&pattern]).await {
            Ok(row) => {
                result.scanned += 1;
                let count: i64 = row.get(0);
                if count > 0 {
                    let sql = format!("SELECT * FROM {target} WHERE {}", cond(&format!("'{literal}'")));
                    result.tables.push(DataMatch { schema, table, count, sql });
                }
            }
            Err(e) if e.as_db_error().is_some_and(|d| d.code().code() == "57014") => {
                result.cancelled = true;
                break;
            }
            // Sem permissão ou tabela apagada no meio: segue para a próxima.
            Err(e) if e.as_db_error().is_some_and(|d| matches!(d.code().code(), "42501" | "42P01")) => result.skipped += 1,
            Err(e) => return Err(map_error(&e)),
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts(text: &str, mode: FindMode, ci: bool) -> FindOptions {
        FindOptions { text: text.into(), schema: None, mode, case_insensitive: ci, tables: true, views: true, matviews: true, functions: true, indexes: true, triggers: true }
    }

    #[test]
    fn name_match_highlights_every_occurrence() {
        let re = build_regex(&opts("client", FindMode::Contains, true)).unwrap();
        let m = name_match("fieldName", "Client_notes_client_id", &re).unwrap();
        assert_eq!(m.ranges, vec![[0, 6], [13, 19]]);
        assert!(name_match("fieldName", "orders", &re).is_none());
    }

    #[test]
    fn modes_differ() {
        let hit = |mode, text: &str, name: &str| name_match("x", name, &build_regex(&opts(text, mode, false)).unwrap()).is_some();
        assert!(hit(FindMode::Contains, "lient", "client"));
        assert!(!hit(FindMode::Whole, "client", "clients"));
        assert!(hit(FindMode::Whole, "client", "my client here"));
        assert!(hit(FindMode::Prefix, "cli", "client"));
        assert!(!hit(FindMode::Prefix, "lient", "client"));
        assert!(hit(FindMode::Regex, "^c.*t$", "client"));
        assert!(!hit(FindMode::Contains, "CLIENT", "client"));
        // Texto com caracteres de expressão regular vale como literal fora do modo Regex.
        assert!(hit(FindMode::Contains, "a.b", "xa.by") && !hit(FindMode::Contains, "a.b", "xaxby"));
        assert!(build_regex(&opts("(", FindMode::Regex, false)).is_err());
    }

    #[test]
    fn body_snippet_centers_on_each_occurrence() {
        let body = "SELECT sum(amount) FROM public.orders WHERE client_id = p_client;";
        let re = build_regex(&opts("client", FindMode::Contains, false)).unwrap();
        let mut budget = 100;
        let found = body_matches("functionBody", body, &re, &mut budget);
        assert_eq!(found.len(), 2);
        assert!(found[0].text.starts_with("..."), "{:?}", found[0]);
        assert!(found[1].text.ends_with("p_client;"), "{:?}", found[1]);
        for f in &found {
            let [a, b] = f.ranges[0];
            let hl: String = f.text.chars().skip(a).take(b - a).collect();
            assert_eq!(hl, "client", "{f:?}");
        }
        assert_eq!(budget, 98);
    }

    #[test]
    fn snippet_flattens_whitespace_and_keeps_the_highlight() {
        let body = "SELECT o.id AS order_id,\n    o.client_id,\n    o.amount FROM x";
        let re = build_regex(&opts("client", FindMode::Contains, false)).unwrap();
        let mut budget = 10;
        let found = body_matches("viewBody", body, &re, &mut budget);
        assert_eq!(found.len(), 1);
        assert!(!found[0].text.contains('\n') && !found[0].text.contains("  "), "{:?}", found[0]);
        let [a, b] = found[0].ranges[0];
        assert_eq!(found[0].text.chars().skip(a).take(b - a).collect::<String>(), "client");
    }

    #[test]
    fn data_condition_per_mode() {
        let (op, p) = data_condition(&opts("ana", FindMode::Contains, true));
        assert_eq!((op, p.as_str()), ("ILIKE", "%ana%"));
        let (op, p) = data_condition(&opts("ana", FindMode::Prefix, false));
        assert_eq!((op, p.as_str()), ("LIKE", "ana%"));
        let (op, p) = data_condition(&opts("a.b", FindMode::Whole, false));
        assert_eq!((op, p.as_str()), ("~", r"\ma\.b\M"));
    }
}
