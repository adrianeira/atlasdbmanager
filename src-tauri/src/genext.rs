//! Data Generation do SQL Server, MySQL e SQLite: descreve as tabelas, pré-visualiza e insere dados de teste.
//! Os geradores, a ordem das tabelas e os formatos de resposta são os do PostgreSQL (`atlas_pg`); aqui ficam só o catálogo, as aspas e a gravação de cada banco.

use crate::mssql::MssqlSessions;
use crate::mysql::MysqlSessions;
use crate::sqlite::SqliteSessions;
use atlas_core::{DatabaseError, ErrorKind, QueryOutcome};
use atlas_mssql::{MssqlSession, MssqlTx};
use atlas_mysql::{MysqlSession, MysqlTx};
use atlas_pg::{
    GenColumnMeta, GenColumnState, GenHooks, GenPreview, GenRng, GenSpec, GenStats, GenTableMeta, GenTableSpec, Generator, TextStyle, check_generation, gen_format_stamp, gen_today,
    make_generation_row, order_generation_tables,
};
use atlas_sqlite::{SqliteSession, SqliteTx};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;
use tauri::{AppHandle, Emitter, State};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Dialect {
    Mysql,
    Mssql,
    Sqlite,
}

/// A conexão da geração: a sessão (cada comando confirma sozinho) ou uma transação aberta.
pub(crate) enum Exec {
    Mysql(Arc<MysqlSession>, String),
    MysqlTx(Arc<MysqlTx>),
    Mssql(Arc<MssqlSession>, String),
    MssqlTx(Arc<MssqlTx>),
    Sqlite(Arc<SqliteSession>),
    SqliteTx(Arc<SqliteTx>),
}

impl Exec {
    pub(crate) async fn run(&self, sql: &str, max: usize) -> Result<QueryOutcome, DatabaseError> {
        match self {
            Exec::Mysql(s, db) => s.execute(db, sql, max).await,
            Exec::MysqlTx(t) => t.execute(sql, max).await,
            Exec::Mssql(s, db) => s.execute(db, sql, max).await,
            Exec::MssqlTx(t) => t.execute(sql, max).await,
            Exec::Sqlite(s) => s.execute(sql, max),
            Exec::SqliteTx(t) => t.execute(sql, max),
        }
    }

    pub(crate) async fn rows(&self, sql: &str, max: usize) -> Result<Vec<Vec<Option<String>>>, DatabaseError> {
        let out = self.run(sql, max).await?;
        Ok(out.statements.into_iter().find_map(|s| s.result).map(|r| r.rows).unwrap_or_default())
    }

    /// Abre uma transação no mesmo banco (ou devolve `None` se este já é uma transação).
    pub(crate) async fn begin(&self) -> Result<Option<Exec>, DatabaseError> {
        Ok(match self {
            Exec::Mysql(s, db) => Some(Exec::MysqlTx(Arc::new(s.begin(db).await?))),
            Exec::Mssql(s, db) => Some(Exec::MssqlTx(Arc::new(s.begin(db).await?))),
            Exec::Sqlite(s) => Some(Exec::SqliteTx(Arc::new(s.begin()?))),
            _ => None,
        })
    }

    /// As colunas, chaves e índices de uma tabela.
    pub(crate) async fn table_detail(&self, schema: &str, table: &str) -> Result<atlas_core::TableDetail, DatabaseError> {
        match self {
            Exec::Mysql(s, _) => s.table_detail(schema, table).await,
            Exec::Mssql(s, db) => s.table_detail(db, schema, table).await,
            Exec::Sqlite(s) => s.table_detail(table),
            Exec::MysqlTx(_) | Exec::MssqlTx(_) | Exec::SqliteTx(_) => Err(DatabaseError::new(ErrorKind::Internal, "table_detail needs a session")),
        }
    }

    /// Tabelas, views e rotinas de um schema.
    pub(crate) async fn objects(&self, schema: &str) -> Result<atlas_core::SchemaObjects, DatabaseError> {
        match self {
            Exec::Mysql(s, _) => s.list_objects(schema).await,
            Exec::Mssql(s, db) => s.list_objects(db, schema).await,
            Exec::Sqlite(s) => s.list_objects(),
            _ => Err(DatabaseError::new(ErrorKind::Internal, "objects needs a session")),
        }
    }

