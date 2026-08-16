// SPDX-License-Identifier: Apache-2.0

use std::{fmt, net::SocketAddr, time::Duration};

use clap::{Parser, ValueEnum};

use crate::endpoint::Endpoint;

pub const DEFAULT_MAX_FRAME_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_CONFIGURABLE_FRAME_BYTES: usize = 16 * 1024 * 1024;
pub const DEFAULT_MAX_CONNECTIONS: usize = 128;
pub const MAX_CONFIGURABLE_CONNECTIONS: usize = 16 * 1024;
pub const DEFAULT_MAX_PENDING_REQUESTS: usize = 1_024;
pub const MAX_CONFIGURABLE_PENDING_REQUESTS: usize = 16 * 1024;

/// A validated per-connection limit for response-bearing requests.
///
/// The inner value is private so library users cannot construct a [`Config`]
/// that bypasses the same hard ceiling enforced by the command-line parser.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PendingRequestLimit(usize);

impl PendingRequestLimit {
    #[must_use]
    pub const fn get(self) -> usize {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum RelayMode {
    Reject,
    SocksElectrum,
}

#[derive(Debug, Parser)]
#[command(author, version, about)]
pub struct Cli {
    /// Validate configuration and exit before binding or connecting.
    #[arg(long, env = "EPR_CHECK_CONFIG", default_value_t = false)]
    pub check_config: bool,

    /// Client-facing Electrum TCP listener.
    #[arg(long, env = "EPR_LISTEN", default_value = "127.0.0.1:50003")]
    pub listen: SocketAddr,

    /// Read-only Electrum upstream used for all non-broadcast requests.
    #[arg(long, env = "EPR_UPSTREAM", default_value = "127.0.0.1:50001")]
    pub upstream: Endpoint,

    /// Private broadcast adapter. The safe default rejects every broadcast.
    #[arg(long, env = "EPR_RELAY_MODE", value_enum, default_value = "reject")]
    pub relay_mode: RelayMode,

    /// Electrum relay reached through SOCKS5; required by socks-electrum mode.
    #[arg(long, env = "EPR_RELAY_ENDPOINT")]
    pub relay_endpoint: Option<Endpoint>,

    /// SOCKS5 proxy used by the private relay adapter.
    #[arg(long, env = "EPR_SOCKS5_PROXY", default_value = "127.0.0.1:9050")]
    pub socks5_proxy: SocketAddr,

    /// Maximum concurrent wallet connections.
    #[arg(long, env = "EPR_MAX_CONNECTIONS", default_value_t = DEFAULT_MAX_CONNECTIONS)]
    pub max_connections: usize,

    /// Maximum response-bearing requests awaiting a reply per wallet connection.
    #[arg(
        long,
        env = "EPR_MAX_PENDING_REQUESTS",
        default_value_t = DEFAULT_MAX_PENDING_REQUESTS
    )]
    pub max_pending_requests: usize,

    /// Maximum newline-delimited Electrum frame size.
    #[arg(
        long,
        env = "EPR_MAX_FRAME_BYTES",
        default_value_t = DEFAULT_MAX_FRAME_BYTES
    )]
    pub max_frame_bytes: usize,

    /// Connection timeout in seconds.
    #[arg(long, env = "EPR_CONNECT_TIMEOUT_SECONDS", default_value_t = 10)]
    pub connect_timeout_seconds: u64,

    /// Private relay response timeout in seconds.
    #[arg(long, env = "EPR_RELAY_TIMEOUT_SECONDS", default_value_t = 30)]
    pub relay_timeout_seconds: u64,

    /// Required to bind the application directly to a non-loopback address.
    #[arg(long, env = "EPR_ALLOW_NON_LOOPBACK_LISTEN", default_value_t = false)]
    pub allow_non_loopback_listen: bool,
}

