//! Driver SQLite do Atlas: abre um arquivo, lê a estrutura e executa SQL devolvendo os mesmos formatos de resposta do PostgreSQL,
//! para a interface usar as mesmas telas. Tudo é síncrono (o SQLite é uma biblioteca no próprio processo); quem chama decide em que
//! thread rodar.

use atlas_core::{
    ColumnInfo, ConstraintInfo, DatabaseError, ErrorKind, IndexInfo, ObjectInfo, OrderBy, QueryOutcome, ResultColumn, ResultSet, SchemaObjects, ServerInfo,
    StatementOutcome, TableDetail, TableFilter, TriggerInfo,
};
use rusqlite::fallible_iterator::FallibleIterator;
use rusqlite::types::ValueRef;
use rusqlite::{Connection, InterruptHandle, OpenFlags};
use serde::{Deserialize, Serialize};

pub mod console;
pub mod diagram;
pub mod view;
pub mod dump;
pub mod find;
pub mod import;
pub mod profile;
pub use console::{Completion as ConsoleCompletion, ConsoleOptions, ConsoleOutput, is_complete as console_is_complete};
pub use dump::DumpPlan;
pub use find::{FindDataResult, FindOptions, FindResult};
pub use import::{Feed as ImportFeed, Hooks as ImportHooks, ImportMode, ImportSpec, ImportStats, TargetColumn, declared_type};
pub use profile::DataProfile;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Um comando com parâmetros posicionais (`?`), como a interface o planeja; os valores vão em texto (ou NULL) e o SQLite aplica a
/// afinidade da coluna.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlannedStatement {
    pub sql: String,
    #[serde(default)]
    pub params: Vec<Option<String>>,
    /// UPDATE/DELETE de linha com chave: precisa afetar exatamente 1 linha, senão é conflito.
    #[serde(default)]
    pub expect_one_row: bool,
    /// UPDATE/DELETE em tabela sem chave: mexe em exatamente esta quantidade de linhas iguais.
    #[serde(default)]
    pub expect_rows: Option<u64>,
}