    pub(crate) async fn finish(&self, commit: bool) -> Result<(), DatabaseError> {
        match self {
            Exec::MysqlTx(t) => t.finish(commit).await,
            Exec::MssqlTx(t) => t.finish(commit).await,
            Exec::SqliteTx(t) => t.finish(commit),
            _ => Ok(()),
        }
    }
}

impl Dialect {
    pub(crate) fn q(self, name: &str) -> String {
        match self {
            Dialect::Mysql => format!("`{}`", name.replace('`', "``")),
            Dialect::Mssql => format!("[{}]", name.replace(']', "]]")),
            Dialect::Sqlite => format!("\"{}\"", name.replace('"', "\"\"")),
        }
    }

    /// `banco.tabela` (MySQL), `schema.tabela` (SQL Server) ou `tabela` (SQLite).
    pub(crate) fn target(self, schema: &str, table: &str) -> String {
        match self {
            Dialect::Sqlite => self.q(table),
            _ => format!("{}.{}", self.q(schema), self.q(table)),
        }
    }

    /// O texto entre aspas do banco: barra invertida dobrada no MySQL, `N'...'` no SQL Server.
    pub(crate) fn text(self, v: &str) -> String {
        match self {
            Dialect::Mysql => format!("'{}'", v.replace('\\', "\\\\").replace('\'', "''")),
            Dialect::Mssql => format!("N'{}'", v.replace('\'', "''")),
            Dialect::Sqlite => format!("'{}'", v.replace('\'', "''")),
        }
    }

    pub(crate) fn literal(self, v: &Option<String>, g: &Generator) -> String {
        match (v, g) {
            (None, _) => "NULL".into(),
            // Nenhum dos três tem `true`/`false` de verdade para todos os tipos: 1 e 0 valem em todos.
            (Some(t), Generator::Boolean { .. }) => if t == "true" { "1" } else { "0" }.into(),
            (Some(t), _) => self.text(t),
        }
    }

    pub(crate) fn savepoint(self) -> &'static str {
        if self == Dialect::Mssql { "SAVE TRANSACTION atlas_dg" } else { "SAVEPOINT atlas_dg" }
    }
    pub(crate) fn rollback_to(self) -> &'static str {
        if self == Dialect::Mssql { "ROLLBACK TRANSACTION atlas_dg" } else { "ROLLBACK TO SAVEPOINT atlas_dg" }
    }
    pub(crate) fn release(self) -> Option<&'static str> {
        (self != Dialect::Mssql).then_some("RELEASE SAVEPOINT atlas_dg")
    }
}

/// A conexão `conn` (aberta, de qualquer um dos três produtos), no banco `database` e no schema `schema`.
pub(crate) fn exec_for(conn: &str, database: &str, schema: &str, mysql: &MysqlSessions, mssql: &MssqlSessions, sqlite: &SqliteSessions) -> Result<(Dialect, Exec), DatabaseError> {
    if conn.starts_with("mysql_") {
        Ok((Dialect::Mysql, Exec::Mysql(mysql.session(conn)?, mysql_db(database, schema).to_string())))
    } else if conn.starts_with("mssql_") {
        Ok((Dialect::Mssql, Exec::Mssql(mssql.session(conn)?, database.to_string())))
    } else if conn.starts_with("sqlite_") {
        Ok((Dialect::Sqlite, Exec::Sqlite(sqlite.session(conn)?)))
    } else {
        Err(DatabaseError::new(ErrorKind::InvalidInput, "this connection is not a SQL Server, MySQL or SQLite connection"))
    }
}

/// Troca o schema `from` por `to` nos nomes qualificados de um texto do SQL Server (`[from].` e `from.`, fora de outras palavras).
pub(crate) fn mssql_schema_rewrite(sql: &str, from: &str, to: &str) -> String {
    if from.eq_ignore_ascii_case(to) {
        return sql.to_string();
    }
    let bracketed = sql.replace(&format!("[{from}]."), &format!("[{to}]."));
    let lower = bracketed.to_lowercase();
    let needle = format!("{}.", from.to_lowercase());
    let mut out = String::with_capacity(bracketed.len());
    let (mut last, mut at) = (0usize, 0usize);
    while let Some(pos) = lower[at..].find(&needle) {
        let i = at + pos;
        let before = bracketed[..i].chars().next_back();
        let boundary = before.is_none_or(|c| !(c.is_alphanumeric() || matches!(c, '_' | '[' | '.' | '@' | '#' | '$')));
        if boundary && bracketed.is_char_boundary(i) && bracketed.is_char_boundary(i + needle.len()) {
            out.push_str(&bracketed[last..i]);
            out.push_str(to);
            out.push('.');
            last = i + needle.len();
        }
        at = i + needle.len();
    }
    out.push_str(&bracketed[last..]);
    out
}

