# Changelog

All notable changes to this project will be documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project intends
to use [Semantic Versioning](https://semver.org/spec/v2.0.0.html) after its first
stable release.

## [Unreleased]

### Added

- Transparent Electrum query proxy.
- Fail-closed broadcast interception.
- Reject-by-default and SOCKS5 Electrum relay adapters.
- Bounded frames and connections.
- Bounded request identifiers and per-connection outstanding-request window
  with fail-closed overflow.
- Process-wide fail-closed broadcast concurrency and aggregate input-payload
  limits without overload queueing or relay fallback.
- Supervised, bounded shutdown for every accepted connection task: new relay
  admission stops first, active calls may flush a correlated wallet response,
  idle connections close promptly, and deadline expiry aborts without fallback.
- Outstanding-request correlation to block query-upstream response spoofing.
- Offline `--check-config` validation and obvious listener-loop/query-relay
  separation guardrails.
- Strict endpoint syntax and non-zero-port validation for listener, query,
  relay, and SOCKS configuration.
- Source-derived Electrum, Sparrow, and BlueWallet wire-profile tests.
- Docker-backed Bitcoin Core 30.2 regtest broadcast gate.
- Scheduled and manually runnable real Tor v3 onion smoke test.
- Coverage-guided fuzz target and CI campaigns for wallet frame classification.
- Native Linux, macOS, and Windows release-candidate double builds with raw
  executable comparison, deterministic ZIPs, SHA-256 sidecars, packaged CLI
  checks, and main-branch SLSA provenance attestations.
- Create-once release-candidate publication with per-target locking, exclusive
  ZIP/sidecar creation, and rollback of partial evidence on interrupted copies.
- A frozen first stable-release audit scope covering implementation, evidence,
  supply chain, deployment claims, and reviewer deliverables.
- Initial architecture, threat model, Tor deployment, and contributor guidance.
