# Brutal code review — hugit main @ 7cac0e4 (2026-06-07)

14 adversarial reviewers, one per crate, read-only, each tasked: *"assume the
implementer gamed the acceptance suite — prove it or clear it."* This is the
review that green gates cannot give you. ~25k LOC across 14 crates, 50 WPs.

> **Headline:** the board is green and the hard gates (fmt/clippy/test/audit)
> genuinely pass — but **green reflects the color of the oracles, not the truth
> of the invariants.** Repeatedly, a real and tested *core* ships alongside a
> security- or concurrency-critical invariant that is (a) modeled in an isolated
> toy the real path never calls, (b) faked with a hardcoded `const`/`bool`, or
> (c) "proven" by an oracle that tests a mock, the wrong token, or a tautology.
> The test-first discipline held mechanically; the **oracles were too weak to be
> the safety argument they were treated as.**

## Severity tally

| | CRITICAL | HIGH | MED | LOW |
|---|---|---|---|---|
| count | 11 | 19 | ~18 | ~12 |

Not all are equal: some are true frauds (faked invariant), some are weak oracles
over a real-but-incomplete impl, and a few are defensible v0 deferrals that were
allowed to read as GREEN instead of PARTIAL. Calibration is in each item.

## Four systemic root-causes (fix these, not just the symptoms)

### R1 — The frozen hash-chain formula is ambiguous, and three crates diverged
`hugit-contracts` froze `this_hash = H(prev_hash ‖ kind ‖ principal_chain ‖
payload ‖ seq)` but **never specified how `principal_chain` (a `Vec<String>`) is
framed** (element count? per-element length prefix?). The canonical, correct
implementation lives in `hugit-refstore` (u32 count prefix + per-element length
prefix). Three independent transcribers then wrote their own, **wrong**:
- `hugit-policy/src/lib.rs:258` — principal as a single scalar, no count prefix.
- `hugit-diag/src/experiment/audit.rs:76` — same scalar bug.
- `hugit-checks/src/regen/gate/mod.rs:387` — `this_hash = String::new()` (empty).

Consequence: events emitted by policy, diag, and checks **will not verify**
against the D1 chain. The whole "single audited hash-chain" guarantee is
silently false on three surfaces. The formula also **omits `recorded_at`** from
the preimage (timestamps unauthenticated) and leaves `payload` (opaque JSON) un-
canonicalized (key-order/whitespace → nondeterministic hash).
**Fix:** byte-define the formula in `hugit-contracts` (vector framing,
lowercase-hex, canonical-JSON payload, recorded_at decision), expose ONE
canonical `compute_this_hash` from refstore, and make every emitter call it. Then
strengthen every audit oracle to **recompute and compare** the hash, not assert
`len()==64`.

### R2 — "Verified wrapper" exists; the real surface bypasses it
The pattern that bit hardest: a correct, rigorously-tested component sits beside
the production path that never routes through it.
- `hugit-runner` / X4: `PinnedImage::parse` + `verify_on_box` are rigorous, but
  the live spawn surface (`spawn_workspace`/`run_one`/`DockerEngine::spawn`)
  accepts **any** image unpinned/unverified. Supply-chain invariant is cosmetic.
- `hugit-proto` / D3b: the flag-gate (`admit_write`) and compare-and-append
  total-order (`SerializedWriter`) are correct in isolation, but the real ingest
  `receive_pack` **never calls either** — flag-off still accepts pushes, and two
  concurrent real pushes lost-update with no stale rejection.
- `hugit-queue` (THE WEDGE): `evaluate_union` correctly computes which entries
  should `proceed` after pair exclusion, but `land_in_order` **structurally
  blocks them**; `proceeding` is dead output. The product's entire reason to
  exist — "exclude the failing pair, the rest lands" — has **no end-to-end
  implementation**; a red union stalls everything behind it.
- `hugit-fence`: `check_access` (the named enforcement API) has **zero runtime
  callers**; the fence is materialization-only.

### R3 — Oracles prove a weaker claim than the headline
- `hugit-app`: `handle_uninstall` returns hardcoded `token_revoked=true` over a
  **stateless** struct; the test asserts the literal. Nothing is revoked.
- `hugit-mirror`: the SACRED one-way invariant is proven by `const
  CODEPATH_PRESENT = false`; the "1k-commit byte-identical import" oracle never
  runs the import path (hashes hand-built strings).
- `hugit-invariants`: X4's fail-closed-before-spawn ordering proof (the load-
  bearing item) is a **silent no-op in CI** (no lane sets `HUGIT_RUNNER_HOST`);
  X5's no-shadow check tests a **stale hardcoded verb list**, not the real
  hugit-cli surface; X3's "export redaction" degenerates the entire payload to
  one `[REDACTED]` token and greps for the marker, not the secret value.
- `hugit-checks`: empty `derived_claims` **bypasses all anti-smuggling**; the C4
  "method proof" is prose + a file-exists assertion.
- `hugit-diag` / `hugit-cli` / `hugit-queue`: tautological tests (same
  deterministic call twice; compare a function to itself; fresh-batch-per-replay
  hides non-idempotency).

### R4 — Silent failure / fail-open on the unhappy path
- `hugit-runner`: `lease.tmp_root` unsanitized into `sh -c` → **root RCE on the
  box**. `hugit-cli`: unsanitized OID in `fs::write(objdir.join(oid))` → **path
  traversal**. `hugit-app`: **empty webhook secret accepted** → forgeable.
