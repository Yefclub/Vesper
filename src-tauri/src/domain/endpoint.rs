//! Where a local OpenAI-compatible server may live.
//!
//! The product's promise is that audio and transcripts stay on the machine
//! unless the user picks a cloud provider on purpose. This provider exists so
//! somebody running Ollama or LM Studio can use the model they already host —
//! and the same field, unchecked, is a way to post every transcript to any
//! address on the internet.
//!
//! So the address is checked here, and the rule is the reachable equivalent of
//! "this machine and this network": loopback and the private ranges. Anything
//! routable is refused, which costs a user with a model on a public VPS the
//! feature and is the correct trade for a product that says what this one says.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

#[derive(Debug, PartialEq, Eq)]
pub enum EndpointError {
    Empty,
    NotHttp,
    NoHost,
    PublicAddress,
    NotAnAddress,
}

impl std::fmt::Display for EndpointError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let msg = match self {
            EndpointError::Empty => "the endpoint address is empty",
            EndpointError::NotHttp => "the endpoint must be an http:// or https:// address",
            EndpointError::NoHost => "the endpoint has no host",
            EndpointError::PublicAddress => {
                "the endpoint must be on this machine or this network — \
                 a public address would send the transcript off it"
            }
            EndpointError::NotAnAddress => {
                "the endpoint host must be an IP address or localhost, \
                 so where it resolves cannot change under you"
            }
        };
        f.write_str(msg)
    }
}

/// Accept a base URL for a local OpenAI-compatible server, or say why not.
///
/// Returns the URL with any trailing slash removed, which is what the caller
/// wants to join `/chat/completions` onto.
pub fn validate_endpoint_url(raw: &str) -> Result<String, EndpointError> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err(EndpointError::Empty);
    }
    let rest = raw
        .strip_prefix("http://")
        .or_else(|| raw.strip_prefix("https://"))
        .ok_or(EndpointError::NotHttp)?;

    // Everything before the first `/`, `?` or `#` is the authority. Credentials
    // in it are ignored on purpose: `http://evil.com@127.0.0.1` reads as
    // loopback to a parser and as `evil.com` to a careless human, so the host is
    // taken as whatever follows the last `@`.
    let authority = rest
        .split(['/', '?', '#'])
        .next()
        .unwrap_or("")
        .rsplit('@')
        .next()
        .unwrap_or("");
    if authority.is_empty() {
        return Err(EndpointError::NoHost);
    }

    let host = strip_port(authority);
    if host.is_empty() {
        return Err(EndpointError::NoHost);
    }
    if host.eq_ignore_ascii_case("localhost") {
        return Ok(raw.trim_end_matches('/').to_string());
    }

    // A name would have to be resolved to be judged, and what it resolves to can
    // change between the check and the request. An address cannot.
    let host = host.trim_start_matches('[').trim_end_matches(']');
    let ip: IpAddr = host.parse().map_err(|_| EndpointError::NotAnAddress)?;
    if is_local(ip) {
        Ok(raw.trim_end_matches('/').to_string())
    } else {
        Err(EndpointError::PublicAddress)
    }
}

/// The host without its port, leaving a bracketed IPv6 literal intact.
fn strip_port(authority: &str) -> &str {
    if let Some(end) = authority.rfind(']') {
        return &authority[..=end];
    }
    match authority.rsplit_once(':') {
        Some((host, _)) => host,
        None => authority,
    }
}

/// This machine, or the network it is on.
fn is_local(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                // `0.0.0.0` is how a server says "every interface"; reaching it
                // is reaching this machine.
                || v4 == Ipv4Addr::UNSPECIFIED
        }
        IpAddr::V6(v6) => {
            v6.is_loopback()
                || v6 == Ipv6Addr::UNSPECIFIED
                // Unique-local (fc00::/7) and link-local (fe80::/10). Neither is
                // stable in `std` yet, so the prefixes are read directly.
                || (v6.segments()[0] & 0xfe00) == 0xfc00
                || (v6.segments()[0] & 0xffc0) == 0xfe80
                // An IPv4 address wearing an IPv6 hat is still that address.
                || v6.to_ipv4_mapped().map(|v4| is_local(IpAddr::V4(v4))).unwrap_or(false)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_two_servers_this_is_for_are_accepted() {
        // Ollama and LM Studio, as their own documentation writes them.
        assert_eq!(
            validate_endpoint_url("http://localhost:11434/v1"),
            Ok("http://localhost:11434/v1".into())
        );
        assert_eq!(
            validate_endpoint_url("http://127.0.0.1:1234/v1"),
            Ok("http://127.0.0.1:1234/v1".into())
        );
    }

    #[test]
    fn a_machine_on_the_same_network_is_accepted() {
        assert!(validate_endpoint_url("http://192.168.1.50:11434/v1").is_ok());
        assert!(validate_endpoint_url("http://10.0.0.4:8000/v1").is_ok());
        assert!(validate_endpoint_url("http://172.16.3.9:8000/v1").is_ok());
        assert!(validate_endpoint_url("http://[::1]:11434/v1").is_ok());
    }

    #[test]
    fn a_public_address_is_refused() {
        assert_eq!(
            validate_endpoint_url("https://8.8.8.8/v1"),
            Err(EndpointError::PublicAddress)
        );
        assert_eq!(
            validate_endpoint_url("http://172.32.0.1/v1"),
            Err(EndpointError::PublicAddress),
            "172.32 is outside the private block that ends at 172.31"
        );
    }

    /// A name is refused rather than resolved: what it points at can change
    /// between the check and the request, and `localhost.evil.com` is a name
    /// somebody owns.
    #[test]
    fn a_hostname_is_refused() {
        assert_eq!(
            validate_endpoint_url("http://my-server.local:11434"),
            Err(EndpointError::NotAnAddress)
        );
        assert_eq!(
            validate_endpoint_url("http://localhost.evil.com/v1"),
            Err(EndpointError::NotAnAddress)
        );
    }

    /// `http://evil.com@127.0.0.1/` reads as loopback to a parser and as
    /// `evil.com` to a person skimming it. The host is what follows the last
    /// `@`, so both readings agree.
    #[test]
    fn credentials_do_not_disguise_the_host() {
        assert!(validate_endpoint_url("http://evil.com@127.0.0.1:11434/v1").is_ok());
        assert_eq!(
            validate_endpoint_url("http://127.0.0.1@8.8.8.8/v1"),
            Err(EndpointError::PublicAddress)
        );
    }

    #[test]
    fn only_http_schemes_are_accepted() {
        assert_eq!(
            validate_endpoint_url("file:///etc/passwd"),
            Err(EndpointError::NotHttp)
        );
        assert_eq!(
            validate_endpoint_url("127.0.0.1:11434"),
            Err(EndpointError::NotHttp)
        );
        assert_eq!(validate_endpoint_url("  "), Err(EndpointError::Empty));
    }

    #[test]
    fn a_trailing_slash_is_dropped_so_paths_join_cleanly() {
        assert_eq!(
            validate_endpoint_url("http://localhost:11434/v1/"),
            Ok("http://localhost:11434/v1".into())
        );
    }
}
