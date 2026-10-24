//! Servidor SSH em processo (senha "segredo", só encaminhamento direto) e um destino TCP de eco:
//! os bytes enviados pela porta local do túnel têm de voltar iguais.

use atlas_ssh::{SshAuth, SshConfig, Tunnel};
use russh::keys::{Algorithm, PrivateKey};
use russh::server::{self, Auth, Msg, Server as _, Session};
use russh::{Channel, ChannelId};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

#[derive(Clone)]
struct Srv;

impl server::Server for Srv {
    type Handler = Srv;
    fn new_client(&mut self, _: Option<std::net::SocketAddr>) -> Srv {
        Srv
    }
}

impl server::Handler for Srv {
    type Error = russh::Error;

    async fn auth_password(&mut self, user: &str, password: &str) -> Result<Auth, Self::Error> {
        Ok(if user == "dev" && password == "segredo" { Auth::Accept } else { Auth::reject() })
    }

    async fn auth_publickey(&mut self, user: &str, _key: &russh::keys::PublicKey) -> Result<Auth, Self::Error> {
        Ok(if user == "dev" { Auth::Accept } else { Auth::reject() })
    }

    async fn channel_open_direct_tcpip(
        &mut self,
        channel: Channel<Msg>,
        host: &str,
        port: u32,
        _originator: &str,
        _originator_port: u32,
        reply: server::ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        let target = format!("{host}:{port}");
        reply.accept().await;
        tokio::spawn(async move {
            let Ok(mut tcp) = TcpStream::connect(target).await else { return };
            let mut stream = channel.into_stream();
            let _ = tokio::io::copy_bidirectional(&mut stream, &mut tcp).await;
        });
        Ok(())
    }

    async fn data(&mut self, _: ChannelId, _: &[u8], _: &mut Session) -> Result<(), Self::Error> {
        Ok(())
    }
}

async fn start_ssh() -> u16 {
    let config = Arc::new(server::Config {
        keys: vec![PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap()],
        auth_rejection_time: Duration::from_millis(10),
        auth_rejection_time_initial: Some(Duration::from_millis(0)),
        ..Default::default()
    });
    let socket = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let port = socket.local_addr().unwrap().port();
    tokio::spawn(async move {
        let mut s = Srv;
        let _ = s.run_on_socket(config, &socket).await;
    });
    port
}

async fn start_echo() -> u16 {
    let l = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let port = l.local_addr().unwrap().port();
    tokio::spawn(async move {
        loop {
            let Ok((mut c, _)) = l.accept().await else { break };
            tokio::spawn(async move {
                let mut buf = [0u8; 1024];
                while let Ok(n) = c.read(&mut buf).await {
                    if n == 0 || c.write_all(&buf[..n]).await.is_err() {
                        break;
                    }
                }
            });
        }
    });
    port
}

fn cfg(port: u16, password: &str, known: Option<std::path::PathBuf>) -> SshConfig {
    SshConfig {
        host: "127.0.0.1".into(),
        port,
        user: "dev".into(),
        auth: SshAuth::Password(password.into()),
        known_hosts: known,
        connect_timeout: Duration::from_secs(10),
    }
}

async fn echo_through(t: &Tunnel, msg: &[u8]) {
    let mut c = TcpStream::connect(("127.0.0.1", t.local_port)).await.unwrap();
    c.write_all(msg).await.unwrap();
    let mut got = vec![0u8; msg.len()];
    tokio::time::timeout(Duration::from_secs(5), c.read_exact(&mut got)).await.expect("timeout").unwrap();
    assert_eq!(got, msg);
}

#[tokio::test]
async fn forwards_bytes_through_the_ssh_server() {
    let (ssh, echo) = (start_ssh().await, start_echo().await);
    let t = Tunnel::open(&cfg(ssh, "segredo", None), "127.0.0.1", echo).await.expect("open");
    echo_through(&t, b"ola, banco de dados").await;
    // Várias conexões ao mesmo tempo pelo mesmo túnel.
    echo_through(&t, &vec![7u8; 50_000]).await;
    echo_through(&t, b"segunda conexao").await;
}

#[tokio::test]
async fn wrong_password_is_an_authentication_error() {
    let ssh = start_ssh().await;
    let err = Tunnel::open(&cfg(ssh, "errada", None), "127.0.0.1", 1).await.err().expect("must fail");
    assert!(format!("{err:?}").contains("AuthenticationFailed"), "{err:?}");
}

#[tokio::test]
async fn unreachable_server_is_an_error_not_a_hang() {
    let l = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let port = l.local_addr().unwrap().port();
    drop(l);
    assert!(Tunnel::open(&cfg(port, "x", None), "127.0.0.1", 1).await.is_err());
}

#[tokio::test]
async fn host_key_is_saved_and_a_changed_key_is_blocked() {
    let dir = std::env::temp_dir().join(format!("atlas_ssh_{}", std::process::id()));
    let known = dir.join("known_hosts");
    let _ = std::fs::remove_dir_all(&dir);
    let echo = start_echo().await;

    // Primeira conexão: a chave do servidor é guardada.
    let first = start_ssh().await;
    Tunnel::open(&cfg(first, "segredo", Some(known.clone())), "127.0.0.1", echo).await.expect("first");
    let saved = std::fs::read_to_string(&known).unwrap();
    assert!(saved.starts_with(&format!("127.0.0.1:{first} SHA256:")), "{saved}");

    // Mesmo servidor de novo: aceita.
    Tunnel::open(&cfg(first, "segredo", Some(known.clone())), "127.0.0.1", echo).await.expect("again");

    // Outro servidor na mesma porta lógica (chave diferente): grava o id do primeiro com outra impressão digital e bloqueia.
    let second = start_ssh().await;
    let tampered = saved.replace(&format!("127.0.0.1:{first}"), &format!("127.0.0.1:{second}"));
    std::fs::write(&known, tampered).unwrap();
    let err = Tunnel::open(&cfg(second, "segredo", Some(known.clone())), "127.0.0.1", echo).await.err().expect("must block");
    assert!(err.message.contains("changed"), "{err:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn private_key_authentication_works() {
    let (ssh, echo) = (start_ssh().await, start_echo().await);
    let dir = std::env::temp_dir().join(format!("atlas_ssh_key_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let key_path = dir.join("id_ed25519");
    let key = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap();
    std::fs::write(&key_path, key.to_openssh(russh::keys::ssh_key::LineEnding::LF).unwrap().as_str()).unwrap();
    let mut c = cfg(ssh, "", None);
    c.auth = SshAuth::Key { path: key_path.to_string_lossy().into_owned(), passphrase: None };
    let t = Tunnel::open(&c, "127.0.0.1", echo).await.expect("key auth");
    echo_through(&t, b"com chave").await;
    // Arquivo que não existe: erro claro, sem derrubar nada.
    c.auth = SshAuth::Key { path: dir.join("nao_existe").to_string_lossy().into_owned(), passphrase: None };
    assert!(Tunnel::open(&c, "127.0.0.1", echo).await.is_err());
    let _ = std::fs::remove_dir_all(&dir);
}
