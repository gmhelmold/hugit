# SOTA audit — 6-perspective fleet, consolidated verdict

> 2026-06-11. Owner directive: *"a régua é simples: tudo que não estiver SOTA
> precisa de atenção."* Fleet: 4 Opus (correctness/contracts · security/
> redaction · architecture/seams · product-vs-SOTA) + 2 Sonnet (docs truth ·
> test quality), all read-only, hugit @ main `12d7e95`. This file is the
> lead's consolidation: deduped across the six reports, false positives
> killed by evidence, triaged. Raw findings live in the session transcripts;
> everything load-bearing is restated here.

## Verdict in one paragraph

The **integrity spine is genuinely SOTA** (single-sourced hash chain with
fail-closed tamper detection · D14 at type+projection level · the
two-transcript imperative unbypassable · broker/AC credential seams above
bar · the three architecture cuts landed clean with byte-verified
conformance · the export envelope is genuinely AHEAD of any mainstream
forge). Below the bar sit **three security structurals** (toy redaction
core feeding a forever-store · the D14 authz guard wired to nothing on the
mutation path · erasure unimplementable on the real cold tier), **one
product structural** (the wedge — memoized CI + union queue — is invisible
through the porcelain), and a tail of correctness debt (f64 money, golden
self-oracle, TOCTOU file seam) and stale records.

## TIER 1 — Security/integrity structurals (fix first)

| # | Finding | Evidence | SOTA bar |
|---|---|---|---|
| S1 | **Redaction core is a planted-marker matcher** — literal `"SECRET:"` only; real PATs/keys pass verbatim into retention-forever storage | `hugit-ledger/src/redact.rs:25-31` | pattern+entropy corpus (gitleaks-class) |
| S2 | **Envelope persists charter/acceptance/files_read/env_manifest UNREDACTED**, contradicting the module's own "never a byte un-redacted" claim; `why` also surfaces charter unredacted while the ledger projection redacts it | `envelope/mod.rs:516-531` vs `:341` · `cli/why/resolver.rs:372-376` | every string field through the scrubber on write; one redaction law across read surfaces |
| S3 | **D14 authz guard is a detached lock**: `authorize`/`AuditedGuard` golden-tested, zero production callers; `EventLog::append`/writer have no gate. CLI `--author-kind` is caller-supplied (subagent can claim orchestrator) | `refstore/authz/mod.rs:215` (no callers) · `cli/pr/mod.rs:88-99` | guard wraps the only mutating primitive; author-kind binds to authenticated principal (full fix lands with identity/P2 — interim: wire the guard + disclose the authn limit at the seam) |
| S4 | **Erasure is a contradiction today**: ratified law = forever + explicit tombstone path, but no real cold-store trait has erase; X7/X12 proofs run against a toy ObjectStore the live seam doesn't share | `cold_store.rs:122-139` vs `invariants x12/erasure.rs:139` | tombstone-erase on the production trait, invariant proven against it |
| S5 | **`canonical_json` depends on an ambient cargo feature**: serde_json default key-sort; any transitive dep enabling `preserve_order` silently diverges every `this_hash` fleet-wide | `refstore/log/mod.rs:202-209` | explicit recursive canonicalization, feature-proof |
| S6 | **Golden discipline is self-referential**: fixtures generated and verified by the same code path; `UPDATE_SCHEMAS=1` is a one-command laundry; the "independent hash pin" re-uses the same sha2 | `contracts/integration_tests.rs:52` · `:380` | externally-authored byte pins; generator never its own oracle |
| S7 | **The fence escape oracle left the repo**: red-team item ⑤ (materialized-path escape) is 6 comment lines locally; the load-bearing invariant is only provable in corelink-runners | `fence acceptance_c5b.rs` item ⑤ | a local regression must pin the traversal/classification law even post-transfer |

## TIER 2 — Product vs SOTA (the wedge is invisible)

| # | Finding | Evidence |
|---|---|---|
| P1 | **No `checks`/`queue` verb exists** — memoized CI hit-rate, memo keys, union-batch state, failure attribution: all unreachable by the agent; `pr land` returns a bare position (no ETA/batch/blame) | `hugit check` → unrecognized; `pr/mod.rs:384` |
| P2 | **Two error schemas + two exit regimes**: flat vs nested `error`, `fix` vs `suggested_fix`, porcelain=JSON/exit-2 vs legacy verbs=plaintext/exit-1 (`tournament`/`export` not under the JSON law at all) | Aud-4 F2/F4/F10 |
| P3 | **`--log` names three incompatible formats** (porcelain array · why's `[{record:…}]` · export's `{events:[…]}`) | Aud-4 F3 |
| P4 | **Missing verbs agents need**: `list` for every noun, `close`/`abandon`, `log init`; fix-hints cite verbs that don't exist ("land or abandon" — no abandon) | Aud-4 F6/F7 |
| P5 | Referential asymmetry (`--run-id` optional with orchestrator; ghost `--campaign` accepted) · missing log file silently = empty world · key-sets unstable across idempotent re-runs | Aud-4 F5/F8/F11 |
| ✦ | **AHEAD (keep + market)**: the export envelope — typed object classes + digest-sealed redaction manifest; no mainstream forge ships it | Aud-4 F12 |

