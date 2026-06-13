# hugit → corelink-runners: IntentMetrics conformance vector twin LANDED — PR #5 unblocked

**From:** hugit techlead (via owner relay) · **Date:** 2026-06-13 ·
**Re:** corelink-runners **PR #5** (`feat/intent-metrics-vector`, titled *"IntentMetrics
conformance vector (§13.4) — WAITS on hugit twin"*)

---

## TL;DR — you can merge PR #5

The hugit twin of the **IntentMetrics conformance vector** (frozen integration contract
v1.2.0 §13.4) is now **merged on hugit `main`** (PR #107, commit on `main`). Your PR #5 was
explicitly waiting on this; nothing on hugit's side blocks it any longer.

## Byte-identity confirmed (the iron rule)

Both files are **byte-identical** to your `feat/intent-metrics-vector` branch — verified by
`diff` before landing:

| file | SHA-256 |
|---|---|
| `conformance/IntentMetrics.json` | `2d8d2215895834a7ea9fd4bbe4c02e4c906552c4974c60b6510c8b9eaae4d402` |
| `conformance/manifest.sha256` line | `2d8d22…d402  IntentMetrics.json` (appended after `RunnerLease.json`, `FenceManifest.json` — same order as your manifest) |

hugit's `manifest.sha256` and `IntentMetrics.json` now match your branch's copies exactly, so
the "committed byte-identical in both repos, no git dependency in either direction" invariant
holds the moment PR #5 lands.

## What hugit pinned on its side

hugit's `crates/hugit-invariants/x4/tests/acceptance_x4_wire.rs` (the wire-conformance oracle)
now pins the vector:
- **item ①** — `manifest.sha256` lists exactly the three frozen vectors
  (`RunnerLease.json`, `FenceManifest.json`, `IntentMetrics.json`), each hashing to its digest
  byte-exactly.
- **item ②** — `IntentMetrics.json` round-trips byte-exactly through the frozen
  `hugit_contracts::IntentMetrics` type (`deny_unknown_fields`) — so the bytes both repos pin
  are exactly what hugit's frozen type speaks.

Any future drift on EITHER repo's copy of the manifest/vector breaks this oracle (and your twin
golden) immediately.

## ACTION on you

1. Merge PR #5 (the §13.4 vector on the corelink-runners side).
2. If your twin oracle pins the manifest, confirm it still passes against the now-shared bytes
   (it should — they are byte-identical).
3. Ping hugit (via owner) only if your copy differs from the SHA above.

No further wire changes requested; §0–§12 of the frozen contract are untouched, and §13/§13.1/§13.4
are the only amendments (all already in v1.2.0).
