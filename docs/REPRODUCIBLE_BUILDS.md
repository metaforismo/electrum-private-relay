# Native release-candidate builds and provenance

The `Release candidate` workflow produces short-lived native artifacts for
review. It does **not** create a GitHub release or tag and does not change the
project's experimental status.

## Automated gate

For each native target currently exercised by GitHub-hosted runners:

- `x86_64-unknown-linux-gnu` on Linux;
- `aarch64-apple-darwin` on macOS; and
- `x86_64-pc-windows-msvc` on Windows,

`scripts/reproducible_release.py` performs two clean, locked release builds in
separate Cargo target directories. Incremental compilation is disabled and
`SOURCE_DATE_EPOCH` is set to the checked-out commit time. The two executables
must be byte-for-byte identical.

Windows MSVC candidates add `/Brepro` through a target-specific `RUSTFLAGS`
entry. Existing flags are preserved and the applied reproducibility flag is
recorded in public build metadata. The comparison remains a raw executable-byte
comparison; Windows is not exempted or normalized after linking.

Both executables then run deterministic packaged checks:

```console
electrum-private-relay --version
electrum-private-relay --check-config
```

The script removes every inherited `EPR_*` variable before those checks, requires
exact stdout, and rejects any stderr output. `--check-config` is the existing
offline path: repository tests prove it exits before binding a listener or
opening an upstream, SOCKS, or relay connection.

A successful job packages one executable with public documentation and a
`BUILD-METADATA.json` file. ZIP members are sorted, receive one normalized UTC
commit timestamp, and use fixed file permissions. A separate `.sha256` sidecar
commits to the complete archive.

Main-branch candidate ZIPs are submitted to GitHub's artifact-attestation
service for SLSA build provenance. Pull-request build jobs retain read-only
repository permissions and the attestation-writing job is skipped.

## Assurance boundary

A green workflow provides **same-runner native double-build evidence**. It means
that two clean builds in one native runner environment produced the same
executable bytes and both executables passed the packaged checks. It does not:

- authenticate a source checkout obtained through another channel;
- prove that GitHub-hosted runners, Rust, LLVM, the linker, Python, dependencies,
  or the attestation service were uncompromised;
- prove that a separately administered machine will reproduce the executable;
- make ZIP compression identical across different Python or zlib versions;
- exercise wallet traffic, Bitcoin Core, Tor, or a private relay provider inside
  the candidate workflow;
- certify packaged Electrum, Sparrow, or BlueWallet applications; or
- replace an independent Bitcoin/network-security review.

Equal output inside one environment can repeat the same compromised toolchain.
Independent reproduction and external review remain stable-release gates.

## Candidate contents

Each target artifact contains:

```text
electrum-private-relay-v<version>-<target>/
├── electrum-private-relay[.exe]
├── BUILD-METADATA.json
├── README.md
├── SECURITY.md
├── LICENSE
└── docs/
    ├── AUDIT_SCOPE.md
    ├── REPRODUCIBLE_BUILDS.md
    ├── architecture.md
    ├── relay-adapters.md
    ├── testing.md
    └── tor.md
```

The metadata records the package version, exact Git commit, target triple,
commit-derived timestamp, toolchain versions, executable SHA-256, applied linker
reproducibility flags, and packaged-check results. It intentionally contains no
raw transaction, transaction ID, wallet request, address, peer IP, onion
credential, API key, local username, hostname, or runner path.

CI artifacts expire after 14 days and are review candidates only. They must not
be presented as an official stable release.

## Run locally

Use a reviewed checkout and the repository's pinned Rust toolchain:

```console
python3 scripts/test_reproducible_release.py
python3 scripts/reproducible_release.py \
  --target x86_64-unknown-linux-gnu \
  --output-dir dist
```

On Apple Silicon use `aarch64-apple-darwin`. On 64-bit Windows use
`x86_64-pc-windows-msvc` and invoke the script with `python` when that is the
local launcher name.

The command exits unsuccessfully when the builds differ, a packaged check
changes or writes to stderr, a required public document is missing, or the
archive/checksum cannot be written.

## Verify a downloaded candidate

First verify the SHA-256 sidecar with a trusted local hashing tool. On GNU/Linux:

```console
sha256sum --check \
  electrum-private-relay-v0.1.0-x86_64-unknown-linux-gnu.zip.sha256
```

For a main-branch artifact, GitHub CLI can additionally verify repository
provenance:

```console
gh attestation verify \
  electrum-private-relay-v0.1.0-x86_64-unknown-linux-gnu.zip \
  --repo metaforismo/electrum-private-relay
```

Checksum and attestation verification identify a candidate produced by a
particular workflow. They do not replace source review, independent
reproduction, integration testing, packaged-wallet certification, or external
audit.
