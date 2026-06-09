# Roadmap-gap build campaign — hugit (2026-06-07)

Measured implementation against the source of truth — the **whitepaper**
(`docs/whitepaper/hugit-v1.md`) and the **67-WP decomposition v2.0**
(`docs/plan/decomposition.md` + `docs/plan/wp-contracts/`) — not against "the
remediation campaign". The cross-check exposed that the prior work had closed the
*review findings* on what was built, but a real slice of the **spec'd product was
never built** — most starkly Squad X (platform invariants): only 4 of 14.

## Method
A 16-agent assess fleet read every missing WP's contract + target crate and
classified each as buildable-now vs P2-blocked, with an oracle-first plan. Then
builders (one WP each, worktree-isolated, opus for design/security, sonnet for
deterministic builds) authored a failing acceptance suite encoding the VERBATIM
owned items, implemented on the **real production surface**, and proved closure.
Integration was serial/batched with the inherited discipline: scope-check, union
the shared files (CHANGELOG, `hugit-invariants/Cargo.toml` `[[test]]` blocks,
`lib.rs` `pub mod`), `cargo metadata --locked`, merge **only on CI
conclusion=success**, verify-landed, prune worktrees to recycle disk.

Every oracle is **mutation-verified**: defeating the invariant (e.g. making the
tenant namespace tenant-independent, dropping the non-determinism threshold 3→2,
forcing boot to always-serve, silently re-linking a tombstone) turns the oracle
RED — proof it is load-bearing, not gamed.

## Built this campaign (13 WPs, all merged, CI green incl. audit)
| WP | Crate | What it closes |
|---|---|---|
| **B2a** | hugit-checks | The WEDGE: three-axis memo key + sensitivity; AC HIT → 0 exec <500ms (live AC HTTP = P2 seam) |
| **B2b** | hugit-checks | runner-side byte-identity (content-digest), non-determinism after 3, honest partial hit-rate (live box = P2 seam) |
| **C3** | hugit-runner | cache-warm boot: warm/cold structural proof + CAS/AC-down fail-closed (box timing = P2 seam) |
| **C6** | hugit-diag | flake-stats + quarantine (annotation-only, auto-act uninhabited) + false-positive guard |
| **X1** | hugit-invariants | tenant isolation red-team (cross-tenant deny, no poison, no side-channel) |
| **X6** | hugit-invariants | intra-fabric non-interference (resource isolation; live CoreLink measure = P2 seam) |
| **X7** | hugit-invariants | right-to-erasure cascade — **closes the X3③ purge PARTIAL** (5-store, no orphans, attestation re-seal-or-fail-closed, erasure×seal precedence) |
| **X8** | hugit-invariants | self-release attestation (ed25519 + transparency log + boot self-verify; live Rekor = P2 seam) |
| **X9** | hugit-invariants | cross-phase object identity (CheckResult B↔D byte-identical; intent_id one-id-one-lifecycle) |
| **X10** | hugit-invariants | focus gate (corelink-server enrollment fails the build; consumption cap; live measure = P2 seam) |
| **X12** | hugit-invariants | erasure × provenance × mirror (tamper-evident tombstone, mirror obligation in exit proof) |
| **X13** | hugit-invariants | legibility × degradation/erasure (down-zoom via plain git or honest failure; chains end in tombstones) |
| **X14** | hugit-invariants | deep-link integrity across lifecycle (compaction → mirror → tombstone; zero dangling) |

PRs: #98 (X12) · #99 (X6) · #100 (B2a+C6+C3) · #101 (X1/X7/X8/X9/X10/X13/X14) ·
#102 (B2b). main @ `1541baf`, CI success including `cargo audit`.

## Roadmap coverage now
- Day 0: 2/2 · Squad B: 10/12 · Squad C: 11/12 · Squad D: 18/18 · Squad E: 8/9
  (E6 deferred by design) · **Squad X: 11/14** (was 4/14).
