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
strict parser -> global admission limit -> relay policy -> one adapter -> destination
                         |
                         +-> explicit error; never queue or public fallback
```

Each wallet connection has one long-lived connection to the query upstream.
This preserves response and subscription behavior for normal Electrum methods.
Broadcast requests are parsed locally and are never written to that upstream
connection.

The proxy records outstanding query request IDs. A response is forwarded only
when its ID belongs to a query actually sent upstream; notifications remain
allowed. IDs reserved by an intercepted broadcast cannot be satisfied by the
query upstream. Unsolicited and colliding responses are dropped. Each wallet
connection has a configurable in-flight request window (1,024 by default,
16,384 maximum); exhausting it rejects the new request and closes the
connection. String request IDs are limited to 256 UTF-8 bytes, while numeric
IDs must be exactly representable as signed or unsigned 64-bit integers. These
constraints give the table a meaningful byte bound and prevent JSON-number
normalization from making distinct IDs collide. A correlated response releases
its slot before it is forwarded.

## Components

### Listener

- TCP on loopback by default.
- Explicit acknowledgement required for a non-loopback bind.
- Fixed maximum concurrent connection count.
- Fixed maximum response-bearing requests awaiting replies per connection.
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

The selected adapter is wrapped in one process-wide admission layer backed by
two non-blocking semaphores: one counts simultaneous submissions and the other
accounts for the actual bytes of raw-transaction hex held inside relay calls.
Eight simultaneous submissions are allowed by default. When either budget is
exhausted, the adapter is not called and the transaction is never sent to the
query upstream or another route. There is intentionally no overload queue.

Configuration also rejects a concurrency × maximum-frame-size product above
128 MiB. The runtime byte semaphore independently enforces the same 128 MiB
ceiling over the actual raw-transaction strings, including when a library user
constructs configuration without the CLI parser. These are input-payload bounds,
not exact RSS limits: parsers, serializers, transports, tasks, and dependencies
add overhead and may temporarily copy data.

The initial `socks-electrum` adapter opens a new SOCKS5 connection, submits the
standard Electrum broadcast method, validates the response ID and transaction ID,
and then drops the connection. Both admission permits remain held until that
call returns or reaches its configured timeout.

### Process shutdown

The selected relay is wrapped in a race-free lifecycle gate. On `Ctrl-C`, the
process atomically stops admitting new private relay calls before it signals the
server loop. The listener is then dropped, so no new wallet connections are
accepted. A broadcast that entered the lifecycle gate before that transition
remains admitted; a later broadcast receives the ordinary generic private-relay
failure and is never redirected elsewhere.

Every accepted wallet connection is owned by one Tokio `JoinSet` rather than
being detached from the server lifetime. A process-wide `watch` channel is
subscribed by each connection task. Once shutdown begins:

1. connection attempts still opening their query-upstream socket are cancelled;
2. idle wallet readers and ordinary query connections observe the signal and
   close;
3. a client task already awaiting one admitted private relay call is allowed to
   receive that result, enqueue the correlated response, and then observes the
   shutdown signal before reading another wallet request; and
4. the connection writer is given its existing one-second bounded flush window.

Each connection owns its writer, upstream-reader, and client-reader subtasks
through abort-on-drop handles. Normal paths explicitly abort and await the
siblings that are no longer needed. If the outer connection task is itself
cancelled at the shutdown deadline, dropping those handles requests cancellation
of all three subtasks instead of detaching them. This also releases any
`DrainingRelay` active-call guard held by a stuck client subtask.

The server waits for the entire supervised connection set, not merely for the
relay active-count to reach zero. The outer deadline is the configured relay
timeout plus one second of response-flush headroom. If every task completes, the
server returns a successful `ShutdownReport`. If the deadline expires or a
connection task fails, the remaining set is aborted and awaited before the
runtime continues. Cancellation never invokes a different adapter, retries a
submission, writes the transaction to the query upstream, or fans out.

This reduces two avoidable ambiguities in the former lifecycle: connection tasks
and their subtasks are no longer detached when the listener stops, and the
process no longer relies on an unconditional one-second sleep after the relay
count becomes idle. It is still not an atomic acknowledgement protocol.
`SIGKILL`, power loss, kernel or process failure, blocked wallet output, and
provider-side acceptance without a usable response remain unknown outcomes.

## Security-relevant design decisions

- No automatic fallback from a private relay to the query upstream.
- No fan-out to multiple relay providers.
- No queue for relay overload.
- No unbounded simultaneous relay submissions or aggregate relay input payload.
- No new relay admission after graceful shutdown begins.
- No detached accepted-connection or per-connection I/O tasks during graceful
  shutdown.
- No unbounded shutdown wait for a stalled relay or connection writer.
- No request bodies or identifiers in application logs.
- No unsolicited query-upstream responses or response-ID collisions.
- No unbounded frame reads.
- No unbounded outstanding-request tables.
- No oversized response-bearing request identifiers.
- No application-level TLS or public listener by default.
- No `unsafe` Rust.
- Locked dependency graph and automated advisory/license/source checks.

## Compatibility boundary

The proxy supports newline-delimited single Electrum requests and asynchronous
upstream notifications. JSON batches are rejected. Compatibility must be tested
against each wallet and protocol version before a support claim is added.
