// SPDX-License-Identifier: Apache-2.0

use std::{
    fmt,
    net::{IpAddr, SocketAddr},
    str::FromStr,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Endpoint {
    host: String,
    port: u16,
}

impl Endpoint {
    #[must_use]
    pub fn new(host: impl Into<String>, port: u16) -> Self {
        Self {
            host: host.into(),
            port,
        }
    }

    #[must_use]
    pub fn host(&self) -> &str {
        &self.host
    }

    #[must_use]
    pub const fn port(&self) -> u16 {
        self.port
    }

    /// Validate an endpoint created programmatically.
    ///
    /// CLI parsing applies the same validation automatically. DNS names must be
    /// ASCII hostnames (internationalized names must use their punycode form),
    /// while IPv6 literals must use bracketed `host:port` syntax when parsed.
    pub(crate) fn validate(&self) -> Result<(), EndpointParseError> {
        if self.port == 0 {
            return Err(EndpointParseError("endpoint port must be non-zero"));
        }
        validate_host(&self.host)
    }

    /// Returns whether two endpoint spellings identify the same obvious target.
    ///
    /// This handles exact IPs, loopback aliases, DNS case, and a trailing DNS
    /// root dot. It deliberately does not perform DNS resolution.
    #[must_use]
    pub fn same_target(&self, other: &Self) -> bool {
        if self.port != other.port {
            return false;
        }

        let left = self.normalized_host();
        let right = other.normalized_host();
        match (left.parse::<IpAddr>(), right.parse::<IpAddr>()) {
            (Ok(left_ip), Ok(right_ip)) => {
                left_ip == right_ip || (left_ip.is_loopback() && right_ip.is_loopback())
            }
            (Ok(ip), Err(_)) => right.eq_ignore_ascii_case("localhost") && ip.is_loopback(),
            (Err(_), Ok(ip)) => left.eq_ignore_ascii_case("localhost") && ip.is_loopback(),
            (Err(_), Err(_)) => left.eq_ignore_ascii_case(right),
        }
    }

    /// Returns whether this endpoint obviously targets the client listener.
    ///
    /// Only literal IPs and the `localhost` alias are evaluated. The check is a
    /// fail-closed guardrail against common loops, not DNS authentication.
    #[must_use]
    pub fn could_target_listener(&self, listener: SocketAddr) -> bool {
        if self.port != listener.port() {
            return false;
        }

        let host = self.normalized_host();
        if host.eq_ignore_ascii_case("localhost") {
            return listener.ip().is_loopback() || listener.ip().is_unspecified();
        }

        let Ok(endpoint_ip) = host.parse::<IpAddr>() else {
            return false;
        };
        endpoint_ip == listener.ip()
            || (listener.ip().is_unspecified()
                && (endpoint_ip.is_loopback() || endpoint_ip.is_unspecified()))
            || (endpoint_ip.is_unspecified() && listener.ip().is_loopback())
    }

    fn normalized_host(&self) -> &str {
        self.host.trim_end_matches('.')
    }
}

impl fmt::Display for Endpoint {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.host.contains(':') {
            write!(formatter, "[{}]:{}", self.host, self.port)
        } else {
            write!(formatter, "{}:{}", self.host, self.port)
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EndpointParseError(&'static str);

impl fmt::Display for EndpointParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.0)
    }
}

impl std::error::Error for EndpointParseError {}

impl FromStr for Endpoint {
    type Err = EndpointParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value.trim() != value || value.is_empty() {
            return Err(EndpointParseError("endpoint must not be empty or padded"));
        }

        if let Ok(socket) = value.parse::<SocketAddr>() {
            let endpoint = Self::new(socket.ip().to_string(), socket.port());
            endpoint.validate()?;
            return Ok(endpoint);
        }

        let (host, port) = value
            .rsplit_once(':')
            .ok_or(EndpointParseError("endpoint must use host:port syntax"))?;
        let port = port
            .parse::<u16>()
            .map_err(|_| EndpointParseError("endpoint port is invalid"))?;
        let endpoint = Self::new(host, port);
        endpoint.validate()?;
        Ok(endpoint)
    }
}

