# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

- feat: hugit-contracts — 15 frozen contract types + JSON Schemas + golden serde suite (WP-00)
- feat: Rust workspace scaffold — 12 crates, CI gates (fmt/clippy/test/audit), DCO + changelog discipline (WP-01)
- feat: hugit-app — GitHub App skeleton: X-Hub-Signature-256 webhook auth + ingest, PR-event persistence, Checks-API write-back, least-privilege manifest, uninstall revoke+halt (WP-B1)
- feat: hugit-policy — declarative gate engine v0: 3 ported gates (DCO/changelog/secrets), fail-closed enforcement, audited policy changes via EventRecord (WP-D6)
