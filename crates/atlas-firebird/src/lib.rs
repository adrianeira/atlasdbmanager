//! Driver Firebird do Atlas: conecta com o cliente em Rust puro (sem `fbclient.dll`, serve do Firebird 2.5 ao 5), lê a estrutura pelas
//! tabelas de sistema `RDB$` e executa SQL devolvendo os mesmos formatos de resposta do PostgreSQL, para a interface usar as mesmas telas.
//! O cliente é síncrono: quem chama decide em que thread rodar. Cada sessão é uma conexão; cada transação manual abre a sua própria.

use atlas_core::{ConnectionParams, DatabaseError, ErrorKind, OrderBy, QueryOutcome, ResultColumn, ResultSet, ServerInfo, StatementOutcome, TableFilter};
use rsfbclient::{Column, Connection, Execute, FbError, Queryable, Row, RustFbClient, SqlType};
use serde::Deserialize;
use std::sync::Mutex;
use std::time::Instant;

pub mod meta;
pub mod split;
mod values;

pub use split::{Piece, split_statements};
pub use values::type_name;

pub type Conn = Connection<RustFbClient>;

/// Um comando com parâmetros posicionais (`?`), como a interface o planeja; os valores vão em texto (ou NULL) e o servidor converte
/// para o tipo da coluna.
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

