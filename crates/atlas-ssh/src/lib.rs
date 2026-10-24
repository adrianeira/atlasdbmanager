//! Túnel SSH: abre uma porta local (127.0.0.1) cujas conexões seguem pelo servidor SSH até o destino
//! (`host:port` visto do servidor SSH). A conexão com o banco passa a apontar para essa porta local.
//!
//! A chave do servidor é guardada na primeira conexão (confiança no primeiro uso) e uma chave diferente
//! depois disso bloqueia a conexão.

use atlas_core::{DatabaseError, ErrorKind};
use russh::client::{self, Handler};
use russh::keys::{load_secret_key, HashAlg, PrivateKeyWithHashAlg, PublicKeyOrCertificate};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::copy_bidirectional;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

pub enum SshAuth {
    Password(String),
    Key { path: String, passphrase: Option<String> },
}

pub struct SshConfig {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub auth: SshAuth,
    /// Arquivo onde as chaves de servidor conhecidas ficam (uma por linha: `host:porta impressão-digital`).
    /// Sem ele, qualquer chave é aceita.
    pub known_hosts: Option<PathBuf>,
    pub connect_timeout: Duration,
}

fn ssh_error(kind: ErrorKind, message: impl Into<String>) -> DatabaseError {
    DatabaseError::new(kind, message)
}

/// Aceita a chave do servidor conforme o arquivo de hosts conhecidos; guarda o motivo da recusa para a mensagem de erro.
struct HostKeys {
    id: String,
    file: Option<PathBuf>,
    refusal: Arc<Mutex<Option<String>>>,
}

impl Handler for HostKeys {
    type Error = russh::Error;

    async fn check_server_key(&mut self, key: &PublicKeyOrCertificate) -> Result<bool, Self::Error> {
        let PublicKeyOrCertificate::PublicKey { key, .. } = key else {
            *self.refusal.lock().unwrap() = Some("the SSH server presented a certificate, which is not supported".into());
            return Ok(false);
        };
        let Some(file) = &self.file else { return Ok(true) };
        let fingerprint = key.fingerprint(HashAlg::Sha256).to_string();
        let known = std::fs::read_to_string(file).unwrap_or_default();
        let saved = known.lines().filter_map(|l| l.split_once(' ')).find(|(id, _)| *id == self.id).map(|(_, fp)| fp.trim().to_string());
        match saved {
            Some(fp) if fp == fingerprint => Ok(true),
            Some(fp) => {
                *self.refusal.lock().unwrap() = Some(format!(
                    "the host key of {} changed (saved {fp}, now {fingerprint}); the connection was blocked. If the server was reinstalled, remove the line for it from {}",
                    self.id,
                    file.display()
                ));
                Ok(false)
            }
            None => {
                // Primeira conexão: guarda a impressão digital.
                if let Some(dir) = file.parent() {
                    let _ = std::fs::create_dir_all(dir);
                }
                let mut text = known;
                if !text.is_empty() && !text.ends_with('\n') {
                    text.push('\n');
                }
                text.push_str(&format!("{} {fingerprint}\n", self.id));
                let _ = std::fs::write(file, text);
                log::info!("SSH: host key of {} saved ({fingerprint})", self.id);
                Ok(true)
            }
        }
    }
}

/// Túnel aberto. Fechar (soltar) encerra a porta local e a conexão SSH.
pub struct Tunnel {
    pub local_port: u16,
    accept: JoinHandle<()>,
}

impl Drop for Tunnel {
    fn drop(&mut self) {
        self.accept.abort();
    }
}

impl Tunnel {
    /// Conecta ao servidor SSH, autentica e deixa a porta local pronta para encaminhar para `target_host:target_port`.
    pub async fn open(cfg: &SshConfig, target_host: &str, target_port: u16) -> Result<Tunnel, DatabaseError> {
        let refusal = Arc::new(Mutex::new(None));
        let handler = HostKeys { id: format!("{}:{}", cfg.host, cfg.port), file: cfg.known_hosts.clone(), refusal: refusal.clone() };
        let config = Arc::new(client::Config { nodelay: true, inactivity_timeout: None, keepalive_interval: Some(Duration::from_secs(30)), ..Default::default() });
        let connect = client::connect(config, (cfg.host.as_str(), cfg.port), handler);
        let mut session = match tokio::time::timeout(cfg.connect_timeout, connect).await {
            Err(_) => return Err(ssh_error(ErrorKind::Timeout, format!("SSH: timed out connecting to {}:{}", cfg.host, cfg.port))),
            Ok(Err(e)) => {
                let why = refusal.lock().unwrap().take().unwrap_or_else(|| e.to_string());
                return Err(ssh_error(ErrorKind::HostUnreachable, format!("SSH: cannot connect to {}:{}: {why}", cfg.host, cfg.port)));
            }
            Ok(Ok(s)) => s,
        };
        let authenticated = match &cfg.auth {
            SshAuth::Password(p) => session.authenticate_password(cfg.user.clone(), p.clone()).await,
            SshAuth::Key { path, passphrase } => {
                let key = load_secret_key(path, passphrase.as_deref())
                    .map_err(|e| ssh_error(ErrorKind::AuthenticationFailed, format!("SSH: cannot read the private key {path}: {e}")))?;
                let hash = session.best_supported_rsa_hash().await.ok().flatten().flatten();
                session.authenticate_publickey(cfg.user.clone(), PrivateKeyWithHashAlg::new(Arc::new(key), hash)).await
            }
        }
        .map_err(|e| ssh_error(ErrorKind::AuthenticationFailed, format!("SSH: {e}")))?;
        if !authenticated.success() {
            return Err(ssh_error(ErrorKind::AuthenticationFailed, format!("SSH: authentication failed for {}@{}", cfg.user, cfg.host)));
        }

        let listener = TcpListener::bind(("127.0.0.1", 0)).await.map_err(|e| ssh_error(ErrorKind::Internal, format!("SSH: cannot open a local port: {e}")))?;
        let local_port = listener.local_addr().map_err(|e| ssh_error(ErrorKind::Internal, e.to_string()))?.port();
        let session = Arc::new(session);
        let target = target_host.to_string();
        let accept = tokio::spawn(async move {
            loop {
                let Ok((mut local, peer)) = listener.accept().await else { break };
                let session = session.clone();
                let target = target.clone();
                tokio::spawn(async move {
                    let channel = match session.channel_open_direct_tcpip(target.as_str(), target_port as u32, peer.ip().to_string(), peer.port() as u32).await {
                        Ok(c) => c,
                        Err(e) => {
                            log::warn!("SSH: cannot forward to {target}:{target_port}: {e}");
                            return;
                        }
                    };
                    let mut remote = channel.into_stream();
                    let _ = copy_bidirectional(&mut local, &mut remote).await;
                });
            }
        });
        Ok(Tunnel { local_port, accept })
    }
}