pub fn quote_ident(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

fn unsupported(what: &str) -> DatabaseError {
    DatabaseError::new(ErrorKind::UnsupportedFeature, format!("{what} is not available for SQLite"))
}

/// Erro do SQLite na forma da interface (categoria, código nativo e posição no SQL, quando há).
fn map_err(e: rusqlite::Error) -> DatabaseError {
    use rusqlite::ErrorCode as C;
    match e {
        rusqlite::Error::SqlInputError { error, msg, offset, .. } => {
            let mut err = DatabaseError::new(ErrorKind::SqlSyntaxError, msg);
            err.code = Some(error.extended_code.to_string());
            err.position = u32::try_from(offset.max(0)).ok().map(|o| o + 1);
            err
        }
        rusqlite::Error::SqliteFailure(f, msg) => {
            let text = msg.unwrap_or_else(|| f.to_string());
            let kind = match f.code {
                C::ConstraintViolation => ErrorKind::ConstraintViolation,
                C::OperationInterrupted => ErrorKind::Canceled,
                C::CannotOpen => ErrorKind::DatabaseNotFound,
                C::PermissionDenied | C::ReadOnly => ErrorKind::PermissionDenied,
                C::NotADatabase => ErrorKind::InvalidInput,
                C::DatabaseBusy | C::DatabaseLocked => ErrorKind::Timeout,
                _ if text.contains("syntax error") => ErrorKind::SqlSyntaxError,
                _ => ErrorKind::Server,
            };
            let mut err = DatabaseError::new(kind, text);
            err.code = Some(f.extended_code.to_string());
            err
        }
        other => DatabaseError::new(ErrorKind::Internal, other.to_string()),
    }
}

/* ---------- Valores ---------- */

/// O valor como texto, no formato que a grade espera: binário em hexa (`\x...`), real sempre com ponto decimal.
fn value_text(v: ValueRef<'_>, boolean: bool) -> Option<String> {
    match v {
        ValueRef::Null => None,
        ValueRef::Integer(i) if boolean => Some(if i == 0 { "f".into() } else { "t".into() }),
        ValueRef::Integer(i) => Some(i.to_string()),
        ValueRef::Real(r) => Some(format!("{r:?}")),
        ValueRef::Text(t) => Some(String::from_utf8_lossy(t).into_owned()),
        ValueRef::Blob(b) => {
            let mut s = String::with_capacity(2 + b.len() * 2);
            s.push_str("\\x");
            for byte in b {
                s.push_str(&format!("{byte:02x}"));
            }
            Some(s)
        }
    }
}

/// Coluna declarada como booleana (o SQLite guarda 0/1; a grade usa `t`/`f`).
fn is_bool_type(decl: Option<&str>) -> bool {
    decl.is_some_and(|d| d.to_ascii_lowercase().contains("bool"))
}

/* ---------- Execução ---------- */

/// Roda um comando que devolve linhas (ou não) e monta o resultado; `max_rows` limita o que fica na memória.
fn run_statement(stmt: &mut rusqlite::Statement<'_>, max_rows: usize, started: Instant) -> Result<StatementOutcome, DatabaseError> {
    let sql_text = stmt.expanded_sql().unwrap_or_default();
    let cols: Vec<(String, Option<String>)> = stmt.columns().iter().map(|c| (c.name().to_string(), c.decl_type().map(str::to_string))).collect();
    if cols.is_empty() {
        let n = stmt.execute([]).map_err(map_err)?;
        let took = started.elapsed().as_secs_f64();
        return Ok(StatementOutcome { result: None, rows_affected: Some(n as u64), sql: sql_text, query_time: took, fetch_time: 0.0 });
    }
    let bools: Vec<bool> = cols.iter().map(|(_, d)| is_bool_type(d.as_deref())).collect();
    let mut rows = stmt.query([]).map_err(map_err)?;
    let mut out: Vec<Vec<Option<String>>> = Vec::new();
    let mut truncated = false;
    let mut first_at: Option<Instant> = None;
    while let Some(row) = rows.next().map_err(map_err)? {
        first_at.get_or_insert_with(Instant::now);
        if out.len() >= max_rows {
            truncated = true;
            break;
        }
        let mut cells = Vec::with_capacity(cols.len());
        for (i, boolean) in bools.iter().enumerate() {
            cells.push(value_text(row.get_ref(i).map_err(map_err)?, *boolean));
        }
        out.push(cells);
    }
    let began = first_at.unwrap_or_else(Instant::now);
    let count = out.len() as u64;
    Ok(StatementOutcome {
        result: Some(ResultSet {
            columns: cols.into_iter().map(|(name, decl)| ResultColumn { name, type_name: decl.filter(|d| !d.is_empty()) }).collect(),
            rows: out,
            truncated,
        }),
        rows_affected: Some(count),
        sql: sql_text,
        query_time: began.duration_since(started).as_secs_f64(),
        fetch_time: began.elapsed().as_secs_f64(),
    })
}

/// Executa o texto (um ou vários comandos separados por `;`).
fn execute_on(conn: &Connection, sql: &str, max_rows: usize) -> Result<QueryOutcome, DatabaseError> {
    let started = Instant::now();
    let mut statements = Vec::new();
    let mut batch = rusqlite::Batch::new(conn, sql);
    loop {
        let mut stmt = match batch.next() {
            Ok(Some(stmt)) => stmt,
            Ok(None) => break,
            Err(e) => return Err(map_err(e)),
        };
        statements.push(run_statement(&mut stmt, max_rows, Instant::now())?);
    }
    Ok(QueryOutcome { statements, elapsed_ms: started.elapsed().as_millis() as u64 })
}

fn count_problem(st: &PlannedStatement, n: u64) -> Option<String> {
    if st.expect_one_row && n != 1 {
        Some("expected exactly 1".to_string())
    } else if let Some(k) = st.expect_rows
        && n != k
    {
        Some(format!("expected exactly {k}"))
    } else {
        None
    }
}

/// Executa os comandos planejados; qualquer falha ou contagem inesperada desfaz o lote (sob SAVEPOINT).
fn apply_on(conn: &Connection, statements: &[PlannedStatement]) -> Result<Vec<u64>, DatabaseError> {
    conn.execute_batch("SAVEPOINT atlas_batch").map_err(map_err)?;
    let mut affected = Vec::with_capacity(statements.len());
    for (i, st) in statements.iter().enumerate() {
        let res = conn.execute(&st.sql, rusqlite::params_from_iter(st.params.iter())).map(|n| n as u64).map_err(map_err);
        let failure = match res {
            Ok(n) => match count_problem(st, n) {
                Some(expected) => {
                    let mut err = DatabaseError::new(ErrorKind::ConcurrencyConflict, format!("statement {} affected {n} rows ({expected}); all changes were rolled back", i + 1));
                    err.detail = Some(st.sql.clone());
                    Some(err)
                }
                None => {
                    affected.push(n);
                    None
                }
            },
            Err(e) => Some(e),
        };
        if let Some(err) = failure {
            let _ = conn.execute_batch("ROLLBACK TO atlas_batch; RELEASE atlas_batch");
            return Err(err);
        }
    }
    conn.execute_batch("RELEASE atlas_batch").map_err(map_err)?;
    Ok(affected)
}

/// `SELECT * FROM tabela [WHERE ...] [ORDER BY ...]`: só identificadores entre aspas entram no texto; o filtro é a expressão do usuário.
pub fn select_sql(table: &str, order_by: &[OrderBy], filter: Option<&TableFilter>) -> String {
    let mut sql = format!("SELECT * FROM {}", quote_ident(table));
    if let Some(f) = filter {
        let expr = f.expr.trim();
        if !expr.is_empty() {
            sql.push_str(&format!(" WHERE {expr}"));
        }
    }
    if !order_by.is_empty() {
        let cols: Vec<String> = order_by.iter().map(|o| format!("{} {}", quote_ident(&o.column), if o.descending { "DESC" } else { "ASC" })).collect();
        sql.push_str(&format!(" ORDER BY {}", cols.join(", ")));
    }
    sql
}

/// Como `select_sql`, mas só com as colunas pedidas (vazio = todas).
pub fn select_columns_sql(table: &str, columns: &[String], order_by: &[OrderBy], filter: Option<&TableFilter>) -> String {
    let sql = select_sql(table, order_by, filter);
    if columns.is_empty() {
        return sql;
    }
    let list = columns.iter().map(|c| quote_ident(c)).collect::<Vec<_>>().join(", ");
    sql.replacen("SELECT *", &format!("SELECT {list}"), 1)
}

/// Lê o resultado de um `SELECT` linha a linha, sem guardar nada na memória, numa conexão própria só de leitura ao arquivo.
/// `on_begin` recebe as colunas e `on_row` cada linha; devolver `Ok(false)` encerra a leitura. Devolve quantas linhas foram entregues.
pub fn stream_rows(
    path: &Path,
    sql: &str,
    on_begin: impl FnOnce(&[ResultColumn]) -> Result<(), DatabaseError>,
    mut on_row: impl FnMut(&[Option<&str>]) -> Result<bool, DatabaseError>,
) -> Result<u64, DatabaseError> {
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX).map_err(map_err)?;
    conn.busy_timeout(Duration::from_secs(5)).map_err(map_err)?;
    let mut stmt = conn.prepare(sql).map_err(map_err)?;
    let cols: Vec<(String, Option<String>)> = stmt.columns().iter().map(|c| (c.name().to_string(), c.decl_type().map(str::to_string))).collect();
    if cols.is_empty() {
        return Ok(0);
    }
    let bools: Vec<bool> = cols.iter().map(|(_, d)| is_bool_type(d.as_deref())).collect();
    let described: Vec<ResultColumn> = cols.into_iter().map(|(name, decl)| ResultColumn { name, type_name: decl.filter(|d| !d.is_empty()) }).collect();
    on_begin(&described)?;
    let mut rows = stmt.query([]).map_err(map_err)?;
    let mut count = 0u64;
    while let Some(row) = rows.next().map_err(map_err)? {
        let cells: Vec<Option<String>> = bools.iter().enumerate().map(|(i, b)| row.get_ref(i).map_err(map_err).map(|v| value_text(v, *b))).collect::<Result<_, _>>()?;
        let refs: Vec<Option<&str>> = cells.iter().map(|c| c.as_deref()).collect();
        if !on_row(&refs)? {
            break;
        }
        count += 1;
    }
    Ok(count)
}

