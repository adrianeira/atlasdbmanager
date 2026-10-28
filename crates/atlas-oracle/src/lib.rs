//! Driver Oracle do Atlas (`oracle-rs`, cliente em Rust puro: não precisa do Oracle Instant Client): abre a conexão, lê a estrutura
//! pelos catálogos `ALL_*` e executa SQL devolvendo os mesmos formatos de resposta do PostgreSQL, para a interface usar as mesmas telas.
//!
//! No Oracle o "banco" da conexão é o serviço (`FREEPDB1`, `XEPDB1`...) e os schemas são os usuários. Uma sessão é uma conexão; cada
//! transação manual abre a sua. Os valores voltam em texto, como a grade os espera.

mod meta;
mod rows;
pub mod split;
mod values;

use atlas_core::{ConnectionParams, DatabaseError, ErrorKind, QueryOutcome, ResultColumn, ResultSet, ServerInfo, SslMode, StatementOutcome};
use oracle_rs::{Config, Connection, Error as OraError, Value};
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

pub use rows::*;
pub use split::{Piece, split_statements};

/// Entre aspas duplas, como o Oracle quer os nomes (aspa dobrada). Os nomes vêm do catálogo como estão (em maiúsculas, em geral).
pub fn quote_ident(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

pub fn unsupported_feature(what: &str) -> DatabaseError {
    DatabaseError::new(ErrorKind::UnsupportedFeature, format!("{what} is not available for Oracle yet"))
}

/// "ORA-00942: tabela..." → "tabela...": o código já vai à parte.
fn without_prefix(msg: &str, code: u32) -> String {
    let prefix = format!("ORA-{code:05}:");
    msg.trim().strip_prefix(&prefix).unwrap_or(msg.trim()).trim().to_string()
}

/// O erro do servidor ou da rede, na categoria da interface.
pub fn map_err(e: OraError) -> DatabaseError {
    // O servidor informa a posição em bytes a partir de 0; aqui fica "bytes + 1" e `execute_on` a converte em caracteres do texto todo.
    let (e, position) = match e {
        OraError::OracleErrorAt { code, message, position } => (OraError::OracleError { code, message }, Some(position + 1)),
        other => (other, None),
    };
    let mut mapped = map_code(e);
    mapped.position = position;
    mapped
}

fn map_code(e: OraError) -> DatabaseError {
    match e {
        OraError::OracleError { code, message } | OraError::ServerError { code, message } => {
            let kind = match code {
                1017 | 1005 | 28000 | 28001 | 28009 | 28040 => ErrorKind::AuthenticationFailed,
                12514 | 12505 | 12154 | 12516 | 12520 => ErrorKind::DatabaseNotFound,
                900..=989 | 991..=999 | 1722 | 1730 | 1747 | 1756 | 1858 | 1861 | 4043 | 6550 => ErrorKind::SqlSyntaxError,
                1 | 1400 | 1407 | 1438 | 1451 | 2290 | 2291 | 2292 | 2299 | 12899 | 1401 => ErrorKind::ConstraintViolation,
                1031 | 1950 | 990 | 1045 => ErrorKind::PermissionDenied,
                1013 => ErrorKind::Canceled,
                51 | 30006 | 54 => ErrorKind::Timeout,
                60 => ErrorKind::ConcurrencyConflict,
                _ => ErrorKind::Server,
            };
            let mut err = DatabaseError::new(kind, without_prefix(&message, code));
            err.code = Some(format!("ORA-{code:05}"));
            err
        }
        OraError::ConnectionRefused { .. } | OraError::Io(_) => DatabaseError::new(ErrorKind::HostUnreachable, e.to_string()),
        OraError::ConnectionTimeout(_) => DatabaseError::new(ErrorKind::Timeout, e.to_string()),
        OraError::AuthenticationFailed(_) | OraError::InvalidCredentials => DatabaseError::new(ErrorKind::AuthenticationFailed, e.to_string()),
        OraError::InvalidServiceName { .. } | OraError::InvalidSid { .. } => DatabaseError::new(ErrorKind::DatabaseNotFound, e.to_string()),
        OraError::ConnectionClosed | OraError::ConnectionClosedByServer(_) | OraError::ConnectionNotReady => DatabaseError::new(ErrorKind::ConnectionLost, e.to_string()),
        other => DatabaseError::new(ErrorKind::Internal, other.to_string()),
    }
}

/// O serviço da conexão: `FREEPDB1` (serviço) ou `sid:ORCL` (SID).
fn config_for(p: &ConnectionParams) -> Result<Config, DatabaseError> {
    let host = if p.host.trim().is_empty() { "127.0.0.1".to_string() } else { p.host.trim().to_string() };
    let port = if p.port == 0 { 1521 } else { p.port };
    let name = p.database.trim();
    if name.is_empty() {
        return Err(DatabaseError::new(ErrorKind::InvalidInput, "inform the service name (for example FREEPDB1 or XEPDB1)"));
    }
    let user = p.user.trim().to_string();
    let pass = p.password.expose().to_string();
    let cfg = match name.strip_prefix("sid:") {
        Some(sid) => Config::with_sid(host, port, sid.trim(), user, pass),
        None => Config::new(host, port, name, user, pass),
    };
    // Sem cache de instruções: o driver reaproveita o cursor quando o SQL se repete, e esse caminho perde linhas em consultas com LOB
    // (e não dá ganho que valha o risco num cliente gráfico). Cada execução faz um parse novo, o caminho que está correto.
    let cfg = cfg.connect_timeout(Duration::from_secs(p.connect_timeout_secs.max(1))).stmtcachesize(0);
    match p.ssl_mode {
        SslMode::Disable => Ok(cfg),
        _ => cfg.with_tls().map_err(map_err),
    }
}

/// Formatos de data e número que a interface usa: valem para o texto que o Atlas manda ao servidor (parâmetros) e para o que o
/// servidor converte em texto dentro de uma consulta (`TO_CHAR` sem formato).
const SESSION_SETUP: &str = "ALTER SESSION SET NLS_DATE_FORMAT = 'YYYY-MM-DD HH24:MI:SS' NLS_TIMESTAMP_FORMAT = 'YYYY-MM-DD HH24:MI:SS.FF6' \
     NLS_TIMESTAMP_TZ_FORMAT = 'YYYY-MM-DD HH24:MI:SS.FF6 TZH:TZM' NLS_NUMERIC_CHARACTERS = '.,'";

/// Abre uma conexão nova e acerta os formatos da sessão.
pub async fn connect(p: &ConnectionParams) -> Result<Connection, DatabaseError> {
    let cfg = config_for(p)?;
    let conn = Connection::connect_with_config(cfg).await.map_err(map_err)?;
    conn.execute(SESSION_SETUP, &[]).await.map_err(map_err)?;
    Ok(conn)
}

/* ---------- Execução ---------- */

pub(crate) fn params_of(values: &[Option<String>]) -> Vec<Value> {
    values.iter().map(|v| v.as_ref().map_or(Value::Null, |s| Value::String(s.clone()))).collect()
}

/// Roda um `SELECT` e monta o resultado; `max_rows` limita o que fica na memória.
async fn run_select(conn: &Connection, sql: &str, max_rows: usize) -> Result<StatementOutcome, DatabaseError> {
    let started = Instant::now();
    let mut res = conn.query(sql, &[]).await.map_err(map_err)?;
    let first_at = Instant::now();
    let columns = res.columns.clone();
    let mut rows = std::mem::take(&mut res.rows);
    while res.has_more_rows && rows.len() <= max_rows {
        res = conn.fetch_more(res.cursor_id, &columns, 500).await.map_err(map_err)?;
        rows.extend(std::mem::take(&mut res.rows));
    }
    let truncated = rows.len() > max_rows;
    rows.truncate(max_rows);
    let mut text: Vec<Vec<Option<String>>> = Vec::with_capacity(rows.len());
    for row in &rows {
        let mut cells = Vec::with_capacity(columns.len());
        for (i, col) in columns.iter().enumerate() {
            cells.push(match row.get(i) {
                Some(v) => values::cell(conn, col, v).await,
                None => None,
            });
        }
        text.push(cells);
    }
    let count = text.len() as u64;
    let result = ResultSet { columns: columns.iter().map(|c| ResultColumn { name: c.name.clone(), type_name: Some(values::type_name(c)) }).collect(), rows: text, truncated };
    Ok(StatementOutcome { result: Some(result), rows_affected: Some(count), sql: sql.to_string(), query_time: first_at.duration_since(started).as_secs_f64(), fetch_time: first_at.elapsed().as_secs_f64() })
}

/// Roda um comando que não devolve linhas (DML, DDL, bloco PL/SQL).
async fn run_command(conn: &Connection, sql: &str) -> Result<StatementOutcome, DatabaseError> {
    let started = Instant::now();
    let res = conn.execute(sql, &[]).await.map_err(map_err)?;
    Ok(StatementOutcome { result: None, rows_affected: Some(res.rows_affected), sql: sql.to_string(), query_time: started.elapsed().as_secs_f64(), fetch_time: 0.0 })
}

/// A posição do erro (bytes + 1 dentro do comando) vira a posição em caracteres no texto inteiro.
fn in_text(mut e: DatabaseError, piece: &Piece) -> DatabaseError {
    if let Some(p) = e.position {
        let bytes = (p as usize).saturating_sub(1);
        let chars = piece.sql.char_indices().take_while(|(i, _)| *i < bytes).count();
        e.position = Some((chars + 1 + piece.start) as u32);
    }
    e
}

/// Executa o texto: cada comando (separado por `;` ou por `/` nos blocos PL/SQL) roda por vez. Fora de uma transação manual (`autocommit`)
/// cada comando que não é consulta é gravado ao terminar.
pub(crate) async fn execute_on(conn: &Connection, sql: &str, max_rows: usize, autocommit: bool) -> Result<QueryOutcome, DatabaseError> {
    let started = Instant::now();
    let mut statements = Vec::new();
    for piece in split_statements(sql) {
        if let Some((name, at)) = split::unbound_placeholder(&piece.sql) {
            let mut e = DatabaseError::new(ErrorKind::SqlSyntaxError, format!("not all variables bound (:{name}): the Atlas does not send values for bind variables"));
            e.code = Some("ORA-01008".into());
            e.position = Some((piece.start + at + 1) as u32);
            return Err(e);
        }
        if split::returns_rows(&piece.sql) {
            statements.push(run_select(conn, &piece.sql, max_rows).await.map_err(|e| in_text(e, &piece))?);
        } else {
            let out = run_command(conn, &piece.sql).await.map_err(|e| in_text(e, &piece))?;
            if autocommit {
                conn.commit().await.map_err(map_err)?;
            }
            statements.push(out);
        }
    }
    Ok(QueryOutcome { statements, elapsed_ms: started.elapsed().as_millis() as u64 })
}

/* ---------- Sessão ---------- */

/// Uma conexão com um banco Oracle.
pub struct OracleSession {
    params: ConnectionParams,
    conn: Connection,
    /// O schema atual da conexão (`ALTER SESSION SET CURRENT_SCHEMA`); troca só quando a consulta pede outro.
    schema: Mutex<String>,
    pub version: String,
    pub version_num: i32,
    pub current_user: String,
    pub service: String,
    /// `sid,serial#` da conexão, para cancelar o que ela executa (vazio se o login não pode lê-los).
    sid_serial: String,
    /// Quantos comandos desta sessão estão rodando agora.
    running: std::sync::atomic::AtomicUsize,
}

/// "21.0.0.0.0" → ("21.0", 210000): maior * 10000 + menor * 100 + correção.
fn parse_version(full: &str) -> (String, i32) {
    let mut parts = full.split(|c: char| !c.is_ascii_digit()).filter(|p| !p.is_empty()).map(|p| p.parse::<i32>().unwrap_or(0));
    let (major, minor, patch) = (parts.next().unwrap_or(0), parts.next().unwrap_or(0), parts.next().unwrap_or(0));
    (format!("{major}.{minor}"), major * 10_000 + minor * 100 + patch)
}

impl OracleSession {
    /// Abre a sessão (conecta e lê a versão).
    pub async fn open(params: ConnectionParams) -> Result<Self, DatabaseError> {
        let conn = connect(&params).await?;
        let res = conn
            .query(
                "SELECT (SELECT version FROM product_component_version WHERE ROWNUM = 1), SYS_CONTEXT('USERENV', 'CURRENT_USER'), \
                        SYS_CONTEXT('USERENV', 'SERVICE_NAME'), SYS_CONTEXT('USERENV', 'SID') FROM dual",
                &[],
            )
            .await
            .map_err(map_err)?;
        let row = res.rows.first().ok_or_else(|| DatabaseError::new(ErrorKind::Internal, "no server information"))?;
        let text = |i: usize| row.get(i).map(|v| v.to_string()).filter(|s| s != "NULL").unwrap_or_default();
        let (version, version_num) = parse_version(&text(0));
        let (user, service, sid) = (text(1), text(2), text(3));
        let serial = match conn.query("SELECT serial# FROM v$session WHERE sid = SYS_CONTEXT('USERENV', 'SID')", &[]).await {
            Ok(r) => r.rows.first().and_then(|r| r.get(0)).map(|v| v.to_string()),
            Err(_) => None,
        };
        let sid_serial = serial.map(|s| format!("{sid},{s}")).unwrap_or_default();
        Ok(Self { params, conn, schema: Mutex::new(user.clone()), running: std::sync::atomic::AtomicUsize::new(0), version, version_num, current_user: user, service, sid_serial })
    }

    pub fn server_info(&self, latency_ms: u64) -> ServerInfo {
        ServerInfo {
            version: self.version.clone(),
            version_num: self.version_num,
            current_database: self.service.clone(),
            current_user: self.current_user.clone(),
            tls: Some(self.params.ssl_mode != SslMode::Disable),
            latency_ms,
        }
    }

    pub fn params(&self) -> &ConnectionParams {
        &self.params
    }

    /// Uma conexão nova e só de quem a pediu (exportação, transação manual).
    pub async fn dedicated(&self) -> Result<Connection, DatabaseError> {
        connect(&self.params).await
    }

    /// Põe a conexão no schema pedido (vazio = o do usuário).
    pub(crate) async fn use_schema(&self, schema: &str) -> Result<tokio::sync::MutexGuard<'_, String>, DatabaseError> {
        let mut current = self.schema.lock().await;
        let wanted = if schema.is_empty() { self.current_user.as_str() } else { schema };
        if *current != wanted {
            self.conn.execute(&format!("ALTER SESSION SET CURRENT_SCHEMA = {}", quote_ident(wanted)), &[]).await.map_err(map_err)?;
            *current = wanted.to_string();
        }
        Ok(current)
    }

    pub async fn execute(&self, schema: &str, sql: &str, max_rows: usize) -> Result<QueryOutcome, DatabaseError> {
        let _guard = self.use_schema(schema).await?;
        self.running.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let out = execute_on(&self.conn, sql, max_rows, true).await;
        self.running.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
        out
    }

    /// Interrompe o comando em andamento: manda o marcador de interrupção pela própria conexão (sem privilégios; a consulta termina com
    /// `ORA-01013`). Com TLS isso não é possível e cai no `ALTER SYSTEM CANCEL SQL` por outra conexão (exige o privilégio, Oracle 18 em
    /// diante). Sem comando rodando não faz nada: o marcador chegaria ao comando seguinte.
    pub async fn cancel(&self) -> Result<(), DatabaseError> {
        if self.running.load(std::sync::atomic::Ordering::SeqCst) == 0 {
            return Ok(());
        }
        match self.conn.cancel().await {
            Ok(()) => Ok(()),
            Err(OraError::FeatureNotSupported(_)) => {
                if self.sid_serial.is_empty() {
                    return Err(unsupported_feature("Cancelling a query over TLS (the login cannot read V$SESSION)"));
                }
                let other = self.dedicated().await?;
                let r = other.execute(&format!("ALTER SYSTEM CANCEL SQL '{}'", self.sid_serial), &[]).await.map_err(map_err);
                let _ = other.close().await;
                r.map(|_| ())
            }
            Err(e) => Err(map_err(e)),
        }
    }

    pub async fn close(&self) {
        let _ = self.conn.close().await;
    }

    /// Abre uma transação manual: uma conexão só dela (o Oracle começa a transação no primeiro comando).
    pub async fn begin(&self) -> Result<OracleTx, DatabaseError> {
        let conn = self.dedicated().await?;
        Ok(OracleTx { conn, user: self.current_user.clone(), schema: Mutex::new(self.current_user.clone()) })
    }

    pub(crate) fn conn(&self) -> &Connection {
        &self.conn
    }
}