fn validate_host(host: &str) -> Result<(), EndpointParseError> {
    if host.is_empty() || host.chars().any(char::is_whitespace) {
        return Err(EndpointParseError("endpoint host is invalid"));
    }
    if host.contains("://")
        || host.contains('@')
        || host.contains('/')
        || host.contains('?')
        || host.contains('#')
    {
        return Err(EndpointParseError(
            "endpoint must use host:port without scheme, credentials, or path",
        ));
    }
    if host.parse::<IpAddr>().is_ok() {
        return Ok(());
    }
    if !host.is_ascii() {
        return Err(EndpointParseError(
            "endpoint DNS host must use ASCII or punycode",
        ));
    }
    if host.contains(':') {
        return Err(EndpointParseError(
            "IPv6 endpoints must use bracketed host:port syntax",
        ));
    }

    let dns_name = host.strip_suffix('.').unwrap_or(host);
    if dns_name.is_empty() || dns_name.len() > 253 {
        return Err(EndpointParseError("endpoint DNS host is invalid"));
    }
    for label in dns_name.split('.') {
        if label.is_empty()
            || label.len() > 63
            || label.starts_with('-')
            || label.ends_with('-')
            || !label
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        {
            return Err(EndpointParseError("endpoint DNS host is invalid"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};

    use super::Endpoint;

    #[test]
    fn parses_dns_ipv6_onion_and_punycode_endpoints() {
        let dns: Endpoint = "relay.example:50001".parse().expect("valid endpoint");
        assert_eq!(dns.host(), "relay.example");
        assert_eq!(dns.port(), 50_001);

        let ipv6: Endpoint = "[::1]:50001".parse().expect("valid IPv6 endpoint");
        assert_eq!(ipv6.host(), "::1");
        assert_eq!(ipv6.to_string(), "[::1]:50001");

        assert!(
            "examplehiddenservice.onion:50001"
                .parse::<Endpoint>()
                .is_ok()
        );
        assert!("xn--bcher-kva.example.:50001".parse::<Endpoint>().is_ok());
    }

    #[test]
    fn rejects_ambiguous_or_zero_port_endpoints() {
        assert!("relay.example".parse::<Endpoint>().is_err());
        assert!("relay.example:0".parse::<Endpoint>().is_err());
        assert!("127.0.0.1:0".parse::<Endpoint>().is_err());
        assert!("[::1]:0".parse::<Endpoint>().is_err());
        assert!(" relay.example:50001".parse::<Endpoint>().is_err());
        assert!("::1:50001".parse::<Endpoint>().is_err());
    }

    #[test]
    fn rejects_urls_userinfo_paths_unicode_and_malformed_dns() {
        for endpoint in [
            "https://relay.example:50001",
            "user@relay.example:50001",
            "relay.example/path:50001",
            "[relay.example]:50001",
            ".relay.example:50001",
            "relay..example:50001",
            "-relay.example:50001",
            "relay-.example:50001",
            "relay_example:50001",
            "bücher.example:50001",
        ] {
            assert!(endpoint.parse::<Endpoint>().is_err(), "accepted {endpoint}");
        }
    }

    #[test]
    fn recognizes_obvious_target_aliases_without_dns() {
        let dns = Endpoint::new("Relay.Example.", 50_001);
        assert!(dns.same_target(&Endpoint::new("relay.example", 50_001)));
        assert!(
            Endpoint::new("localhost", 50_001).same_target(&Endpoint::new("127.0.0.1", 50_001))
        );
        assert!(Endpoint::new("127.0.0.2", 50_001).same_target(&Endpoint::new("::1", 50_001)));
        assert!(!dns.same_target(&Endpoint::new("relay.example", 50_002)));
        assert!(!dns.same_target(&Endpoint::new("other.example", 50_001)));
    }

    #[test]
    fn detects_obvious_listener_loops() {
        let loopback = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 50_003);
        let wildcard = SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 50_003);

        assert!(Endpoint::new("localhost", 50_003).could_target_listener(loopback));
        assert!(Endpoint::new("127.0.0.1", 50_003).could_target_listener(loopback));
        assert!(Endpoint::new("127.0.0.1", 50_003).could_target_listener(wildcard));
        assert!(!Endpoint::new("relay.example", 50_003).could_target_listener(loopback));
        assert!(!Endpoint::new("127.0.0.1", 50_001).could_target_listener(loopback));
    }
}
