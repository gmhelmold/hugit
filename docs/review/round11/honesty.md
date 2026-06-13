# Round 11 — Honesty cross-check: docs vs code vs HEAD `5b73a8b`

**Auditor:** fresh-context, read-only, hostile. No code edits, no commits.
**Target:** `main` HEAD `5b73a8b` (Wave O — WO-GLOBMISS + proptest suite + test-quality).
**Scope:** CLAUDE.md status banner · `docs/plan/2026-06-11-pending-seams.md` ·
closed-seam table · envelope handoff docs · family-tense rule · sweep-2026-06-12
false-positive status.

---

## Method

- Cross-checked CLAUDE.md claims against `git log --oneline` and actual HEAD.
- Ran `cargo metadata --no-deps` to verify package count.
- Grepped codebase for claimed symbols (chokepoints, proptest files, verify_chain
  paths, restore_from_bytes CLI wiring).
- Read `docs/review/round11/wedge-stale-green.md` (already committed) to establish
  what Round 11 actually found vs what PS-16 asserts.
- Read all four envelope handoff docs and cross-checked against ADR-0002 and
  `crates/hugit-ledger/src/envelope/`.

---

## Findings

### F-1 — CLAUDE.md: HEAD commit hashes are two merge generations stale (P1)

**Claim (CLAUDE.md lines 103, 126):**
- "main (HEAD `def8a18`, Wave K) is green by the LOCAL gate …"
- "So Wave K + Wave L on `main` (HEAD `3aa62a4`) is green …"

**Truth:** `git log --oneline` shows HEAD is `5b73a8b` (Wave O merge). `def8a18` is
Wave K; `3aa62a4` is Wave L — both two or four merges behind. The test counts
"1200 tests / 135 suites" (Wave K) and "1234 tests / 140 suites" (Wave L) are
likewise stale; the current count is not stated anywhere in CLAUDE.md.

**Severity: P1** — a fresh agent seeded from CLAUDE.md will operate on false HEAD
state, affecting any git-anchored reasoning.

---

### F-2 — CLAUDE.md: Wave O and Round 11 are completely absent (P1)

**Claim (CLAUDE.md status banner, lines 11-20):** The narrative ends at "Wave N
fixed a memo stale-green SHIP-BLOCKER". Zero mentions of Wave O, WO-GLOBMISS,
PS-16, the proptest suite (T-8 closed), Round 11, or commit `5b73a8b`.

**Truth:**
- Wave O (commits `66767bd`→`551dac3`→`67bfaac`→`5b73a8b`) closed the 3rd wedge
  stale-green (WO-GLOBMISS: toolchain config files outside the original glob),
  added `proptest` suites on the security spine (scrubber/chain/memo-key),
  hardened test quality (T-1/T-3/T-5/T-6), and tightened the PS-13 invariant to
  walk `src/**` rather than a hard-coded file list. These are real code changes
  that alter the gate's correctness boundary.
- Round 11 (`docs/review/round11/wedge-stale-green.md`, committed) is a live
  ongoing audit that has already found a 4th P0 stale-green (see F-4).

**Severity: P1** — CLAUDE.md's most-recently-closed stale-green is Wave N's N-1
mode-bit; Wave O's WO-GLOBMISS (a separate stale-green of the same class) is not
reflected. Any agent reasoning about "what's fixed" will be one wave short.

---

### F-3 — Pending-seams.md closed-table: seven rows still say "pending merge to main" (P2)

**Claim (`docs/plan/2026-06-11-pending-seams.md`, lines 696-702):** Every Wave L
entry in the closed-seam table carries "integ/wave-l — **pending merge to main**"
or "integ/wave-l — **pending merge**".

**Truth:** Wave L merged to `main` at `3aa62a4` (2026-06-12). The label is false
for all seven rows — they are merged, not pending.

**Severity: P2** — confusing to a fresh agent but the rows ARE in the closed table
(no false "open" implication). Already identified in the sweep's F-4 (which ran on
HEAD `5457730`) and still not corrected at `5b73a8b`.

---

### F-4 — PS-16 asserts the 3rd-stale-green class is closed; Round 11 proves it is not (P0)

**Claim (`docs/plan/2026-06-11-pending-seams.md` PS-16, lines 649-682):**
> "WO-GLOBMISS (3rd wedge stale-green) — CLOSED (Wave O follow-up). … The three
> wedge stale-greens (env→K-RUN, mode→N-1, config-glob→WO-GLOBMISS) are all one
> class … the class-killing fix is hermetic execution (the P2 corelink-runners
> sandbox seam). Locally we capture the bounded/known axes and disclose the
> unbounded-read residual; it is NOT closable locally by enumeration."

