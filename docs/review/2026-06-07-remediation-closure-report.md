# Remediation closure report — hugit (2026-06-07)

Sequel to `2026-06-07-brutal-code-review.md`. That review found that **green gates
reflected oracle color, not invariant truth** — 11 CRITICAL / 19 HIGH across four
systemic root-causes. The owner's mandate: *"address everything, literally
everything, zero debt, only SOTA."* This report records the remediation and its
independent closure verification.

## Method

Oracle-first, per the review's own prescription: for each finding, **strengthen
the acceptance oracle until it goes RED on current `main` (proving the bug at the
gate), then fix the implementation until it is genuinely GREEN.** Every fix
shipped as a small branch → PR → CI-green-verified merge. After all fixes landed,
a **second adversarial pass** (14 re-reviewers, one per crate) verified closure
independently — confirming the real production path is fixed, not just that an
oracle passes, and hunting for regressions the remediation itself introduced.

## What was fixed (16 remediation work-packages + closure fixes)

**Root-cause R1 — frozen hash formula single-sourced.** The canonical
`hugit_refstore::log::compute_this_hash` is now the one byte-exact implementation
(u32 count-prefixed vector framing, genesis 64×'0', canonical-JSON payload,
lowercase-hex memo_key); contracts docs match it byte-for-byte; an independent
hash-pin in the golden tests catches drift. policy, diag, checks were routed to
it. **Closure pass caught R1 reborn** in three more emitters
(app/webhook, app/exit, mirror/import) that had re-transcribed the formula without
the count-prefix — all subsequently routed to the canonical fn; **zero divergent
`compute_event_hash` remain in the tree.**

**Root-cause R2 — verified wrappers wired into the real surface.**
- `hugit-queue` (the wedge): "exclude the failing pair, the rest proceeds" now
  works **end-to-end** through the real driver (was unimplemented — a red union
  stalled everything behind it forever).
- `hugit-proto`: `receive_pack` now routes through the flag-gate + `SerializedWriter`
  compare-and-append (flag-off refuses; concurrent pushes get stale-rejection).
- `hugit-runner`: X4 pin enforced on the **real** spawn surface (every `docker
  run` gated by parse+verify), proven by a hermetic bare-gate oracle.
- `hugit-fence`: `check_access` wired as the live admission gate; a real
  fence-escape redteam (hermetic, bare-gate, load-bearing).

**Root-cause R3 — gamed oracles replaced with real ones.** app uninstall does
real stateful revocation; mirror's one-way invariant is now **structural**
(a `MirrorMutation` private-field type makes mirror→forge unrepresentable, plus an
architecture-scan oracle); mirror's 1k-commit import oracle actually imports
(real git fast-import → `import_commits` → `git rev-list` byte-compare); X4's
fail-closed-before-spawn proof and X5's no-shadow law run in the bare gate against
real surfaces; checks signs attestations with real ed25519 + verifies.

**Root-cause R4 — fail-open / silent-failure closed.** runner `tmp_root` sanitized
(was root RCE); cli OID sanitized (was path traversal); app empty-webhook-secret
rejected (was forgeable); mirror InstallationToken Debug redacted (was cleartext);
refstore backpressure bound holds under contention (semaphore); refstore `undo`
folds `intent.landed` (correct compensator) and recovery splices the hot tail.

## Closure verification — 14 adversarial re-reviewers