/// Transação manual aberta: tudo que roda nela fica pendente até `finish`.
pub struct OracleTx {
    conn: Connection,
    user: String,
    schema: Mutex<String>,
}

impl OracleTx {
    async fn use_schema(&self, schema: &str) -> Result<tokio::sync::MutexGuard<'_, String>, DatabaseError> {
        let mut current = self.schema.lock().await;
        let wanted = if schema.is_empty() { self.user.as_str() } else { schema };
        if *current != wanted {
            self.conn.execute(&format!("ALTER SESSION SET CURRENT_SCHEMA = {}", quote_ident(wanted)), &[]).await.map_err(map_err)?;
            *current = wanted.to_string();
        }
        Ok(current)
    }

    pub async fn execute(&self, schema: &str, sql: &str, max_rows: usize) -> Result<QueryOutcome, DatabaseError> {
        let _guard = self.use_schema(schema).await?;
        execute_on(&self.conn, sql, max_rows, false).await
    }

    /// Grava (`commit`) ou desfaz a transação e fecha a conexão dela.
    pub async fn finish(&self, commit: bool) -> Result<(), DatabaseError> {
        let r = if commit { self.conn.commit().await } else { self.conn.rollback().await }.map_err(map_err);
        let _ = self.conn.close().await;
        r
    }
}

#[cfg(test)]
mod tests;
