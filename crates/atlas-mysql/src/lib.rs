//! Driver MySQL do Atlas (`mysql_async`): abre a conexão, lê a estrutura e executa SQL devolvendo os mesmos formatos de resposta do
//! PostgreSQL, para a interface usar as mesmas telas.
//!
//! No MySQL "banco" e "schema" são a mesma coisa: cada banco aparece na árvore com um schema de mesmo nome. Uma sessão guarda uma
//! conexão por banco (aberta quando alguém pede). Os valores voltam em texto, como a grade os espera.

mod console;
mod dump;
mod extra;
mod find;
mod import;
mod meta;
mod profile;
mod rows;
mod values;

use atlas_core::{ConnectionParams, DatabaseError, ErrorKind, QueryOutcome, ResultColumn, ResultSet, ServerInfo, SslMode, StatementOutcome};
use mysql_async::prelude::*;
use mysql_async::{Conn, OptsBuilder, Row, SslOpts};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

pub use console::{Completion as ConsoleCompletion, ConsoleOptions, ConsoleOutput};
pub use dump::{DumpPlan, DumpTable, literal};
pub use extra::FkLabels;
pub use profile::DataProfile;
pub use find::{FindDataResult, FindMode, FindOptions, FindResult};
pub use import::{Feed as ImportFeed, Hooks as ImportHooks, ImportMode, ImportSpec, ImportStats, TargetColumn, declared_type};
pub use rows::*;

/// Entre crases, como o MySQL quer os nomes (crase dobrada).
pub fn quote_ident(name: &str) -> String {
    format!("`{}`", name.replace('`', "``"))
}

/// O erro do servidor ou da rede, na categoria da interface.
pub fn map_err(e: mysql_async::Error) -> DatabaseError {
    match e {
        mysql_async::Error::Server(s) => {
            let kind = match s.code {
                1045 | 1698 | 1251 => ErrorKind::AuthenticationFailed,
                1049 => ErrorKind::DatabaseNotFound,
                1064 | 1054 | 1146 | 1149 | 1065 => ErrorKind::SqlSyntaxError,
                1062 | 1451 | 1452 | 1048 | 1216 | 1217 | 3819 | 1264 | 1406 => ErrorKind::ConstraintViolation,
                1044 | 1142 | 1143 | 1227 | 1370 => ErrorKind::PermissionDenied,
                1317 | 3024 => ErrorKind::Canceled,
                _ => ErrorKind::Server,
            };
            let mut err = DatabaseError::new(kind, s.message);
            err.code = Some(s.code.to_string());
            err
        }
        mysql_async::Error::Io(e) => DatabaseError::new(ErrorKind::HostUnreachable, e.to_string()),
        other => DatabaseError::new(ErrorKind::Internal, other.to_string()),
    }
}

fn opts_for(p: &ConnectionParams, database: &str) -> OptsBuilder {
    let mut b = OptsBuilder::default()
        .ip_or_hostname(p.host.clone())
        .tcp_port(p.port)
        .user(Some(p.user.clone()))
        .pass(Some(p.password.expose().to_string()))
        // `UPDATE` devolve as linhas que casaram (não só as que mudaram): a conferência de "exatamente 1 linha" depende disso.
        .client_found_rows(true)
        .prefer_socket(false);
    if !database.is_empty() {
        b = b.db_name(Some(database.to_string()));
    }
    match p.ssl_mode {
        SslMode::Disable => b,
        SslMode::Prefer | SslMode::Require => b.ssl_opts(Some(SslOpts::default().with_danger_accept_invalid_certs(true).with_danger_skip_domain_validation(true))),
        SslMode::VerifyCa => b.ssl_opts(Some(SslOpts::default().with_danger_skip_domain_validation(true))),
        SslMode::VerifyFull => b.ssl_opts(Some(SslOpts::default())),
    }
}

/// Abre uma conexão nova com o banco pedido (vazio = nenhum).
pub async fn connect(p: &ConnectionParams, database: &str) -> Result<Conn, DatabaseError> {
    let timeout = Duration::from_secs(p.connect_timeout_secs.max(1));
    let opts = opts_for(p, database);
    tokio::time::timeout(timeout, Conn::new(opts)).await.map_err(|_| DatabaseError::new(ErrorKind::Timeout, "connection timed out"))?.map_err(map_err)
}

/// Uma sessão: os parâmetros e uma conexão por banco.
pub struct MysqlSession {
    params: ConnectionParams,
    clients: Mutex<HashMap<String, Arc<Mutex<Conn>>>>,
    /// O `CONNECTION_ID()` da conexão de cada banco: é por ele que uma consulta em andamento é cancelada (`KILL QUERY`).
    ids: std::sync::Mutex<HashMap<String, u32>>,
    pub version: String,
    pub version_num: i32,
    pub current_user: String,
    initial_db: String,
}