| Crate | Verdict |
|---|---|
| hugit-queue | ✅ wedge genuinely closed (pair-proceed e2e through real driver) |
| hugit-proto | ✅ flag-gate + CAS wired into real `receive_pack`; oracles RED-on-revert |
| hugit-mirror | ✅ one-way type-level; real 1k import; token redacted |
| hugit-runner | ✅ X4 on real surface + hermetic proof; tamper still fails closed |
| hugit-invariants | ✅ X2 ed25519 genuine; X4 ordering load-bearing in bare gate; X5 live registry |
| hugit-contracts + refstore | ✅ R0 single-sourced; backpressure + undo/recovery; d1c **strengthened** |
| hugit-policy | ✅ canonical hash (recompute oracle); DCO parent-count; gates fixed |
| hugit-checks | ✅ real ed25519 attestation; deterministic regen; anti-smuggle |
| hugit-ledger | ✅ full view-redaction coverage; malformed surfaced |
| hugit-app | ✅ after closure fix (uninstall revoke + R1 hash routed) |
| hugit-diag | ✅ after closure fix (canonical hash + real recorded_at) |
| hugit-cli | ✅ after closure fix (real bin; HUGIT_VERBS = live surface) |
| hugit-fence | ✅ after closure fix (broker traversal guard + hermetic redteam) |

The re-review found **5 residual gaps the first remediation missed** — most
importantly R1 reborn in three emitters, and `diag.recorded_at` fixed in
signature but not at its call sites. All five were closed in a final fix wave and
re-verified. This is the report's central lesson: **closure must be verified by an
independent adversary, not assumed from a green gate** — the same failure mode the
original review exposed, caught one level up.

## CI/process hardening (discovered during integration)

- **Merge automation now gates on CI conclusion=success**, not "checks no longer
  pending" — the old loop merged red checks (it landed an fmt violation and two
  flaky tests before this was fixed; all corrected).
- **Real-git/concurrency tests hardened for CI**: the honest "use real git, not
  mocks" fixes introduced env-sensitive flakes — a `git gc --auto` race
  (fixed via atomic `fast-import` + `fsck` + neutralized ambient config), a
  saturation-deadline flake, and a non-deterministic backpressure test (made
  deterministic via permit-acquisition rendezvous). main CI is now reliably green.
- **D1c admit/lock rendezvous deadlock closed** (caught by the *final* cold-verify,
  which wedged for ~4h on `acceptance_d1c::admit_lock_split_*`). A stack sample
  proved a test-only liveness race: the test released the pinned writer gate the
  instant it observed `CAPACITY` admit signals, without waiting for the `SURPLUS`
  submitters to be refused. A straggler surplus thread reaching the gate *after*
  the admitted ops drained their permits was wrongly admitted into an admission
  hook with no release left and blocked forever on `recv()`. Production is correct
  (`try_acquire` is non-blocking, the bound is atomic) — the bug was purely the
  test's choreography. Fixed with a **surplus-rejection rendezvous**: main now
  awaits `CAPACITY` admits **and** `SURPLUS` refusals before sampling/releasing, so
  no straggler can still be en route to the gate. Proven with 45 watchdog'd runs.
- **Stale `Cargo.lock` made a hard gate.** The final `--locked` verify revealed the
  committed lock was missing `clap` and its transitive deps (the cli `[[bin]] hugit`
  added in RWP-cli) — dropped during a lock-conflict integration. CI ran plain
  `cargo test --workspace` (no `--locked`), so cargo silently regenerated the lock
  at build time and the staleness stayed invisible. The lock is regenerated, and CI
  `clippy`/`test` now run with `--locked` so a manifest/lock divergence fails the
  gate going forward.

## Known residual (documented, not debt-by-omission)

- **X3③ context-store purge** remains a disclosed PARTIAL: there is no production
  context-store erasure surface to drive yet (that is WP-X7's domain). Flagged
  explicitly, not green-washed.
- Minor LOW/by-design notes (marker-driven redaction scope, grep-oracle coverage
  backstopped by type-level guarantees, an unexercised trait→live-GitHub seam in
  the queue) are recorded in `.techlead/state/closure-verification.md`.

## State

`main` green: `cargo fmt --check`, `clippy --workspace --all-targets -D warnings`,
`cargo test --workspace`, `cargo audit` all pass; every implemented WP's
acceptance suite green; remediation findings closed and independently re-verified.
The remaining open work is the P2-gated WP set (CoreLink tenant) — unchanged.