PS-16 frames the remaining local residual as ONLY the "unbounded check reads =
P2 hermetic seam" (files outside `--root`, network, clock).

**Truth (from `docs/review/round11/wedge-stale-green.md`, already committed):**
A 4th stale-green was live-reproduced on `5b73a8b`:

- **Ancestor `.cargo/config.toml` above `--root`** — cargo and rustfmt walk upward
  past `--root` when searching for config files; the per-def glob (`**/…`) is
  `--root`-relative and does not see them. A parent `.cargo/config.toml` change
  (e.g., flipping `rustflags` from `["--cap-lints=allow"]` to `[]`) changes the
  real gate outcome but leaves the memo key unchanged → a warm HIT serves a stale
  GREEN. Live P0 repro in the report (`key f78881ae…` served GREEN after parent
  config changed to FAIL).

The Round 11 report classifies this as the **same bounded, locally-closable class**
as WO-GLOBMISS — cargo's ancestor config search is well-known and deterministic,
not in the "unbounded fixture/outside-root" P2 category that PS-16 uses to declare
the local scope "not closable by enumeration."

**What PS-16 says vs truth:**
| PS-16 claim | Reality at `5b73a8b` |
|---|---|
| Three stale-greens closed (env/mode/in-root config) | Correct for those three |
| Remaining local residual = disclosed P2 hermetic seam (unbounded) | WRONG — a 4th bounded, locally-closable gap exists (ancestor configs) |
| Local scope not closable further by enumeration | WRONG for this class — ancestor walk is bounded and deterministic |

**Severity: P0** — this is the honesty finding with the greatest operational impact.
A fleet that reads PS-16 and CLAUDE.md believes the local stale-green class is
closed modulo an acknowledged P2 seam. It is not. A green memoized verdict can be
served after a parent `.cargo/config.toml` change that would flip the gate to FAIL.

---

### F-5 — CLAUDE.md: test gate counts are stale and partly contradictory (P2)

**Claim (CLAUDE.md lines 103-109):** Two separate "green by the LOCAL gate"
paragraphs cite Wave K (1200/135) and Wave L (1234/140) test counts. No Wave M,
N, or O count is given anywhere.

**Truth:** HEAD is `5b73a8b` (Wave O). The proptest suites alone added three new
test files (`proptest_scrubber.rs`, `proptest_chain.rs`, `proptest_memo_key.rs`).
The stated counts are from commits that are 3+ merge generations old. There is no
single authoritative current gate-green line in CLAUDE.md.

**Severity: P2** — does not mislead about correctness but erodes the "verified by
real bare exit code" credibility signal.

---

### F-6 — Sweep F-3 false positive: "17-package" is CORRECT (confirmed, no overclaim) (INFO)

The 2026-06-12 sweep's docs-truth.md Finding F-3 claimed the workspace has 16
packages and CLAUDE.md's "17-package" count was wrong. PS-15 in pending-seams.md
already records this as a **false positive**: "The N-1/N-6 false-positive corrected:
`cargo metadata --no-deps` shows 17 workspace members."

**Confirmed at `5b73a8b`:** `cargo metadata --no-deps` returns 17 packages,
including `hugit-app-sidecar` (a `path` dependency compiled by cargo, not a
`workspace.members` entry). The CLAUDE.md "17-package" claim is accurate. The
sweep's finding was wrong; PS-15's correction is correct. CLAUDE.md was not
incorrectly changed in response.

---

### F-7 — Envelope handoff docs: consistent with ADR-0002 and code (PASS)

`docs/handoff/2026-06-12-envelope-credential-seam-decision.md` and
`docs/handoff/2026-06-12-to-corelink-runners-envelope-reply.md`:

- Make no production-state claims about corelink-server or corelink-runners code
  (correct per the family-tense rule).
- Reference ADR-0002 §2.2/§4/§6.4 for the "one PAT, no second auth domain"
  decision — consistent with the ADR (checked).
- State "verified at `main` HEAD `5457730`" for the code-check point —
  now stale (HEAD is `5b73a8b`) but the claim is about `hugit-ledger/src/envelope/`
  being a pure producer/redaction module with no credential concept. That is still
  true at `5b73a8b` (checked: no `PAT`/`Bearer`/`lease`/`acquire` in the module).
- No fabricated production-state claims about siblings.

**Severity: LOW** — the code-check cite is one HEAD stale but the structural claim
it verifies is still true.

