//! Console do servidor (Tools → Console): um terminal no estilo do `psql`.
//!
//! A tela só guarda a rolagem e o que se digita; aqui ficam a decisão de o comando estar completo, os comandos com `\`,
//! a formatação alinhada dos resultados e das mensagens de erro, para o texto sair igual ao do `psql` e poder ser testado.

use crate::connect::PgSession;
use crate::exec::execute;
use crate::history::format_atlas_error;
use atlas_core::{DatabaseError, ResultSet};
use serde::{Deserialize, Serialize};

/// Ajustes da sessão do console (`\timing` e `\x`).
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConsoleOptions {
    pub timing: bool,
    pub expanded: bool,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConsoleOutput {
    pub text: String,
    /// `\c banco`: o console passa a falar com este banco.
    pub switch_db: Option<String>,
    /// `\q`.
    pub quit: bool,
    pub options: ConsoleOptions,
}

/// O texto digitado já forma um comando completo? Termina em `;` (fora de aspas, `$tag$` e comentários) ou é um comando com
/// `\` numa linha só.
pub fn is_complete(buffer: &str) -> bool {
    let t = buffer.trim_start();
    if t.starts_with('\\') {
        return !t.trim().is_empty();
    }
    let b = buffer.as_bytes();
    let mut i = 0;
    let mut ended = false;
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
                let mut depth = 0;
                while i < b.len() {
                    if b[i] == b'/' && b.get(i + 1) == Some(&b'*') {
                        depth += 1;
                        i += 2;
                    } else if b[i] == b'*' && b.get(i + 1) == Some(&b'/') {
                        depth -= 1;
                        i += 2;
                        if depth == 0 {
                            break;
                        }
                    } else {
                        i += 1;
                    }
                }
                if depth > 0 {
                    return false;
                }
            }
            q @ (b'\'' | b'"') => {
                ended = false;
                i += 1;
                loop {
                    if i >= b.len() {
                        return false;
                    }
                    if b[i] == q {
                        if b.get(i + 1) == Some(&q) {
                            i += 2;
                            continue;
                        }
                        i += 1;
                        break;
                    }
                    i += 1;
                }
            }
            b'$' => {
                // `$tag$ ... $tag$`
                let rest = &buffer[i..];
                if let Some(end) = rest[1..].find('$') {
                    let tag = &rest[..end + 2];
                    if tag[1..tag.len() - 1].chars().all(|c| c.is_alphanumeric() || c == '_') {
                        ended = false;
                        match rest[tag.len()..].find(tag) {
                            Some(close) => {
                                i += tag.len() + close + tag.len();
                                continue;
                            }
                            None => return false,
                        }
                    }
                }
                ended = false;
                i += 1;
            }
            _ => {
                ended = false;
                i += 1;
            }
        }
    }
    ended || split_go(buffer).is_some()
}

/// `comando \g` ou `comando \gx` (resultado expandido só desta vez): o texto sem o sufixo e se é o `\gx`.
fn split_go(buffer: &str) -> Option<(&str, bool)> {
    let t = buffer.trim_end();
    for (suffix, expanded) in [("\\gx", true), ("\\g", false)] {
        if let Some(head) = t.strip_suffix(suffix)
            && !head.trim().is_empty()
        {
            return Some((head, expanded));
        }
    }
    None
}

fn is_numeric_type(t: Option<&str>) -> bool {
    matches!(t, Some("int2" | "int4" | "int8" | "numeric" | "float4" | "float8" | "oid" | "money" | "xid" | "cid"))
}

fn center(text: &str, width: usize) -> String {
    let len = text.chars().count();
    if len >= width {
        return text.to_string();
    }
    let left = (width - len) / 2;
    format!("{}{text}{}", " ".repeat(left), " ".repeat(width - len - left))
}

/// O resultado como o `psql` o imprime (aligned): cabeçalho centralizado, números à direita, `(n rows)` no fim.
pub fn format_table(rs: &ResultSet, title: Option<&str>, expanded: bool) -> String {
    render(rs, title, expanded, true)
}

