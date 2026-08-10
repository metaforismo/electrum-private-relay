# Architecture

## Goal

Provide an Electrum-compatible network endpoint that ordinary wallets can use
while ensuring signed transaction broadcast follows an explicit private route.
The proxy is not a wallet, signer, indexer, custody service, or anonymity proof.

## Data flow

```text
                    non-broadcast request / notification
Electrum wallet <------------------------------------------> query upstream
       |
       | blockchain.transaction.broadcast
       v
strict parser -> relay policy -> one adapter -> destination
                   |
                   +-> explicit error; never public fallback
```

Each wallet connection has one long-lived connection to the query upstream.
This preserves response and subscription behavior for normal Electrum methods.
Broadcast requests are parsed locally and are never written to that upstream
connection.

The proxy records outstanding query request IDs. A response is forwarded only
when its ID belongs to a query actually sent upstream; notifications remain
allowed. IDs reserved by an intercepted broadcast cannot be satisfied by the
query upstream. Unsolicited and colliding responses are dropped.

## Components

### Listener

- TCP on loopback by default.
- Explicit acknowledgement required for a non-loopback bind.
- Fixed maximum concurrent connection count.
- No peer-address logging.

### Frame boundary

Electrum messages are newline-delimited JSON. Both directions enforce a
configurable maximum frame size. The default is 2 MiB and the hard configuration
ceiling is 16 MiB.

Malformed and batched client frames close the connection after a generic error.
This is a deliberate safety choice: an ambiguous frame must not bypass the
broadcast interceptor because another parser interprets it differently.

### Query upstream

All well-formed methods other than `blockchain.transaction.broadcast` are passed
through byte-for-byte. The upstream can observe wallet query activity and can
lie by omission. Operators should normally use a local Electrum server backed
by their own Bitcoin node.

The current transport is plain Electrum TCP. A local upstream is the supported
secure deployment. Remote TLS upstream support is a future milestone.

### Broadcast relay

The relay interface receives only a validated, hex-encoded signed transaction.
Exactly one adapter is active for a process. The safe default adapter rejects
all submissions.

The initial `socks-electrum` adapter opens a new SOCKS5 connection, submits the
standard Electrum broadcast method, validates the response ID and transaction ID,
and then drops the connection.

## Security-relevant design decisions

- No automatic fallback from a private relay to the query upstream.
- No fan-out to multiple relay providers.
- No request bodies or identifiers in application logs.
- No unsolicited query-upstream responses or response-ID collisions.
- No unbounded frame reads.
- No application-level TLS or public listener by default.
- No `unsafe` Rust.
- Locked dependency graph and automated advisory/license/source checks.

## Compatibility boundary

The proxy supports newline-delimited single Electrum requests and asynchronous
upstream notifications. JSON batches are rejected. Compatibility must be tested
against each wallet and protocol version before a support claim is added.
