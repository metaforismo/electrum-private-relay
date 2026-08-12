# Changelog

All notable changes to this project will be documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project intends
to use [Semantic Versioning](https://semver.org/spec/v2.0.0.html) after its first
stable release.

## [Unreleased]

### Added

- Transparent Electrum query proxy.
- Fail-closed broadcast interception.
- Reject-by-default and SOCKS5 Electrum relay adapters.
- Bounded frames and connections.
- Bounded request identifiers and per-connection outstanding-request window
  with fail-closed overflow.
- Outstanding-request correlation to block query-upstream response spoofing.
- Initial architecture, threat model, Tor deployment, and contributor guidance.