fn render(rs: &ResultSet, title: Option<&str>, expanded: bool, with_footer: bool) -> String {
    let count = rs.rows.len();
    let footer = format!("({count} {})", if count == 1 { "row" } else { "rows" });
    let cell = |v: &Option<String>| v.clone().unwrap_or_default().replace('\n', "\u{21b5}");
    if expanded {
        let name_w = rs.columns.iter().map(|c| c.name.chars().count()).max().unwrap_or(0);
        let mut out = String::new();
        if let Some(t) = title {
            out.push_str(&format!("{t}\n"));
        }
        if count == 0 {
            return format!("{out}(0 rows)\n");
        }
        let val_w = rs.rows.iter().flat_map(|r| r.iter().map(|v| cell(v).chars().count())).max().unwrap_or(0).max(1);
        for (n, row) in rs.rows.iter().enumerate() {
            let head = format!("-[ RECORD {} ]", n + 1);
            let left = head.chars().count().max(name_w + 1);
            out.push_str(&format!("{head}{}+{}\n", "-".repeat(left - head.chars().count()), "-".repeat(val_w + 1)));
            for (c, v) in rs.columns.iter().zip(row) {
                out.push_str(&format!("{:<name_w$} | {}\n", c.name, cell(v)));
            }
        }
        return out;
    }
    let widths: Vec<usize> = rs
        .columns
        .iter()
        .enumerate()
        .map(|(i, c)| rs.rows.iter().map(|r| cell(&r[i]).chars().count()).max().unwrap_or(0).max(c.name.chars().count()))
        .collect();
    let mut out = String::new();
    let total: usize = widths.iter().sum::<usize>() + 3 * widths.len().saturating_sub(1) + 2;
    if let Some(t) = title {
        out.push_str(&format!("{}\n", center(t, total).trim_end()));
    }
    let head: Vec<String> = rs.columns.iter().zip(&widths).map(|(c, w)| center(&c.name, *w)).collect();
    out.push_str(&format!(" {}\n", head.join(" | ").trim_end()));
    out.push_str(&format!("{}\n", widths.iter().map(|w| "-".repeat(w + 2)).collect::<Vec<_>>().join("+")));
    for row in &rs.rows {
        let cells: Vec<String> = row
            .iter()
            .enumerate()
            .map(|(i, v)| {
                let text = cell(v);
                if is_numeric_type(rs.columns[i].type_name.as_deref()) { format!("{text:>w$}", w = widths[i]) } else { format!("{text:<w$}", w = widths[i]) }
            })
            .collect();
        out.push_str(&format!(" {}\n", cells.join(" | ").trim_end()));
    }
    if with_footer {
        out.push_str(&format!("{footer}\n"));
    }
    out
}

/// A etiqueta que o `psql` imprime depois de um comando sem resultado (`INSERT 0 1`, `CREATE TABLE`...).
pub fn command_tag(statement: &str, affected: u64) -> String {
    let mut words: Vec<String> = Vec::new();
    let mut rest = statement.trim_start();
    // Comentários no começo não contam.
    loop {
        if let Some(r) = rest.strip_prefix("--") {
            rest = r.split_once('\n').map(|(_, x)| x).unwrap_or("").trim_start();
        } else if let Some(r) = rest.strip_prefix("/*") {
            rest = r.split_once("*/").map(|(_, x)| x).unwrap_or("").trim_start();
        } else {
            break;
        }
    }
    for w in rest.split_whitespace().take(6) {
        words.push(w.trim_end_matches(';').to_uppercase());
    }
    let first = words.first().cloned().unwrap_or_default();
    match first.as_str() {
        "INSERT" => format!("INSERT 0 {affected}"),
        "UPDATE" | "DELETE" | "MERGE" | "SELECT" | "COPY" | "FETCH" | "MOVE" => format!("{first} {affected}"),
        "CREATE" | "ALTER" | "DROP" => {
            let noise = ["OR", "REPLACE", "UNIQUE", "TEMP", "TEMPORARY", "UNLOGGED", "GLOBAL", "LOCAL", "CONSTRAINT", "RECURSIVE", "CONCURRENTLY"];
            let kind: Vec<&String> = words.iter().skip(1).filter(|w| !noise.contains(&w.as_str())).take(if words.get(1).map(String::as_str) == Some("MATERIALIZED") || words.get(1).map(String::as_str) == Some("FOREIGN") { 2 } else { 1 }).collect();
            format!("{first} {}", kind.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(" "))
        }
        "WITH" => format!("SELECT {affected}"),
        "" => String::new(),
        "START" => "START TRANSACTION".into(),
        _ => first,
    }
}

