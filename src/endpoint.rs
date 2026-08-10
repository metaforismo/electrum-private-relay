// SPDX-License-Identifier: Apache-2.0

use std::{fmt, net::SocketAddr, str::FromStr};

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
            return Ok(Self::new(socket.ip().to_string(), socket.port()));
        }

        let (host, port) = value
            .rsplit_once(':')
            .ok_or(EndpointParseError("endpoint must use host:port syntax"))?;
        let host = host.trim_matches(['[', ']']);
        if host.is_empty() || host.chars().any(char::is_whitespace) {
            return Err(EndpointParseError("endpoint host is invalid"));
        }

        let port = port
            .parse::<u16>()
            .map_err(|_| EndpointParseError("endpoint port is invalid"))?;
        if port == 0 {
            return Err(EndpointParseError("endpoint port must be non-zero"));
        }

        Ok(Self::new(host, port))
    }
}

#[cfg(test)]
mod tests {
    use super::Endpoint;

    #[test]
    fn parses_dns_and_ipv6_endpoints() {
        let dns: Endpoint = "relay.example:50001".parse().expect("valid endpoint");
        assert_eq!(dns.host(), "relay.example");
        assert_eq!(dns.port(), 50_001);

        let ipv6: Endpoint = "[::1]:50001".parse().expect("valid IPv6 endpoint");
        assert_eq!(ipv6.host(), "::1");
        assert_eq!(ipv6.to_string(), "[::1]:50001");
    }

    #[test]
    fn rejects_ambiguous_or_zero_port_endpoints() {
        assert!("relay.example".parse::<Endpoint>().is_err());
        assert!("relay.example:0".parse::<Endpoint>().is_err());
        assert!(" relay.example:50001".parse::<Endpoint>().is_err());
    }
}
