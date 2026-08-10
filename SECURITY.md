# Security Policy

## Supported Versions

This project is experimental and has no stable release. Security fixes are
applied only to the latest commit on `main` until a versioned support policy is
published. Do not use the software with real funds unless you have independently
reviewed the code and accepted the documented limitations.

## Reporting a Vulnerability

Do not open a public issue. Use GitHub's private vulnerability reporting:

https://github.com/metaforismo/electrum-private-relay/security/advisories/new

Include the affected commit, deployment assumptions, realistic impact, and a
minimal regtest or testnet reproduction when possible. Do not include seeds,
private keys, production raw transactions, transaction IDs, wallet addresses,
IP addresses, onion credentials, API keys, or unrelated personal data.

The maintainer will acknowledge a complete report as availability permits,
coordinate validation and remediation privately, and credit the reporter if
requested. Please allow a reasonable remediation window before public
disclosure. This policy does not authorize testing against systems, wallets,
relays, or infrastructure you do not own or have permission to test.

## System and Scope

The covered system is the Rust Electrum protocol proxy, its request parser,
query-upstream forwarding, broadcast relay boundary, configuration, and
repository-supplied deployment guidance.

The proxy receives attacker-controlled newline-delimited Electrum frames,
forwards non-broadcast traffic to one configured Electrum upstream, and routes a
recognized `blockchain.transaction.broadcast` request to one selected adapter.
The local host and operator configuration are trusted. Upstream responses,
relay responses, wallet input, and network peers are untrusted.

## Security Invariants

1. A recognized broadcast request must never reach the query upstream.
2. Relay failure must never trigger public fallback or multi-relay fan-out.
3. Malformed, oversized, duplicate-field, or batched requests must fail closed.
4. Raw transactions, transaction IDs, wallet queries, addresses, peer IPs, and
   credentials must not be persisted or emitted in application logs.
5. The default listener must remain loopback-only; non-loopback binding requires
   explicit operator acknowledgement.
6. Frame sizes, response sizes, timeouts, and concurrent connections must remain
   bounded.
7. Relay success must be correlated to the request and validate the returned
   transaction-ID shape.
8. Secrets must not be accepted through committed configuration or exposed in
   diagnostics, tests, examples, CI output, or issue templates.

## Reportable Findings and Severity Context

Reportable issues include practical bypass of broadcast interception, silent
privacy downgrade, sensitive-data retention or disclosure, unauthenticated
remote exposure caused by a default, parser or resource-exhaustion flaws,
credential leakage, supply-chain execution weaknesses, and relay-response
confusion that can produce a false success.

Severity depends on realistic reachability and impact. A remotely reachable path
that links a user's network identity to a signed transaction, leaks credentials,
or silently broadcasts through a public path is high impact. A finding requiring
operator compromise or an explicitly unsafe override has materially lower
reachability and must state that prerequisite.

## Out of Scope, Exclusions, and Accepted Risk

- Compromise or malicious behavior of the wallet, operator host, Tor daemon,
  query upstream, relay provider, miner, operating system, or network.
- Transaction-graph, timing, fee, script, amount, and global passive observer
  analysis that the proxy does not claim to prevent.
- Wallet-query privacy from the configured query upstream.
- Miner inclusion, censorship resistance, fee estimation, and local Bitcoin
  consensus or mempool-policy validation.
- Multi-user isolation, billing, and denial-of-service resistance for a public
  hosted service; those deployment modes are not supported.
- Bugs in third-party services that are not caused or amplified by this project.

These exclusions do not suppress a finding where this project violates one of
its own invariants or misrepresents a boundary.

## Known Limitations and Compensating Controls

The current upstream transport is plain TCP and is intended for a local
self-hosted Electrum server. Application TLS termination, real-wallet
compatibility certification, fuzzing, regtest integration, a stable Slipstream
adapter, and an independent security audit are not complete.

Compensating defaults are loopback-only listening, client-authenticated onion
deployment guidance, reject-by-default broadcast behavior, one relay per
transaction, no silent fallback, bounded parsing, no sensitive persistence,
locked dependencies, and automated dependency policy checks.
