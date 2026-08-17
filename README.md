# electrum-private-relay

[![CI](https://github.com/metaforismo/electrum-private-relay/actions/workflows/ci.yml/badge.svg)](https://github.com/metaforismo/electrum-private-relay/actions/workflows/ci.yml)
[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)

An experimental, self-hosted Electrum protocol proxy that separates wallet
queries from transaction broadcast and routes signed transactions through one
explicitly selected private relay.

> [!WARNING]
> This project is early-stage software. It has not been independently audited
> or tested with real funds. Use regtest or testnet while the compatibility and
> security milestones remain open.

## Why

Most wallets can select an Electrum server but cannot select a private
broadcast transport. This project aims to make private broadcast available to
ordinary Electrum-compatible wallets without requiring wallet-specific code or
command-line transaction handling.

The proxy forwards normal Electrum traffic to a configured upstream. It
intercepts `blockchain.transaction.broadcast` and sends the raw signed
transaction to exactly one configured adapter. It never silently falls back to
the read upstream or fans out to several relays.

```text
wallet -- Electrum TCP --> proxy -- read/query methods --> Electrum upstream
                              |
                              +-- broadcast method --> selected private relay
```

## Privacy properties

- Listens on `127.0.0.1` by default and requires an explicit unsafe override for
  a non-loopback bind.
- Defaults to `reject`, so broadcast fails until a private relay is configured.
- Does not persist raw transactions, transaction IDs, wallet queries, addresses,
  IP addresses, or connection metadata.
- Uses bounded newline-delimited frames, a bounded connection pool, and a
  bounded per-connection window of response-bearing requests.
- Limits simultaneous relay submissions across the entire process and rejects
  overload immediately without a queue or a call to the configured relay.
- On graceful shutdown, atomically rejects new relay submissions and waits a
  bounded interval for already admitted calls to return before runtime teardown.
- Correlates upstream response IDs and drops unsolicited responses or responses
  that collide with an intercepted broadcast.
- Rejects malformed or batched client requests rather than risk bypassing the
  broadcast interceptor.
- Keeps private relay failures fail-closed: the wallet receives an error and no
  public fallback is attempted.
- Rejects obvious configuration loops into the client listener and obvious
  query/relay endpoint reuse before any socket is opened.
- Rejects zero ports, URL/userinfo/path syntax, unbracketed IPv6 literals,
  non-ASCII DNS names, and malformed DNS labels before network setup.

Endpoint arguments accept only `host:port` syntax. IPv6 literals must use
`[address]:port`; DNS names must be ASCII (use punycode for internationalized
names), and a final DNS root dot is allowed. Onion names use the same DNS-style
syntax. Ports must be in `1..=65535`. Do not pass schemes such as `tcp://` or
`ssl://`, credentials, query strings, or paths.

The endpoint guardrails compare literal IPs, loopback aliases, DNS case, and a
trailing root dot. They intentionally do not resolve DNS and therefore do not
prevent DNS rebinding, split-horizon aliases, or every operator error. Review
the resolved deployment topology independently.

These properties do **not** make upstream wallet queries private. The configured
Electrum upstream still observes the wallet's query set. Use a self-hosted
Electrum server when that privacy boundary matters.

## Current adapter

`socks-electrum` submits a broadcast request to a separate Electrum endpoint
through a SOCKS5 proxy such as Tor. The default SOCKS address is
`127.0.0.1:9050`.

Tor protects the network path; it does not prove that the destination is a
private miner relay, does not remove transaction-level fingerprints, and does
not prevent a relay account or client code from linking submissions.

MARA Slipstream is intentionally only a planned adapter. Its current API is
beta and requires a client code, so the core proxy does not depend on it.

## Build and test

Requirements: Rust 1.96.0 and Cargo.

```bash
cargo fmt --all -- --check
cargo test --locked --all-targets
cargo clippy --locked --all-targets -- -D warnings
```

The Rust suite includes black-box CLI checks proving that configuration-only
validation exits without binding or connecting and that unsafe loops, endpoint
reuse, zero ports, ambiguous endpoint syntax, and invalid resource limits fail
closed. Controlled asynchronous tests cover relay saturation and the shutdown
lifecycle, including a real TCP path where a broadcast already in progress
finishes and its txid still reaches the wallet after the listener stops.

The pull-request integration gate also replays source-derived Electrum,
Sparrow, and BlueWallet protocol profiles and broadcasts a real signed regtest
transaction into a disposable Bitcoin Core mempool. A separate scheduled smoke
routes a broadcast through a real ephemeral v3 onion, and the parser has a
coverage-guided fuzz target with PR smoke and weekly campaigns. See
[integration testing](docs/testing.md) for exact commands and claim boundaries.

## Run safely

Validate the complete CLI/environment configuration without opening a listener,
connecting to the query upstream, or contacting the SOCKS proxy:

```bash
cargo run --locked -- \
  --check-config \
  --upstream 127.0.0.1:50001
```

The default runtime starts on loopback and rejects broadcasts:

```bash
cargo run --locked -- \
  --upstream 127.0.0.1:50001
```

The default connection pool is 128 and the hard configuration ceiling is
16,384. Each wallet connection may have at most 1,024 response-bearing requests
in flight by default. Operators can lower this with `--max-pending-requests`;
the hard configuration ceiling is 16,384. String request IDs are limited to 256
UTF-8 bytes; numeric IDs must be integers representable as signed or unsigned
64-bit values. Reaching a per-connection request limit or using an ambiguous ID
fails closed and closes that wallet connection after a generic error.

At most eight broadcasts may be inside a private relay submission at once by
default. Configure this with `--max-concurrent-broadcasts` or
`EPR_MAX_CONCURRENT_BROADCASTS`. The numeric hard ceiling is 1,024, but the
product of that value and `--max-frame-bytes` may never exceed 128 MiB. With the
default 2 MiB frame limit, at most 64 slots can be configured; with the 16 MiB
maximum frame size, at most eight can be configured. These products bound input
payload volume rather than promising an exact process-RSS ceiling.

There is deliberately no overload queue. When every process-wide broadcast slot
is occupied, the new request receives the existing generic private-relay error,
the selected adapter is not called, the query upstream sees no transaction, and
no fallback is attempted. The wallet connection remains available for later
requests.

`Ctrl-C` begins a bounded two-phase shutdown. New private relay submissions are
rejected first, then the listener stops accepting connections. A relay call
already admitted may run for at most the configured relay timeout, followed by
one second of response-flush headroom. If that interval expires, remaining work
is cancelled without retry or fallback. This improves orderly operator shutdown,
but it cannot guarantee the wallet receives a result after `SIGKILL`, power
loss, a process crash, or a provider that accepted the transaction and never
returned a valid response. Treat those cases as an unknown outcome rather than
blindly rebroadcasting over a public path.

To use a separate Electrum relay over Tor SOCKS5:

```bash
cargo run --locked -- \
  --upstream 127.0.0.1:50001 \
  --relay-mode socks-electrum \
  --relay-endpoint examplehiddenservice.onion:50001 \
  --socks5-proxy 127.0.0.1:9050 \
  --max-concurrent-broadcasts 4
```

The relay endpoint must be distinct from the query upstream and neither may
obviously point back to the client listener. The SOCKS proxy address must also
be distinct from that listener.

Do not expose the application port directly to the internet. For remote wallet
access, keep the loopback listener and publish it as a client-authenticated v3
onion service. See [Tor deployment](docs/tor.md).

## Scope and status

Implemented:

- transparent full-duplex forwarding for non-broadcast Electrum frames;
- strict interception of transaction broadcast requests;
- query-response ID correlation that blocks upstream broadcast spoofing;
- fail-closed relay behavior;
- SOCKS5-to-Electrum relay adapter;
- bounded connections, frames, request tables, and process-wide relay
  submissions with an aggregate payload budget;
- bounded graceful shutdown for already admitted relay calls;
- privacy-preserving operational output;
- offline configuration validation, strict endpoint syntax, and obvious
  endpoint-loop guardrails;
- unit, black-box CLI, and in-memory integration tests;
- source-derived wire profiles for Electrum, Sparrow, and BlueWallet;
- a real Bitcoin Core regtest broadcast gate;
- an opt-in and scheduled real Tor v3 onion smoke test; and
- a coverage-guided Electrum frame-classifier fuzz target.

Tracked release gates:

- an opt-in MARA Slipstream adapter after its external provider gate
  ([#4](https://github.com/metaforismo/electrum-private-relay/issues/4));
- packaged-wallet UI and transport compatibility certification
  ([#5](https://github.com/metaforismo/electrum-private-relay/issues/5)); and
- an independent review and stable-release security gate
  ([#6](https://github.com/metaforismo/electrum-private-relay/issues/6)).

Application TLS termination, public clearnet hosting, multi-user authentication,
and tenant isolation are not supported by the current self-hosted onion-service
scope. Adding any of them requires a separate threat model and security review.

See the [architecture](docs/architecture.md), [threat model](docs/threat-model.md),
and [relay adapter contract](docs/relay-adapters.md) before deploying.

## Contributing

Issues and pull requests are welcome. Start with [CONTRIBUTING.md](CONTRIBUTING.md).
Please do not disclose suspected vulnerabilities in a public issue; follow the
private reporting process in [SECURITY.md](SECURITY.md).

## License

Apache License 2.0. See [LICENSE](LICENSE).
