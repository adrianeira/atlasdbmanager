//! Túnel SSH das conexões: troca `host`/`port` pelos da porta local do túnel antes de conectar ao banco.

use crate::local::Local;
use atlas_core::{ConnectionParams, DatabaseError};
use atlas_ssh::{SshAuth, SshConfig, Tunnel};
use std::time::Duration;

/// Abre o túnel SSH da conexão (se ela usa um) e aponta `params` para a porta local. O túnel precisa ficar vivo
/// enquanto a conexão existir: quem chama guarda o valor devolvido.
pub async fn start(local: &Local, params: &mut ConnectionParams) -> Result<Option<Tunnel>, DatabaseError> {
    let Some(ssh) = params.ssh.clone() else { return Ok(None) };
    let auth = if ssh.key_path.trim().is_empty() {
        SshAuth::Password(ssh.password.expose().to_string())
    } else {
        let pass = ssh.password.expose();
        SshAuth::Key { path: ssh.key_path.clone(), passphrase: if pass.is_empty() { None } else { Some(pass.to_string()) } }
    };
    let cfg = SshConfig {
        host: ssh.host.trim().to_string(),
        port: ssh.port,
        user: ssh.user.clone(),
        auth,
        known_hosts: Some(local.data_dir().join("ssh_known_hosts")),
        connect_timeout: Duration::from_secs(params.connect_timeout_secs.max(5)),
    };
    let tunnel = Tunnel::open(&cfg, &params.host, params.port).await?;
    params.host = "127.0.0.1".into();
    params.port = tunnel.local_port;
    Ok(Some(tunnel))
}
