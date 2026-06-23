# Design — git `push` (receive-pack) serve-wiring wave

**Date:** 2026-06-22 · **Owner:** hugit TL · **Status:** design → WP1 in progress

## Why / scope
`git push` against the engine is deliberately **403** today (`git.rs:133`
`respond_push_forbidden`). The *core* receive logic already exists and is hermetically
built in **`hugit-proto::write::receive::receive_pack()`** — it bounds the pack, unpacks
via the system git binary, verifies every oid, stores loose bytes to CAS, anchors the ref
update (target MUST be delivered), and records ONE `ref.update` raw-push event on the D1
log. What's missing is the **serve-layer glue**: parse the wire, gate the write, call the
core, write back refs, answer the report. This wave delivers `git push` to **BUILT +
hermetic-green** (a real `git push` succeeds against a test serve), exactly as clone/fetch
landed before deploy. LIVE-serving is gated on one infra secret (GAP 2) + the ref hot-swap.

## The 3 gaps (Explore-confirmed) and the decisions
- **GAP 1 — no wire-command parser.** The receive-pack POST body is pkt-lines of
  `<old-oid> SP <new-oid> SP <ref-name> NUL <caps>` (caps on the FIRST command only), a
  `flush-pkt`, then the raw packfile. Nothing parses this yet. **Decision:** add a pure
  `parse_receive_pack_body(&[u8]) -> Result<ReceiveCommands, RecvParseError>` (WP1) — pure,
  hermetic, no behavior change. Reuse `gix-packetline` (already in-tree).
- **GAP 2 — serve PAT is `cas:r` (read-only).** Pushed objects need a CAS **write**
  (`cas:rw`); the standing engine cred is read-only (R2 PUT → 403). **Decision:** the
  receive write path uses a SEPARATE, push-only `cas:rw` credential via a new env
  (`HUGIT_SERVE_CAS_RW_PAT_FILE`), **absent by default → push stays disabled** (the existing
  `WritePathDisabled` flag is the gate). Reads keep the single `cas:r` invariant; the
  write-PAT is deploy-gated (owner/server-TL, same shape as the ingest `cas:rw`). Hermetic
  tests use a local in-memory `Cas`.
- **GAP 3 — refs are static after boot.** `RepoState.git_refs: BTreeMap` is loaded once
  from `refs.json` and never mutated; a pushed tip wouldn't advertise. **Decision (v0):** on
  a successful push, (a) append the `ref.update` event (core already does), (b) **hot-swap**
  the in-memory ref under a write-lock, (c) **write-back** a new `refs.json` to R2,
  **CAS-guarded** (compare-and-swap against the version seen at serve time) so two concurrent
  pushes can't race. Full log-derived refs is a later refactor; v0 is write-back + hot-swap.

## WP decomposition (sequential — coupled files, built by the lead, not fanned out)
| WP | Deliverable | Files (disjoint-ish) | Gate | LIVE-gated? |
|----|-------------|----------------------|------|-------------|
| **W1** | `parse_receive_pack_body` + `ReceiveCommands` + unit tests (crafted pkt-lines, caps-on-first, flush, pack split, malformed→fail-closed) | new `git/receive_parse.rs` in hugit-serve (or hugit-proto wire mod) | fmt+clippy+test | no (pure) |
| **W2** | wire the handler: replace the 403 with parse → `authorize_write` (404-no-oracle) → build `ReceiveRequest` → call `receive_pack` with the serve `Cas`+`EventLog` → emit the report (`unpack ok` / `ok <ref>` / `ng <ref> <reason>`) | `git.rs` handler + a serve `Cas`/`EventLog` adapter | +e2e `real_git_push_succeeds` (hermetic, local CAS) | partial (needs W3 cred for live) |
| **W3** | ref hot-swap (`Arc<RwLock>` or equivalent) + R2 `refs.json` CAS-guarded write-back + the `HUGIT_SERVE_CAS_RW_PAT_FILE` write-cred wiring + the WritePathDisabled deploy gate | `state.rs` (`git_refs` → live), `git.rs` advertise reads the live view | +concurrency test (two pushes, one wins CAS) | LIVE (owner sets the cas:rw secret) |

## Invariants (the rigor bar — non-negotiable)
- **Write-authz ≠ read-authz** — gate on `authorize_write` (ownership), NEVER the public read
  predicate (the banked #129 hole). 404-no-oracle on denial.
- **Fail-closed** — any parse/bound/verify/anchor error commits NOTHING to CAS and appends
  NO event (the core already guarantees the second half; W1/W2 guarantee the first).
- **Anchor** — a ref update whose target oid the push didn't deliver is tampering → reject
  (core already enforces; W2 must pass the delivered-oid set through honestly).
- **DoS bounds** — `RecvLimits` (16 MiB pack / 64 MiB inflated / 1e6 objects) applied before
  unpack; the serve handler must also cap the POST body before buffering.

## Out of scope (this wave)
Multi-ref atomic push (v0 = single ref, matching `ReceiveRequest.update`); delete-ref
(`new_oid` all-zero); push-options; sideband progress; the live `cas:rw` provisioning (an
ASK to the server TL when W3 lands).