fn rows_on(conn: &Connection, table: &str, order_by: &[OrderBy], filter: Option<&TableFilter>, limit: Option<u32>, offset: u64, max_rows: usize) -> Result<(String, QueryOutcome), DatabaseError> {
    let mut sql = select_sql(table, order_by, filter);
    let keep = match limit {
        Some(limit) => {
            sql.push_str(&format!(" LIMIT {limit} OFFSET {offset}"));
            (limit as usize).min(max_rows)
        }
        None => max_rows,
    };
    let out = execute_on(conn, &sql, keep)?;
    Ok((sql, out))
}

fn count_on(conn: &Connection, table: &str, filter: Option<&TableFilter>) -> Result<u64, DatabaseError> {
    let mut sql = format!("SELECT COUNT(*) FROM {}", quote_ident(table));
    if let Some(f) = filter {
        let expr = f.expr.trim();
        if !expr.is_empty() {
            sql.push_str(&format!(" WHERE {expr}"));
        }
    }
    conn.query_row(&sql, [], |r| r.get::<_, i64>(0)).map(|n| n.max(0) as u64).map_err(map_err)
}

/* ---------- Estrutura ---------- */

/// `VARCHAR(20)` → (20, 0); `NUMERIC(10,2)` → (10, 2); sem parênteses → (0, 0).
fn length_decimal(decl: &str) -> (i32, i32) {
    let Some(open) = decl.find('(') else { return (0, 0) };
    let Some(close) = decl[open..].find(')') else { return (0, 0) };
    let mut parts = decl[open + 1..open + close].split(',').map(|p| p.trim().parse::<i32>().unwrap_or(0));
    (parts.next().unwrap_or(0), parts.next().unwrap_or(0))
}

fn list_objects_on(conn: &Connection) -> Result<SchemaObjects, DatabaseError> {
    let mut out = SchemaObjects::default();
    let mut stmt = conn.prepare("SELECT name, type FROM sqlite_master WHERE type IN ('table', 'view') AND name NOT LIKE 'sqlite\\_%' ESCAPE '\\' ORDER BY name COLLATE NOCASE").map_err(map_err)?;
    let mut rows = stmt.query([]).map_err(map_err)?;
    while let Some(r) = rows.next().map_err(map_err)? {
        let name: String = r.get(0).map_err(map_err)?;
        let kind: String = r.get(1).map_err(map_err)?;
        let info = ObjectInfo { name, rows: None, comment: None, owner: None, args: None, return_type: None, language: None, data_type: None, procedure: false };
        if kind == "view" { out.views.push(info) } else { out.tables.push(info) }
    }
    Ok(out)
}