/// Quebra a linha de um comando com `\` em nome e argumento.
fn split_meta(line: &str) -> (String, String) {
    let t = line.trim();
    let name_end = t[1..].find(|c: char| c.is_whitespace()).map(|i| i + 1).unwrap_or(t.len());
    (t[1..name_end].to_string(), t[name_end..].trim().to_string())
}

fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

async fn query(s: &PgSession, sql: &str) -> Result<ResultSet, DatabaseError> {
    let out = execute(s, sql, 10_000).await?;
    Ok(out.statements.into_iter().find_map(|st| st.result).unwrap_or(ResultSet { columns: vec![], rows: vec![], truncated: false }))
}

const HELP: &str = "General\n  \\q                     quit the console\n  \\?                     show this help\n  \\conninfo              display information about the current connection\n  \\c [DBNAME]            connect to another database\n\nInformational\n  \\l                     list databases\n  \\dn                    list schemas\n  \\dt                    list tables\n  \\dv                    list views\n  \\dm                    list materialized views\n  \\di                    list indexes\n  \\ds                    list sequences\n  \\df                    list functions\n  \\du                    list roles\n  \\d NAME                describe a table, view, index or sequence\n\nFormatting\n  \\timing [on|off]       toggle the time each command took\n  \\x [on|off]            toggle expanded output\n";

fn relations(kinds: &str, title: &str, none: &str, plus: bool) -> (String, String, String) {
    // `+`: persistência, tamanho e descrição.
    let more = if plus {
        ", CASE c.relpersistence WHEN 'p' THEN 'permanent' WHEN 'u' THEN 'unlogged' ELSE 'temporary' END AS \"Persistence\", \
           pg_catalog.pg_size_pretty(pg_catalog.pg_table_size(c.oid)) AS \"Size\", pg_catalog.obj_description(c.oid, 'pg_class') AS \"Description\""
    } else {
        ""
    };
    (
        format!(
            "SELECT n.nspname AS \"Schema\", c.relname AS \"Name\", \
                    CASE c.relkind WHEN 'r' THEN 'table' WHEN 'v' THEN 'view' WHEN 'm' THEN 'materialized view' WHEN 'i' THEN 'index' \
                         WHEN 'S' THEN 'sequence' WHEN 'f' THEN 'foreign table' WHEN 'p' THEN 'partitioned table' END AS \"Type\", \
                    pg_catalog.pg_get_userbyid(c.relowner) AS \"Owner\"{more} \
               FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
              WHERE c.relkind IN ({kinds}) AND n.nspname <> 'pg_catalog' AND n.nspname !~ '^pg_toast' AND n.nspname <> 'information_schema' \
                AND pg_catalog.pg_table_is_visible(c.oid) ORDER BY 1, 2"
        ),
        title.to_string(),
        none.to_string(),
    )
}

