# Docs-truth sweep — 2026-06-12

**Target:** `main` HEAD `5457730` (Wave M, post-Round-10).
**Scope:** docs/comments vs code truth + dead weight — CLAUDE.md, module-level
`//!` headers, `docs/plan/2026-06-11-pending-seams.md`, `docs/interop.md`,
inline code comments. Cross-checked a representative sample of strong claims
against the actual source. Read-only.

---

## Findings

### F-1 — CLAUDE.md status banner contradicts the body (P1 — false state claim)

**Severity:** P1  
**File:line:** `CLAUDE.md:11-14`  
**Claim vs truth:**  
The four-line status banner reads:

> "Round 8 = the SEVERE class sweep RAN and Wave L remediated all 6 classes
> on branch `integ/wave-l` — Round 9 re-audit + merge PENDING, push HELD"

The banner was stamped at the Wave-L + Round-9 milestone and never updated.
The body of the same file (lines 68-95) narrates the full truth:

- Round 9 converged 5/6 on first pass; the 6th (PS-14) was owner-decided.
- `integ/wave-l` was merged to `main` (HEAD `3aa62a4`).
- Wave M followed (PS-13 · R9-3 · C5-F3), merged to `main` (HEAD `5457730`).
- Round 10 confirmed 4/4 CONVERGED, zero DO-NOT-SHIP.

Any reader who parses only the status banner gets a state that is at least two
merge generations behind reality.  
**Fix:** Update the banner to reflect Wave M on `main` (HEAD `5457730`) and
Round 10 4/4 CONVERGED.

---

### F-2 — CLAUDE.md cites two stale HEAD commit hashes (P1 — false document)

**Severity:** P1  
**File:line:** `CLAUDE.md:98` and `CLAUDE.md:121`  
**Claim vs truth:**

- Line 98: "`main` (HEAD `def8a18`, Wave K) is green by the LOCAL gate …"
- Line 121: "So Wave K + Wave L on `main` (HEAD `3aa62a4`) is green …"

Neither hash is HEAD. Actual HEAD is `5457730` (Wave M merge commit, per `git
log -1`). Both fragments describe historical states that are now superseded. The
Wave K paragraph (lines 98-100) still leads with "1200 tests / 135 suites" as
if those are the current counts; the Wave L paragraph (lines 101-105) claims
"1234 tests / 140 suites". No Wave M test count is stated anywhere in the doc.  
**Fix:** Consolidate the gate-green record to a single current paragraph citing
HEAD `5457730` (Wave M), remove or clearly date the Wave K / Wave L entries as
historical.

---

### F-3 — CLAUDE.md "17-package" workspace count is wrong (P2 — stale count)

**Severity:** P2  
**File:line:** `CLAUDE.md:16-17`  
**Claim vs truth:**

> "**17-package** Rust workspace (hugit-app + {ui,exit,sidecar} sub-crates = 4
> crates + 13 feature crates — verified by `cargo metadata --no-deps` 2026-06-11"

Actual workspace members (`Cargo.toml:2`):

```
members = ["crates/*", "crates/hugit-app/ui", "crates/hugit-app/exit", "crates/hugit-dogfood"]
```

- `crates/*` expands to 14 directories.
- `ui` and `exit` add 2 more (`hugit-app-ui`, `hugit-app-exit`).
- `hugit-dogfood` is already in `crates/*`; the explicit entry is a no-op.
- `hugit-app/sidecar` has a `Cargo.toml` (and is a `path` dependency of
  `hugit-app`) but has **never appeared in the workspace `members` list** — it
  compiles as a transitive path dep, not as a standalone workspace member.

`cargo metadata --no-deps` counts workspace members: **16**, not 17.

The 17-count was accurate before `hugit-web` migrated out (when `hugit-web` was
still in `crates/`, giving 15 + ui + exit = 17). The migration removed
`hugit-web` but the count was only corrected from 18 → 17 in CHANGELOG
discussions, missing the additional correction to 16 because sidecar was
conflated with ui/exit as a workspace member when it never was.

The sub-crate tally is also wrong: `{ui,exit,sidecar} = 4 crates` is only 3
names, and sidecar is not a workspace member, so `hugit-app` family = 3
workspace packages (hugit-app + ui + exit), not 4.  
**Fix:** Correct to "16-package"; correct the sub-crate tally to
"hugit-app + {ui,exit} sub-crates = 3 workspace packages + 13 feature crates".

---

### F-4 — pending-seams.md closed-defects table: 7 rows still say "pending merge" (P2 — stale)

**Severity:** P2  
**File:line:** `docs/plan/2026-06-11-pending-seams.md:631-637`  
**Claim vs truth:**

Every Wave L row in the "CLOSED DEFECTS" table carries the label
"`integ/wave-l` — **pending merge to main**" or "`integ/wave-l` —
**pending merge**". Wave L merged at commit `3aa62a4`; Wave M then merged at
`5457730`. The "pending" qualifier is false for every row.  
**Fix:** Replace "pending merge to main" / "pending merge" with "merged to
`main` (`3aa62a4`, 2026-06-12)".

