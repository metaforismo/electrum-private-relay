# Threat Model

## Assets

- The user's network origin and the association between origin and transaction.
- The signed raw transaction before it reaches the selected destination.
- Wallet query history and address or script-hash interest set.
- Availability of the broadcast path and accuracy of the returned result.
- Onion service keys and optional client-authorization credentials.

The proxy never receives wallet seeds, private keys, or unsigned signing intent.

## Trust boundaries

- The local host and operator configuration are trusted, but malformed endpoint
  syntax and common self-referential endpoint mistakes are rejected before
  runtime.
- Electrum client frames, upstream frames, and relay responses are attacker-
  controlled inputs.
- The query upstream can observe queries and lie by omission.
- A broadcast relay can observe the raw transaction, reject it, delay it, return
  a false result, or correlate it with an account or client code.
- Tor and the local SOCKS5 daemon are external dependencies, not controls proven
  by this repository.
- DNS resolution and routing are external to configuration validation; textual
  endpoint checks do not authenticate their eventual destination.

## Security invariants

1. A recognized broadcast request is never written to the query-upstream socket.
2. A relay failure never causes public fallback or multi-relay fan-out.
3. Malformed, oversized, duplicated, or batched request syntax fails closed.
4. Raw transactions, transaction IDs, queries, addresses, and peer IPs are not
   persisted or emitted in application logs.
5. The application listens only on loopback unless the operator explicitly
   acknowledges the risk.
6. Concurrent connections, frame sizes, response-bearing request identifiers,
   per-connection outstanding requests, and simultaneous relay submissions
   remain bounded.
7. The configured broadcast concurrency multiplied by the maximum frame size
   remains within a fixed aggregate input-payload budget, and runtime admission
   independently accounts for the actual raw-transaction bytes in relay calls.
8. Relay overload is rejected immediately without queueing, invoking the
   selected adapter, leaking to the query upstream, or attempting fallback.
9. Relay responses are accepted only when their request ID and transaction-ID
   shape match the expected contract.
10. Query-upstream responses are forwarded only for outstanding query IDs and
    cannot satisfy an intercepted broadcast ID.
11. Listener, upstream, relay, and SOCKS ports are non-zero; Electrum endpoint
    values use unambiguous `host:port` syntax with strict IP or DNS host grammar.
12. Obvious listener loops and obvious reuse of the query upstream as the
    private relay are rejected before any runtime socket is opened.
13. Graceful shutdown rejects new relay admission, supervises every accepted
    connection task, bounds the complete drain wait, and never turns
    cancellation or timeout into retry, fallback, or fan-out.

## Attacker goals considered

- Bypass interception so a transaction reaches the normal Electrum upstream.
- Exhaust memory or tasks with oversized frames, many connections, or an
  upstream that withholds responses while the wallet pipelines unique IDs.
- Hold many slow private relay submissions open across separate wallet
  connections to exhaust sockets, tasks, memory, Tor circuits, or provider
  capacity.
- Bypass CLI-only configuration checks through direct use of the Rust library.
- Inject a fake or malformed relay response.
- Inject an unsolicited query-upstream response that impersonates broadcast
  success.
- Cause a private route failure to downgrade into public propagation.
- Exploit process shutdown to admit another relay call, detach untracked
  connection work, duplicate an existing submission, or silently redirect it
  after cancellation.
- Recover sensitive wallet or transaction data from logs or crash output.
- Reach an unintentionally public unauthenticated listener.
- Smuggle a URL, credential, path, malformed DNS name, ambiguous IPv6 spelling,
  or disabled port into an endpoint field.
- Induce an obvious self-loop or query/relay endpoint collision through an
  operator configuration error.

## Configuration guardrail boundary

`--check-config` runs parsing and semantic validation, then exits before binding
the client listener or connecting to the query upstream, relay, or SOCKS proxy.
The same validation always runs during normal startup. It rejects:

- zero listener, query-upstream, relay, or SOCKS-proxy ports;
- endpoint values containing schemes, user information, paths, whitespace,
  unbracketed IPv6 literals, raw non-ASCII DNS names, or malformed DNS labels;
- zero or excessive connection, pending-request, or concurrent-broadcast limits;
- a concurrent-broadcast/frame-size product above the 128 MiB aggregate input
  budget, including arithmetic overflow;
- a query upstream that is an obvious literal-IP or `localhost` alias of the
  client listener;
- a SOCKS Electrum relay that is an obvious alias of the query upstream;
- a relay endpoint or SOCKS proxy that obviously targets the client listener;
  and
- the existing unsafe non-loopback and missing-relay conditions.

Endpoint parsing accepts literal IPv4, bracketed IPv6 in CLI `host:port` input,
ASCII DNS names, punycode names, onion names, and an optional final DNS root dot.
Endpoint comparisons cover exact IPs, loopback aliases, DNS case, and that final
root dot. They intentionally perform no DNS lookup. DNS rebinding, CNAMEs,
split-horizon names, routing changes, proxies behind the same hostname, and
malicious local name resolution remain outside this guardrail and require
operator review.