fn table_detail_on(conn: &Connection, table: &str) -> Result<TableDetail, DatabaseError> {
    let q = quote_ident(table);
    let create_sql: Option<String> = conn.query_row("SELECT sql FROM sqlite_master WHERE name = ?1 AND type IN ('table', 'view')", [table], |r| r.get(0)).ok();
    let without_rowid = create_sql.as_deref().is_some_and(|s| s.to_ascii_uppercase().contains("WITHOUT ROWID"));
    // table_xinfo traz também as colunas geradas (hidden 2 = virtual, 3 = stored); hidden 1 é de tabela virtual e não aparece.
    let mut cols: Vec<(String, String, bool, Option<String>, i32, i32)> = Vec::new();
    {
        let mut stmt = conn.prepare(&format!("PRAGMA table_xinfo({q})")).map_err(map_err)?;
        let mut rows = stmt.query([]).map_err(map_err)?;
        while let Some(r) = rows.next().map_err(map_err)? {
            let hidden: i32 = r.get(6).map_err(map_err)?;
            if hidden == 1 {
                continue;
            }
            cols.push((r.get(1).map_err(map_err)?, r.get::<_, String>(2).map_err(map_err)?, r.get::<_, i64>(3).map_err(map_err)? != 0, r.get(4).map_err(map_err)?, r.get(5).map_err(map_err)?, hidden));
        }
    }
    if cols.is_empty() {
        return Err(DatabaseError::new(ErrorKind::InvalidInput, format!("table \"{table}\" does not exist")));
    }
    let pk_count = cols.iter().filter(|c| c.4 > 0).count();
    let columns: Vec<ColumnInfo> = cols
        .iter()
        .map(|(name, decl, not_null, default, pk, hidden)| {
            let (length, decimal) = length_decimal(decl);
            // `INTEGER PRIMARY KEY` é o apelido do rowid: o próprio SQLite numera, então a coluna se comporta como identidade.
            let rowid_alias = !without_rowid && pk_count == 1 && *pk == 1 && decl.eq_ignore_ascii_case("integer");
            ColumnInfo {
                name: name.clone(),
                data_type: decl.clone(),
                formatted_type: decl.clone(),
                length,
                decimal,
                not_null: *not_null,
                key: if *pk > 0 { Some(*pk) } else { None },
                default: default.clone(),
                comment: None,
                collation: None,
                identity: if rowid_alias { Some("d".into()) } else { None },
                generated: if *hidden == 2 || *hidden == 3 { Some("s".into()) } else { None },
            }
        })
        .collect();

    let mut indexes = Vec::new();
    let mut constraints = Vec::new();
    {
        let mut stmt = conn.prepare(&format!("PRAGMA index_list({q})")).map_err(map_err)?;
        let list: Vec<(String, bool, String)> = stmt
            .query_map([], |r| Ok((r.get::<_, String>(1)?, r.get::<_, i64>(2)? != 0, r.get::<_, String>(3)?)))
            .map_err(map_err)?
            .collect::<Result<_, _>>()
            .map_err(map_err)?;
        for (name, unique, origin) in list {
            let mut info = conn.prepare(&format!("PRAGMA index_info({})", quote_ident(&name))).map_err(map_err)?;
            let fields: Vec<String> = info
                .query_map([], |r| r.get::<_, Option<String>>(2))
                .map_err(map_err)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(map_err)?
                .into_iter()
                .map(|c| c.unwrap_or_else(|| "(expression)".into()))
                .collect();
            let definition: Option<String> = conn.query_row("SELECT sql FROM sqlite_master WHERE type = 'index' AND name = ?1", [&name], |r| r.get(0)).ok().flatten();
            let joined = fields.join(", ");
            indexes.push(IndexInfo {
                name: name.clone(),
                fields: joined.clone(),
                method: "btree".into(),
                unique,
                primary: origin == "pk",
                definition: definition.unwrap_or_else(|| format!("(automatic index) ({joined})")),
                comment: None,
            });
            if origin == "u" {
                constraints.push(ConstraintInfo { name, kind: "u".into(), fields: joined.clone(), ref_schema: None, ref_table: None, ref_fields: None, on_delete: None, on_update: None, definition: format!("UNIQUE ({joined})"), comment: None });
            }
        }
    }
    {
        // Chaves estrangeiras: cada linha do pragma é uma coluna; as de mesmo id formam uma chave composta.
        let mut stmt = conn.prepare(&format!("PRAGMA foreign_key_list({q})")).map_err(map_err)?;
        let rows: Vec<(i64, String, String, Option<String>, String, String)> = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?)))
            .map_err(map_err)?
            .collect::<Result<_, _>>()
            .map_err(map_err)?;
        let mut ids: Vec<i64> = rows.iter().map(|r| r.0).collect();
        ids.dedup();
        for id in ids {
            let part: Vec<_> = rows.iter().filter(|r| r.0 == id).collect();
            let fields = part.iter().map(|r| r.2.clone()).collect::<Vec<_>>().join(", ");
            let ref_fields = part.iter().map(|r| r.3.clone().unwrap_or_default()).collect::<Vec<_>>().join(", ");
            constraints.push(ConstraintInfo {
                name: format!("fk_{table}_{id}"),
                kind: "f".into(),
                fields: fields.clone(),
                ref_schema: Some("main".into()),
                ref_table: Some(part[0].1.clone()),
                ref_fields: Some(ref_fields.clone()),
                on_delete: Some(part[0].5.clone()),
                on_update: Some(part[0].4.clone()),
                definition: format!("FOREIGN KEY ({fields}) REFERENCES {} ({ref_fields})", quote_ident(&part[0].1)),
                comment: None,
            });
        }
    }
    for (i, (name, expr)) in parse_checks(create_sql.as_deref().unwrap_or("")).into_iter().enumerate() {
        constraints.push(ConstraintInfo {
            name: name.unwrap_or_else(|| format!("check_{}", i + 1)),
            kind: "c".into(),
            fields: String::new(),
            ref_schema: None,
            ref_table: None,
            ref_fields: None,
            on_delete: None,
            on_update: None,
            definition: format!("CHECK ({expr})"),
            comment: None,
        });
    }
    let mut triggers = Vec::new();
    {
        let mut stmt = conn.prepare("SELECT name, sql FROM sqlite_master WHERE type = 'trigger' AND tbl_name = ?1 ORDER BY name").map_err(map_err)?;
        let rows: Vec<(String, Option<String>)> = stmt.query_map([table], |r| Ok((r.get(0)?, r.get(1)?))).map_err(map_err)?.collect::<Result<_, _>>().map_err(map_err)?;
        for (name, sql) in rows {
            let sql = sql.unwrap_or_default();
            let upper = sql.to_ascii_uppercase();
            let timing = if upper.contains("INSTEAD OF") { "INSTEAD OF" } else if upper.contains(" BEFORE ") { "BEFORE" } else { "AFTER" };
            let events: Vec<String> = ["INSERT", "UPDATE", "DELETE"].iter().filter(|e| upper.contains(&format!(" {e} "))).map(|e| e.to_string()).collect();
            triggers.push(TriggerInfo {
                name,
                definition: sql,
                enabled: true,
                timing: timing.into(),
                events,
                for_each_row: true,
                when: None,
                function: String::new(),
                args: Vec::new(),
                update_columns: Vec::new(),
                comment: None,
                read_only: true,
            });
        }
    }
    Ok(TableDetail { columns, indexes, constraints, triggers, owner: None, comment: None })
}