/// Uma coluna como o catálogo a devolve.
struct RawColumn {
    table: String,
    name: String,
    data_type: String,
    not_null: bool,
    is_pk: bool,
    generated: bool,
    /// Numera sozinha (`AUTO_INCREMENT`, `IDENTITY`, chave `INTEGER` do SQLite).
    auto: bool,
    char_len: Option<u32>,
    kind: &'static str,
    precision: Option<u32>,
    scale: Option<u32>,
    labels: Vec<String>,
}

fn cell(r: &[Option<String>], i: usize) -> String {
    r.get(i).cloned().flatten().unwrap_or_default()
}
fn num(r: &[Option<String>], i: usize) -> Option<u32> {
    r.get(i).cloned().flatten().and_then(|v| v.parse::<i64>().ok()).filter(|n| *n >= 0).map(|n| n as u32)
}

/// Os rótulos de `enum('a','b')`.
fn enum_labels(column_type: &str) -> Vec<String> {
    let Some(inner) = column_type.strip_prefix("enum(").and_then(|s| s.strip_suffix(')')) else { return Vec::new() };
    inner.split("','").map(|p| p.trim_matches('\'').replace("''", "'")).collect()
}

async fn raw_mysql(exec: &Exec, db: &str) -> Result<(Vec<RawColumn>, Vec<[String; 4]>), DatabaseError> {
    let lit = Dialect::Mysql.text(db);
    let cols = exec
        .rows(
            &format!(
                "SELECT c.table_name, c.column_name, c.column_type, c.data_type, c.is_nullable, c.column_key, c.extra, c.character_maximum_length, c.numeric_precision, c.numeric_scale \
                   FROM information_schema.columns c JOIN information_schema.tables t ON t.table_schema = c.table_schema AND t.table_name = c.table_name AND t.table_type = 'BASE TABLE' \
                  WHERE c.table_schema = {lit} ORDER BY c.table_name, c.ordinal_position"
            ),
            100_000,
        )
        .await?;
    let out = cols
        .iter()
        .map(|r| {
            let (column_type, data_type, extra) = (cell(r, 2), cell(r, 3).to_lowercase(), cell(r, 6).to_lowercase());
            let kind = if column_type == "tinyint(1)" || column_type.starts_with("bit(1)") {
                "bool"
            } else {
                match data_type.as_str() {
                    "tinyint" | "smallint" | "mediumint" | "int" | "bigint" | "year" => "int",
                    "decimal" | "numeric" | "float" | "double" => "numeric",
                    "char" | "varchar" | "tinytext" | "text" | "mediumtext" | "longtext" => "text",
                    "date" => "date",
                    "datetime" | "timestamp" => "timestamp",
                    "json" => "json",
                    "enum" => "enum",
                    _ => "other",
                }
            };
            RawColumn {
                table: cell(r, 0),
                name: cell(r, 1),
                data_type: column_type.clone(),
                not_null: cell(r, 4) == "NO",
                is_pk: cell(r, 5) == "PRI",
                generated: extra.contains("generated") && !extra.contains("default_generated"),
                auto: extra.contains("auto_increment"),
                char_len: num(r, 7),
                kind,
                precision: num(r, 8),
                scale: num(r, 9),
                labels: enum_labels(&column_type),
            }
        })
        .collect();
    let fks = exec
        .rows(
            &format!(
                "SELECT table_name, column_name, referenced_table_name, referenced_column_name FROM information_schema.key_column_usage \
                  WHERE table_schema = {lit} AND referenced_table_schema = {lit} AND referenced_table_name IS NOT NULL"
            ),
            100_000,
        )
        .await?
        .iter()
        .map(|r| [cell(r, 0), cell(r, 1), cell(r, 2), cell(r, 3)])
        .collect();
    Ok((out, fks))
}