- **63 of 67 WPs built.** The 4 not built are all gated, none by omission:
  - **E6** bidirectional mirror — explicit gate-bound deferral (months of one-way
    soak first; acceptance pre-registered). By design, per decomposition §5.
  - **B5** auto-bisect — needs the QueueApi prod auto-trigger seam (P2).
  - **B8** dogfood harness — needs a real PR wave + 48h soak (P2/live).
  - **X11** degradation composition — needs live mid-operation broker fault
    injection on the runner box (P2).

## P2 seams (disclosed, not debt)
Several built WPs prove their logic hermetically and defer only the live-infra
wiring behind a trait/env gate that **fail-not-skips when the env is set** and
asserts "not wired" in the bare gate so it cannot rot to green: B2a (CoreLink AC
HTTP), B2b/C3/X11 (runner box `HUGIT_RUNNER_HOST`), X6/X10 (live CoreLink
measurement), X8 (public transparency log). These close when the P2 CoreLink
tenant + box are provisioned (owner-gated, task #7).

## Update — 2026-06-08: B5/B8/X11 hermetic builds + AC seam + hygiene
The owner directed "do all of it" with CI gated locally (GitHub Actions quota
exhausted). A second wave built the hermetic portions of the three previously
"P2-blocked" WPs (partial-over-fake: prove the logic now, defer only live infra)
plus the wedge's transport seam and a hygiene pass:
- **B5** auto-bisect over memoized checks + DiagnosisObject (culprit ≤log₂ execs,
  bounded diagnosis, auto-trigger on red) — QueueApi prod auto-trigger = seam.
- **B8** dogfood harness — new `crates/hugit-dogfood`: real in-process 5-PR wave
  e2e + memoization-off baseline (versioned report) + soak invariant harness;
  corelink-server excluded. 48h wall-clock soak + live install = seam.
- **X11** degradation composition — broker fail-closed mid-op (FakeBox), objects
  provenance-absent with no fabricated intent/attestation, X10 holds while
  degraded; mutation-verified. Live box fault injection = seam.
- **B2a AC seam** — `HttpAcClient` wired to CoreLink's `{GET,PUT} /v1/ac/{tenant}/
  {action_digest}` + Bearer PAT behind a mockable transport (content-address
  guard; fail-closed when unconfigured); live network = seam.
- **Hygiene pass** — misleading comments, a dead-code test promoted to run,
  orphaned fence exports, vacuous fleet-validation checks removed (behavior-
  preserving).

**Coverage now: 66 of 67 WPs built** (only E6 remains, deferred by design). Every
WP except E6 has its logic on main; all that's left is flipping the disclosed P2
live-infra seams (CoreLink tenant + runner box) — owner-gated (task #7).

Note: these landed via **local cold-verify** (fmt + clippy --workspace
--all-targets --locked + test --workspace --no-fail-fast --locked, toolchain
1.96.0 per rust-toolchain.toml) and direct merge to main, because GitHub Actions
quota is exhausted. Re-run CI when quota is restored.

## Update — 2026-06-08: bidirectional sync built → 67/67
E6 (the last deferred WP) was **superseded** by the forge-arbitrated seamless-sync
design (`docs/design/2026-06-08-seamless-bidirectional-sync.md`) and **built** on
`main`: `crates/hugit-mirror/src/sync/{mod,engine,detect}.rs` +
`tests/acceptance_bidir.rs` (5 owned items, the live GitHub-detect arm gated as a P2
seam). **Coverage is now 67 of 67 WPs built** — every WP's logic is on `main`; all
that remains is flipping the disclosed P2 live-infra seams (owner-gated, task #7),
for which the exact tested sequence is `docs/handoff/2026-06-08-p2-go-live-runbook.md`.

## State
`main` (`4e03f1b`) green by local cold-verify: fmt · clippy --workspace
--all-targets --locked -D warnings · test --workspace --no-fail-fast --locked ·
`cargo audit`. Earlier waves also passed GitHub CI through main `1541baf`; CI
enforces `--locked` (cloud `gates` job currently quota-paused — re-enable when
restored). Repo clean: only `main` on origin, zero stray branches, zero worktrees.
