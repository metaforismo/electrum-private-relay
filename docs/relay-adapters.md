# Relay Adapter Contract

## Required behavior

Every broadcast adapter must:

1. accept one validated hex-encoded signed transaction;
2. use only the configured destination and transport;
3. apply explicit timeouts and bounded responses;
4. return a 64-character hexadecimal transaction ID on success;
5. return a generic failure without logging sensitive request or response data;
6. avoid retries unless the adapter documents their privacy and idempotency
   effect; and
7. never call another adapter or the query upstream as fallback.

## Current adapters

### `reject`

Safe default. Returns an Electrum error and makes no network request.

### `socks-electrum`

Connects to one Electrum endpoint through SOCKS5, submits the standard
`blockchain.transaction.broadcast` request, validates the correlated response,
and closes the connection.

This adapter describes its transport, not the destination's propagation policy.
An ordinary Electrum server reached through Tor can still gossip the transaction
through the public Bitcoin network.

## Planned Slipstream adapter

MARA Slipstream currently exposes a beta API and requires a client code for
submission. A future adapter must keep credentials out of CLI arguments and Git,
use Tor only when the provider contract and network behavior support it, parse
bounded responses, and document the account-correlation boundary.

It must remain opt-in. The core service will not silently substitute Slipstream
for another route or fan out to Slipstream plus public peers.

## Review checklist for new adapters

- Is the provider API stable and documented by a primary source?
- What identity, account, API key, client code, or payment link can correlate a
  submission?
- Does the provider rebroadcast publicly, mine directly, or do both?
- Are transaction packages, RBF, CPFP, and non-standard policy supported?
- Can an error or timeout mean the transaction was accepted anyway?
- Are retries safe, bounded, and observable without sensitive logs?
- Are DNS, TLS validation, proxying, credentials, and redirects fail-closed?
- Is the adapter covered by unit, mocked integration, regtest, and negative tests?
