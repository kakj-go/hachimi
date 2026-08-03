use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::time::Duration;

use crate::BrowserHostError;

pub async fn validate_agent_browser_target(
    value: &str,
    allow_private_network: bool,
) -> Result<(), BrowserHostError> {
    let url = url::Url::parse(value).map_err(|_| BrowserHostError::InvalidOrigin)?;
    if !matches!(url.scheme(), "http" | "https") || url.username() != "" || url.password().is_some()
    {
        return Err(BrowserHostError::InvalidOrigin);
    }
    let host = url.host().ok_or(BrowserHostError::InvalidOrigin)?;
    if let url::Host::Ipv4(ip) = host {
        return (!ipv4_non_public(ip) || allow_private_network)
            .then_some(())
            .ok_or(BrowserHostError::PrivateNetworkDenied);
    }
    if let url::Host::Ipv6(ip) = host {
        return (!ipv6_non_public(ip) || allow_private_network)
            .then_some(())
            .ok_or(BrowserHostError::PrivateNetworkDenied);
    }
    let url::Host::Domain(host) = host else {
        return Err(BrowserHostError::InvalidOrigin);
    };
    if host.eq_ignore_ascii_case("localhost") || host.to_ascii_lowercase().ends_with(".localhost") {
        return allow_private_network
            .then_some(())
            .ok_or(BrowserHostError::PrivateNetworkDenied);
    }
    let port = url
        .port_or_known_default()
        .ok_or(BrowserHostError::InvalidOrigin)?;
    let resolved = tokio::time::timeout(
        Duration::from_secs(5),
        tokio::net::lookup_host((host, port)),
    )
    .await
    .map_err(|_| BrowserHostError::NetworkResolutionDenied)?
    .map_err(|_| BrowserHostError::NetworkResolutionDenied)?
    .map(|address| address.ip())
    .collect::<Vec<_>>();
    if resolved.is_empty() {
        return Err(BrowserHostError::NetworkResolutionDenied);
    }
    if !allow_private_network && resolved.into_iter().any(is_non_public) {
        return Err(BrowserHostError::PrivateNetworkDenied);
    }
    Ok(())
}

fn is_non_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => ipv4_non_public(ip),
        IpAddr::V6(ip) => ipv6_non_public(ip),
    }
}

fn ipv4_non_public(ip: Ipv4Addr) -> bool {
    let octets = ip.octets();
    ip.is_private()
        || ip.is_loopback()
        || ip.is_link_local()
        || ip.is_broadcast()
        || ip.is_documentation()
        || ip.is_unspecified()
        || ip.is_multicast()
        || octets[0] == 0
        || (octets[0] == 100 && (64..=127).contains(&octets[1]))
        || (octets[0] == 198 && matches!(octets[1], 18 | 19))
        || octets[0] >= 240
}

fn ipv6_non_public(ip: Ipv6Addr) -> bool {
    let segments = ip.segments();
    ip.is_loopback()
        || ip.is_unspecified()
        || ip.is_multicast()
        || (segments[0] & 0xfe00) == 0xfc00
        || (segments[0] & 0xffc0) == 0xfe80
        || (segments[0] == 0x2001 && segments[1] == 0x0db8)
        || ip.to_ipv4_mapped().is_some_and(ipv4_non_public)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn direct_private_targets_require_an_explicit_private_grant() {
        for url in [
            "http://localhost/",
            "http://127.0.0.1/",
            "http://10.0.0.1/",
            "http://100.64.0.1/",
            "http://169.254.1.1/",
            "http://[::1]/",
            "http://[fd00::1]/",
        ] {
            assert_eq!(
                validate_agent_browser_target(url, false).await,
                Err(BrowserHostError::PrivateNetworkDenied),
                "{url}"
            );
            assert!(
                validate_agent_browser_target(url, true).await.is_ok(),
                "{url}"
            );
        }
    }

    #[tokio::test]
    async fn public_ip_literals_do_not_require_a_private_grant() {
        assert!(
            validate_agent_browser_target("https://1.1.1.1/", false)
                .await
                .is_ok()
        );
    }
}
