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
- Correlates upstream response IDs and drops unsolicited responses or responses
  that collide with an intercepted broadcast.
- Rejects malformed or batched client requests rather than risk bypassing the
  broadcast interceptor.
- Keeps private relay failures fail-closed: the wallet receives an error and no
  public fallback is attempted.

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

## Run safely

The default starts on loopback and rejects broadcasts:

```bash
cargo run --locked -- \
  --upstream 127.0.0.1:50001
```

Each wallet connection may have at most 1,024 response-bearing requests in
flight by default. Operators can lower this with `--max-pending-requests`; the
hard configuration ceiling is 16,384. String request IDs are limited to 256
UTF-8 bytes. Reaching either boundary fails closed and closes that wallet
connection after a generic error.

To use a separate Electrum relay over Tor SOCKS5:

```bash
cargo run --locked -- \
  --upstream 127.0.0.1:50001 \
  --relay-mode socks-electrum \
  --relay-endpoint examplehiddenservice.onion:50001 \
  --socks5-proxy 127.0.0.1:9050
```

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
- bounded resources and privacy-preserving operational output;
- unit and in-memory integration tests.

Not yet implemented or claimed:

- a MARA Slipstream adapter;
- TLS termination in the application;
- wallet-specific compatibility certification;
- multi-user authentication or tenant isolation;
- production readiness or an independent security audit.

See the [architecture](docs/architecture.md), [threat model](docs/threat-model.md),
and [relay adapter contract](docs/relay-adapters.md) before deploying.

## Contributing

Issues and pull requests are welcome. Start with [CONTRIBUTING.md](CONTRIBUTING.md).
Please do not disclose suspected vulnerabilities in a public issue; follow the
private reporting process in `SECURITY.md` once it is published.

## License

Apache License 2.0. See [LICENSE](LICENSE).
