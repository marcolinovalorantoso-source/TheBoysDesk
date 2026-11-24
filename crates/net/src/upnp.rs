//! Automatic UPnP (IGD) UDP port forwarding on the user's router.

use std::net::{IpAddr, SocketAddr, SocketAddrV4};
use std::time::Duration;

use igd_next::aio::tokio::search_gateway;
use igd_next::{PortMappingProtocol, SearchOptions};

use crate::{local_lan_ipv4, NetError};

/// Result of a successful UPnP port mapping.
#[derive(Debug, Clone)]
pub struct UpnpMapping {
    pub external_ip: IpAddr,
    pub external_port: u16,
}

/// Attempts to open `local_port/UDP` on the default LAN router via UPnP IGD.
pub async fn try_map_udp_port(local_port: u16) -> Result<UpnpMapping, NetError> {
    let lan_ip = local_lan_ipv4()
        .ok_or_else(|| NetError::Upnp("unable to determine local LAN IPv4".into()))?;

    let bound_opts = SearchOptions {
        bind_addr: SocketAddr::V4(SocketAddrV4::new(lan_ip, 0)),
        timeout: Some(Duration::from_secs(5)),
        ..Default::default()
    };

    let gateway = match search_gateway(bound_opts).await {
        Ok(gw) => gw,
        Err(_) => {
            let fallback_opts = SearchOptions {
                timeout: Some(Duration::from_secs(3)),
                ..Default::default()
            };
            search_gateway(fallback_opts)
                .await
                .map_err(|e| NetError::Upnp(e.to_string()))?
        }
    };

    let local_addr = SocketAddr::V4(SocketAddrV4::new(lan_ip, local_port));
    gateway
        .add_port(
            PortMappingProtocol::UDP,
            local_port,
            local_addr,
            3600,
            "TheBoysDesk P2P",
        )
        .await
        .map_err(|e| NetError::Upnp(e.to_string()))?;

    let external_ip = gateway
        .get_external_ip()
        .await
        .map_err(|e| NetError::Upnp(e.to_string()))?;

    Ok(UpnpMapping {
        external_ip,
        external_port: local_port,
    })
}