/// As restrições `CHECK (...)` do `CREATE TABLE` (o SQLite não as expõe por PRAGMA), com o nome de `CONSTRAINT nome` quando houver.
/// Lê o texto respeitando aspas e parênteses.
fn parse_checks(sql: &str) -> Vec<(Option<String>, String)> {
    let chars: Vec<char> = sql.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '\'' || c == '"' || c == '`' || c == '[' {
            let close = if c == '[' { ']' } else { c };
            i += 1;
            while i < chars.len() && chars[i] != close {
                i += 1;
            }
            i += 1;
            continue;
        }
        let word_start = i == 0 || !(chars[i - 1].is_alphanumeric() || chars[i - 1] == '_');
        if word_start && chars[i..].iter().take(5).collect::<String>().eq_ignore_ascii_case("check") && !chars.get(i + 5).is_some_and(|c| c.is_alphanumeric() || *c == '_') {
            let mut j = i + 5;
            while j < chars.len() && chars[j].is_whitespace() {
                j += 1;
            }
            if chars.get(j) == Some(&'(') {
                let start = j + 1;
                let mut depth = 1;
                let mut k = start;
                while k < chars.len() && depth > 0 {
                    match chars[k] {
                        '(' => depth += 1,
                        ')' => depth -= 1,
                        q @ ('\'' | '"' | '`') => {
                            k += 1;
                            while k < chars.len() && chars[k] != q {
                                k += 1;
                            }
                        }
                        _ => {}
                    }
                    k += 1;
                }
                if depth == 0 {
                    let expr: String = chars[start..k - 1].iter().collect();
                    let before: String = chars[..i].iter().collect();
                    let tokens: Vec<&str> = before.split_whitespace().collect();
                    let name = match tokens.as_slice() {
                        [.., kw, name] if kw.eq_ignore_ascii_case("constraint") => Some(name.trim_matches(|c| c == '"' || c == '`' || c == '[' || c == ']' || c == '\'').to_string()),
                        _ => None,
                    };
                    out.push((name, expr.trim().to_string()));
                    i = k;
                    continue;
                }
            }
        }
        i += 1;
    }
    out
}

/// Rótulos de chaves estrangeiras: para cada chave pedida, o texto da coluna de descrição da tabela referenciada.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FkLabels {
    /// Coluna usada como descrição; `None` se a tabela não tem uma que sirva (a tela mostra a chave mesmo).
    pub label_column: Option<String>,
    /// Pares `[chave, rótulo]` (só das chaves que existem na tabela).
    pub labels: Vec<[String; 2]>,
}