## TIER 3 — Correctness/robustness debt

- **f64 money** summed across altitudes (identity not bit-exact; 1e-9 epsilon
  drifts at scale) + unchecked u64 sums (silent wrap) — *fixing the wire type
  touches frozen contract 1.1.0 → needs an owner-signed 1.2.0 amendment;
  internal accumulation can move to integers without wire change.*
  (`rollup.rs:207-237,511-514`)
- TOCTOU on the porcelain file seam (read-modify-write, no lock/atomic
  rename); corrupted/truncated log → unstructured error; mid-write cold-store
  failure untested. (`canonical_log.rs:34-61` · Aud-6 F11/19/20)
- `born_at` tie → nondeterministic work/waste split; span≤sum asserted not
  enforced; `pr land` "queue authority" is a tail counter fed empty
  affected-sets (claim overstates); `InMemoryColdStore::get` skips the
  digest re-check; `serde_json::to_vec(...).expect` can panic the capture
  path on non-finite cost_usd. (`rollup.rs:226-238` · `pr/mod.rs:421-470` ·
  `cold_store.rs:256-278` · `envelope/mod.rs:539`)
- Env-gated lanes skip silently (the green count carries no live-lane
  signal); wire round-trip is byte-exact-modulo-trailing-whitespace.
  (Aud-6 F16/17)

## TIER 4 — Records staleness (cheap, sweep now)

`docs/interop.md` (runner-as-client-crate · "TTL-bound" — both false post
2026-06-10, no supersession note) · **ADR-0001's inline JSONC still says
`1.0.0` + three altitudes** (the most dangerous doc/code divergence) ·
status lines never flipped to COMPLETE in 3 plan docs + wp-contracts INDEX ·
CHANGELOG feat(web) parenthetical ("WP-F2 pending") · headless-engine table
row (`hugit-runner::envelope`) · CLAUDE.md status-date anchor + sidecar
wording vs cargo's 17 · dogfood comment citing the departed crate ·
wave2-handoff's `cargo run -p hugit-web` · handoff docs lacking
APPLIED/CLOSED markers · transplant naming debt (`HUGIT_RUNNER_HOST`,
`hugit-runner` doc title inside the CoreLink product).

## False positives killed by the lead (evidence)

- "deny.toml lacks [advisories] → unscanned": cargo-deny checks advisories
  by default (the gate printed `advisories ok`) and CI runs
  `cargo audit --deny warnings` besides. Dismissed; an explicit section is
  cosmetic.
- "Live soak lane panics when enabled → broken": that IS the disclosed
  P2 seam behaving run-not-skip (fail loudly until infra exists). By design;
  wording could say so at the panic site.
- Manifest-tamper finding downgraded: local edit of vector+manifest evades
  the local pin, but the cross-repo byte-identity (independently verified)
  is the designed tripwire — single-repo tamper breaks the other side.

## Fix plan (small WPs, partition rule applied)

- **Wave A (security)**: A1 redaction engine + all-fields scrub + why parity
  (opus) · A2 wire the D14 guard onto the mutation path + honest authn
  disclosure (opus) · A3 tombstone-erase on the real trait + X12 re-pointed
  (opus) · A4 explicit canonical_json (sonnet) · A5 golden de-laundering:
  external byte pins (sonnet) · A6 local escape-law regression (sonnet).
- **Wave B (product)**: B1 one error schema + one exit law, all verbs incl.
  legacy (opus) · B2 `hugit checks` + `queue show` — make the wedge visible
  (opus) · B3 list/close/abandon/init + real fix-hints + stable key-sets
  (sonnet) · B4 referential symmetry + explicit missing-log error (sonnet).
- **Wave C (robustness)**: C1 file-seam lock/atomic + truncated-log
  structured error (opus) · C2 tiebreak/overflow/NaN/read-guard/span batch
  (sonnet) · C3 loud skips + trailing-byte exactness (sonnet).
- **Wave D (records)**: D1 hugit docs sweep (sonnet) · D2 transplant naming
  + doc identity in corelink-runners (sonnet).
- **Owner decision needed**: money representation (frozen `cost_usd: f64` →
  integer micro-USD = contract 1.2.0 amendment) — recommended, not assumed.