impl MysqlSession {
    /// Abre a sessão (conecta e lê a versão).
    pub async fn open(params: ConnectionParams) -> Result<Self, DatabaseError> {
        let mut conn = connect(&params, &params.database).await?;
        let row: Option<(String, Option<String>, String)> = conn.query_first("SELECT VERSION(), DATABASE(), CURRENT_USER()").await.map_err(map_err)?;
        let (full, db, user) = row.ok_or_else(|| DatabaseError::new(ErrorKind::Internal, "no server information"))?;
        let (version, version_num) = parse_version(&full);
        let initial_db = db.unwrap_or_default();
        let id = conn.id();
        let mut clients = HashMap::new();
        clients.insert(initial_db.clone(), Arc::new(Mutex::new(conn)));
        let ids = std::sync::Mutex::new(HashMap::from([(initial_db.clone(), id)]));
        Ok(Self { params, clients: Mutex::new(clients), ids, version, version_num, current_user: user, initial_db })
    }

    pub fn server_info(&self, latency_ms: u64) -> ServerInfo {
        ServerInfo {
            version: self.version.clone(),
            version_num: self.version_num,
            current_database: self.initial_db.clone(),
            current_user: self.current_user.clone(),
            tls: Some(self.params.ssl_mode != SslMode::Disable),
            latency_ms,
        }
    }

    pub fn initial_database(&self) -> &str {
        &self.initial_db
    }

    pub fn params(&self) -> &ConnectionParams {
        &self.params
    }

    /// A conexão do banco pedido; abre se ainda não existe.
    pub async fn client(&self, database: &str) -> Result<Arc<Mutex<Conn>>, DatabaseError> {
        let db = if database.is_empty() { self.initial_db.as_str() } else { database };
        let mut map = self.clients.lock().await;
        if let Some(c) = map.get(db) {
            return Ok(c.clone());
        }
        let fresh = connect(&self.params, db).await?;
        if let Ok(mut m) = self.ids.lock() {
            m.insert(db.to_string(), fresh.id());
        }
        let conn = Arc::new(Mutex::new(fresh));
        map.insert(db.to_string(), conn.clone());
        Ok(conn)
    }

    /// Fecha a conexão de um banco.
    pub async fn close_database(&self, database: &str) {
        self.clients.lock().await.remove(database);
    }

    /// Uma conexão nova e só de quem a pediu (exportação, transação manual).
    pub async fn dedicated(&self, database: &str) -> Result<Conn, DatabaseError> {
        connect(&self.params, if database.is_empty() { &self.initial_db } else { database }).await
    }

    /// Cancela o que o banco está executando (`KILL QUERY` por outra conexão): a conexão continua viva.
    pub async fn cancel(&self, database: &str) -> Result<(), DatabaseError> {
        let db = if database.is_empty() { self.initial_db.clone() } else { database.to_string() };
        let id = self.ids.lock().ok().and_then(|m| m.get(&db).copied());
        let Some(id) = id else { return Ok(()) };
        let mut killer = connect(&self.params, "").await?;
        killer.query_drop(format!("KILL QUERY {id}")).await.map_err(map_err)
    }
}

/// `8.4.3` ou `10.11.5-MariaDB` → ("8.4", 80403): a versão numérica no formato maior*10000 + menor*100 + correção.
fn parse_version(full: &str) -> (String, i32) {
    let mut parts = full.split(|c: char| !c.is_ascii_digit()).filter(|p| !p.is_empty()).map(|p| p.parse::<i32>().unwrap_or(0));
    let (major, minor, patch) = (parts.next().unwrap_or(0), parts.next().unwrap_or(0), parts.next().unwrap_or(0));
    (format!("{major}.{minor}"), major * 10_000 + minor * 100 + patch)
}

/// Roda um texto (um ou vários comandos) e devolve um resultado por result set; sem result set, um resultado vazio com as linhas afetadas.
pub async fn run_text(conn: &mut Conn, sql: &str, max_rows: usize) -> Result<QueryOutcome, DatabaseError> {
    let started = Instant::now();
    let mut result = conn.query_iter(sql).await.map_err(map_err)?;
    let mut statements = Vec::new();
    let mut first_at: Option<Instant> = None;
    loop {
        let columns: Vec<ResultColumn> = result.columns_ref().iter().map(|c| ResultColumn { name: c.name_str().to_string(), type_name: Some(values::type_name(c)) }).collect();
        let cols = result.columns();
        let affected = result.affected_rows();
        let rows: Vec<Row> = result.collect().await.map_err(map_err)?;
        first_at.get_or_insert_with(Instant::now);
        if columns.is_empty() {
            statements.push(StatementOutcome { result: None, rows_affected: Some(affected), sql: sql.to_string(), query_time: started.elapsed().as_secs_f64(), fetch_time: 0.0 });
        } else {
            let truncated = rows.len() > max_rows;
            let text: Vec<Vec<Option<String>>> = rows.iter().take(max_rows).map(|r| values::row_text(r, cols.as_deref().unwrap_or(&[]))).collect();
            let count = text.len() as u64;
            let began = first_at.unwrap_or_else(Instant::now);
            statements.push(StatementOutcome {
                result: Some(ResultSet { columns, rows: text, truncated }),
                rows_affected: Some(count),
                sql: sql.to_string(),
                query_time: began.duration_since(started).as_secs_f64(),
                fetch_time: began.elapsed().as_secs_f64(),
            });
        }
        if result.is_empty() {
            break;
        }
    }
    // Um comando que falha depois do último conjunto de resultados só mostra o erro ao esvaziar o que sobrou.
    result.drop_result().await.map_err(map_err)?;
    Ok(QueryOutcome { statements, elapsed_ms: started.elapsed().as_millis() as u64 })
}

#[cfg(test)]
mod tests;
