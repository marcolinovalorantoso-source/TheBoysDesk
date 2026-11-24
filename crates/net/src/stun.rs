//! Minimal RFC 5389 STUN client for discovering the public IPv4/port mapping.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::Duration;

use rand::Rng;
use tokio::net::{lookup_host, UdpSocket};
use tokio::time::timeout;

use crate::NetError;

const STUN_MAGIC_COOKIE: u32 = 0x2112_A442;
const STUN_BINDING_REQUEST: u16 = 0x0001;
const STUN_BINDING_RESPONSE: u16 = 0x0101;
const ATTR_MAPPED_ADDRESS: u16 = 0x0001;
const ATTR_XOR_MAPPED_ADDRESS: u16 = 0x0020;

const PUBLIC_STUN_SERVERS: &[&str] = &[
    "stun.l.google.com:19302",
    "stun1.l.google.com:19302",
    "stun.cloudflare.com:3478",
];

/// Queries public STUN servers to determine our public IPv4 address.
pub async fn discover_public_addr() -> Result<SocketAddr, NetError> {
    for server in PUBLIC_STUN_SERVERS {
        if let Ok(addr) = query_stun_server(server).await {
            return Ok(addr);
        }
    }
    Err(NetError::Stun("all STUN servers timed out or failed".into()))
}

async fn query_stun_server(host: &str) -> Result<SocketAddr, NetError> {
    let target = timeout(Duration::from_secs(2), lookup_host(host))
        .await
        .map_err(|_| NetError::Stun("DNS timeout".into()))??
        .find(|a| a.is_ipv4())
        .ok_or_else(|| NetError::Stun("no IPv4 address for STUN host".into()))?;

    let sock = UdpSocket::bind("0.0.0.0:0").await?;
    sock.connect(target).await?;

    let mut txn_id = [0u8; 12];
    rand::thread_rng().fill(&mut txn_id);

    let mut req = [0u8; 20];
    req[0..2].copy_from_slice(&STUN_BINDING_REQUEST.to_be_bytes());
    req[2..4].copy_from_slice(&0u16.to_be_bytes());
    req[4..8].copy_from_slice(&STUN_MAGIC_COOKIE.to_be_bytes());
    req[8..20].copy_from_slice(&txn_id);

    sock.send(&req).await?;

    let mut buf = [0u8; 512];
    let n = timeout(Duration::from_secs(2), sock.recv(&mut buf))
        .await
        .map_err(|_| NetError::Stun("STUN recv timeout".into()))??;

    parse_binding_response(&buf[..n], &txn_id)
}

fn parse_binding_response(pkt: &[u8], expected_txn: &[u8; 12]) -> Result<SocketAddr, NetError> {
    if pkt.len() < 20 {
        return Err(NetError::Stun("response too short".into()));
    }
    let msg_type = u16::from_be_bytes([pkt[0], pkt[1]]);
    let msg_len = u16::from_be_bytes([pkt[2], pkt[3]]) as usize;
    let cookie = u32::from_be_bytes([pkt[4], pkt[5], pkt[6], pkt[7]]);
    if msg_type != STUN_BINDING_RESPONSE || cookie != STUN_MAGIC_COOKIE || &pkt[8..20] != expected_txn {
        return Err(NetError::Stun("invalid STUN header".into()));
    }
    if pkt.len() < 20 + msg_len {
        return Err(NetError::Stun("truncated STUN body".into()));
    }

    let mut offset = 20;
    let end = 20 + msg_len;
    while offset + 4 <= end {
        let attr_type = u16::from_be_bytes([pkt[offset], pkt[offset + 1]]);
        let attr_len = u16::from_be_bytes([pkt[offset + 2], pkt[offset + 3]]) as usize;
        let val_start = offset + 4;
        if val_start + attr_len > end {
            break;
        }
        let val = &pkt[val_start..val_start + attr_len];

        if attr_type == ATTR_XOR_MAPPED_ADDRESS && val.len() >= 8 && val[1] == 0x01 {
            let xport = u16::from_be_bytes([val[2], val[3]]) ^ ((STUN_MAGIC_COOKIE >> 16) as u16);
            let xip = u32::from_be_bytes([val[4], val[5], val[6], val[7]]) ^ STUN_MAGIC_COOKIE;
            return Ok(SocketAddr::new(IpAddr::V4(Ipv4Addr::from(xip)), xport));
        }

        if attr_type == ATTR_MAPPED_ADDRESS && val.len() >= 8 && val[1] == 0x01 {
            let port = u16::from_be_bytes([val[2], val[3]]);
            let ip = Ipv4Addr::new(val[4], val[5], val[6], val[7]);
            return Ok(SocketAddr::new(IpAddr::V4(ip), port));
        }

        let padded = (attr_len + 3) & !3;
        offset = val_start + padded;
    }

    Err(NetError::Stun("no mapped address attribute found".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_xor_mapped_address() {
        let txn = [7u8; 12];
        let ip = Ipv4Addr::new(203, 0, 113, 42);
        let port: u16 = 54321;

        let xport = port ^ ((STUN_MAGIC_COOKIE >> 16) as u16);
        let xip = u32::from(ip) ^ STUN_MAGIC_COOKIE;

        let mut pkt = vec![0u8; 32];
        pkt[0..2].copy_from_slice(&STUN_BINDING_RESPONSE.to_be_bytes());
        pkt[2..4].copy_from_slice(&12u16.to_be_bytes());
        pkt[4..8].copy_from_slice(&STUN_MAGIC_COOKIE.to_be_bytes());
        pkt[8..20].copy_from_slice(&txn);
        pkt[20..22].copy_from_slice(&ATTR_XOR_MAPPED_ADDRESS.to_be_bytes());
        pkt[22..24].copy_from_slice(&8u16.to_be_bytes());
        pkt[24] = 0;
        pkt[25] = 0x01; // IPv4
        pkt[26..28].copy_from_slice(&xport.to_be_bytes());
        pkt[28..32].copy_from_slice(&xip.to_be_bytes());

        let parsed = parse_binding_response(&pkt, &txn).unwrap();
        assert_eq!(parsed, SocketAddr::new(IpAddr::V4(ip), port));
    }
}