async fn raw_mssql(exec: &Exec, schema: &str) -> Result<(Vec<RawColumn>, Vec<[String; 4]>), DatabaseError> {
    let lit = Dialect::Mssql.text(schema);
    let cols = exec
        .rows(
            &format!(
                "SELECT t.name, c.name, ty.name, CAST(c.is_nullable AS int), CAST(c.is_identity AS int), CAST(c.is_computed AS int), CAST(c.max_length AS int), CAST(c.precision AS int), CAST(c.scale AS int), \
                        CAST(CASE WHEN EXISTS (SELECT 1 FROM sys.index_columns ic JOIN sys.indexes i ON i.object_id = ic.object_id AND i.index_id = ic.index_id \
                                                WHERE i.is_primary_key = 1 AND ic.object_id = c.object_id AND ic.column_id = c.column_id) THEN 1 ELSE 0 END AS int) \
                   FROM sys.tables t JOIN sys.schemas sc ON sc.schema_id = t.schema_id JOIN sys.columns c ON c.object_id = t.object_id JOIN sys.types ty ON ty.user_type_id = c.user_type_id \
                  WHERE sc.name = {lit} AND t.is_ms_shipped = 0 ORDER BY t.name, c.column_id"
            ),
            100_000,
        )
        .await?;
    let out = cols
        .iter()
        .map(|r| {
            let ty = cell(r, 2).to_lowercase();
            let max_length = cell(r, 6).parse::<i64>().unwrap_or(0);
            let unicode = ty.starts_with('n');
            let kind = match ty.as_str() {
                "tinyint" | "smallint" | "int" | "bigint" => "int",
                "decimal" | "numeric" | "float" | "real" | "money" | "smallmoney" => "numeric",
                "char" | "varchar" | "nchar" | "nvarchar" | "text" | "ntext" => "text",
                "bit" => "bool",
                "date" => "date",
                "datetime" | "datetime2" | "smalldatetime" | "datetimeoffset" => "timestamp",
                "uniqueidentifier" => "uuid",
                _ => "other",
            };
            RawColumn {
                table: cell(r, 0),
                name: cell(r, 1),
                data_type: ty.clone(),
                not_null: cell(r, 3) == "0",
                is_pk: cell(r, 9) == "1",
                generated: cell(r, 5) == "1",
                auto: cell(r, 4) == "1",
                char_len: (kind == "text" && max_length > 0).then_some((if unicode { max_length / 2 } else { max_length }) as u32),
                kind,
                precision: num(r, 7),
                scale: num(r, 8),
                labels: Vec::new(),
            }
        })
        .collect();
    let fks = exec
        .rows(
            &format!(
                "SELECT t.name, pc.name, rt.name, rc.name FROM sys.foreign_key_columns fkc \
                   JOIN sys.tables t ON t.object_id = fkc.parent_object_id JOIN sys.schemas sc ON sc.schema_id = t.schema_id \
                   JOIN sys.tables rt ON rt.object_id = fkc.referenced_object_id JOIN sys.schemas rs ON rs.schema_id = rt.schema_id \
                   JOIN sys.columns pc ON pc.object_id = fkc.parent_object_id AND pc.column_id = fkc.parent_column_id \
                   JOIN sys.columns rc ON rc.object_id = fkc.referenced_object_id AND rc.column_id = fkc.referenced_column_id \
                  WHERE sc.name = {lit} AND rs.name = {lit}"
            ),
            100_000,
        )
        .await?
        .iter()
        .map(|r| [cell(r, 0), cell(r, 1), cell(r, 2), cell(r, 3)])
        .collect();
    Ok((out, fks))
}