#[derive(Clone, Debug)]
pub struct Config {
    pub listen: SocketAddr,
    pub upstream: Endpoint,
    pub relay_mode: RelayMode,
    pub relay_endpoint: Option<Endpoint>,
    pub socks5_proxy: SocketAddr,
    pub max_connections: usize,
    pub max_pending_requests: PendingRequestLimit,
    pub max_frame_bytes: usize,
    pub connect_timeout: Duration,
    pub relay_timeout: Duration,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConfigError(String);

impl fmt::Display for ConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for ConfigError {}

impl TryFrom<usize> for PendingRequestLimit {
    type Error = ConfigError;

    fn try_from(value: usize) -> Result<Self, Self::Error> {
        if !(1..=MAX_CONFIGURABLE_PENDING_REQUESTS).contains(&value) {
            return Err(ConfigError(format!(
                "max pending requests must be between 1 and {MAX_CONFIGURABLE_PENDING_REQUESTS}"
            )));
        }
        Ok(Self(value))
    }
}

impl TryFrom<Cli> for Config {
    type Error = ConfigError;

    fn try_from(cli: Cli) -> Result<Self, Self::Error> {
        if !cli.listen.ip().is_loopback() && !cli.allow_non_loopback_listen {
            return Err(ConfigError(
                "refusing a non-loopback listener without --allow-non-loopback-listen".into(),
            ));
        }
        if cli.upstream.could_target_listener(cli.listen) {
            return Err(ConfigError(
                "query upstream must not target the client listener".into(),
            ));
        }
        if !(1..=MAX_CONFIGURABLE_CONNECTIONS).contains(&cli.max_connections) {
            return Err(ConfigError(format!(
                "max connections must be between 1 and {MAX_CONFIGURABLE_CONNECTIONS}"
            )));
        }
        let max_pending_requests = PendingRequestLimit::try_from(cli.max_pending_requests)?;
        if !(1_024..=MAX_CONFIGURABLE_FRAME_BYTES).contains(&cli.max_frame_bytes) {
            return Err(ConfigError(format!(
                "max frame bytes must be between 1024 and {MAX_CONFIGURABLE_FRAME_BYTES}"
            )));
        }
        if cli.connect_timeout_seconds == 0 || cli.relay_timeout_seconds == 0 {
            return Err(ConfigError("timeouts must be greater than zero".into()));
        }

        if cli.relay_mode == RelayMode::SocksElectrum {
            let relay_endpoint = cli.relay_endpoint.as_ref().ok_or_else(|| {
                ConfigError("socks-electrum mode requires --relay-endpoint".into())
            })?;
            if relay_endpoint.same_target(&cli.upstream) {
                return Err(ConfigError(
                    "private relay endpoint must differ from the query upstream".into(),
                ));
            }
            if relay_endpoint.could_target_listener(cli.listen) {
                return Err(ConfigError(
                    "private relay endpoint must not target the client listener".into(),
                ));
            }
            let socks_endpoint =
                Endpoint::new(cli.socks5_proxy.ip().to_string(), cli.socks5_proxy.port());
            if socks_endpoint.could_target_listener(cli.listen) {
                return Err(ConfigError(
                    "SOCKS5 proxy must not target the client listener".into(),
                ));
            }
        }

        Ok(Self {
            listen: cli.listen,
            upstream: cli.upstream,
            relay_mode: cli.relay_mode,
            relay_endpoint: cli.relay_endpoint,
            socks5_proxy: cli.socks5_proxy,
            max_connections: cli.max_connections,
            max_pending_requests,
            max_frame_bytes: cli.max_frame_bytes,
            connect_timeout: Duration::from_secs(cli.connect_timeout_seconds),
            relay_timeout: Duration::from_secs(cli.relay_timeout_seconds),
        })
    }
}

#[cfg(test)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};

    use super::{
        Cli, Config, DEFAULT_MAX_CONNECTIONS, DEFAULT_MAX_FRAME_BYTES,
        DEFAULT_MAX_PENDING_REQUESTS, MAX_CONFIGURABLE_CONNECTIONS,
        MAX_CONFIGURABLE_PENDING_REQUESTS, PendingRequestLimit, RelayMode,
    };
    use crate::endpoint::Endpoint;

