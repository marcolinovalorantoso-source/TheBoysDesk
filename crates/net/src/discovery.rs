//! Automatic LAN peer discovery using UDP broadcast beacons.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, Instant};

use protocol::{decode_payload, DiscoveryBeacon, DEFAULT_DISCOVERY_PORT, DISCOVERY_MAGIC};
use tokio::net::UdpSocket;
use tokio::sync::RwLock;

#[derive(Debug, Clone)]
pub struct DiscoveredPeer {
    pub device_id: String,
    pub device_name: String,
    pub addr: SocketAddr,
    pub last_seen: Instant,
}

/// Spawns background tasks to broadcast our presence on the LAN and collect
/// beacons from other peers.
pub async fn start_lan_discovery(
    device_id: String,
    device_name: String,
    quic_port: u16,
) -> std::io::Result<Arc<RwLock<HashMap<String, DiscoveredPeer>>>> {
    let peers = Arc::new(RwLock::new(HashMap::<String, DiscoveredPeer>::new()));

    let recv_sock = UdpSocket::bind(SocketAddr::new(
        IpAddr::V4(Ipv4Addr::UNSPECIFIED),
        DEFAULT_DISCOVERY_PORT,
    ))
    .await;

    if let Ok(sock) = recv_sock {
        let _ = sock.set_broadcast(true);
        let peers_clone = peers.clone();
        let own_id = device_id.clone();

        tokio::spawn(async move {
            let mut buf = [0u8; 512];
            loop {
                match sock.recv_from(&mut buf).await {
                    Ok((n, src)) => {
                        if let Ok(beacon) = decode_payload::<DiscoveryBeacon>(&buf[..n]) {
                            if beacon.magic == DISCOVERY_MAGIC && beacon.device_id != own_id {
                                let peer_addr = SocketAddr::new(src.ip(), beacon.quic_port);
                                let clean_id: String = beacon
                                    .device_id
                                    .chars()
                                    .filter(|c| !c.is_control())
                                    .take(32)
                                    .collect();
                                let clean_name: String = beacon
                                    .device_name
                                    .chars()
                                    .filter(|c| !c.is_control())
                                    .take(64)
                                    .collect();
                                let mut map = peers_clone.write().await;
                                map.retain(|_, p| p.last_seen.elapsed() < Duration::from_secs(10));
                                if map.contains_key(&clean_id) || map.len() < 64 {
                                    map.insert(
                                        clean_id.clone(),
                                        DiscoveredPeer {
                                            device_id: clean_id,
                                            device_name: clean_name,
                                            addr: peer_addr,
                                            last_seen: Instant::now(),
                                        },
                                    );
                                }
                            }
                        }
                    }
                    Err(_) => tokio::time::sleep(Duration::from_millis(250)).await,
                }
            }
        });
    }

    // Broadcaster task
    if let Ok(send_sock) = UdpSocket::bind("0.0.0.0:0").await {
        let _ = send_sock.set_broadcast(true);
        let beacon = DiscoveryBeacon {
            magic: DISCOVERY_MAGIC,
            device_id,
            device_name,
            quic_port,
        };
        let payload = postcard::to_allocvec(&beacon).unwrap_or_default();
        let bcast_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::BROADCAST), DEFAULT_DISCOVERY_PORT);

        tokio::spawn(async move {
            loop {
                let _ = send_sock.send_to(&payload, bcast_addr).await;
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        });
    }

    Ok(peers)
}