async fn raw_sqlite(exec: &Exec) -> Result<(Vec<RawColumn>, Vec<[String; 4]>), DatabaseError> {
    let cols = exec
        .rows(
            "SELECT m.name, p.name, p.type, p.\"notnull\", p.pk, p.hidden, (SELECT count(*) FROM pragma_table_info(m.name) WHERE pk > 0) \
               FROM sqlite_master m, pragma_table_xinfo(m.name) p WHERE m.type = 'table' AND m.name NOT LIKE 'sqlite\\_%' ESCAPE '\\' ORDER BY m.name, p.cid",
            100_000,
        )
        .await?;
    let out = cols
        .iter()
        .map(|r| {
            let decl = cell(r, 2).to_uppercase();
            let single_pk = cell(r, 6) == "1" && cell(r, 4) != "0";
            let kind = if decl.contains("BOOL") {
                "bool"
            } else if decl.contains("INT") {
                "int"
            } else if decl.contains("CHAR") || decl.contains("CLOB") || decl.contains("TEXT") {
                "text"
            } else if decl.contains("REAL") || decl.contains("FLOA") || decl.contains("DOUB") || decl.contains("DEC") || decl.contains("NUM") {
                "numeric"
            } else if decl.contains("DATETIME") || decl.contains("TIMESTAMP") {
                "timestamp"
            } else if decl.contains("DATE") {
                "date"
            } else {
                "other"
            };
            // `(n)` do tipo declarado.
            let char_len = decl.split_once('(').and_then(|(_, rest)| rest.split([')', ',']).next()).and_then(|n| n.trim().parse::<u32>().ok());
            RawColumn {
                table: cell(r, 0),
                name: cell(r, 1),
                data_type: cell(r, 2),
                not_null: cell(r, 3) == "1",
                is_pk: cell(r, 4) != "0",
                generated: matches!(cell(r, 5).as_str(), "2" | "3"),
                auto: single_pk && decl == "INTEGER",
                char_len,
                kind,
                precision: None,
                scale: None,
                labels: Vec::new(),
            }
        })
        .collect();
    let fks = exec
        .rows("SELECT m.name, f.\"from\", f.\"table\", f.\"to\" FROM sqlite_master m, pragma_foreign_key_list(m.name) f WHERE m.type = 'table'", 100_000)
        .await?
        .iter()
        .map(|r| [cell(r, 0), cell(r, 1), cell(r, 2), cell(r, 3)])
        .collect();
    Ok((out, fks))
}

/// O gerador sugerido para a coluna (pelo tipo, pela chave e pelo padrão da coluna), como no PostgreSQL.
fn suggest(c: &GenColumnMeta, raw: &RawColumn, reference: Option<(String, String)>) -> Generator {
    if raw.auto {
        return Generator::Sequence { start: None, step: 1 };
    }
    if let Some((table, column)) = reference {
        return Generator::Reference { table, column };
    }
    let now = gen_format_stamp(gen_today(), false);
    match raw.kind {
        "int" => {
            if c.is_pk {
                return Generator::Sequence { start: None, step: 1 };
            }
            let max = if c.data_type.contains("smallint") || c.data_type == "tinyint" { 127 } else { 1_000_000 };
            Generator::Integer { min: 0, max }
        }
        "numeric" => {
            let scale = raw.scale.unwrap_or(2).min(6);
            let digits = raw.precision.map(|p| p.saturating_sub(scale.min(p))).unwrap_or(6).clamp(1, 6);
            Generator::Decimal { min: 0.0, max: 10f64.powi(digits as i32) - 1.0, scale }
        }
        "text" => {
            let max = c.char_len.map(|n| n.min(100)).unwrap_or(100);
            Generator::Text { min_len: max.min(10), max_len: max, style: if max >= 20 { TextStyle::Sentence } else { TextStyle::Words } }
        }
        "bool" => Generator::Boolean { true_percent: 50 },
        "date" => Generator::Date { from: "2000-01-01".into(), to: now },
        "timestamp" => Generator::DateTime { from: "2000-01-01 00:00:00".into(), to: format!("{now} 23:59:59") },
        "uuid" => Generator::Uuid,
        "json" => Generator::Fixed { value: "{}".into() },
        "enum" if !raw.labels.is_empty() => Generator::List { values: raw.labels.clone() },
        _ if !c.not_null => Generator::Null,
        _ => Generator::Fixed { value: String::new() },
    }
}

async fn describe(dialect: Dialect, exec: &Exec, schema: &str) -> Result<Vec<GenTableMeta>, DatabaseError> {
    let (raw, fks) = match dialect {
        Dialect::Mysql => raw_mysql(exec, schema).await?,
        Dialect::Mssql => raw_mssql(exec, schema).await?,
        Dialect::Sqlite => raw_sqlite(exec).await?,
    };
    let mut refs: HashMap<(String, String), (String, String)> = HashMap::new();
    let mut deps: HashMap<String, std::collections::HashSet<String>> = HashMap::new();
    for [t, col, rt, rc] in fks {
        if t != rt {
            deps.entry(t.clone()).or_default().insert(rt.clone());
        }
        refs.insert((t, col), (rt, rc));
    }
    let mut tables: Vec<GenTableMeta> = Vec::new();
    for r in raw {
        if tables.last().is_none_or(|t| t.name != r.table) {
            let mut depends_on: Vec<String> = deps.get(&r.table).map(|d| d.iter().cloned().collect()).unwrap_or_default();
            depends_on.sort();
            tables.push(GenTableMeta { name: r.table.clone(), columns: Vec::new(), depends_on });
        }
        let mut c = GenColumnMeta { name: r.name.clone(), data_type: r.data_type.clone(), not_null: r.not_null, is_pk: r.is_pk, generated: r.generated, char_len: r.char_len, suggested: Generator::Null };
        c.suggested = suggest(&c, &r, refs.get(&(r.table.clone(), r.name.clone())).cloned());
        tables.last_mut().expect("table").columns.push(c);
    }
    Ok(order_generation_tables(tables))
}