/// `\d nome`: colunas e, depois, índices, restrições, chaves estrangeiras, referências e gatilhos.
async fn describe(s: &PgSession, name: &str, expanded: bool, plus: bool) -> Result<String, DatabaseError> {
    let found = query(s, &format!("SELECT c.oid::int8, n.nspname, c.relname, c.relkind::text FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace WHERE c.oid = pg_catalog.to_regclass({})::oid", quote(name))).await?;
    let Some(row) = found.rows.first() else { return Ok(format!("Did not find any relation named \"{name}\".\n")) };
    let (oid, schema, rel, kind) = (row[0].clone().unwrap_or_default(), row[1].clone().unwrap_or_default(), row[2].clone().unwrap_or_default(), row[3].clone().unwrap_or_default());
    let label = match kind.as_str() {
        "r" => "Table",
        "v" => "View",
        "m" => "Materialized view",
        "i" => "Index",
        "S" => "Sequence",
        "f" => "Foreign table",
        "p" => "Partitioned table",
        _ => "Relation",
    };
    // `\d+`: armazenamento e descrição de cada coluna.
    let more = if plus {
        ", CASE a.attstorage WHEN 'p' THEN 'plain' WHEN 'e' THEN 'external' WHEN 'm' THEN 'main' WHEN 'x' THEN 'extended' END AS \"Storage\", \
           pg_catalog.col_description(a.attrelid, a.attnum) AS \"Description\""
    } else {
        ""
    };
    let columns = query(
        s,
        &format!(
            "SELECT a.attname AS \"Column\", pg_catalog.format_type(a.atttypid, a.atttypmod) AS \"Type\", \
                    (SELECT c.collname FROM pg_catalog.pg_collation c, pg_catalog.pg_type t WHERE c.oid = a.attcollation AND t.oid = a.atttypid AND a.attcollation <> t.typcollation) AS \"Collation\", \
                    CASE WHEN a.attnotnull THEN 'not null' ELSE '' END AS \"Nullable\", \
                    COALESCE(pg_catalog.pg_get_expr(d.adbin, d.adrelid), '') AS \"Default\"{more} \
               FROM pg_catalog.pg_attribute a LEFT JOIN pg_catalog.pg_attrdef d ON d.adrelid = a.attrelid AND d.adnum = a.attnum \
              WHERE a.attrelid = {oid} AND a.attnum > 0 AND NOT a.attisdropped ORDER BY a.attnum"
        ),
    )
    .await?;
    let mut out = render(&columns, Some(&format!("{label} \"{schema}.{rel}\"")), expanded, false);
    out.push('\n');
    let section = |title: &str, rows: &[String]| -> String {
        if rows.is_empty() { String::new() } else { format!("{title}\n{}\n", rows.iter().map(|r| format!("    {r}")).collect::<Vec<_>>().join("\n")) }
    };
    let lines = |rs: ResultSet| -> Vec<String> { rs.rows.into_iter().filter_map(|r| r.into_iter().next().flatten()).collect() };
    let indexes = lines(
        query(
            s,
            &format!(
                "SELECT '\"' || c2.relname || '\" ' || CASE WHEN i.indisprimary THEN 'PRIMARY KEY, ' WHEN i.indisunique THEN 'UNIQUE, ' ELSE '' END \
                        || substring(pg_catalog.pg_get_indexdef(i.indexrelid) from 'USING (.*)$') \
                   FROM pg_catalog.pg_index i JOIN pg_catalog.pg_class c2 ON c2.oid = i.indexrelid WHERE i.indrelid = {oid} \
                  ORDER BY i.indisprimary DESC, c2.relname"
            ),
        )
        .await?,
    );
    let constraint = |ty: &str| format!("SELECT '\"' || conname || '\" ' || pg_catalog.pg_get_constraintdef(oid, true) FROM pg_catalog.pg_constraint WHERE conrelid = {oid} AND contype = '{ty}' ORDER BY conname");
    let checks = lines(query(s, &constraint("c")).await?);
    let fks = lines(query(s, &constraint("f")).await?);
    let refs = lines(
        query(
            s,
            &format!(
                "SELECT 'TABLE \"' || c.relname || '\" CONSTRAINT \"' || con.conname || '\" ' || pg_catalog.pg_get_constraintdef(con.oid, true) \
                   FROM pg_catalog.pg_constraint con JOIN pg_catalog.pg_class c ON c.oid = con.conrelid WHERE con.confrelid = {oid} AND con.contype = 'f' ORDER BY 1"
            ),
        )
        .await?,
    );
    let triggers = lines(
        query(
            s,
            &format!(
                "SELECT regexp_replace(pg_catalog.pg_get_triggerdef(t.oid, true), '^CREATE( CONSTRAINT)? TRIGGER ', '') FROM pg_catalog.pg_trigger t WHERE t.tgrelid = {oid} AND NOT t.tgisinternal ORDER BY t.tgname"
            ),
        )
        .await?,
    );
    out.push_str(&section("Indexes:", &indexes));
    out.push_str(&section("Check constraints:", &checks));
    out.push_str(&section("Foreign-key constraints:", &fks));
    out.push_str(&section("Referenced by:", &refs));
    out.push_str(&section("Triggers:", &triggers));
    if plus {
        // `\d+`: as opções de armazenamento e o tamanho da relação.
        let extra = query(
            s,
            &format!(
                "SELECT COALESCE(array_to_string(c.reloptions, ', '), ''), pg_catalog.pg_size_pretty(pg_catalog.pg_table_size(c.oid)), COALESCE(pg_catalog.obj_description(c.oid, 'pg_class'), '') \
                   FROM pg_catalog.pg_class c WHERE c.oid = {oid}"
            ),
        )
        .await?;
        if let Some(r) = extra.rows.first() {
            let get = |i: usize| r[i].clone().unwrap_or_default();
            if !get(2).is_empty() {
                out.push_str(&format!("Description: {}\n", get(2)));
            }
            if !get(0).is_empty() {
                out.push_str(&format!("Options: {}\n", get(0)));
            }
            out.push_str(&format!("Size: {}\n", get(1)));
        }
    }
    Ok(out)
}