---

### F-8 — P2/hermetic residual scope: honestly stated except for the F-4 gap (PARTIAL)

PS-8, PS-11 (post-L-C), and PS-16 together honestly scope the P2 hermetic seam:
files outside `--root`, network, clock cannot be captured locally and require the
runner-rootfs sandbox. This is accurate.

**Exception:** as documented in F-4, PS-16 uses the P2 framing to declare the
local scope closed. That framing is false for the ancestor-config class. The P2
seam is honestly scoped; the LOCAL scope is NOT honestly closed.

---

## What is accurately stated

- The structural chokepoint PS-13 claim is true: `checks::rehydrate_and_verify`
  exists at `crates/hugit-cli/src/checks/mod.rs:411`; `load_event_log` at 439;
  the build-failing invariant test `canonical_log_loaders_route_through_the_chokepoint`
  is at `crates/hugit-cli/tests/acceptance_wave_m_readpath.rs:273`. Code confirms.
- `hugit why` verifies the chain: `main.rs:159` calls `hugit_refstore::verify_chain`
  on the embedded records before projecting (K-CHAIN claim is true).
- `export::restore_from_bytes` is NOT CLI-wired (no `restore` subcommand in
  `main.rs`); the PS-13 "dead, R10-2 F-2" note is accurate.
- Proptest suites exist at HEAD: `proptest_scrubber.rs`, `proptest_chain.rs`,
  `proptest_memo_key.rs` — T-8 closed claim is true.
- The WO-GLOBMISS fix is real: `builtin_glob_set` in `checks/run.rs` correctly
  adds `**/rustfmt.toml`, `**/clippy.toml`, `**/.cargo/config.toml`,
  `**/rust-toolchain.toml` etc. per-def. Re-confirmed by Round 11 report (in-root
  config changes do bust the key).
- The 17-package count is correct per `cargo metadata --no-deps`.
- Envelope docs are consistent with ADR-0002 and code; no family-tense violations.
- PS-2 through PS-10 open/deferred seams: code matches (D14 guard, cold-store erase
  gap, queue verdict null, etc.) — not re-verified individually but no evidence of
  false "closed" in the open section.

---

## Verdict table

| ID | Severity | Claim | Truth |
|---|---|---|---|
| F-1 | P1 | HEAD is `def8a18`/`3aa62a4` | HEAD is `5b73a8b` (Wave O) |
| F-2 | P1 | Wave O and Round 11 absent from CLAUDE.md | Both exist and contain real code + a live P0 |
| F-3 | P2 | Closed-table rows "pending merge" | Merged at `3aa62a4` |
| **F-4** | **P0** | PS-16: local stale-green class closed (only P2 residual) | 4th bounded stale-green live-reproduced; ancestor-dir configs are locally-closable |
| F-5 | P2 | Test counts 1200/135 + 1234/140 | 3+ merge generations stale; Wave O added proptest suites |
| F-6 | INFO | Sweep said "16 packages" | CLAUDE.md "17" is correct; false positive |
| F-7 | LOW | Envelope code cite at `5457730` | HEAD is `5b73a8b`; structural claim still true |
| F-8 | PARTIAL | P2/hermetic residual honestly scoped | True except F-4 gap conflated into P2 framing |

---

## Overall verdict

**YES — overclaim present. The worst single finding is F-4 (P0):**

PS-16 in `docs/plan/2026-06-11-pending-seams.md` asserts the local stale-green
class is closed (only the disclosed P2 hermetic seam remains). This is false:
`docs/review/round11/wedge-stale-green.md` live-reproduced a 4th gap in the same
bounded-and-locally-closable class — ancestor `.cargo/config.toml` / `rustfmt.toml`
above `--root`. The framing that collapses this into "the unbounded P2 seam" is the
honesty drift; it would cause a fresh agent to conclude the wedge is locally safe
when it is not.

Secondary: CLAUDE.md's status banner is two merge waves behind HEAD (F-1/F-2),
meaning any agent seeded from it will operate on a false picture of what rounds
have run and what code changes are at HEAD.

The P2/hermetic residual itself is honestly scoped for its own domain; the error is
claiming that domain is exhaustive of the local residual.

**Actions implied (not taken here — read-only):**
1. PS-16 needs a "4th stale-green OPEN" addendum acknowledging the ancestor-config
   gap as locally-closable.
2. CLAUDE.md status banner needs updating to Wave O + Round 11 + HEAD `5b73a8b`.
3. Closed-seam table "pending merge" labels need removal.
