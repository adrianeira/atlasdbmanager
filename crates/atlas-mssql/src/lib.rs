//! Driver SQL Server do Atlas (protocolo TDS, via `tiberius`): abre a conexão, lê a estrutura e executa SQL devolvendo os mesmos
//! formatos de resposta do PostgreSQL, para a interface usar as mesmas telas.
//!
//! Uma sessão guarda uma conexão por banco (o SQL Server troca de banco com `USE`; aqui cada banco tem a sua, aberta quando
//! alguém pede). Os valores voltam em texto, como a grade os espera.

mod meta;
mod rows;
mod values;

use atlas_core::{ConnectionParams, DatabaseError, ErrorKind, ServerInfo, SslMode};
use futures_util::TryStreamExt;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tiberius::{AuthMethod, Client, Config, EncryptionLevel, error::Error as TdsError};
use tokio::net::TcpStream;
use tokio::sync::Mutex;
use tokio_util::compat::{Compat, TokioAsyncWriteCompatExt};

pub use rows::*;

pub type Conn = Client<Compat<TcpStream>>;

/// Entre colchetes, como o SQL Server quer os nomes (`]` dobrado).
pub fn quote_ident(name: &str) -> String {
    format!("[{}]", name.replace(']', "]]"))
}

/// Texto entre aspas simples (nome em `OBJECT_ID` e afins).
pub fn quote_literal(s: &str) -> String {
    format!("N'{}'", s.replace('\'', "''"))
}

/// O erro do servidor ou da rede, na categoria da interface.
pub fn map_err(e: TdsError) -> DatabaseError {
    match e {
        TdsError::Server(t) => {
            let kind = match t.code() {
                18456 | 18452 => ErrorKind::AuthenticationFailed,
                4060 | 911 => ErrorKind::DatabaseNotFound,
                102 | 105 | 156 | 207 | 208 | 209 | 213 | 245 | 8152 => ErrorKind::SqlSyntaxError,
                547 | 2601 | 2627 | 515 => ErrorKind::ConstraintViolation,
                229 | 230 | 262 | 297 | 300 => ErrorKind::PermissionDenied,
                _ => ErrorKind::Server,
            };
            let mut err = DatabaseError::new(kind, t.message().to_string());
            err.code = Some(t.code().to_string());
            if t.line() > 0 {
                err.detail = Some(format!("line {}", t.line()));
            }
            err
        }
        TdsError::Io { message, .. } => DatabaseError::new(ErrorKind::HostUnreachable, message),
        TdsError::Tls(m) => DatabaseError::new(ErrorKind::CertificateRejected, m),
        other => DatabaseError::new(ErrorKind::Internal, other.to_string()),
    }
}

pub(crate) fn config_for(p: &ConnectionParams, database: &str) -> Config {
    let mut cfg = Config::new();
    cfg.host(&p.host);
    cfg.port(p.port);
    cfg.authentication(AuthMethod::sql_server(&p.user, p.password.expose()));
    if !database.is_empty() {
        cfg.database(database);
    }
    match p.ssl_mode {
        SslMode::Disable => cfg.encryption(EncryptionLevel::Off),
        SslMode::Prefer | SslMode::Require => {
            cfg.encryption(EncryptionLevel::Required);
            cfg.trust_cert();
        }
        SslMode::VerifyCa | SslMode::VerifyFull => cfg.encryption(EncryptionLevel::Required),
    }
    cfg.application_name("Atlas DB Manager");
    cfg
}

/// Abre uma conexão nova com o banco pedido (vazio = o padrão do login).
pub async fn connect(p: &ConnectionParams, database: &str) -> Result<Conn, DatabaseError> {
    if p.ssh.is_some() {
        return Err(DatabaseError::new(ErrorKind::UnsupportedFeature, "SSH tunnels are not available for SQL Server yet"));
    }
    let cfg = config_for(p, database);
    let timeout = Duration::from_secs(p.connect_timeout_secs.max(1));
    let attempt = async {
        let tcp = TcpStream::connect(cfg.get_addr()).await.map_err(|e| DatabaseError::new(ErrorKind::HostUnreachable, e.to_string()))?;
        tcp.set_nodelay(true).ok();
        Client::connect(cfg, tcp.compat_write()).await.map_err(map_err)
    };
    tokio::time::timeout(timeout, attempt).await.map_err(|_| DatabaseError::new(ErrorKind::Timeout, "connection timed out"))?
}

/// Uma sessão: os parâmetros e uma conexão por banco.
pub struct MssqlSession {
    params: ConnectionParams,
    clients: Mutex<HashMap<String, Arc<Mutex<Conn>>>>,
    pub version: String,
    pub version_num: i32,
    pub current_user: String,
    initial_db: String,
}