/// Onde a sequência continua e os valores que as referências sorteiam.
async fn prepare(dialect: Dialect, exec: &Exec, schema: &str, table: &GenTableSpec) -> Result<Vec<GenColumnState>, DatabaseError> {
    check_generation(table)?;
    let mut out = Vec::new();
    for c in &table.columns {
        let mut st = GenColumnState::default();
        match &c.generator {
            Generator::Sequence { start: Some(v), .. } => st.next = *v,
            Generator::Sequence { start: None, .. } => {
                let big = match dialect {
                    Dialect::Mysql => format!("CAST(MAX({}) AS SIGNED)", dialect.q(&c.name)),
                    Dialect::Mssql => format!("CAST(MAX({}) AS bigint)", dialect.q(&c.name)),
                    Dialect::Sqlite => format!("MAX({})", dialect.q(&c.name)),
                };
                let rows = exec.rows(&format!("SELECT COALESCE({big}, 0) + 1 FROM {}", dialect.target(schema, &table.name)), 1).await?;
                st.next = rows.first().map(|r| cell(r, 0)).and_then(|v| v.parse::<i64>().ok()).unwrap_or(1);
            }
            Generator::Reference { table: t, column } => {
                let col = dialect.q(column);
                let sql = match dialect {
                    Dialect::Mysql => format!("SELECT DISTINCT CAST({col} AS CHAR) FROM {} WHERE {col} IS NOT NULL LIMIT 10000", dialect.target(schema, t)),
                    Dialect::Mssql => format!("SELECT DISTINCT TOP 10000 CAST({col} AS nvarchar(4000)) FROM {} WHERE {col} IS NOT NULL", dialect.target(schema, t)),
                    Dialect::Sqlite => format!("SELECT DISTINCT CAST({col} AS TEXT) FROM {} WHERE {col} IS NOT NULL LIMIT 10000", dialect.target(schema, t)),
                };
                st.pool = exec.rows(&sql, 10_000).await?.iter().map(|r| cell(r, 0)).collect();
            }
            _ => {}
        }
        out.push(st);
    }
    Ok(out)
}

async fn preview(dialect: Dialect, exec: &Exec, schema: &str, table: &GenTableSpec, rows: u64) -> Result<GenPreview, DatabaseError> {
    let mut states = prepare(dialect, exec, schema, table).await?;
    let mut rng = GenRng::from_clock();
    Ok(GenPreview { columns: table.columns.iter().map(|c| c.name.clone()).collect(), rows: (0..rows).map(|_| make_generation_row(table, &mut states, &mut rng)).collect() })
}

const BATCH: u64 = 500;

