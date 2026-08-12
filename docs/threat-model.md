# Threat Model

## Assets

- The user's network origin and the association between origin and transaction.
- The signed raw transaction before it reaches the selected destination.
- Wallet query history and address or script-hash interest set.
- Availability of the broadcast path and accuracy of the returned result.
- Onion service keys and optional client-authorization credentials.

The proxy never receives wallet seeds, private keys, or unsigned signing intent.

## Trust boundaries

- The local host and operator configuration are trusted.
- Electrum client frames, upstream frames, and relay responses are attacker-
  controlled inputs.
- The query upstream can observe queries and lie by omission.
- A broadcast relay can observe the raw transaction, reject it, delay it, return
  a false result, or correlate it with an account or client code.
- Tor and the local SOCKS5 daemon are external dependencies, not controls proven
  by this repository.

## Security invariants

1. A recognized broadcast request is never written to the query-upstream socket.
2. A relay failure never causes public fallback or multi-relay fan-out.
3. Malformed, oversized, duplicated, or batched request syntax fails closed.
4. Raw transactions, transaction IDs, queries, addresses, and peer IPs are not
   persisted or emitted in application logs.
5. The application listens only on loopback unless the operator explicitly
   acknowledges the risk.
6. Concurrent connections, frame sizes, response-bearing request identifiers,
   and per-connection outstanding requests remain bounded.
7. Relay responses are accepted only when their request ID and transaction-ID
   shape match the expected contract.
8. Query-upstream responses are forwarded only for outstanding query IDs and
   cannot satisfy an intercepted broadcast ID.

## Attacker goals considered

- Bypass interception so a transaction reaches the normal Electrum upstream.
- Exhaust memory or tasks with oversized frames, many connections, or an
  upstream that withholds responses while the wallet pipelines unique IDs.
- Inject a fake or malformed relay response.
- Inject an unsolicited query-upstream response that impersonates broadcast
  success.
- Cause a private route failure to downgrade into public propagation.
- Recover sensitive wallet or transaction data from logs or crash output.
- Reach an unintentionally public unauthenticated listener.

## Out of scope for the current milestone

- Compromise of the operator's host, wallet, Tor daemon, upstream, or relay.
- Transaction-graph, timing, fee, script, or amount fingerprinting.
- Sybil and global passive network observation resistance.
- Hiding wallet queries from the configured query upstream.
- Miner inclusion guarantees, censorship resistance, or fee estimation.
- Multi-user isolation, billing, hosted-service operation, and denial-of-service
  resistance for a public clearnet deployment.
- Validating Bitcoin consensus or mempool policy locally.

## Validation strategy

- Unit tests exercise strict parsing and configuration validation.
- An in-memory integration test proves that normal queries reach the upstream
  while a broadcast is handled by the relay and is not observed upstream.
- CI runs formatting, tests, Clippy, dependency policy, and pull-request
  dependency review.
- Real-wallet compatibility, Tor integration, regtest behavior, fuzzing, and an
  independent audit remain release gates.