/// Identificador entre aspas (o Firebird distingue maiúsculas de minúsculas quando há aspas; os nomes vêm do catálogo como estão).
pub fn quote_ident(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

pub fn unsupported_feature(what: &str) -> DatabaseError {
    DatabaseError::new(ErrorKind::UnsupportedFeature, format!("{what} is not available for Firebird"))
}

/// Erro do Firebird na forma da interface (categoria, código SQL e, quando há, a posição no texto).
pub fn map_err(e: FbError) -> DatabaseError {
    match e {
        FbError::Sql { msg, code } => {
            let lower = msg.to_lowercase();
            let kind = if lower.contains("user name and password") || lower.contains("login") && lower.contains("not defined") {
                ErrorKind::AuthenticationFailed
            } else if lower.contains("i/o error") || lower.contains("unavailable database") || lower.contains("cannot attach") || lower.contains("file ") && lower.contains("not found") {
                ErrorKind::DatabaseNotFound
            } else if lower.contains("operation was cancelled") || lower.contains("statement cancelled") || lower.contains("cancelled by") {
                ErrorKind::Canceled
            } else if lower.contains("no permission") || lower.contains("not have permission") {
                ErrorKind::PermissionDenied
            } else if lower.contains("violation of") || lower.contains("validation error") || lower.contains("attempt to store duplicate") {
                ErrorKind::ConstraintViolation
            } else if lower.contains("lock conflict") || lower.contains("deadlock") {
                ErrorKind::Timeout
            } else {
                match code {
                    -104 | -204 | -206 | -607 => ErrorKind::SqlSyntaxError,
                    -803 | -530 | -531 | -297 | -625 => ErrorKind::ConstraintViolation,
                    _ => ErrorKind::Server,
                }
            };
            let mut err = DatabaseError::new(kind, msg.trim().to_string());
            err.code = Some(code.to_string());
            err.position = position_of(&err.message);
            err
        }
        FbError::Io(e) => DatabaseError::new(ErrorKind::HostUnreachable, e.to_string()),
        FbError::Other(m) => {
            let kind = if m.contains("Connection refused") || m.contains("timed out") || m.contains("network") { ErrorKind::HostUnreachable } else { ErrorKind::Internal };
            DatabaseError::new(kind, m)
        }
    }
}

/// "... Token unknown - line 1, column 8": a coluna é a posição do erro (a linha só conta quando o texto tem mais de uma).
fn position_of(msg: &str) -> Option<u32> {
    let i = msg.find("column ")?;
    let digits: String = msg[i + 7..].chars().take_while(|c| c.is_ascii_digit()).collect();
    digits.parse().ok()
}

/// Onde e como conectar.
#[derive(Debug, Clone)]
pub struct Target {
    pub host: String,
    pub port: u16,
    /// Caminho do arquivo no servidor ou apelido (`alias`) do `databases.conf`.
    pub database: String,
    pub user: String,
    pub password: String,
}

impl Target {
    pub fn from_params(p: &ConnectionParams) -> Self {
        Self {
            host: if p.host.trim().is_empty() { "127.0.0.1".into() } else { p.host.trim().to_string() },
            port: if p.port == 0 { 3050 } else { p.port },
            database: p.database.trim().to_string(),
            user: if p.user.trim().is_empty() { "SYSDBA".into() } else { p.user.trim().to_string() },
            password: p.password.expose().to_string(),
        }
    }
}

fn builder(t: &Target) -> rsfbclient::PureRustConnectionBuilder {
    let mut b = rsfbclient::builder_pure_rust();
    b.host(t.host.clone()).port(t.port).db_name(t.database.clone()).user(t.user.clone()).pass(t.password.clone()).charset(rsfbclient::charset::UTF_8).stmt_cache_size(0);
    b
}

pub fn connect(t: &Target) -> Result<Conn, DatabaseError> {
    if t.database.is_empty() {
        return Err(DatabaseError::new(ErrorKind::InvalidInput, "inform the database file or alias"));
    }
    builder(t).connect().map_err(map_err)
}

/// Cria um banco novo (o arquivo não pode existir; a pasta é a do servidor).
pub fn create_database(t: &Target, page_size: Option<u32>) -> Result<(), DatabaseError> {
    let mut b = builder(t);
    if let Some(p) = page_size {
        b.page_size(p);
    }
    b.create_database().map_err(map_err)?.close().map_err(map_err)
}

/* ---------- Execução ---------- */

fn params_of(values: &[Option<String>]) -> Vec<SqlType> {
    values.iter().map(|v| v.as_ref().map_or(SqlType::Null, |s| SqlType::Text(s.clone()))).collect()
}

fn returns_rows(sql: &str) -> bool {
    let w = split::words(sql, 2);
    match w.first().map(String::as_str) {
        Some("SELECT" | "WITH") => true,
        _ => false,
    }
}

/// Comandos que devolvem uma linha ao executar (`EXECUTE PROCEDURE`, `... RETURNING`).
fn returns_one_row(sql: &str) -> bool {
    let w = split::words(sql, 4);
    if w.first().map(String::as_str) == Some("EXECUTE") && w.get(1).map(String::as_str) == Some("PROCEDURE") {
        return true;
    }
    matches!(w.first().map(String::as_str), Some("INSERT" | "UPDATE" | "DELETE" | "MERGE")) && split::words(sql, usize::MAX).iter().any(|x| x == "RETURNING")
}

fn result_set(meta: &[rsfbclient_rust::ColumnMeta], rows: Vec<Vec<Option<String>>>, truncated: bool) -> ResultSet {
    ResultSet { columns: meta.iter().map(|m| ResultColumn { name: m.name.clone(), type_name: Some(values::type_name(m.raw_type)) }).collect(), rows, truncated }
}

/// Roda um comando e monta o resultado; `max_rows` limita o que fica na memória.
fn run_statement(conn: &mut Conn, sql: &str, max_rows: usize) -> Result<StatementOutcome, DatabaseError> {
    let started = Instant::now();
    if returns_rows(sql) {
        let iter = conn.query_iter::<_, Row>(sql, ()).map_err(map_err)?;
        let meta = rsfbclient_rust::last_describe();
        let began = Instant::now();
        let mut out: Vec<Vec<Option<String>>> = Vec::new();
        let mut truncated = false;
        for row in iter {
            let row = row.map_err(map_err)?;
            if out.len() >= max_rows {
                truncated = true;
                break;
            }
            out.push(row.cols.iter().map(values::format).collect());
        }
        let count = out.len() as u64;
        return Ok(StatementOutcome { result: Some(result_set(&meta, out, truncated)), rows_affected: Some(count), sql: sql.to_string(), query_time: began.duration_since(started).as_secs_f64(), fetch_time: began.elapsed().as_secs_f64() });
    }
    if returns_one_row(sql) {
        let row: Row = conn.execute_returnable(sql, ()).map_err(map_err)?;
        let meta = rsfbclient_rust::last_describe();
        let cells: Vec<Option<String>> = row.cols.iter().map(values::format).collect();
        let took = started.elapsed().as_secs_f64();
        return Ok(StatementOutcome { result: Some(result_set(&meta, vec![cells], false)), rows_affected: Some(1), sql: sql.to_string(), query_time: took, fetch_time: 0.0 });
    }
    let n = conn.execute(sql, ()).map_err(map_err)?;
    Ok(StatementOutcome { result: None, rows_affected: Some(n as u64), sql: sql.to_string(), query_time: started.elapsed().as_secs_f64(), fetch_time: 0.0 })
}

/// Executa o texto: cada comando separado por `;` (ou pelo terminador de `SET TERM`) roda por vez.
pub fn execute_on(conn: &mut Conn, sql: &str, max_rows: usize) -> Result<QueryOutcome, DatabaseError> {
    let started = Instant::now();
    let mut statements = Vec::new();
    for piece in split_statements(sql) {
        statements.push(run_statement(conn, &piece.sql, max_rows).map_err(|mut e| {
            // A posição vale para o texto todo, não só para o comando.
            if let Some(p) = e.position {
                e.position = Some(p + piece.start as u32);
            }
            e
        })?);
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
fn apply_on(conn: &mut Conn, statements: &[PlannedStatement], own_transaction: bool) -> Result<Vec<u64>, DatabaseError> {
    if own_transaction {
        conn.begin_transaction().map_err(map_err)?;
    }
    let finish = |conn: &mut Conn, ok: bool| {
        if own_transaction {
            let _ = if ok { conn.commit() } else { conn.rollback() };
        } else if !ok {
            let _ = conn.execute("ROLLBACK TO SAVEPOINT atlas_batch", ());
        }
    };
    if !own_transaction {
        conn.execute("SAVEPOINT atlas_batch", ()).map_err(map_err)?;
    }
    let mut affected = Vec::with_capacity(statements.len());
    for (i, st) in statements.iter().enumerate() {
        let failure = match conn.execute(&st.sql, params_of(&st.params)).map(|n| n as u64).map_err(map_err) {
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
            finish(conn, false);
            return Err(err);
        }
    }
    if own_transaction {
        conn.commit().map_err(map_err)?;
    } else {
        conn.execute("RELEASE SAVEPOINT atlas_batch", ()).map_err(map_err)?;
    }
    Ok(affected)
}

fn where_clause(filter: Option<&TableFilter>) -> String {
    match filter.map(|f| f.expr.trim()) {
        Some(expr) if !expr.is_empty() => format!(" WHERE {expr}"),
        _ => String::new(),
    }
}

/// `SELECT * FROM tabela [WHERE ...] [ORDER BY ...]`: só identificadores entre aspas entram no texto; o filtro é a expressão do usuário.
pub fn select_sql(table: &str, order_by: &[OrderBy], filter: Option<&TableFilter>) -> String {
    let mut sql = format!("SELECT * FROM {}{}", quote_ident(table), where_clause(filter));
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

fn rows_on(conn: &mut Conn, table: &str, order_by: &[OrderBy], filter: Option<&TableFilter>, limit: Option<u32>, offset: u64, max_rows: usize) -> Result<(String, QueryOutcome), DatabaseError> {
    let mut sql = select_sql(table, order_by, filter);
    let keep = match limit {
        Some(limit) => {
            sql.push_str(&format!(" ROWS {} TO {}", offset + 1, offset + u64::from(limit)));
            (limit as usize).min(max_rows)
        }
        None => max_rows,
    };
    let out = execute_on(conn, &sql, keep)?;
    Ok((sql, out))
}

fn count_on(conn: &mut Conn, table: &str, filter: Option<&TableFilter>) -> Result<u64, DatabaseError> {
    let sql = format!("SELECT COUNT(*) FROM {}{}", quote_ident(table), where_clause(filter));
    let rows: Vec<Row> = conn.query(&sql, ()).map_err(map_err)?;
    let n = rows.first().and_then(|r| r.cols.first()).map(|c| match &c.value {
        SqlType::Integer(i) => *i,
        SqlType::Int128(i) => *i as i64,
        _ => 0,
    });
    Ok(n.unwrap_or(0).max(0) as u64)
}

/// Lê as linhas de um `SELECT` uma a uma, numa conexão própria, sem guardar nada na memória.
/// `on_begin` recebe as colunas e `on_row` cada linha; devolver `Ok(false)` encerra a leitura. Devolve quantas linhas foram entregues.
pub fn stream_rows(
    target: &Target,
    sql: &str,
    on_begin: impl FnOnce(&[ResultColumn]) -> Result<(), DatabaseError>,
    mut on_row: impl FnMut(&[Option<&str>]) -> Result<bool, DatabaseError>,
) -> Result<u64, DatabaseError> {
    let mut conn = connect(target)?;
    let iter = conn.query_iter::<_, Row>(sql, ()).map_err(map_err)?;
    let meta = rsfbclient_rust::last_describe();
    let described: Vec<ResultColumn> = meta.iter().map(|m| ResultColumn { name: m.name.clone(), type_name: Some(values::type_name(m.raw_type)) }).collect();
    on_begin(&described)?;
    let mut count = 0u64;
    for row in iter {
        let row = row.map_err(map_err)?;
        let cells: Vec<Option<String>> = row.cols.iter().map(values::format).collect();
        let refs: Vec<Option<&str>> = cells.iter().map(|c| c.as_deref()).collect();
        if !on_row(&refs)? {
            break;
        }
        count += 1;
    }
    Ok(count)
}

/* ---------- Sessão ---------- */

/// Uma conexão com um banco Firebird.
pub struct FirebirdSession {
    pub target: Target,
    conn: Mutex<Conn>,
    pub version: String,
    pub version_num: i32,
    pub user: String,
    /// Número da conexão no servidor (`MON$ATTACHMENT_ID`): serve para cancelar e para o monitor.
    pub attachment_id: i64,
}

fn text_of(c: &Column) -> String {
    values::format(c).unwrap_or_default()
}

/// "4.0.2" → 4_000_002, no mesmo formato dos outros bancos (maior * 1_000_000 + menor * 1_000 + correção).
fn version_number(v: &str) -> i32 {
    let mut n = v.split('.').map(|p| p.chars().take_while(|c| c.is_ascii_digit()).collect::<String>().parse::<i32>().unwrap_or(0));
    n.next().unwrap_or(0) * 1_000_000 + n.next().unwrap_or(0) * 1_000 + n.next().unwrap_or(0)
}

impl FirebirdSession {
    pub fn open(target: Target) -> Result<Self, DatabaseError> {
        let mut conn = connect(&target)?;
        let rows: Vec<Row> = conn
            .query("SELECT RDB$GET_CONTEXT('SYSTEM', 'ENGINE_VERSION'), RDB$GET_CONTEXT('SYSTEM', 'CURRENT_USER'), CURRENT_CONNECTION FROM RDB$DATABASE", ())
            .map_err(map_err)?;
        let row = rows.first().ok_or_else(|| DatabaseError::new(ErrorKind::Internal, "the server returned no version"))?;
        let version = text_of(&row.cols[0]);
        let user = text_of(&row.cols[1]);
        let attachment_id = text_of(&row.cols[2]).parse().unwrap_or(0);
        let version_num = version_number(&version);
        Ok(Self { target, conn: Mutex::new(conn), version, version_num, user, attachment_id })
    }

    pub fn with<T>(&self, f: impl FnOnce(&mut Conn) -> Result<T, DatabaseError>) -> Result<T, DatabaseError> {
        let mut guard = self.conn.lock().map_err(|_| DatabaseError::new(ErrorKind::Internal, "connection lock poisoned"))?;
        f(&mut guard)
    }

    /// Uma conexão nova ao mesmo banco, para o que não pode esperar a sessão (cancelar, buscar nos dados, importar).
    pub fn dedicated(&self) -> Result<Conn, DatabaseError> {
        connect(&self.target)
    }

    pub fn server_info(&self) -> ServerInfo {
        ServerInfo { version: self.version.clone(), version_num: self.version_num, current_database: self.target.database.clone(), current_user: self.user.clone(), tls: None, latency_ms: 0 }
    }

    pub fn list_objects(&self) -> Result<atlas_core::SchemaObjects, DatabaseError> {
        self.with(meta::list_objects_on)
    }

    pub fn table_detail(&self, table: &str) -> Result<atlas_core::TableDetail, DatabaseError> {
        self.with(|c| meta::table_detail_on(c, table))
    }

    pub fn view_detail(&self, view: &str) -> Result<atlas_core::ViewDetail, DatabaseError> {
        self.with(|c| meta::view_detail_on(c, view))
    }

    pub fn execute(&self, sql: &str, max_rows: usize) -> Result<QueryOutcome, DatabaseError> {
        self.with(|c| execute_on(c, sql, max_rows))
    }

    pub fn table_rows(&self, table: &str, order_by: &[OrderBy], filter: Option<&TableFilter>, limit: Option<u32>, offset: u64, max_rows: usize) -> Result<(String, QueryOutcome), DatabaseError> {
        self.with(|c| rows_on(c, table, order_by, filter, limit, offset, max_rows))
    }

    pub fn table_count(&self, table: &str, filter: Option<&TableFilter>) -> Result<u64, DatabaseError> {
        self.with(|c| count_on(c, table, filter))
    }

    pub fn apply(&self, statements: &[PlannedStatement]) -> Result<Vec<u64>, DatabaseError> {
        self.with(|c| apply_on(c, statements, true))
    }

    /// Interrompe o comando em andamento: pede ao servidor, por outra conexão, que cancele a consulta desta (FB 2.5 em diante).
    pub fn cancel(&self) -> Result<(), DatabaseError> {
        let mut other = self.dedicated()?;
        other.execute("DELETE FROM MON$STATEMENTS WHERE MON$ATTACHMENT_ID = ? AND MON$STATE = 1", vec![SqlType::Integer(self.attachment_id)]).map_err(map_err)?;
        Ok(())
    }

    /// Abre uma transação manual (conexão própria ao mesmo banco).
    pub fn begin(&self) -> Result<FirebirdTx, DatabaseError> {
        let mut conn = connect(&self.target)?;
        conn.begin_transaction().map_err(map_err)?;
        Ok(FirebirdTx { conn: Mutex::new(conn) })
    }
}

/// Transação manual aberta: tudo que roda nela fica pendente até `finish`.
pub struct FirebirdTx {
    conn: Mutex<Conn>,
}

impl FirebirdTx {
    fn with<T>(&self, f: impl FnOnce(&mut Conn) -> Result<T, DatabaseError>) -> Result<T, DatabaseError> {
        let mut guard = self.conn.lock().map_err(|_| DatabaseError::new(ErrorKind::Internal, "transaction lock poisoned"))?;
        f(&mut guard)
    }

    pub fn execute(&self, sql: &str, max_rows: usize) -> Result<QueryOutcome, DatabaseError> {
        self.with(|c| execute_on(c, sql, max_rows))
    }

    pub fn apply(&self, statements: &[PlannedStatement]) -> Result<Vec<u64>, DatabaseError> {
        self.with(|c| apply_on(c, statements, false))
    }

    pub fn table_rows(&self, table: &str, order_by: &[OrderBy], filter: Option<&TableFilter>, limit: Option<u32>, offset: u64, max_rows: usize) -> Result<(String, QueryOutcome), DatabaseError> {
        self.with(|c| rows_on(c, table, order_by, filter, limit, offset, max_rows))
    }

    pub fn table_count(&self, table: &str, filter: Option<&TableFilter>) -> Result<u64, DatabaseError> {
        self.with(|c| count_on(c, table, filter))
    }

    /// Grava (`commit`) ou desfaz a transação.
    pub fn finish(&self, commit: bool) -> Result<(), DatabaseError> {
        self.with(|c| if commit { c.commit() } else { c.rollback() }.map_err(map_err))
    }
}

#[cfg(test)]
mod tests;