async fn run(dialect: Dialect, exec: &Exec, spec: &GenSpec, hooks: GenHooks<'_>) -> Result<GenStats, DatabaseError> {
    let GenHooks { log, progress, cancelled } = hooks;
    let opts = &spec.options;
    let mut stats = GenStats::default();
    let mut rng = GenRng::from_clock();
    log("[DGR] Data Generation started".into());
    let abort = |stats: &GenStats, log: &mut (dyn FnMut(String) + Send)| {
        log("[DGR] Finished with error".into());
        Ok(stats.clone())
    };

    if opts.empty_first {
        for (i, t) in spec.tables.iter().enumerate().rev() {
            log(format!("[DGR] {}> {}: Emptying table", i + 1, t.name));
            if let Err(e) = exec.run(&format!("DELETE FROM {}", dialect.target(&spec.schema, &t.name)), 1).await {
                log(format!("[DGR] {}> {}: ERROR: {}", i + 1, t.name, e.message));
                stats.errors += 1;
                progress(&stats);
                return abort(&stats, log);
            }
        }
    }

    for (i, t) in spec.tables.iter().enumerate() {
        let tag = format!("[DGR] {}> {}", i + 1, t.name);
        if cancelled() {
            return Err(DatabaseError::new(ErrorKind::Canceled, "data generation canceled"));
        }
        // Com "transação" cada tabela roda numa transação própria; sem ela, cada comando confirma sozinho.
        let tx = if opts.transaction { exec.begin().await? } else { None };
        let conn: &Exec = tx.as_ref().unwrap_or(exec);
        if tx.is_some() {
            log(format!("{tag}: --Transaction - Begin--"));
        }
        log(format!("{tag}: Preparing field"));
        let mut states = match prepare(dialect, conn, &spec.schema, t).await {
            Ok(v) => v,
            Err(e) => {
                log(format!("{tag}: ERROR: {}", e.message));
                stats.errors += 1;
                progress(&stats);
                if let Some(tx) = &tx {
                    let _ = tx.finish(false).await;
                    log(format!("{tag}: --Transaction - Rollback--"));
                }
                if opts.continue_on_error {
                    continue;
                }
                return abort(&stats, log);
            }
        };
        // SQL Server: a coluna `IDENTITY` só aceita valor explícito com `IDENTITY_INSERT`.
        let identity_insert = dialect == Dialect::Mssql && {
            let names = t.columns.iter().map(|c| dialect.text(&c.name)).collect::<Vec<_>>().join(", ");
            let sql = format!(
                "SELECT 1 FROM sys.identity_columns WHERE object_id = OBJECT_ID({}) AND name IN ({names})",
                dialect.text(&dialect.target(&spec.schema, &t.name))
            );
            !names.is_empty() && !conn.rows(&sql, 1).await?.is_empty()
        };
        log(format!("{tag}: Start records generation"));
        let target = dialect.target(&spec.schema, &t.name);
        let columns = t.columns.iter().map(|c| dialect.q(&c.name)).collect::<Vec<_>>().join(", ");
        let per_batch = if opts.multi_insert { BATCH } else { 1 };
        let mut done: u64 = 0;
        let mut failed = false;
        while done < t.rows {
            if cancelled() {
                if let Some(tx) = &tx {
                    let _ = tx.finish(false).await;
                }
                return Err(DatabaseError::new(ErrorKind::Canceled, "data generation canceled"));
            }
            let n = per_batch.min(t.rows - done);
            let rows: Vec<String> = (0..n)
                .map(|_| {
                    let row = make_generation_row(t, &mut states, &mut rng);
                    format!("({})", row.iter().zip(&t.columns).map(|(v, c)| dialect.literal(v, &c.generator)).collect::<Vec<_>>().join(", "))
                })
                .collect();
            stats.generated += n;
            let insert = if t.columns.is_empty() {
                match dialect {
                    Dialect::Mssql => format!("INSERT INTO {target} DEFAULT VALUES"),
                    Dialect::Mysql => format!("INSERT INTO {target} () VALUES ()"),
                    Dialect::Sqlite => format!("INSERT INTO {target} DEFAULT VALUES"),
                }
            } else {
                format!("INSERT INTO {target} ({columns}) VALUES {}", rows.join(", "))
            };
            let sql = if identity_insert { format!("SET IDENTITY_INSERT {target} ON; {insert}; SET IDENTITY_INSERT {target} OFF") } else { insert };
            let savepoint = tx.is_some() && opts.continue_on_error;
            if savepoint {
                conn.run(dialect.savepoint(), 1).await?;
            }
            match conn.run(&sql, 1).await {
                Ok(_) => {
                    stats.inserted += n;
                    if savepoint && let Some(r) = dialect.release() {
                        conn.run(r, 1).await?;
                    }
                }
                Err(e) => {
                    log(format!("{tag}: ERROR: {}", e.message));
                    stats.errors += 1;
                    if savepoint {
                        conn.run(dialect.rollback_to(), 1).await?;
                    } else if !opts.continue_on_error {
                        failed = true;
                    }
                }
            }
            done += n;
            progress(&stats);
            if failed {
                break;
            }
        }
        if !failed {
            log(format!("{tag}: Records generated"));
        }
        if let Some(tx) = &tx {
            if failed {
                let _ = tx.finish(false).await;
                log(format!("{tag}: --Transaction - Rollback--"));
            } else {
                match tx.finish(true).await {
                    Ok(()) => log(format!("{tag}: --Transaction - Commit--")),
                    Err(e) => {
                        log(format!("{tag}: ERROR: {}", e.message));
                        stats.errors += 1;
                        failed = true;
                    }
                }
            }
        }
        if failed {
            progress(&stats);
            return abort(&stats, log);
        }
        stats.objects += 1;
        progress(&stats);
    }
    log(if stats.errors > 0 { "[DGR] Finished with errors" } else { "[DGR] Finished successfully" }.into());
    Ok(stats)
}

