//! Peer-to-peer QUIC networking, UPnP port mapping, STUN public IP discovery,
//! and UDP LAN peer discovery.

pub mod discovery;
pub mod stun;
pub mod upnp;

use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::sync::Arc;
use std::time::Duration;

use crypto::DeviceIdentity;
use protocol::{ProtocolError, StreamKind, DEFAULT_QUIC_PORT};
use quinn::crypto::rustls::{QuicClientConfig, QuicServerConfig};
use quinn::{ClientConfig, Connection, Endpoint, Incoming, RecvStream, SendStream, ServerConfig, TransportConfig};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum NetError {
    #[error("crypto error: {0}")]
    Crypto(#[from] crypto::CryptoError),
    #[error("protocol error: {0}")]
    Protocol(#[from] ProtocolError),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("QUIC connect error: {0}")]
    Connect(#[from] quinn::ConnectError),
    #[error("QUIC connection error: {0}")]
    Connection(#[from] quinn::ConnectionError),
    #[error("QUIC write error: {0}")]
    Write(#[from] quinn::WriteError),
    #[error("QUIC read error: {0}")]
    Read(#[from] quinn::ReadExactError),
    #[error("TLS config error: {0}")]
    NoInitialCipher(#[from] quinn::crypto::rustls::NoInitialCipherSuite),
    #[error("STUN discovery failed: {0}")]
    Stun(String),
    #[error("UPnP mapping failed: {0}")]
    Upnp(String),
}

/// Pure-P2P QUIC endpoint capable of both accepting and initiating sessions.
pub struct P2pEndpoint {
    endpoint: Endpoint,
    local_port: u16,
}

impl P2pEndpoint {
    /// Creates a QUIC endpoint bound to `0.0.0.0:DEFAULT_QUIC_PORT` (falling back
    /// to an ephemeral port if the default port is already in use).
    pub fn bind(identity: &DeviceIdentity) -> Result<Self, NetError> {
        let server_tls = identity.server_config()?;
        let quic_server = QuicServerConfig::try_from(server_tls)?;
        let mut server_cfg = ServerConfig::with_crypto(Arc::new(quic_server));
        server_cfg.transport_config(Arc::new(low_latency_transport()));

        let client_tls = DeviceIdentity::client_config()?;
        let quic_client = QuicClientConfig::try_from(client_tls)?;
        let mut client_cfg = ClientConfig::new(Arc::new(quic_client));
        client_cfg.transport_config(Arc::new(low_latency_transport()));

        let bind_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), DEFAULT_QUIC_PORT);
        let endpoint = match Endpoint::server(server_cfg.clone(), bind_addr) {
            Ok(ep) => ep,
            Err(_) => {
                let fallback = SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0);
                Endpoint::server(server_cfg, fallback)?
            }
        };

        let mut ep = endpoint;
        ep.set_default_client_config(client_cfg);
        let local_port = ep.local_addr()?.port();

        Ok(Self {
            endpoint: ep,
            local_port,
        })
    }

    pub fn local_port(&self) -> u16 {
        self.local_port
    }

    pub async fn accept(&self) -> Option<Incoming> {
        self.endpoint.accept().await
    }

    pub async fn connect(&self, addr: SocketAddr) -> Result<Connection, NetError> {
        let connecting = self.endpoint.connect(addr, "anydesk-alt.p2p")?;
        let conn = connecting.await?;
        Ok(conn)
    }

    pub fn close(&self) {
        self.endpoint.close(0u32.into(), b"shutdown");
    }
}

fn low_latency_transport() -> TransportConfig {
    let mut cfg = TransportConfig::default();
    cfg.keep_alive_interval(Some(Duration::from_secs(4)));
    cfg.max_idle_timeout(Some(Duration::from_secs(20).try_into().unwrap()));
    cfg
}

/// Opens a bidirectional QUIC stream and writes the 1-byte `StreamKind` tag.
pub async fn open_bi_stream(
    conn: &Connection,
    kind: StreamKind,
) -> Result<(SendStream, RecvStream), NetError> {
    let (mut send, recv) = conn.open_bi().await?;
    send.write_all(&[kind as u8]).await?;
    Ok((send, recv))
}

/// Accepts a bidirectional QUIC stream and reads its 1-byte `StreamKind` tag.
pub async fn accept_bi_stream(
    conn: &Connection,
) -> Result<(StreamKind, SendStream, RecvStream), NetError> {
    let (send, mut recv) = conn.accept_bi().await?;
    let mut tag = [0u8; 1];
    recv.read_exact(&mut tag).await?;
    let kind = StreamKind::try_from(tag[0])?;
    Ok((kind, send, recv))
}

/// Opens a unidirectional QUIC stream and writes the 1-byte `StreamKind` tag.
pub async fn open_uni_stream(conn: &Connection, kind: StreamKind) -> Result<SendStream, NetError> {
    let mut send = conn.open_uni().await?;
    send.write_all(&[kind as u8]).await?;
    Ok(send)
}

/// Accepts a unidirectional QUIC stream and reads its 1-byte `StreamKind` tag.
pub async fn accept_uni_stream(conn: &Connection) -> Result<(StreamKind, RecvStream), NetError> {
    let mut recv = conn.accept_uni().await?;
    let mut tag = [0u8; 1];
    recv.read_exact(&mut tag).await?;
    let kind = StreamKind::try_from(tag[0])?;
    Ok((kind, recv))
}

/// Determines the primary local LAN IPv4 address by opening a non-sending UDP socket.
pub fn local_lan_ipv4() -> Option<Ipv4Addr> {
    let sock = UdpSocket::bind("0.0.0.0:0").ok()?;
    sock.connect("8.8.8.8:80").ok()?;
    match sock.local_addr().ok()?.ip() {
        IpAddr::V4(v4) => Some(v4),
        IpAddr::V6(_) => None,
    }
}