fn timing_line(ms: f64) -> String {
    format!("Time: {ms:.3} ms\n")
}

fn flag(arg: &str, current: bool) -> bool {
    match arg.to_ascii_lowercase().as_str() {
        "on" | "true" | "1" => true,
        "off" | "false" | "0" => false,
        _ => !current,
    }
}

/// Executa um comando completo (SQL ou com `\`) e devolve o texto a imprimir.
pub async fn run(s: &PgSession, input: &str, options: ConsoleOptions) -> ConsoleOutput {
    let mut out = ConsoleOutput { options, ..Default::default() };
    let started = std::time::Instant::now();
    let trimmed = input.trim();
    if let Some(meta) = trimmed.strip_prefix('\\') {
        let _ = meta;
        let (name, arg) = split_meta(trimmed);
        let plus = name.ends_with('+');
        let result: Result<Option<(String, String, String)>, DatabaseError> = match name.as_str() {
            "q" | "quit" => {
                out.quit = true;
                Ok(None)
            }
            "?" => {
                out.text.push_str(HELP);
                Ok(None)
            }
            "timing" => {
                out.options.timing = flag(&arg, options.timing);
                out.text.push_str(&format!("Timing is {}.\n", if out.options.timing { "on" } else { "off" }));
                Ok(None)
            }
            "x" => {
                out.options.expanded = flag(&arg, options.expanded);
                out.text.push_str(&format!("Expanded display is {}.\n", if out.options.expanded { "on" } else { "off" }));
                Ok(None)
            }
            "conninfo" => {
                out.text.push_str(&format!(
                    "You are connected to database \"{}\" as user \"{}\" via PostgreSQL {}.\n",
                    s.server.current_database, s.server.current_user, s.server.version
                ));
                Ok(None)
            }
            "c" | "connect" => {
                if arg.is_empty() {
                    out.text.push_str(&format!("You are currently connected to database \"{}\" as user \"{}\".\n", s.server.current_database, s.server.current_user));
                } else {
                    out.switch_db = Some(arg.split_whitespace().next().unwrap_or("").to_string());
                }
                Ok(None)
            }
            "l+" | "list+" => Ok(Some((
                "SELECT d.datname AS \"Name\", pg_catalog.pg_get_userbyid(d.datdba) AS \"Owner\", pg_catalog.pg_encoding_to_char(d.encoding) AS \"Encoding\", d.datcollate AS \"Collate\", d.datctype AS \"Ctype\", \
                        CASE WHEN pg_catalog.has_database_privilege(d.datname, 'CONNECT') THEN pg_catalog.pg_size_pretty(pg_catalog.pg_database_size(d.datname)) ELSE 'No Access' END AS \"Size\", \
                        t.spcname AS \"Tablespace\", pg_catalog.shobj_description(d.oid, 'pg_database') AS \"Description\" \
                   FROM pg_catalog.pg_database d JOIN pg_catalog.pg_tablespace t ON t.oid = d.dattablespace ORDER BY 1"
                    .into(),
                "List of databases".into(),
                String::new(),
            ))),
            "dn+" => Ok(Some((
                "SELECT n.nspname AS \"Name\", pg_catalog.pg_get_userbyid(n.nspowner) AS \"Owner\", COALESCE(pg_catalog.array_to_string(n.nspacl, E'\\n'), '') AS \"Access privileges\", \
                        pg_catalog.obj_description(n.oid, 'pg_namespace') AS \"Description\" \
                   FROM pg_catalog.pg_namespace n WHERE n.nspname !~ '^pg_' AND n.nspname <> 'information_schema' ORDER BY 1"
                    .into(),
                "List of schemas".into(),
                String::new(),
            ))),
            "l" | "list" => Ok(Some((
                "SELECT d.datname AS \"Name\", pg_catalog.pg_get_userbyid(d.datdba) AS \"Owner\", pg_catalog.pg_encoding_to_char(d.encoding) AS \"Encoding\", d.datcollate AS \"Collate\", d.datctype AS \"Ctype\" FROM pg_catalog.pg_database d ORDER BY 1".into(),
                "List of databases".into(),
                String::new(),
            ))),
            "dn" => Ok(Some((
                "SELECT n.nspname AS \"Name\", pg_catalog.pg_get_userbyid(n.nspowner) AS \"Owner\" FROM pg_catalog.pg_namespace n WHERE n.nspname !~ '^pg_' AND n.nspname <> 'information_schema' ORDER BY 1".into(),
                "List of schemas".into(),
                String::new(),
            ))),
            "dt" | "dt+" => Ok(Some(relations("'r','p'", "List of relations", "Did not find any relations.", plus))),
            "dv" | "dv+" => Ok(Some(relations("'v'", "List of relations", "Did not find any relations.", plus))),
            "dm" | "dm+" => Ok(Some(relations("'m'", "List of relations", "Did not find any relations.", plus))),
            "di" | "di+" => Ok(Some(relations("'i'", "List of relations", "Did not find any relations.", plus))),
            "ds" | "ds+" => Ok(Some(relations("'S'", "List of relations", "Did not find any relations.", plus))),
            "df" | "df+" => {
                let kinds = if s.server.version_num >= 110000 { "p.prokind IN ('f','p')" } else { "NOT p.proisagg AND NOT p.proiswindow" };
                Ok(Some((
                    format!(
                        "SELECT n.nspname AS \"Schema\", p.proname AS \"Name\", pg_catalog.pg_get_function_result(p.oid) AS \"Result data type\", \
                                pg_catalog.pg_get_function_arguments(p.oid) AS \"Argument data types\" \
                           FROM pg_catalog.pg_proc p JOIN pg_catalog.pg_namespace n ON n.oid = p.pronamespace \
                          WHERE {kinds} AND n.nspname <> 'pg_catalog' AND n.nspname <> 'information_schema' AND pg_catalog.pg_function_is_visible(p.oid) ORDER BY 1, 2, 4"
                    ),
                    "List of functions".into(),
                    "Did not find any functions.".into(),
                )))
            }
            "du+" | "dg+" => Ok(Some((
                "SELECT r.rolname AS \"Role name\", \
                        trim(both ', ' from concat_ws(', ', CASE WHEN r.rolsuper THEN 'Superuser' END, CASE WHEN NOT r.rolinherit THEN 'No inheritance' END, \
                             CASE WHEN r.rolcreaterole THEN 'Create role' END, CASE WHEN r.rolcreatedb THEN 'Create DB' END, CASE WHEN NOT r.rolcanlogin THEN 'Cannot login' END)) AS \"Attributes\", \
                        pg_catalog.shobj_description(r.oid, 'pg_authid') AS \"Description\" \
                   FROM pg_catalog.pg_roles r WHERE r.rolname !~ '^pg_' ORDER BY 1"
                    .into(),
                "List of roles".into(),
                String::new(),
            ))),
            "du" | "dg" => Ok(Some((
                "SELECT r.rolname AS \"Role name\", \
                        trim(both ', ' from concat_ws(', ', CASE WHEN r.rolsuper THEN 'Superuser' END, CASE WHEN NOT r.rolinherit THEN 'No inheritance' END, \
                             CASE WHEN r.rolcreaterole THEN 'Create role' END, CASE WHEN r.rolcreatedb THEN 'Create DB' END, CASE WHEN NOT r.rolcanlogin THEN 'Cannot login' END)) AS \"Attributes\" \
                   FROM pg_catalog.pg_roles r WHERE r.rolname !~ '^pg_' ORDER BY 1"
                    .into(),
                "List of roles".into(),
                String::new(),
            ))),
            "d" if arg.is_empty() => Ok(Some(relations("'r','v','m','S','f','p'", "List of relations", "Did not find any relations.", plus))),
            "d" | "d+" => match describe(s, &arg, options.expanded, plus).await {
                Ok(text) => {
                    out.text.push_str(&text);
                    Ok(None)
                }
                Err(e) => Err(e),
            },
            other => {
                out.text.push_str(&format!("invalid command \\{other}\nTry \\? for help.\n"));
                Ok(None)
            }
        };
        match result {
            Ok(Some((sql, title, none))) => match query(s, &sql).await {
                Ok(rs) if rs.rows.is_empty() && !none.is_empty() => out.text.push_str(&format!("{none}\n")),
                Ok(rs) => out.text.push_str(&format_table(&rs, Some(&title), options.expanded)),
                Err(e) => out.text.push_str(&format!("{}\n", format_atlas_error(&e, &sql))),
            },
            Ok(None) => {}
            Err(e) => out.text.push_str(&format!("{}\n", format_atlas_error(&e, trimmed))),
        }
        return out;
    }

    // `\g` e `\gx` fecham o comando no lugar do `;`.
    let (input, once_expanded) = match split_go(input) {
        Some((head, expanded)) => (head, expanded),
        None => (input, false),
    };
    let expanded = options.expanded || once_expanded;
    match execute(s, input, 10_000).await {
        Ok(outcome) => {
            // O servidor devolve um resultado por comando; as etiquetas dependem do texto de cada um, então separa os comandos
            // só para dar nome às etiquetas (a execução já aconteceu).
            let mut tags = split_statements(input).into_iter();
            for st in &outcome.statements {
                let sql = tags.next().unwrap_or_default();
                match &st.result {
                    Some(rs) => {
                        out.text.push_str(&format_table(rs, None, expanded));
                        if rs.truncated {
                            out.text.push_str("(output truncated)\n");
                        }
                    }
                    None => out.text.push_str(&format!("{}\n", command_tag(&sql, st.rows_affected.unwrap_or(0)))),
                }
                out.text.push('\n');
            }
        }
        Err(e) => {
            out.text.push_str(&format!("{}\n", format_atlas_error(&e, input)));
        }
    }
    if options.timing {
        out.text.push_str(&timing_line(started.elapsed().as_secs_f64() * 1000.0));
    }
    out
}