/* ---------- comandos ---------- */

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunRequest {
    pub run_id: String,
    pub conn: String,
    pub database: String,
    pub spec: GenSpec,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunResult {
    #[serde(flatten)]
    pub stats: GenStats,
    pub elapsed_ms: u64,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct LogEvent<'a> {
    id: &'a str,
    line: String,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct ProgressEvent<'a> {
    id: &'a str,
    #[serde(flatten)]
    stats: GenStats,
}

/// O banco e o schema que a geração usa: no MySQL o schema é o próprio banco.
pub(crate) fn mysql_db<'a>(database: &'a str, schema: &'a str) -> &'a str {
    if schema.is_empty() { database } else { schema }
}

async fn finish_run(app: AppHandle, exec: Exec, dialect: Dialect, req: RunRequest, cancel: Arc<AtomicBool>) -> Result<RunResult, DatabaseError> {
    let started = Instant::now();
    let (log_app, log_id) = (app.clone(), req.run_id.clone());
    let mut on_log = move |line: String| {
        let _ = log_app.emit("datagen-log", LogEvent { id: &log_id, line });
    };
    let (progress_app, progress_id) = (app.clone(), req.run_id.clone());
    let mut on_progress = move |stats: &GenStats| {
        let _ = progress_app.emit("datagen-progress", ProgressEvent { id: &progress_id, stats: stats.clone() });
    };
    let cancelled = || cancel.load(Ordering::Relaxed);
    let stats = run(dialect, &exec, &req.spec, GenHooks { log: &mut on_log, progress: &mut on_progress, cancelled: &cancelled }).await?;
    Ok(RunResult { stats, elapsed_ms: started.elapsed().as_millis() as u64 })
}

macro_rules! provider_commands {
    ($describe:ident, $preview:ident, $run:ident, $sessions:ty, $dialect:expr, $exec:expr) => {
        #[tauri::command]
        pub async fn $describe(sessions: State<'_, $sessions>, conn: String, database: String, schema: String) -> Result<Vec<GenTableMeta>, DatabaseError> {
            let s = sessions.session(&conn)?;
            #[allow(clippy::redundant_closure_call)]
            let exec = ($exec)(s, &database, &schema);
            describe($dialect, &exec, &schema).await
        }

        #[tauri::command]
        pub async fn $preview(sessions: State<'_, $sessions>, conn: String, database: String, schema: String, table: GenTableSpec, rows: u64) -> Result<GenPreview, DatabaseError> {
            let s = sessions.session(&conn)?;
            #[allow(clippy::redundant_closure_call)]
            let exec = ($exec)(s, &database, &schema);
            preview($dialect, &exec, &schema, &table, rows.min(1000)).await
        }

        #[tauri::command]
        pub async fn $run(app: AppHandle, sessions: State<'_, $sessions>, req: RunRequest) -> Result<RunResult, DatabaseError> {
            let s = sessions.session(&req.conn)?;
            #[allow(clippy::redundant_closure_call)]
            let exec = ($exec)(s, &req.database, &req.spec.schema);
            let cancel = Arc::new(AtomicBool::new(false));
            sessions.register_cancel(&req.run_id, cancel.clone());
            let id = req.run_id.clone();
            let out = finish_run(app, exec, $dialect, req, cancel).await;
            sessions.unregister_cancel(&id);
            out
        }
    };
}

provider_commands!(mysql_datagen_describe, mysql_datagen_preview, mysql_datagen_run, MysqlSessions, Dialect::Mysql, |s: Arc<MysqlSession>, db: &str, schema: &str| Exec::Mysql(s, mysql_db(db, schema).to_string()));
provider_commands!(mssql_datagen_describe, mssql_datagen_preview, mssql_datagen_run, MssqlSessions, Dialect::Mssql, |s: Arc<MssqlSession>, db: &str, _schema: &str| Exec::Mssql(s, db.to_string()));
provider_commands!(sqlite_datagen_describe, sqlite_datagen_preview, sqlite_datagen_run, SqliteSessions, Dialect::Sqlite, |s: Arc<SqliteSession>, _db: &str, _schema: &str| Exec::Sqlite(s));