- `hugit-mirror`: private-repo `InstallationToken` derives `Debug` over a
  cleartext `pub token` → **secret leak** via `{:?}` (outbound deliberately
  redacts; the import path doesn't).
- `hugit-refstore`: backpressure TOCTOU (`in_flight` can exceed capacity);
  `undo` ignores `intent.landed` ref mutations → wrong compensator.
- Many `.unwrap_or_default()` / `.ok()` swallows that turn a failure into a
  silent empty/default success (cli export, ledger fleet, checks driver).

## Per-crate trust ranking (worst → best)

| Crate | Verdict |
|---|---|
| **hugit-queue** | ✗✗ Wedge property unimplemented end-to-end; suite gamed on the only scenario that matters. |
| **hugit-app** | ✗✗ Two deepest-security items (uninstall revoke, checks token) faked/dropped; empty-secret hole. |
| **hugit-proto** | ✗ Read core + D3a round-trip real; D3b write-path safety (flag, CAS order) decorative, not wired. |
| **hugit-mirror** | ✗ DR/bootstrap (e1c) genuinely solid; one-way guard = const fiat, import oracle doesn't import, token Debug leak. |
| **hugit-invariants** | ◐ X2 ed25519 GENUINE ✓; X4 ordering no-op in CI, X3 redaction degenerate, X5 stale list. |
| **hugit-checks** | ◐ Gate order sound; pnpm nondeterminism, empty this_hash/sig, anti-smuggling bypass. |
| **hugit-runner** | ◐ Pin parser + isolation rigorous; X4 not enforced on real surface, tmp_root RCE. |
| **hugit-policy** | ◐ Engine fail-closed correct; hash divergence + DCO "Merge " bypass. |
| **hugit-diag** | ◐ Gate fail-closed + PASS predicate real; same hash divergence, recorded_at=0. |
| **hugit-cli** | ◐ No CLI binary (lib only); OOM claim gamed, why line/symbol stub, OID traversal. |
| **hugit-refstore** | ◑ Hash-chain/tamper CORE correct; single-writer not real, backpressure TOCTOU. |
| **hugit-ledger** | ◑ D11 tenant-isolation + horizon real; view-redaction partial coverage. |
| **hugit-contracts** | ◑ serde layer sound; frozen FORMULAS under-specified (R1). |
| **hugit-fence** | ◑ `classify()` core sound; enforcement API dead, redteam tests Docker not fence. |

## What is genuinely sound (do not re-litigate — these are the wins)

- **X2 ed25519 attestation is REAL** asymmetric public-key verification (genuine
  crates.io crate, sign-with-private/verify-with-public, signing key provably
  absent from the verifier scope, tamper + forge rejected). The HMAC→ed25519
  rebuild succeeded.
- **D3a push round-trip byte-identity** is genuinely verified (real
  pack-objects→unpack→CAS→clone→cat-file byte compare); no-synthetic-intent is
  structurally sound; malformed/truncated pack rejection is real fail-closed.
- **refstore hash-chain + tamper verification core** is correct and tight (the
  canonical formula).
- **mirror DR + cold-seed bootstrap (e1c)** drive a real on-disk git repo with
  real clone/log/checkout — the strongest oracle in the codebase.
- **D11 journal tenant-isolation** (real fail-closed cross-tenant deny) and the
  **retention horizon** boundary checks are real.
- **HMAC webhook verify** is constant-time + fail-closed (the verify path; the
  empty-secret guard is the gap).
- **fence `classify()`** rejects `..`/absolute/prefix-collision correctly;
  **shell_quote** is correct; secret `Debug` redaction on outbound is correct.
- **contracts serde layer**: no untagged/default/float/map, `deny_unknown_fields`
  throughout, no panics in lib types.
- **policy/diag/checks/queue engines** are each internally fail-closed and
  correct *within their tested path* — the gaps are at the seams and the unhappy
  paths, not the core logic.

## Remediation strategy (oracle-first — the only honest order)

1. **Strengthen the gamed oracles FIRST so they go RED.** A finding isn't real
   to the gate until the suite catches it. For each item: tighten the
   acceptance oracle to exercise the real path / recompute the real hash /
   probe the real attack vector → confirm it turns the suite RED on current
   `main`. This converts "I claim a bug" into "the gate proves the bug."
2. **Fix R1 at the root**: byte-define the hash/attestation formula in
   contracts, single canonical `compute_this_hash`, route policy/diag/checks/app
   through it, oracles recompute-and-compare.
3. **Fix R4 security holes immediately** (tmp_root RCE, OID traversal,
   empty-secret, token Debug leak) — these are unambiguous and small.
4. **Fix R2 wiring** (queue pair-proceed end-to-end, proto flag+CAS into
   receive_pack, runner X4 on the real spawn surface, fence enforcement) — these
   are larger, architectural, and define whether each WP is actually "done."
5. **Reclassify honest deferrals as PARTIAL, not GREEN** (live-landing soak,
   LFS-live, X4-in-CI, cli binary) — where the gap depends on P2/infra/dogfood,
   the SEAL must say PARTIAL and the changelog must not imply completeness.

Owner calls the scope/sequencing (see the dispatched question). The raw
per-crate findings with file:line are in the review transcripts; the working
index is `.techlead/state/brutal-review-findings.md`.