/// Máximo de chaves por chamada (a grade mostra uma página por vez).
const MAX_FK_KEYS: usize = 5_000;

/// Escolhe a coluna de descrição: nomes típicos primeiro (`nome`, `name`, `descricao`...), depois a primeira coluna de texto
/// que não seja a própria chave nem pareça outra chave (`..._id`, `id_...`, `codigo`).
fn pick_label_column(columns: &[(String, String)], key: &str) -> Option<String> {
    let textual = |t: &str| {
        let t = t.to_ascii_lowercase();
        t.contains("char") || t.contains("text") || t.contains("clob")
    };
    let candidates: Vec<&(String, String)> = columns.iter().filter(|(n, t)| n != key && textual(t)).collect();
    const PREFERRED: [&str; 12] = ["nome", "name", "titulo", "title", "descricao", "description", "descr", "label", "razao_social", "fantasia", "apelido", "rotulo"];
    for want in PREFERRED {
        if let Some((n, _)) = candidates.iter().find(|(n, _)| n.to_lowercase() == want) {
            return Some(n.clone());
        }
    }
    for want in PREFERRED {
        if let Some((n, _)) = candidates.iter().find(|(n, _)| n.to_lowercase().contains(want)) {
            return Some(n.clone());
        }
    }
    candidates
        .iter()
        .find(|(n, _)| {
            let l = n.to_lowercase();
            !(l.ends_with("_id") || l.starts_with("id_") || l.starts_with("cod") || l == "id")
        })
        .map(|(n, _)| n.clone())
}

fn fk_labels_on(conn: &Connection, table: &str, key_column: &str, keys: &[String]) -> Result<FkLabels, DatabaseError> {
    let none = FkLabels { label_column: None, labels: Vec::new() };
    if keys.is_empty() {
        return Ok(none);
    }
    let mut columns: Vec<(String, String)> = Vec::new();
    {
        let mut stmt = conn.prepare(&format!("PRAGMA table_info({})", quote_ident(table))).map_err(map_err)?;
        let mut rows = stmt.query([]).map_err(map_err)?;
        while let Some(r) = rows.next().map_err(map_err)? {
            columns.push((r.get(1).map_err(map_err)?, r.get::<_, String>(2).map_err(map_err)?));
        }
    }
    let Some(label) = pick_label_column(&columns, key_column) else { return Ok(none) };
    let mut labels = Vec::new();
    // Em pedaços, para ficar longe do limite de parâmetros do SQLite.
    for chunk in keys.iter().take(MAX_FK_KEYS).collect::<Vec<_>>().chunks(500) {
        let marks = vec!["?"; chunk.len()].join(", ");
        let sql = format!(
            "SELECT CAST({k} AS TEXT), CAST({l} AS TEXT) FROM {t} WHERE CAST({k} AS TEXT) IN ({marks}) AND {l} IS NOT NULL",
            k = quote_ident(key_column),
            l = quote_ident(&label),
            t = quote_ident(table),
        );
        let mut stmt = conn.prepare(&sql).map_err(map_err)?;
        let mut rows = stmt.query(rusqlite::params_from_iter(chunk.iter())).map_err(map_err)?;
        while let Some(r) = rows.next().map_err(map_err)? {
            labels.push([r.get::<_, String>(0).map_err(map_err)?, r.get::<_, String>(1).map_err(map_err)?]);
        }
    }
    Ok(FkLabels { label_column: Some(label), labels })
}

/* ---------- Sessão ---------- */

/// Uma conexão com um arquivo SQLite. Cada transação manual abre a sua própria conexão ao mesmo arquivo.
pub struct SqliteSession {
    pub path: PathBuf,
    conn: Mutex<Connection>,
    interrupt: InterruptHandle,
    pub version: String,
    pub version_num: i32,
}

fn open_connection(path: &Path) -> Result<Connection, DatabaseError> {
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX).map_err(map_err)?;
    conn.busy_timeout(Duration::from_secs(5)).map_err(map_err)?;
    // O SQLite só confere chaves estrangeiras se for pedido a cada conexão.
    conn.execute_batch("PRAGMA foreign_keys = ON").map_err(map_err)?;
    // `coluna REGEXP 'padrão'` também vale nas consultas do usuário.
    find::register_regexp(&conn)?;
    Ok(conn)
}

impl SqliteSession {
    /// Abre um arquivo que já existe (não cria um novo por engano).
    pub fn open(path: &str) -> Result<Self, DatabaseError> {
        let path = PathBuf::from(path.trim());
        if path.as_os_str().is_empty() {
            return Err(DatabaseError::new(ErrorKind::InvalidInput, "choose the database file"));
        }
        if !path.is_file() {
            return Err(DatabaseError::new(ErrorKind::DatabaseNotFound, format!("database file not found: {}", path.display())));
        }
        let conn = open_connection(&path)?;
        // Um arquivo que não é SQLite só reclama na primeira leitura.
        let version: String = conn.query_row("SELECT sqlite_version()", [], |r| r.get(0)).map_err(map_err)?;
        conn.query_row("SELECT count(*) FROM sqlite_master", [], |r| r.get::<_, i64>(0)).map_err(map_err)?;
        let mut nums = version.split('.').map(|p| p.parse::<i32>().unwrap_or(0));
        let version_num = nums.next().unwrap_or(0) * 1_000_000 + nums.next().unwrap_or(0) * 1_000 + nums.next().unwrap_or(0);
        let interrupt = conn.get_interrupt_handle();
        Ok(Self { path, conn: Mutex::new(conn), interrupt, version, version_num })
    }

