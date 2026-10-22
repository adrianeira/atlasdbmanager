use crate::error::map_error;
use atlas_core::{ConnectionParams, DatabaseError, ErrorKind, ServerInfo, SslMode};
use postgres_native_tls::MakeTlsConnector;
use std::time::{Duration, Instant};
use tokio_postgres::{CancelToken, Client, Config, NoTls};

/// Uma conexão física aberta com um banco PostgreSQL.
pub struct PgSession {
    pub client: Client,
    /// Serializa as execuções do editor que trocam o `search_path`: sem isso duas abas na mesma
    /// conexão física poderiam se atropelar entre o `SET` e a consulta.
    pub(crate) exec_lock: tokio::sync::Mutex<()>,
    cancel: CancelToken,
    tls: Option<MakeTlsConnector>,
    pub server: ServerInfo,
}

impl PgSession {
    /// Pede ao servidor o cancelamento do comando em execução (não garante efeito imediato — spec §29.3).
    pub async fn cancel(&self) -> Result<(), DatabaseError> {
        let res = match &self.tls {
            Some(tls) => self.cancel.cancel_query(tls.clone()).await,
            None => self.cancel.cancel_query(NoTls).await,
        };
        res.map_err(|e| map_error(&e))
    }

    pub fn is_closed(&self) -> bool {
        self.client.is_closed()
    }
}

/// Monta o conector TLS conforme o modo (semântica do libpq).
/// `verify-full` valida cadeia e host; `verify-ca` só a cadeia; `prefer`/`require` só criptografam.
fn tls_connector(mode: SslMode) -> Result<Option<MakeTlsConnector>, DatabaseError> {
    if mode == SslMode::Disable {
        return Ok(None);
    }
    let mut builder = native_tls::TlsConnector::builder();
    match mode {
        SslMode::Prefer | SslMode::Require => {
            builder.danger_accept_invalid_certs(true).danger_accept_invalid_hostnames(true);
        }
        SslMode::VerifyCa => {
            builder.danger_accept_invalid_hostnames(true);
        }
        SslMode::VerifyFull | SslMode::Disable => {}
    }
    let connector = builder
        .build()
        .map_err(|e| DatabaseError::new(ErrorKind::CertificateRejected, format!("TLS: {e}")))?;
    Ok(Some(MakeTlsConnector::new(connector)))
}

fn config_for(params: &ConnectionParams, database: &str) -> Config {
    let mut cfg = Config::new();
    cfg.host(&params.host)
        .port(params.port)
        .user(&params.user)
        .dbname(database)
        .application_name("Atlas DB Manager")
        .connect_timeout(Duration::from_secs(params.connect_timeout_secs.max(1)))
        .keepalives(true)
        .ssl_mode(match params.ssl_mode {
            SslMode::Disable => tokio_postgres::config::SslMode::Disable,
            SslMode::Prefer => tokio_postgres::config::SslMode::Prefer,
            _ => tokio_postgres::config::SslMode::Require,
        });
    if !params.password.is_empty() {
        cfg.password(params.password.expose());
    }
    cfg
}

/// Abre uma conexão com `database` (ou o banco inicial do perfil) e lê a identificação do servidor.
pub async fn connect(params: &ConnectionParams, database: Option<&str>) -> Result<PgSession, DatabaseError> {
    if params.host.trim().is_empty() {
        return Err(DatabaseError::new(ErrorKind::InvalidInput, "host is empty"));
    }
    let database = database.unwrap_or(&params.database);
    let database = if database.is_empty() { "postgres" } else { database };
    let cfg = config_for(params, database);
    let tls = tls_connector(params.ssl_mode)?;

    let started = Instant::now();
    let client = match &tls {
        Some(tls) => {
            let (client, conn) = cfg.connect(tls.clone()).await.map_err(|e| map_error(&e))?;
            tokio::spawn(async move {
                if let Err(e) = conn.await {
                    log::warn!("conexão PostgreSQL encerrada: {e}");
                }
            });
            client
        }
        None => {
            let (client, conn) = cfg.connect(NoTls).await.map_err(|e| map_error(&e))?;
            tokio::spawn(async move {
                if let Err(e) = conn.await {
                    log::warn!("conexão PostgreSQL encerrada: {e}");
                }
            });
            client
        }
    };
    let latency_ms = started.elapsed().as_millis() as u64;

    let row = client
        .query_one(
            "SELECT current_setting('server_version'), current_setting('server_version_num')::int, \
                    current_database()::text, current_user::text",
            &[],
        )
        .await
        .map_err(|e| map_error(&e))?;
    let version: String = row.get(0);
    let version_num: i32 = row.get(1);
    // pg_stat_ssl existe a partir do 9.5; em versões antigas o estado TLS fica desconhecido.
    let tls_active = if version_num >= 90500 {
        client
            .query_opt("SELECT ssl FROM pg_stat_ssl WHERE pid = pg_backend_pid()", &[])
            .await
            .ok()
            .flatten()
            .map(|r| r.get::<_, bool>(0))
    } else {
        None
    };

    let server = ServerInfo {
        version: version.split_whitespace().next().unwrap_or(&version).to_string(),
        version_num,
        current_database: row.get(2),
        current_user: row.get(3),
        tls: tls_active,
        latency_ms,
    };
    let cancel = client.cancel_token();
    Ok(PgSession { client, exec_lock: tokio::sync::Mutex::new(()), cancel, tls, server })
}
