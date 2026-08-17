# Integration Testing

The repository separates deterministic protocol checks from environment-backed
integration checks. None of these tests uses mainnet, real funds, wallet secrets,
or third-party Electrum infrastructure.

## Native release candidates

`scripts/reproducible_release.py` performs two clean native release builds in
separate target directories, requires byte-identical executables, removes all
inherited `EPR_*` variables, and runs exact `--version` and `--check-config`
checks on both binaries. It then creates a deterministic ZIP and SHA-256 sidecar.

The GitHub workflow exercises Linux x86_64, Apple Silicon macOS, and Windows
x86_64. Main-branch candidate ZIPs receive GitHub SLSA provenance attestations.
This is same-runner double-build evidence, not independent cross-machine
reproducibility or a stable release.

Run the unit tests and one native candidate locally with:

```bash
python3 scripts/test_reproducible_release.py
python3 scripts/reproducible_release.py \
  --target x86_64-unknown-linux-gnu \
  --output-dir dist
```

See [release-candidate builds and provenance](REPRODUCIBLE_BUILDS.md) for the
artifact contents, verification commands, and assurance boundary. See the
[first stable-release audit scope](AUDIT_SCOPE.md) for the evidence expected on
an externally reviewed commit.

## Wallet protocol profiles

`tests/wallet_protocol_compat.py` replays newline-delimited request sequences
derived from current source snapshots of:

- Electrum at `c4cc40fdb8555b21f45da91ea7f85f1145907aea`;
- Sparrow at `b99b880c9fe75565921af9ef438d6314fdd73d6f`; and
- BlueWallet at `d8cb05c3997d3ead42487cc79008d3f59b539707`.

Each profile negotiates `server.version`, performs a wallet-specific normal
request, and submits `blockchain.transaction.broadcast`. The harness proves the
normal requests reach the query upstream and every broadcast reaches only the
selected SOCKS-Electrum relay.

This is an automated wire-protocol compatibility gate. It is not a claim that
the current packaged desktop or mobile app, TLS settings, Tor credential UI, or
every wallet workflow has been manually certified.

Run it with:

```bash
cargo build --locked --release
python3 tests/wallet_protocol_compat.py \
  --binary target/release/electrum-private-relay
```

Source references:

- <https://github.com/spesmilo/electrum/blob/c4cc40fdb8555b21f45da91ea7f85f1145907aea/electrum/interface.py>
- <https://github.com/sparrowwallet/sparrow/blob/b99b880c9fe75565921af9ef438d6314fdd73d6f/src/main/java/com/sparrowwallet/sparrow/net/SimpleElectrumServerRpc.java>
- <https://github.com/BlueWallet/BlueWallet/blob/d8cb05c3997d3ead42487cc79008d3f59b539707/blue_modules/BlueElectrum.ts>

## Bitcoin Core regtest

`tests/regtest_e2e.py` starts a disposable Bitcoin Core 30.2 node, creates a
wallet, mines only regtest blocks, constructs and signs a transaction without
broadcasting it, then submits it through the proxy and SOCKS adapter. The relay
calls `sendrawtransaction`, and the test requires the signed transaction to be
present in the real Core mempool while remaining absent from the query upstream.

The test image is pinned to:

```text
bitcoin/bitcoin:30.2@sha256:2b1d8a8d23c67b426afa8cd5b3ffd66b8c4ebbbceef3e214df8627aaead1f517
```

The image describes itself as unofficial and intended for testing. It contains
release binaries whose signatures are checked during its image build; it must
not be treated as a production Bitcoin Core distribution.

Run it with Docker Desktop or Docker Engine active:

```bash
python3 tests/regtest_e2e.py \
  --binary target/release/electrum-private-relay
```

References:

- <https://bitcoincore.org/en/releases/30.2/>
- <https://github.com/bitcoin/bitcoin/blob/master/doc/developer-notes.md#signet-testnet-and-regtest-modes>
- <https://hub.docker.com/r/bitcoin/bitcoin>

## Tor v3 onion smoke

`tests/tor_onion_smoke.py` builds a small test-only image from a digest-pinned
Debian base and the exact Debian Tor package `0.4.9.11-0+deb13u1`. It launches
two disposable daemons: a SOCKS client and a v3 onion service. A broadcast must
traverse the public Tor network to the controlled onion relay and must not reach
the query upstream.

Because public-network availability is outside the codebase, this smoke runs on
a weekly schedule and by manual dispatch rather than blocking every pull
request. Run it locally with:

```bash
python3 tests/tor_onion_smoke.py \
  --binary target/release/electrum-private-relay
```

The generated onion key and address live only inside an ephemeral container.
The test removes both Tor containers on completion.

Reference: <https://community.torproject.org/onion-services/setup/>

## Coverage-guided fuzzing

`fuzz/fuzz_targets/classify_frame.rs` sends arbitrary bytes into the same
`protocol::classify` entry point used for every wallet frame. The checked-in
corpus seeds valid queries and broadcasts, notifications, batches, duplicate
method keys, and out-of-range numeric IDs. Pull requests that touch the parser
run a bounded 30-second smoke campaign; the weekly schedule runs for five
minutes.

The workflow pins nightly `2026-08-12`, `cargo-fuzz` 0.13.2, and
`libfuzzer-sys` 0.4.13. To run the campaign locally after installing those
tools explicitly:

```bash
cargo +nightly-2026-08-12 fuzz run classify_frame \
  fuzz/corpus/classify_frame -- -max_total_time=60 -timeout=5
```

Reference: <https://rust-fuzz.github.io/book/cargo-fuzz/tutorial.html>