    fn cli() -> Cli {
        Cli {
            check_config: false,
            listen: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 50_003),
            upstream: Endpoint::new("127.0.0.1", 50_001),
            relay_mode: RelayMode::Reject,
            relay_endpoint: None,
            socks5_proxy: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 9_050),
            max_connections: DEFAULT_MAX_CONNECTIONS,
            max_pending_requests: DEFAULT_MAX_PENDING_REQUESTS,
            max_frame_bytes: DEFAULT_MAX_FRAME_BYTES,
            connect_timeout_seconds: 10,
            relay_timeout_seconds: 30,
            allow_non_loopback_listen: false,
        }
    }

    #[test]
    fn rejects_non_loopback_by_default() {
        let mut input = cli();
        input.listen = SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 50_003);
        assert!(Config::try_from(input).is_err());
    }

    #[test]
    fn rejects_query_upstream_listener_loop() {
        let mut input = cli();
        input.upstream = Endpoint::new("localhost", input.listen.port());
        assert!(Config::try_from(input).is_err());
    }

    #[test]
    fn requires_relay_endpoint_for_socks_mode() {
        let mut input = cli();
        input.relay_mode = RelayMode::SocksElectrum;
        assert!(Config::try_from(input).is_err());
    }

    #[test]
    fn requires_query_and_relay_endpoint_separation() {
        let mut input = cli();
        input.relay_mode = RelayMode::SocksElectrum;
        input.relay_endpoint = Some(Endpoint::new("localhost", 50_001));
        assert!(Config::try_from(input).is_err());
    }

    #[test]
    fn rejects_relay_or_socks_listener_loops() {
        let mut relay_loop = cli();
        relay_loop.relay_mode = RelayMode::SocksElectrum;
        relay_loop.relay_endpoint = Some(Endpoint::new("127.0.0.1", 50_003));
        assert!(Config::try_from(relay_loop).is_err());

        let mut socks_loop = cli();
        socks_loop.relay_mode = RelayMode::SocksElectrum;
        socks_loop.relay_endpoint = Some(Endpoint::new("relay.example", 50_001));
        socks_loop.socks5_proxy = socks_loop.listen;
        assert!(Config::try_from(socks_loop).is_err());
    }

    #[test]
    fn accepts_distinct_socks_relay_configuration() {
        let mut input = cli();
        input.relay_mode = RelayMode::SocksElectrum;
        input.relay_endpoint = Some(Endpoint::new("relay.example", 50_001));
        assert!(Config::try_from(input).is_ok());
    }

    #[test]
    fn bounds_connection_and_pending_request_configuration() {
        let mut zero_connections = cli();
        zero_connections.max_connections = 0;
        assert!(Config::try_from(zero_connections).is_err());

        let mut excessive_connections = cli();
        excessive_connections.max_connections = MAX_CONFIGURABLE_CONNECTIONS + 1;
        assert!(Config::try_from(excessive_connections).is_err());

        let mut zero_pending = cli();
        zero_pending.max_pending_requests = 0;
        assert!(Config::try_from(zero_pending).is_err());

        let mut excessive_pending = cli();
        excessive_pending.max_pending_requests = MAX_CONFIGURABLE_PENDING_REQUESTS + 1;
        assert!(Config::try_from(excessive_pending).is_err());
    }

    #[test]
    fn pending_request_limit_cannot_exceed_the_hard_ceiling() {
        assert!(PendingRequestLimit::try_from(0).is_err());
        assert_eq!(
            PendingRequestLimit::try_from(DEFAULT_MAX_PENDING_REQUESTS)
                .expect("default pending request limit")
                .get(),
            DEFAULT_MAX_PENDING_REQUESTS
        );
        assert!(PendingRequestLimit::try_from(MAX_CONFIGURABLE_PENDING_REQUESTS + 1).is_err());
    }
}