    fn with<T>(&self, f: impl FnOnce(&Connection) -> Result<T, DatabaseError>) -> Result<T, DatabaseError> {
        let guard = self.conn.lock().map_err(|_| DatabaseError::new(ErrorKind::Internal, "connection lock poisoned"))?;
        f(&guard)
    }

    pub fn server_info(&self) -> ServerInfo {
        ServerInfo { version: self.version.clone(), version_num: self.version_num, current_database: "main".into(), current_user: String::new(), tls: None, latency_ms: 0 }
    }

    pub fn list_objects(&self) -> Result<SchemaObjects, DatabaseError> {
        self.with(list_objects_on)
    }

    pub fn table_detail(&self, table: &str) -> Result<TableDetail, DatabaseError> {
        self.with(|c| table_detail_on(c, table))
    }

    pub fn execute(&self, sql: &str, max_rows: usize) -> Result<QueryOutcome, DatabaseError> {
        self.with(|c| execute_on(c, sql, max_rows))
    }

    pub fn data_profile(&self, table: &str, columns: &[String], order_by: &[OrderBy], filter: Option<&TableFilter>, limit: Option<u64>) -> Result<DataProfile, DatabaseError> {
        self.with(|c| profile::data_profile_on(c, table, columns, order_by, filter, limit))
    }

    pub fn table_profile(&self, table: &str) -> Result<atlas_core::TableProfile, DatabaseError> {
        self.with(|c| profile::table_profile_on(c, table))
    }

    /// Grava as linhas de uma importação, numa conexão própria ao arquivo e numa transação só.
    pub fn import(&self, spec: &ImportSpec, feed: &mut dyn FnMut() -> ImportFeed, hooks: ImportHooks<'_>) -> Result<ImportStats, DatabaseError> {
        let conn = open_connection(&self.path)?;
        import::run(&conn, spec, feed, hooks)
    }

    /// Copia o banco inteiro para `dest` (que não pode existir) com `VACUUM INTO`, que já grava o arquivo compactado e consistente.
    /// Os objetos de `exclude` (nomes de tabelas e views) são apagados da cópia. `log` recebe uma linha por tabela copiada.
    pub fn backup_to(&self, dest: &Path, exclude: &[String], log: &mut dyn FnMut(String)) -> Result<(), DatabaseError> {
        let tables = self.with(|c| {
            let mut stmt = c.prepare("SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite\\_%' ESCAPE '\\' ORDER BY name").map_err(map_err)?;
            let rows = stmt.query_map([], |r| r.get::<_, String>(0)).map_err(map_err)?.collect::<Result<Vec<_>, _>>().map_err(map_err)?;
            Ok(rows)
        })?;
        self.with(|c| c.execute("VACUUM INTO ?1", [dest.to_string_lossy().as_ref()]).map(|_| ()).map_err(map_err))?;
        if !exclude.is_empty() {
            let copy = Connection::open(dest).map_err(map_err)?;
            copy.execute_batch("PRAGMA foreign_keys = OFF").map_err(map_err)?;
            for name in exclude {
                let plain = name.rsplit('.').next().unwrap_or(name);
                let q = quote_ident(plain);
                if copy.execute_batch(&format!("DROP TABLE IF EXISTS {q}")).is_err() {
                    copy.execute_batch(&format!("DROP VIEW IF EXISTS {q}")).map_err(map_err)?;
                }
            }
            copy.execute_batch("VACUUM").map_err(map_err)?;
        }
        for t in tables.iter().filter(|t| !exclude.iter().any(|e| e.rsplit('.').next() == Some(t.as_str()))) {
            log(format!("dumping contents of table \"main.{t}\""));
        }
        Ok(())
    }

    /// Troca o conteúdo do banco pelo de um backup (um arquivo SQLite), com a API de cópia do SQLite: a conexão aberta enxerga o novo conteúdo.
    pub fn restore_from(&self, source: &Path) -> Result<(), DatabaseError> {
        let src = Connection::open_with_flags(source, OpenFlags::SQLITE_OPEN_READ_ONLY).map_err(map_err)?;
        // Um arquivo que não é SQLite falha aqui, antes de tocar no banco.
        src.query_row("SELECT count(*) FROM sqlite_master", [], |r| r.get::<_, i64>(0)).map_err(|_| DatabaseError::new(ErrorKind::InvalidInput, "o arquivo não é um backup do SQLite"))?;
        let mut dst = open_connection(&self.path)?;
        let backup = rusqlite::backup::Backup::new(&src, &mut dst).map_err(map_err)?;
        backup.run_to_completion(256, Duration::from_millis(0), None).map_err(map_err)
    }

