# Contributing

Thank you for helping improve private Bitcoin transaction broadcast.

## Before opening a change

- Use a public issue for feature or compatibility discussion.
- Do not open a public issue for a suspected vulnerability; use the private
  reporting process in `SECURITY.md` once published.
- Keep privacy claims narrow and evidence-backed.
- Do not add telemetry, sensitive logging, public fallback, fan-out, or a
  non-loopback default.

## Development

Use the pinned Rust toolchain and keep `Cargo.lock` committed.

```bash
cargo fmt --all -- --check
cargo test --locked --all-targets
cargo clippy --locked --all-targets -- -D warnings
```

Changes to parsing, routing, relay adapters, logging, configuration, or network
exposure require tests covering both success and fail-closed behavior. A provider
adapter must also update `docs/relay-adapters.md` with trust and correlation
boundaries.

## Pull requests

- Keep each pull request focused.
- Explain the user-visible behavior and privacy impact.
- List exact validation commands and remaining gaps.
- Update the changelog for externally visible behavior.
- Use conventional commit prefixes such as `feat:`, `fix:`, `docs:`, `test:`,
  `chore:`, or `security:`.

By contributing, you agree that your contribution is licensed under Apache-2.0.