## Relay saturation boundary

Every process wraps its selected relay in one admission layer with two
semaphores. A request-slot permit and a number of byte permits equal to the
raw-transaction string length are acquired with non-blocking operations
immediately before the adapter is called. Both are held until the adapter
returns. The default request limit is eight simultaneous submissions; the byte
budget is fixed at 128 MiB.

A saturated request receives the same generic private-relay failure as other
relay errors. Its transaction is not queued, sent to the adapter, or forwarded
elsewhere. The runtime byte semaphore remains effective even when an embedding
application constructs `Config` directly instead of using `Cli::try_from`.

The configuration product check bounds the sum of configured maximum input
frames, while the runtime byte permits bound actual raw-transaction strings
inside relay calls. Neither is an exact resident-memory promise: JSON parsing,
serialization, transport buffers, task state, and third-party libraries may
hold additional copies or overhead. A compromised or slow relay can still
consume all permitted slots until the configured relay timeout expires, causing
temporary fail-closed unavailability rather than unbounded growth.

## Graceful shutdown boundary

The `Ctrl-C` path changes relay admission under one mutex before notifying the
server loop. A call that incremented the active count first remains admitted; a
call that reaches the gate after shutdown starts receives a generic relay error.
This ordering prevents a check-then-increment race from admitting untracked relay
work.

Accepted wallet connections are stored in one supervised Tokio task set. After
the server drops the listener, it broadcasts a shutdown state to every
connection task. A connection still opening its query-upstream socket exits;
an idle client reader exits; and a client already awaiting an admitted relay
call may finish that call, enqueue the correlated result, and then exits before
reading another request. The per-connection writer retains its one-second
bounded flush window.

The outer server waits for all connection tasks for at most the configured relay
timeout plus one second. A clean outcome is reported only when the full task set
has joined. If the deadline expires, or a supervised task terminates
unexpectedly, the remaining tasks are aborted and awaited. Task cancellation
drops any relay admission guards, but it never calls an alternate adapter,
retries the transaction, writes it to the query upstream, or fans out. Only a
non-sensitive process warning distinguishes a forced drain from a clean one.

This is a best-effort reduction of local cancellation ambiguity, not an atomic
network protocol. A provider may accept a transaction immediately before a
socket error or process failure, and the wallet may still miss the result after
`SIGKILL`, power loss, kernel failure, blocked output, or a crash outside the
controlled `Ctrl-C` path. Operators must treat such cases as an unknown outcome
and inspect their selected relay or mempool through an independent safe channel;
they must not assume failure and blindly resubmit through a public route.

## Out of scope for the current milestone

- Compromise of the operator's host, wallet, Tor daemon, upstream, or relay.
- Transaction-graph, timing, fee, script, or amount fingerprinting.
- Sybil and global passive network observation resistance.
- Hiding wallet queries from the configured query upstream.
- Miner inclusion guarantees, censorship resistance, or fee estimation.
- Multi-user isolation, billing, hosted-service operation, and denial-of-service
  resistance for a public clearnet deployment.
- Validating Bitcoin consensus or mempool policy locally.
- Authenticating DNS or proving that two differently named endpoints cannot
  resolve to the same destination.
- Guaranteed response delivery after abrupt process or operating-system
  termination.

## Validation strategy

- Unit tests exercise strict parsing, zero-port rejection, bounded
  configuration, aggregate payload budgeting, endpoint alias checks, and
  listener-loop/query-relay rejection.
- Controlled asynchronous relay tests separately saturate the request-slot and
  actual-payload-byte budgets, prove the wrapped adapter is not called, release
  the held permits, and prove the admitted request completes.
- Relay-lifecycle tests prove shutdown rejects a new relay call, waits for a call
  that was already admitted, and expires its wait for a deliberately stuck
  relay.
- TCP shutdown tests prove three connection-level outcomes: an admitted call
  completes and its correlated txid reaches the wallet without appearing at the
  query upstream; a stuck call is force-cancelled after the bounded connection
  deadline; and an idle connection closes promptly.
- Black-box CLI tests prove `--check-config` exits successfully without network
  setup and fails closed on representative unsafe, ambiguous, or zero-valued
  endpoint and resource configurations.
- An in-memory integration test proves that normal queries reach the upstream
  while a broadcast is handled by the relay and is not observed upstream.
- Source-derived Electrum, Sparrow, and BlueWallet wire profiles exercise
  handshake, query, and isolated broadcast behavior.
- A Docker-backed Bitcoin Core regtest test proves that a real signed
  transaction reaches a mempool only through the selected relay path.
- A scheduled smoke test routes through a real ephemeral v3 onion, and bounded
  PR plus weekly fuzz campaigns exercise the wallet-frame parser.
- CI also runs formatting, tests, Clippy, dependency policy, and pull-request
  dependency review.
- Packaged-wallet UI and TLS certification and an independent audit remain
  external release gates.
