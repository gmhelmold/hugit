# WP-DOCK-6 — landing per-dock: byte-identity + acceptance verification

squad: hugit-core · M · opus · 80k · branch: wp/dock-6

## Charter
**What:** make `land` verify **per-dock**: the byte-identity check (exists —
the mirror's per-push verify) PLUS the acceptance-list execution via the
existing `verdict`/`run_memoized` machinery — so landing is *verification*
(matching the declared promise: did the committed work match the charter's
acceptance list?), not narrative.
**Why:** the auditor's question ("did rate-limiting actually land as
promised and verified?") is answered by this WP; without it, landing is
transport, not proof.

## Owned acceptance (VERBATIM from design §6, §7)
- (§6) — per-dock: byte-identity (exists: mirror verify) + acceptance-list
  execution (reuse `run_memoized`/`verdict`) → union verdict decides.
- (§7 / A4) — reconciliation hooks: a dock's landing closes its accounting
  (buckets finalized via WP-DOCK-3).
- (F2 compatibility) — `land` per-dock works with cost present OR absent
  (`None` honest); never fabricates.

## Contract deps
- `run_memoized` (`hugit_checks::client::executor`) + `verdict` machinery —
  exists, hermetic.
- Mirror per-push byte-identity verify (`hugit-mirror/verify`) — exists.
- Dock records + reconcile (WP-DOCK-1/2/3); cost samples (WP-DOCK-4).
- `land` current verb (`hugit-cli/src/land/`) — extended, not replaced.

## Claims (paths)
- `crates/hugit-cli/src/dock/land.rs` — per-dock landing (byte-identity +
  acceptance, union verdict).
- `crates/hugit-cli/src/land/mod.rs` — gains `--dock <id>` / per-dock mode.
- `crates/hugit-cli/tests/dock_land_*.rs`.

## Dispatch packet
- This contract + design §6 + ADR-0005 §2.6.
- Anchor: `run_memoized` + verdict API; `land` current semantics; dock records.

## Properties (Lamport-style)

**L6 (safety — verified transport):** IF `land --dock` reports SUCCESS, THEN
the landed commits are byte-identical to what the dock's worktree produced
(fail-closed: any divergence rejected, never "landed" with a mismatch).

**L7 (safety — acceptance gate):** IF the dock's acceptance list has items,
THEN landing requires the acceptance execution to be GREEN via the union
verdict; a RED acceptance never lands that dock (excluded, `queue.union_fail`
bisect semantics reused).

**L8 (safety — no fabrication):** the acceptance execution is REAL (the
existing `run_memoized` machinery) — a dock with no acceptance result is
honest `None`, never "accepted".

**L9 (liveness — settles):** a dock whose acceptance is green and whose
commits are byte-identical is LANDABLE — the verb completes with the landed
refs recorded (or an explicit fixable error).

## Implementation notes (pre-decided)
- 1. Per-dock = filter the land queue by the dock's branch + commits refs.
- 2. Byte-identity: reuse the mirror verify seam against the dock's scratch
  (the SAME check proves the push landed).
- 3. Acceptance: run the dock's accepted items through `run_memoized`; union
  verdict GREEN→land, RED→exclude + bisect (reuse `land` semantics).
- 4. `--dock` without cost data still lands (cost is independent; `None` is
  honest); per-dock billing is closed by reconcile on land (WP-DOCK-3).

## DoD
- Global gate green.
- §6/A4/F2 red→green, hermetic + e2e (real worktrees, byte-mismatch rejected,
  acceptance red excluded, green lands, cost-present and cost-absent both
  correct).
- L6-L9 proven by ≥1 hermetic test each.
- Cold-verify pass by non-author.

## Completeness
All owned items green · zero writes outside claims · evidence bundle attached
to SEAL.

## Return shape (SEAL)
status, evidence refs, deviations.