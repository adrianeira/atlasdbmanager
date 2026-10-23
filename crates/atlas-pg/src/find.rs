//! "Localizar no banco/schema": procura texto nos nomes, colunas, definições (views, views materializadas, funções)
//! e comentários dos objetos de usuário. Só lê o catálogo; o texto procurado entra sempre como parâmetro.

use crate::connect::PgSession;
use crate::error::map_error;
use atlas_core::DatabaseError;
use serde::{Deserialize, Serialize};

/// Máximo de resultados devolvidos (acima disso a busca avisa que foi cortada).
const MAX_HITS: usize = 500;
/// Quanto de cada definição vem do servidor para montar o trecho mostrado.
const MAX_DEFINITION_CHARS: i32 = 20_000;
const SNIPPET_BEFORE: usize = 40;
const SNIPPET_LEN: usize = 140;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FindOptions {
    pub text: String,
    /// Só este schema; `None` = todos os schemas do usuário.
    pub schema: Option<String>,
    pub match_case: bool,
    /// Expressão regular do PostgreSQL em vez de texto simples.
    pub regex: bool,
    pub names: bool,
    pub columns: bool,
    pub definitions: bool,
    pub comments: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FindHit {
    pub schema: String,
    /// "table" | "view" | "matview" | "sequence" | "type" | "function".
    pub kind: String,
    pub name: String,
    /// Funções: argumentos de identidade (a chave da função com sobrecarga).
    pub identity: Option<String>,
    /// Coluna encontrada (quando o achado é de coluna).
    pub column: Option<String>,
    /// Onde achou: "name" | "column" | "definition" | "comment".
    pub matched_in: String,
    /// Trecho do texto onde achou (definição ou comentário).
    pub snippet: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FindResult {
    pub hits: Vec<FindHit>,
    /// Havia mais achados que o limite.
    pub truncated: bool,
}

fn is_transient_catalog_error(e: &tokio_postgres::Error) -> bool {
    e.as_db_error().is_some_and(|d| d.message().contains("could not open relation") || d.message().contains("cache lookup failed"))
}

/// Condição "a coluna contém o texto" para o modo escolhido (parâmetro `$1`).
fn matches(col: &str, o: &FindOptions) -> String {
    match (o.regex, o.match_case) {
        (false, false) => format!("position(lower($1) in lower({col})) > 0"),
        (false, true) => format!("position($1 in {col}) > 0"),
        (true, false) => format!("{col} ~* $1"),
        (true, true) => format!("{col} ~ $1"),
    }
}

/// Trecho de `text` ao redor da primeira ocorrência de `needle` (texto simples). Sem ocorrência literal (expressão
/// regular), o começo do texto.
fn snippet(text: &str, needle: &str, match_case: bool, regex: bool) -> String {
    let flat = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
    let chars: Vec<char> = text.chars().collect();
    let start = if regex || needle.is_empty() {
        0
    } else {
        let hay: String = if match_case { text.to_string() } else { text.to_lowercase() };
        let want: String = if match_case { needle.to_string() } else { needle.to_lowercase() };
        match hay.find(&want) {
            // O índice é em bytes do texto em minúsculas; converte para caracteres.
            Some(byte) => hay[..byte].chars().count().saturating_sub(SNIPPET_BEFORE),
            None => 0,
        }
    };
    let end = (start + SNIPPET_LEN).min(chars.len());
    let body: String = chars[start.min(chars.len())..end].iter().collect();
    format!("{}{}{}", if start > 0 { "…" } else { "" }, flat(&body), if end < chars.len() { "…" } else { "" })
}

pub async fn find(s: &PgSession, o: &FindOptions) -> Result<FindResult, DatabaseError> {
    let text = o.text.trim_end_matches(['\r', '\n']);
    if text.is_empty() || !(o.names || o.columns || o.definitions || o.comments) {
        return Ok(FindResult { hits: Vec::new(), truncated: false });
    }
    let v = s.server.version_num;
    // Funções (sem agregações/janelas): `prokind` é do PostgreSQL 11; antes eram flags.
    let fn_filter = if v >= 110000 { "p.prokind IN ('f','p')" } else { "NOT p.proisagg AND NOT p.proiswindow" };
    let mat = v >= 90300;
    let schema_ok = "n.nspname !~ '^pg_' AND n.nspname <> 'information_schema' AND ($2::text IS NULL OR n.nspname = $2)";
    let rel_kinds = if mat { "'r','p','v','m','S','f'" } else { "'r','p','v','S','f'" };
    let kind_case = "CASE c.relkind WHEN 'v' THEN 'view'::text WHEN 'm' THEN 'matview' WHEN 'S' THEN 'sequence' ELSE 'table' END";
    let col_kinds = if mat { "'r','p','v','m','f'" } else { "'r','p','v','f'" };
    let def_kinds = if mat { "'v','m'" } else { "'v'" };
    // Tabelas particionadas ('p') existem só a partir do 10; o valor inexistente é inofensivo no IN.
    let type_where = "(t.typtype IN ('e','d','r') OR (t.typtype = 'c' AND EXISTS (SELECT 1 FROM pg_catalog.pg_class tc WHERE tc.oid = t.typrelid AND tc.relkind = 'c')))";

    // Cada parte devolve: schema, kind, name, identity, column, matched_in, text (definição/comentário para o trecho).
    let mut parts: Vec<String> = Vec::new();
    if o.names {
        parts.push(format!(
            "SELECT n.nspname::text, {kind_case}, c.relname::text, NULL::text, NULL::text, 'name'::text, NULL::text \
               FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
              WHERE c.relkind IN ({rel_kinds}) AND {schema_ok} AND {}",
            matches("c.relname", o)
        ));
        parts.push(format!(
            "SELECT n.nspname::text, 'function'::text, p.proname::text, pg_get_function_identity_arguments(p.oid), NULL::text, 'name'::text, NULL::text \
               FROM pg_catalog.pg_proc p JOIN pg_catalog.pg_namespace n ON n.oid = p.pronamespace \
              WHERE {fn_filter} AND {schema_ok} AND {}",
            matches("p.proname", o)
        ));
        parts.push(format!(
            "SELECT n.nspname::text, 'type'::text, t.typname::text, NULL::text, NULL::text, 'name'::text, NULL::text \
               FROM pg_catalog.pg_type t JOIN pg_catalog.pg_namespace n ON n.oid = t.typnamespace \
              WHERE {type_where} AND {schema_ok} AND {}",
            matches("t.typname", o)
        ));
    }
    if o.columns {
        parts.push(format!(
            "SELECT n.nspname::text, {kind_case}, c.relname::text, NULL::text, a.attname::text, 'column'::text, NULL::text \
               FROM pg_catalog.pg_attribute a JOIN pg_catalog.pg_class c ON c.oid = a.attrelid \
               JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
              WHERE c.relkind IN ({col_kinds}) AND a.attnum > 0 AND NOT a.attisdropped AND {schema_ok} AND {}",
            matches("a.attname", o)
        ));
    }
    if o.definitions {
        parts.push(format!(
            "SELECT n.nspname::text, {kind_case}, c.relname::text, NULL::text, NULL::text, 'definition'::text, left(pg_get_viewdef(c.oid), {MAX_DEFINITION_CHARS}) \
               FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
              WHERE c.relkind IN ({def_kinds}) AND {schema_ok} AND {}",
            matches("pg_get_viewdef(c.oid)", o)
        ));
        parts.push(format!(
            "SELECT n.nspname::text, 'function'::text, p.proname::text, pg_get_function_identity_arguments(p.oid), NULL::text, 'definition'::text, left(p.prosrc, {MAX_DEFINITION_CHARS}) \
               FROM pg_catalog.pg_proc p JOIN pg_catalog.pg_namespace n ON n.oid = p.pronamespace \
              WHERE {fn_filter} AND p.prolang NOT IN (SELECT oid FROM pg_catalog.pg_language WHERE lanname IN ('internal', 'c')) AND {schema_ok} AND {}",
            matches("p.prosrc", o)
        ));
    }
    if o.comments {
        parts.push(format!(
            "SELECT n.nspname::text, {kind_case}, c.relname::text, NULL::text, NULL::text, 'comment'::text, obj_description(c.oid, 'pg_class') \
               FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
              WHERE c.relkind IN ({rel_kinds}) AND {schema_ok} AND {}",
            matches("obj_description(c.oid, 'pg_class')", o)
        ));
        parts.push(format!(
            "SELECT n.nspname::text, {kind_case}, c.relname::text, NULL::text, a.attname::text, 'comment'::text, col_description(c.oid, a.attnum) \
               FROM pg_catalog.pg_attribute a JOIN pg_catalog.pg_class c ON c.oid = a.attrelid \
               JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
              WHERE c.relkind IN ({col_kinds}) AND a.attnum > 0 AND NOT a.attisdropped AND {schema_ok} AND {}",
            matches("col_description(c.oid, a.attnum)", o)
        ));
        parts.push(format!(
            "SELECT n.nspname::text, 'function'::text, p.proname::text, pg_get_function_identity_arguments(p.oid), NULL::text, 'comment'::text, obj_description(p.oid, 'pg_proc') \
               FROM pg_catalog.pg_proc p JOIN pg_catalog.pg_namespace n ON n.oid = p.pronamespace \
              WHERE {fn_filter} AND {schema_ok} AND {}",
            matches("obj_description(p.oid, 'pg_proc')", o)
        ));
        parts.push(format!(
            "SELECT n.nspname::text, 'type'::text, t.typname::text, NULL::text, NULL::text, 'comment'::text, obj_description(t.oid, 'pg_type') \
               FROM pg_catalog.pg_type t JOIN pg_catalog.pg_namespace n ON n.oid = t.typnamespace \
              WHERE {type_where} AND {schema_ok} AND {}",
            matches("obj_description(t.oid, 'pg_type')", o)
        ));
    }
    let sql = format!(
        "SELECT * FROM ({}) q ORDER BY 1, 2, 3, 5 NULLS FIRST, 6 LIMIT {}",
        parts.join(" UNION ALL "),
        MAX_HITS + 1
    );
    // Outra sessão pode apagar um objeto entre a leitura do catálogo e `pg_get_viewdef`/`obj_description`: o servidor
    // responde "could not open relation" / "cache lookup failed". É passageiro, então tenta mais uma ou duas vezes.
    let mut attempt = 0;
    let rows = loop {
        match s.client.query(&sql, &[&text, &o.schema]).await {
            Ok(rows) => break rows,
            Err(e) if attempt < 2 && is_transient_catalog_error(&e) => attempt += 1,
            Err(e) => return Err(map_error(&e)),
        }
    };
    let truncated = rows.len() > MAX_HITS;
    let hits = rows
        .iter()
        .take(MAX_HITS)
        .map(|r| {
            let matched_in: String = r.get(5);
            let body: Option<String> = r.get(6);
            FindHit {
                schema: r.get(0),
                kind: r.get(1),
                name: r.get(2),
                identity: r.get(3),
                column: r.get(4),
                snippet: body.map(|b| snippet(&b, text, o.match_case, o.regex)),
                matched_in,
            }
        })
        .collect();
    Ok(FindResult { hits, truncated })
}

#[cfg(test)]
mod tests {
    use super::snippet;

    #[test]
    fn snippet_centers_on_the_match_and_flattens_whitespace() {
        let body = format!("{}\n\nSELECT   Cliente_Id FROM x {}", "a".repeat(200), "b".repeat(200));
        let s = snippet(&body, "cliente_id", false, false);
        assert!(s.starts_with('…') && s.ends_with('…'), "{s}");
        assert!(s.contains("SELECT Cliente_Id FROM x"), "{s}");
        assert!(!s.contains('\n'));
        // Sem ocorrência literal (regex) mostra o começo.
        assert!(snippet("abc def", "x+", false, true).starts_with("abc"));
        // Texto curto: sem reticências.
        assert_eq!(snippet("uma frase", "frase", false, false), "uma frase");
    }
}