---

### F-5 — pending-seams.md PS-13 body: "TRUE on integ/wave-l for all five current loaders" (P2 — stale branch ref)

**Severity:** P2  
**File:line:** `docs/plan/2026-06-11-pending-seams.md:543`  
**Claim vs truth:**

The PS-13 section (marked RESOLVED) says: "This is TRUE on `integ/wave-l` for
all five current loaders". The branch is merged; the correct referent is `main`
(HEAD `5457730`). Minor history-context confusion, since PS-13 is labelled
RESOLVED, but a branch name in a RESOLVED section should cite the merge commit,
not the pre-merge branch.  
**Fix:** Update to "TRUE on `main` (merged `3aa62a4`, then `5457730`)".

---

### F-6 — CLAUDE.md "adversarial rounds 1–7 closed" in the status banner omits Rounds 8-10 (P2 — incomplete enumeration)

**Severity:** P2  
**File:line:** `CLAUDE.md:12`  
**Claim vs truth:**

The status banner says "adversarial rounds 1–7 closed". The body (and
`docs/review/round8/`, `round9/`, `round10/`) documents Rounds 8, 9, and 10
all completed. The banner should say "rounds 1–10 closed" or equivalent.  
**Fix:** Part of the F-1 banner update; explicitly list rounds 1–10.

---

## TODO / FIXME / HACK / XXX

- **0 actionable code markers** found across all `crates/**/*.rs`.
- The only `XXX`-like string in production code is `\uXXXX` in
  `crates/hugit-proto/src/write/mod.rs:30` — a Unicode-escape notation in an
  English doc comment, not a placeholder.

---

## Dead code

- **`#[allow(dead_code)]`:** 2 instances, both in test scaffolding
  (`crates/hugit-queue/tests/negative_scope/mod.rs:13,28`). Acceptable in test
  helpers; no production dead-code suppression.
- **`hugit-app-sidecar`:** The sidecar crate lives at `crates/hugit-app/sidecar/`
  and is a `path` dependency of `hugit-app`; it is NOT a workspace member but IS
  compiled. Not dead — just not independently testable via `cargo test -p
  hugit-app-sidecar`. The CLAUDE.md over-count (F-3) stems from conflating this
  with workspace membership.
- **`restore_from_bytes`:** Mentioned in earlier audit notes as a "not-wired"
  path. It IS wired and tested (`crates/hugit-cli/src/export/mod.rs:630`;
  `tests/acceptance_e5.rs:593`). No dead code.
- **Commented-out code:** None found. All `// …` comment blocks in production
  source are explanatory prose, not dead code.

---

## What is accurately documented

- All module-level `//!` headers match the actual source structure: attention is
  correctly placed under `#[path = "../attention/mod.rs"]`; ledger/watch/fleet
  modules exist; broker/seam in hugit-fence; bisect/experiment/flake in
  hugit-diag.
- `append` is `pub(crate)`, `append_authorized` is `pub`, `append_for_test` is
  `#[cfg(feature="test-support")]` — matches the Wave L L-D claim exactly
  (`crates/hugit-refstore/src/log/mod.rs:386,432,473`).
- `checks::rehydrate_and_verify` exists at line 411, `load_event_log` at 439
  — matches the Round-10 and PS-13 RESOLVED documentation.
- `AuditedGuard` and `append_authorized` exist in `hugit-refstore/authz/mod.rs`
  — matches PS-2 / D14 claims.
- `secret_shape.rs` exists as the unified scrub-primitive source for R9-3
  (`crates/hugit-ledger/src/secret_shape.rs`) — matches Wave M / M-2 claim.
- PS-12, PS-13, PS-14 are marked RESOLVED/DECIDED in pending-seams.md with
  accurate resolution notes.
- `interop.md` accurately reflects the runner transfer, v1.2.0 contract
  amendment, and the runner-box interim transport.
- `docs/review/round8/`, `round9/`, `round10/` exist and match the CLAUDE.md
  narrative.
- Sidecar, ui, exit sub-crate relationship to hugit-app is materially accurate
  except for the workspace-member count (F-3).

---

## Summary

| Severity | Count | Key finding |
|---|---|---|
| P1 | 2 | Status banner says merge PENDING; HEAD hashes are stale (Wave K/L cited, Wave M is HEAD) |
| P2 | 4 | Package count 17 vs actual 16; "pending merge" labels in closed-defects table; stale branch ref in PS-13; rounds 1–7 vs 1–10 |
| P3 | 0 | — |
| TODO/FIXME | 0 | No actionable markers |
| `#[allow(dead_code)]` | 2 | Test scaffolding only |
| Commented-out code | 0 | — |

**Worst single finding:** `CLAUDE.md:11-14` status banner asserts merge PENDING
and push HELD, contradicting the body (which records the merge) and git log
(HEAD `5457730`, Wave M post-Round-10). Any agent seeded from that banner
operates on a false premise about the repo's integration state.