impl MssqlSession {
    /// Abre a sessão (conecta no banco inicial e lê a versão).
    pub async fn open(params: ConnectionParams) -> Result<Self, DatabaseError> {
        let started = Instant::now();
        let mut conn = connect(&params, &params.database).await?;
        let row = conn
            .simple_query("SELECT CAST(SERVERPROPERTY('ProductVersion') AS nvarchar(50)), DB_NAME(), SUSER_SNAME()")
            .await
            .map_err(map_err)?
            .into_row()
            .await
            .map_err(map_err)?
            .ok_or_else(|| DatabaseError::new(ErrorKind::Internal, "no server information"))?;
        let product: &str = row.get(0).unwrap_or("0.0");
        let (version, version_num) = parse_version(product);
        let initial_db: String = row.get::<&str, _>(1).unwrap_or("master").to_string();
        let current_user: String = row.get::<&str, _>(2).unwrap_or("").to_string();
        let _ = started;
        let mut clients = HashMap::new();
        clients.insert(initial_db.clone(), Arc::new(Mutex::new(conn)));
        Ok(Self { params, clients: Mutex::new(clients), version, version_num, current_user, initial_db })
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

    /// A conexão do banco pedido; abre se ainda não existe.
    pub async fn client(&self, database: &str) -> Result<Arc<Mutex<Conn>>, DatabaseError> {
        let db = if database.is_empty() { self.initial_db.as_str() } else { database };
        let mut map = self.clients.lock().await;
        if let Some(c) = map.get(db) {
            return Ok(c.clone());
        }
        let conn = Arc::new(Mutex::new(connect(&self.params, db).await?));
        map.insert(db.to_string(), conn.clone());
        Ok(conn)
    }

    /// Fecha a conexão de um banco (a árvore fechou o banco).
    pub async fn close_database(&self, database: &str) {
        self.clients.lock().await.remove(database);
    }

    /// Uma conexão nova e só de quem a pediu (exportação, transação manual).
    pub async fn dedicated(&self, database: &str) -> Result<Conn, DatabaseError> {
        connect(&self.params, if database.is_empty() { &self.initial_db } else { database }).await
    }

    pub fn params(&self) -> &ConnectionParams {
        &self.params
    }
}

/// `16.0.4235.2` → ("16.0", 160000): a versão numérica no formato maior*10000 + menor*100, como o PostgreSQL.
fn parse_version(product: &str) -> (String, i32) {
    let mut parts = product.split('.').map(|p| p.parse::<i32>().unwrap_or(0));
    let (major, minor) = (parts.next().unwrap_or(0), parts.next().unwrap_or(0));
    (format!("{major}.{minor}"), major * 10_000 + minor * 100)
}

/// Roda um texto (um ou vários comandos) e devolve um resultado por result set; sem result set, um resultado vazio com o texto.
pub async fn run_text(conn: &mut Conn, sql: &str, max_rows: usize) -> Result<atlas_core::QueryOutcome, DatabaseError> {
    use atlas_core::{QueryOutcome, ResultColumn, ResultSet, StatementOutcome};
    use tiberius::QueryItem;
    let started = Instant::now();
    let mut stream = conn.simple_query(sql).await.map_err(map_err)?;
    let mut sets: Vec<(Vec<ResultColumn>, Vec<Vec<Option<String>>>, bool)> = Vec::new();
    let mut first_at: Option<Instant> = None;
    while let Some(item) = stream.try_next().await.map_err(map_err)? {
        match item {
            QueryItem::Metadata(meta) => {
                let cols = meta.columns().iter().map(|c| ResultColumn { name: c.name().to_string(), type_name: Some(values::type_name(c.column_type())) }).collect();
                sets.push((cols, Vec::new(), false));
            }
            QueryItem::Row(row) => {
                first_at.get_or_insert_with(Instant::now);
                if let Some(cur) = sets.last_mut() {
                    if cur.1.len() >= max_rows {
                        cur.2 = true;
                    } else {
                        cur.1.push(values::row_text(&row));
                    }
                }
            }
        }
    }
    let began = first_at.unwrap_or_else(Instant::now);
    let (query_time, fetch_time) = (began.duration_since(started).as_secs_f64(), began.elapsed().as_secs_f64());
    let statements: Vec<StatementOutcome> = if sets.is_empty() {
        vec![StatementOutcome { result: None, rows_affected: None, sql: sql.to_string(), query_time: started.elapsed().as_secs_f64(), fetch_time: 0.0 }]
    } else {
        sets.into_iter()
            .map(|(columns, rows, truncated)| {
                let count = rows.len() as u64;
                StatementOutcome { result: Some(ResultSet { columns, rows, truncated }), rows_affected: Some(count), sql: sql.to_string(), query_time, fetch_time }
            })
            .collect()
    };
    Ok(QueryOutcome { statements, elapsed_ms: started.elapsed().as_millis() as u64 })
}

#[cfg(test)]
mod tests;
