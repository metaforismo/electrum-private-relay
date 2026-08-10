# ADR 0001: Transparent query proxy with intercepted broadcast

- Status: accepted
- Date: 2026-08-10

## Context

Wallets commonly expect header, fee, history, transaction, and subscription
methods immediately after connecting to an Electrum server. A broadcast-only
subset would fail before many wallets reach transaction submission.

## Decision

Forward all well-formed non-broadcast Electrum frames to a configured upstream
and intercept `blockchain.transaction.broadcast` locally.

Use one explicit relay adapter, reject broadcasts when no adapter is configured,
and never fall back to the query upstream.

## Consequences

- Wallet compatibility is substantially more plausible than with a broadcast-
  only mock server, but still requires per-wallet testing.
- The query upstream observes wallet query activity.
- Parser ambiguity becomes security-critical, so malformed and batch requests
  fail closed.
- Operators can evolve relay providers without changing wallet configuration.
