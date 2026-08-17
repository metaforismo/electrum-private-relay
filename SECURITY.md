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
query-upstream forwarding, broadcast relay boundary, configuration, process
lifecycle, and repository-supplied deployment guidance.

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
6. Frame sizes, response sizes, timeouts, concurrent connections, simultaneous
   relay submissions, and shutdown drain time must remain bounded.
7. Relay success must be correlated to the request and validate the returned
   transaction-ID shape.
8. Secrets must not be accepted through committed configuration or exposed in
   diagnostics, tests, examples, CI output, or issue templates.
9. Graceful shutdown must reject new relay admission before stopping the
   listener, supervise every accepted connection task through a bounded drain,
   and never convert cancellation into retry, fallback, or fan-out.

## Reportable Findings and Severity Context

Reportable issues include practical bypass of broadcast interception, silent
privacy downgrade, sensitive-data retention or disclosure, unauthenticated
remote exposure caused by a default, parser or resource-exhaustion flaws,
credential leakage, supply-chain execution weaknesses, relay-response confusion
that can produce a false success, and lifecycle races that detach, duplicate, or
reroute a submission during graceful shutdown.

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
- Guaranteed wallet response delivery after `SIGKILL`, power loss, process or
  operating-system failure, blocked client output, or a provider-side ambiguous
  acceptance outcome.
- Bugs in third-party services that are not caused or amplified by this project.

These exclusions do not suppress a finding where this project violates one of
its own invariants or misrepresents a boundary.

## Known Limitations and Compensating Controls

The current upstream transport is plain TCP and is intended for a local
self-hosted Electrum server. Application TLS termination, packaged-wallet UI
and TLS compatibility certification, a stable Slipstream adapter, and an
independent security audit are not complete. Source-derived wallet protocol
profiles, real Bitcoin Core regtest broadcast, and a scheduled real Tor onion
smoke test are automated. The frame classifier also has bounded PR fuzzing and
a longer weekly campaign. These controls do not replace external assurances.

The controlled `Ctrl-C` path stops new relay admission, signals and supervises
all accepted connection tasks, and gives an already admitted call a
relay-timeout-bounded completion plus one second of response-flush headroom. A
stuck task is aborted and awaited without retry or fallback. This is not an
atomic acknowledgement protocol and cannot determine whether a remote relay
accepted a transaction when the process or network fails before a valid response
reaches the wallet.

Compensating defaults are loopback-only listening, client-authenticated onion
deployment guidance, reject-by-default broadcast behavior, one relay per
transaction, no silent fallback, bounded parsing, supervised bounded shutdown,
no sensitive persistence, locked dependencies, and automated dependency policy
checks.