/// Divide o texto nos `;` de fora de aspas, `$tag$` e comentários (só para dar nome às etiquetas dos comandos).
pub fn split_statements(sql: &str) -> Vec<String> {
    let b = sql.as_bytes();
    let (mut out, mut start, mut i) = (Vec::new(), 0usize, 0usize);
    while i < b.len() {
        match b[i] {
            b';' => {
                out.push(sql[start..i].trim().to_string());
                start = i + 1;
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
            q @ (b'\'' | b'"') => {
                i += 1;
                while i < b.len() {
                    if b[i] == q {
                        if b.get(i + 1) == Some(&q) {
                            i += 2;
                            continue;
                        }
                        break;
                    }
                    i += 1;
                }
                i += 1;
            }
            b'$' => {
                let rest = &sql[i..];
                match rest[1..].find('$') {
                    Some(end) if rest[1..end + 1].chars().all(|c| c.is_alphanumeric() || c == '_') => {
                        let tag = &rest[..end + 2];
                        match rest[tag.len()..].find(tag) {
                            Some(close) => i += tag.len() + close + tag.len(),
                            None => i = b.len(),
                        }
                    }
                    _ => i += 1,
                }
            }
            _ => i += 1,
        }
    }
    if !sql[start.min(sql.len())..].trim().is_empty() {
        out.push(sql[start..].trim().to_string());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use atlas_core::ResultColumn;

    fn rs(cols: &[(&str, Option<&str>)], rows: &[&[Option<&str>]]) -> ResultSet {
        ResultSet {
            columns: cols.iter().map(|(n, t)| ResultColumn { name: n.to_string(), type_name: t.map(String::from) }).collect(),
            rows: rows.iter().map(|r| r.iter().map(|v| v.map(String::from)).collect()).collect(),
            truncated: false,
        }
    }

    #[test]
    fn a_command_is_complete_after_a_closing_semicolon_outside_quotes_and_comments() {
        assert!(is_complete("SELECT 1;"));
        assert!(is_complete("SELECT 1;   -- fim\n"));
        assert!(is_complete("select 1; select 2;"));
        assert!(!is_complete("SELECT 1"));
        assert!(!is_complete("SELECT ';'"));
        assert!(!is_complete("SELECT 'abc"));
        assert!(!is_complete("SELECT 1 /* ; */"));
        assert!(!is_complete("DO $$ BEGIN PERFORM 1; END $$"));
        assert!(is_complete("DO $$ BEGIN PERFORM 1; END $$;"));
        assert!(!is_complete("SELECT 1; SELECT 2"));
        assert!(is_complete("\\dt"));
        assert!(!is_complete("   "));
        // `\g` e `\gx` terminam o comando no lugar do `;`.
        assert!(is_complete("SELECT 1 \\g"));
        assert!(is_complete("SELECT 1\n\\gx"));
        assert!(!is_complete("SELECT '\\g"));
        assert_eq!(split_go("SELECT 1 \\g"), Some(("SELECT 1 ", false)));
        assert_eq!(split_go("SELECT 1 \\gx "), Some(("SELECT 1 ", true)));
        assert_eq!(split_go("SELECT 1;"), None);
    }

    #[test]
    fn tables_print_like_psql_with_centered_headers_and_right_aligned_numbers() {
        let r = rs(&[("id", Some("int4")), ("name", Some("text"))], &[&[Some("1"), Some("Ana")], &[Some("22"), None], &[Some("3"), Some("Bruno")]]);
        assert_eq!(format_table(&r, None, false), " id | name\n----+-------\n  1 | Ana\n 22 |\n  3 | Bruno\n(3 rows)\n");
        let one = rs(&[("x", Some("int4"))], &[&[Some("7")]]);
        assert_eq!(format_table(&one, Some("Title"), false), "Title\n x\n---\n 7\n(1 row)\n");
        let wide = rs(&[("name", Some("text"))], &[&[Some("abcdefghij")]]);
        assert_eq!(format_table(&wide, Some("List"), false), "    List\n    name\n------------\n abcdefghij\n(1 row)\n");
        let empty = rs(&[("a", None)], &[]);
        assert_eq!(format_table(&empty, None, false), " a\n---\n(0 rows)\n");
    }

    #[test]
    fn expanded_output_uses_record_headers() {
        let r = rs(&[("id", Some("int4")), ("name", Some("text"))], &[&[Some("1"), Some("Ana")], &[Some("2"), Some("Bruno")]]);
        assert_eq!(format_table(&r, None, true), "-[ RECORD 1 ]+------\nid   | 1\nname | Ana\n-[ RECORD 2 ]+------\nid   | 2\nname | Bruno\n");
    }

    #[test]
    fn command_tags_follow_psql() {
        assert_eq!(command_tag("insert into t values (1)", 1), "INSERT 0 1");
        assert_eq!(command_tag("UPDATE t SET a = 1", 4), "UPDATE 4");
        assert_eq!(command_tag("DELETE FROM t", 0), "DELETE 0");
        assert_eq!(command_tag("CREATE TABLE t (a int)", 0), "CREATE TABLE");
        assert_eq!(command_tag("create or replace function f() returns int as $$ select 1 $$ language sql", 0), "CREATE FUNCTION");
        assert_eq!(command_tag("CREATE UNIQUE INDEX i ON t (a)", 0), "CREATE INDEX");
        assert_eq!(command_tag("CREATE MATERIALIZED VIEW m AS SELECT 1", 0), "CREATE MATERIALIZED VIEW");
        assert_eq!(command_tag("-- nota\nDROP TABLE t", 0), "DROP TABLE");
        assert_eq!(command_tag("BEGIN", 0), "BEGIN");
        assert_eq!(command_tag("set search_path = x", 0), "SET");
    }

    #[test]
    fn statements_split_outside_quotes() {
        assert_eq!(split_statements("select 1; select ';'; select $$a;b$$"), vec!["select 1", "select ';'", "select $$a;b$$"]);
    }
}
