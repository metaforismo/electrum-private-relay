# First stable-release audit scope

This document defines the normative scope for the independent assessment
required before the first stable release. It is intentionally broader than a
single vulnerability scan and narrower than auditing Bitcoin Core, wallets,
Tor, operating systems, or private relay providers themselves.

## Freeze the target

An assessment must identify one immutable Git commit, not `main` or another
moving branch. The reviewer and maintainer must record:

- the full 40-character commit SHA;
- the repository URL;
- the release-candidate archive names and SHA-256 values, when binaries are in
  scope;
- the relevant GitHub provenance attestation URLs or IDs; and
- the exact Rust, Cargo, LLVM/linker, operating-system, Python, Docker, Bitcoin
  Core, Tor, and fuzz-tool versions used for reproduced evidence.

Any source or workflow change after that point creates a new audit target. A
finding is closed only against an explicit remediation commit and, for a
release-blocking finding, an independent recheck.

## In-scope implementation

The first assessment covers the complete repository at the frozen commit, with
particular attention to:

- `src/protocol.rs`: strict JSON parsing, duplicate keys, batches, broadcast
  recognition, transaction validation, request-ID constraints, and errors;
- `src/proxy.rs`: byte-for-byte query forwarding, broadcast interception,
  outstanding-request correlation, notifications, frame limits, connection
  supervision, cancellation, and response flushing;
- `src/relay/`: exactly-one-adapter routing, reject-by-default behavior, SOCKS5
  transport, response correlation, transaction-ID validation, timeouts,
  process-wide admission, byte budgeting, and lifecycle draining;
- `src/config.rs` and `src/endpoint.rs`: CLI/environment parsing, hard resource
  ceilings, loopback defaults, endpoint grammar, alias checks, and fail-closed
  startup;
- `src/main.rs`: signal ordering, admission shutdown, generic diagnostics, and
  process exit behavior;
- unit, CLI, TCP shutdown, wallet-profile, Bitcoin Core regtest, Tor onion, and
  fuzz harnesses as evidence for the properties they explicitly claim;
- `Cargo.toml`, both lockfiles, `deny.toml`, the pinned Rust toolchain, and all
  GitHub Actions workflows and action SHAs;
- release-candidate packaging, checksums, metadata, artifact permissions, and
  provenance generation; and
- README, architecture, relay-adapter, testing, Tor, threat-model, security, and
  operational guidance for accuracy and unsafe deployment claims.

Tests and documentation are evidence and attack surface; they are not accepted
as proof merely because they exist.

## Security properties to assess

The reviewer should attempt to falsify at least these properties:

1. A recognized `blockchain.transaction.broadcast` request never reaches the
   query upstream.
2. Relay failure, overload, timeout, cancellation, or shutdown never invokes a
   public fallback, a second adapter, or fan-out.
3. A query-upstream response cannot impersonate success for an intercepted
   broadcast or satisfy an uncorrelated wallet request.
4. Malformed, oversized, batched, duplicate-field, ambiguous-ID, and
   resource-exhaustion inputs fail closed within documented bounds.
5. Raw transactions, transaction IDs, wallet queries, addresses, peer IPs,
   credentials, and onion authorization material are not persisted or emitted
   in application diagnostics, tests, examples, artifacts, or CI output.
6. Loopback-only and reject-by-default behavior cannot be bypassed accidentally
   through CLI, environment, direct library construction, or endpoint aliases
   covered by the documented guardrails.
7. SOCKS-Electrum success is bound to the correct request and a valid transaction
   ID; truncation, oversized responses, protocol confusion, and ambiguous remote
   acceptance are handled conservatively.
8. Connection, frame, pending-request, relay-call, aggregate-payload, timeout,
   and shutdown limits remain effective on every reachable path.
9. Controlled shutdown cannot admit untracked work, detach a relay guard,
   duplicate a submission, or reroute a transaction after cancellation.
10. Dependency, workflow, build, packaging, checksum, and provenance controls do
    not grant broader permissions or make stronger authenticity claims than the
    evidence supports.

## Required evidence on the frozen commit

Before review sign-off, rerun and retain privacy-safe results for:

```console
cargo fmt --all -- --check
cargo test --locked --all-targets
cargo clippy --locked --all-targets -- -D warnings
python3 scripts/test_reproducible_release.py
```

Also require the repository's current:

- main and fuzz-crate dependency-policy checks;
- pull-request dependency review;
- CodeQL analyses;
- source-derived Electrum, Sparrow, and BlueWallet profiles;
- real Bitcoin Core regtest broadcast path;
- real Tor v3 onion smoke;
- bounded and scheduled coverage-guided fuzz campaigns; and
- native Linux, macOS, and Windows double-build candidate jobs.

Evidence must not contain production transactions, txids, addresses, wallet
history, IPs, onion keys, client authorization, seeds, private keys, or provider
credentials.

## Explicit exclusions and accepted boundaries

The assessment does not certify:

- a compromised wallet, host, kernel, firmware, compiler, Tor daemon, query
  upstream, relay provider, miner, DNS/routing layer, or GitHub service;
- transaction-graph, timing, fee, script, amount, Sybil, or global passive
  observer resistance;
- wallet-query privacy from the configured query upstream;
- miner inclusion, censorship resistance, fee estimation, or local consensus and
  mempool policy;
- public clearnet, multi-user, or multi-tenant hosting, authentication, billing,
  tenant isolation, application TLS termination, or internet-scale abuse
  resistance;
- packaged-wallet UI compatibility, which is tracked separately; or
- MARA Slipstream semantics until a stable provider contract and reproducible
  credentialed test path exist.

An exclusion does not excuse a violation of this project's own documented
invariants or a misleading security claim.

## Reviewer deliverables

The independent reviewer should provide:

- identity or organization and relevant Bitcoin/network-security experience;
- frozen commit and artifact identifiers;
- methodology, tooling, environment, and limitations;
- findings with realistic preconditions, impact, severity, and minimal regtest or
  testnet reproduction where safe;
- confirmation of which security properties were actively tested;
- a list of unreviewed areas or inconclusive results;
- recheck status for every release-blocking remediation; and
- a non-sensitive conclusion suitable for publication.

Potentially harmful details must use GitHub private vulnerability reporting as
described in `SECURITY.md`. Public summaries should be published only after
coordinated remediation.

## Stable-release decision

A stable tag or release remains blocked until:

- all Critical, High, and release-blocking findings are remediated and
  independently rechecked;
- the frozen commit passes all required automated and manual evidence gates;
- supported deployment modes and unsupported modes are explicit;
- packaged-wallet certification is complete for every claimed wallet/platform
  combination; and
- independent reproduction has been performed outside the original CI runner
  environment.