    /// Os comandos que recriam as tabelas e views pedidas (`None` = todas) e os dados a copiar; `clean` põe um `DROP ... IF EXISTS` antes.
    pub fn dump_plan(&self, tables: Option<&[String]>, views: Option<&[String]>, clean: bool) -> Result<DumpPlan, DatabaseError> {
        self.with(|c| dump::plan_on(c, tables, views, clean))
    }

    /// A definição de uma view (só o `SELECT`).
    pub fn view_detail(&self, view: &str) -> Result<atlas_core::ViewDetail, DatabaseError> {
        self.with(|c| view::view_detail_on(c, view))
    }

    pub fn console_run(&self, input: &str, options: ConsoleOptions) -> Result<ConsoleOutput, DatabaseError> {
        self.with(|c| Ok(console::run_on(c, input, options)))
    }

    pub fn console_complete(&self, text: &str) -> Result<ConsoleCompletion, DatabaseError> {
        self.with(|c| Ok(console::complete_on(c, text)))
    }

    pub fn console_prompt(&self) -> Result<String, DatabaseError> {
        self.with(console::prompt)
    }

    pub fn schema_diagram(&self) -> Result<atlas_core::SchemaDiagram, DatabaseError> {
        self.with(diagram::schema_diagram_on)
    }

    pub fn find(&self, options: &FindOptions) -> Result<FindResult, DatabaseError> {
        self.with(|c| find::find_on(c, options))
    }

    /// Procura nos dados numa conexão própria ao arquivo; `cancelled` é consultado a cada tabela.
    pub fn find_data(&self, options: &FindOptions, cancelled: &(dyn Fn() -> bool + Sync)) -> Result<FindDataResult, DatabaseError> {
        find::find_data_at(&self.path, options, cancelled)
    }

    pub fn fk_labels(&self, table: &str, key_column: &str, keys: &[String]) -> Result<FkLabels, DatabaseError> {
        self.with(|c| fk_labels_on(c, table, key_column, keys))
    }

    pub fn table_rows(&self, table: &str, order_by: &[OrderBy], filter: Option<&TableFilter>, limit: Option<u32>, offset: u64, max_rows: usize) -> Result<(String, QueryOutcome), DatabaseError> {
        self.with(|c| rows_on(c, table, order_by, filter, limit, offset, max_rows))
    }

    pub fn table_count(&self, table: &str, filter: Option<&TableFilter>) -> Result<u64, DatabaseError> {
        self.with(|c| count_on(c, table, filter))
    }

    pub fn apply(&self, statements: &[PlannedStatement]) -> Result<Vec<u64>, DatabaseError> {
        self.with(|c| apply_on(c, statements))
    }

    /// Interrompe o comando em andamento (o SQLite o encerra com "interrupted").
    pub fn cancel(&self) {
        self.interrupt.interrupt();
    }

    /// Abre uma transação manual (conexão própria ao mesmo arquivo, com `BEGIN`).
    pub fn begin(&self) -> Result<SqliteTx, DatabaseError> {
        let conn = open_connection(&self.path)?;
        conn.execute_batch("BEGIN").map_err(map_err)?;
        Ok(SqliteTx { conn: Mutex::new(conn) })
    }
}

/// Transação manual aberta: tudo que roda nela fica pendente até `finish`.
pub struct SqliteTx {
    conn: Mutex<Connection>,
}

impl SqliteTx {
    fn with<T>(&self, f: impl FnOnce(&Connection) -> Result<T, DatabaseError>) -> Result<T, DatabaseError> {
        let guard = self.conn.lock().map_err(|_| DatabaseError::new(ErrorKind::Internal, "transaction lock poisoned"))?;
        f(&guard)
    }

    pub fn execute(&self, sql: &str, max_rows: usize) -> Result<QueryOutcome, DatabaseError> {
        self.with(|c| execute_on(c, sql, max_rows))
    }

    pub fn apply(&self, statements: &[PlannedStatement]) -> Result<Vec<u64>, DatabaseError> {
        self.with(|c| apply_on(c, statements))
    }

    pub fn table_rows(&self, table: &str, order_by: &[OrderBy], filter: Option<&TableFilter>, limit: Option<u32>, offset: u64, max_rows: usize) -> Result<(String, QueryOutcome), DatabaseError> {
        self.with(|c| rows_on(c, table, order_by, filter, limit, offset, max_rows))
    }

    pub fn table_count(&self, table: &str, filter: Option<&TableFilter>) -> Result<u64, DatabaseError> {
        self.with(|c| count_on(c, table, filter))
    }

    /// Grava (`commit`) ou desfaz a transação.
    pub fn finish(&self, commit: bool) -> Result<(), DatabaseError> {
        self.with(|c| c.execute_batch(if commit { "COMMIT" } else { "ROLLBACK" }).map_err(map_err))
    }
}

/// Funções que o SQLite não tem como o PostgreSQL: a interface as chama e recebe "indisponível".
pub fn unsupported_feature(what: &str) -> DatabaseError {
    unsupported(what)
}

#[cfg(test)]
mod tests;
