//! Servidor SSH de desenvolvimento, só para testar o túnel pela tela: usuário `dev`, senha `segredo` (ou qualquer chave),
//! encaminha qualquer destino. Uso: `cargo run -p atlas-ssh --example dev_server -- 2222`.
//! NÃO é seguro: não use fora da máquina de desenvolvimento.

use russh::keys::{Algorithm, PrivateKey};
use russh::server::{self, Auth, Msg, Server as _, Session};
use russh::{Channel, ChannelId};
use std::sync::Arc;
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

#[tokio::main]
async fn main() {
    let port: u16 = std::env::args().nth(1).and_then(|p| p.parse().ok()).unwrap_or(2222);
    let config = Arc::new(server::Config { keys: vec![PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap()], ..Default::default() });
    let socket = TcpListener::bind(("127.0.0.1", port)).await.unwrap();
    println!("dev SSH server on 127.0.0.1:{port} (user dev / password segredo)");
    let mut s = Srv;
    let _ = s.run_on_socket(config, &socket).await;
}
