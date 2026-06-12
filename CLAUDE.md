# CLAUDE.md

Context for AI agents working in this repo. Keep it lean + high-signal.

## What hugit is

The **git-compatible, LLM-native forge** — CoreLink expansion campaign #3.
VCS + merge + CI designed for orchestrated agent fleets, built on CoreLink's
production CAS (Cloudflare Workers/R2/D1/DO). Founded 2026-06-05.

**Status (as of 2026-06-12, post Waves A/B/C/D/E+F + the wedge wave + G + H + I +
J + WK-AC + Wave K, adversarial rounds 1–7 closed; Round 8 = the SEVERE class
sweep RAN and Wave L remediated all 6 classes on branch `integ/wave-l` —
Round 9 re-audit + merge PENDING, push HELD): the buildable
product is complete; adversarial hardening is ongoing, not closed.** A
**17-package** Rust workspace (hugit-app + {ui,exit,sidecar} sub-crates = 4
crates + 13 feature crates — verified by `cargo metadata --no-deps`
2026-06-11; `hugit-web` MIGRATED OUT 2026-06-10 to ../githugr per the
headless-engine doctrine, and `hugit-runner` TRANSFERRED 2026-06-10 to
../corelink-runners per the runner-transfer campaign: hugit is git+forge,
compute is campaign #1's product; the seam is the wire contract — shared
`conformance/` vectors byte-identical in both repos, no git dependency in
either direction) implements all 67 work-packages of decomposition v2.0 (E6
superseded by the forge-arbitrated bidirectional-sync design). Adversarial
audit arc: Round 1 (fresh 7-agent fleet, after Wave D) found **7/7
DO-NOT-SHIP**; Wave E remediated all seven. Round 2 (after Wave E) found
**7/7 DO-NOT-SHIP again** (narrower — spine held; Wave F remediated all
four: WF-REDACT bare-hex+unify, WF-CLI deadlock/verify_chain/error-law,
WF-AUTHZ ref-guard, WF-CLI2 ghost-record+TOCTOU). The wedge wave
(W0→W-INT) is COMPLETE: `hugit check`/`verdict`/`pr.landed` are dispatched
end-to-end; the memoized-CI wedge is observable locally TODAY (PS-1 closed,
P2-independent). Round 3 (after Wave F + the wedge wave) found **7/7
DO-NOT-SHIP** (spine confirmed held across all three rounds; Wave G hardened
the wedge wave — Cluster A code, Cluster B docs). Round 4 (after Wave G)
found **7/7 DO-NOT-SHIP** (Wave H remediated Cluster A/B/C/D findings —
complete). Round 5 (after Wave H) found **7/7 DO-NOT-SHIP** (strongest
finding: event-log hash chain is tamper-EVIDENT not tamper-PROOF — honesty
gap, not a code defect; PS-8 tracks log-auth as P2 seam; Wave I remediated).
Round 6 (after Wave I) found **7/7 DO-NOT-SHIP** (root cause: Wave I's
single structural scrub boundary was verified on one verb and asserted for
all; Wave J remediated — the structural detector is now reused by both the
ident door and the ledger engine, with a per-verb secret MATRIX as the
permanent guard, then WK-AC closed an `.ac` toolchain leak the matrix
itself found). Round 7 (after Wave J + WK-AC) found **7/7 DO-NOT-SHIP** —
and unlike Rounds 2–6 (which skewed to honesty-gaps once the spine held),
Round 7 surfaced **multiple confirmed CODE defects**, orchestrator-verified
by live reproduction: (1) a `cas:` exemption that leaked a PAT verbatim
(exemption-is-a-hole #5, pre-existing since WH-SCRUB); (2) `hugit why`/`export`
skipped `verify_chain` (read-path integrity + a PS-8 overclaim); (3) verdict
rejection-laundering by lens substitution; (4) wedge stale-green from an
uncaptured env axis; (5) `export` raw-append D14 bypass; (6) an
`ac_busy`→`ac_error` taxonomy collapse that made the concurrency gate
flaky. **Wave K (`def8a18`) remediated all of them** (K-SCRUB · K-CHAIN ·
K-VERDICT · K-RUN · K-ERRLAW2, each cold-verified by live attack
reproduction + a stressed gate); residuals tracked as AR-5/PS-11/PS-12.
**Round 8 (the SEVERE class sweep, 2026-06-12) ran** — a method shift from
point-finding to exhaustive root-cause CLASS audits (6 SOTA reports in
`docs/review/round8/`), commissioned after the owner judged the prior fix
waves to be band-aiding instances rather than killing the class. All 6
classes (redaction · read-path · memo-key · authz · state-machine ·
error-law) shared ONE root: open-by-default + enforced-by-convention /
per-verb. **Wave L remediated all six STRUCTURALLY** (close-by-construction:
deny-by-default identifier scrub · verified-loader on `intent list` ·
hermetic check execution · `pub(crate)` append door + typed shim · single
seal-guard chokepoint + within-record reject-sticky fold · typed
`AcError::Busy` + clap envelope) on branch `integ/wave-l`
(L-A·L-B·L-C·L-D), each live-attack cold-verified by the orchestrator.
Round 9 (re-audit the same matrices to confirm convergence = zero P0/P1
code holes) is PENDING; `integ/wave-l` is NOT yet merged and the push is
HELD until Round 9 converges + owner sign-off. Residuals tracked:
PS-11 (now closed by L-C hermetic), PS-13 (read-path single-chokepoint
refactor, defence-in-depth), C3 FS/network/clock (P2 runner sandbox).
The integrity spine has held under every adversarial round (1–7) plus the
SOTA audit.
`main` (HEAD `def8a18`, Wave K) is green by the LOCAL gate (fmt + clippy
`--workspace --all-targets --locked -D warnings` = 0/0 + test
`--workspace --locked` = 1200 tests / 135 suites, 0 failed + `cargo deny
check` = 0), read by real bare exit code. The Wave L integration branch
`integ/wave-l` (3 docs/audit commits + L-A·L-B·L-C·L-D) is green by the same
LOCAL gate (fmt + clippy `--workspace --all-targets --locked -D warnings` =
0/0 + test `--workspace --locked` = **1233 tests / 140 suites, 0 failed**),
read by real bare exit code; it merges to `main` only after Round 9 confirms
convergence (the lesson banked below — a fresh adversarial round runs BEFORE
any push). **Honest Round-7 correction:** the
earlier "green, verified by real exit code" claim on the Wave J + WK-AC push
(`bd95162`) was over-stated — the workspace gate was load-FLAKY
(`concurrent_checks…` could surface a terminal `ac_error` under parallel
load) and that push's CI went RED on the `deny` step (exit 127 =
`cargo-deny` absent on the runner, PS-12 infra). Wave K's K-RUN fixed the
taxonomy (stress-verified 10/10 + the full workspace run); `cargo audit` is
not installed locally and the runner lacks `cargo-deny`, so the advisory
gate is LOCAL-verified via `cargo deny check` only (PS-12). A concluded
remote green is the source of truth and remains pending on the runner-tooling
gap. Lesson banked: a fresh adversarial round must run BEFORE a push, and a
green claim requires a STRESSED flaky-path, not one lucky run. There were two code-gate failures at the Wave-H/I boundary: a fmt
failure (hotfixed eec3eab) and a clippy `collapsible_if` failure that
survived the fmt hotfix and was closed by WI-PR (3e49c14, via the Rust
let-chain collapse) — both traced to a piped gate-check that masked the real
exit code; gates are now read bare. The single self-hosted runner is
contention-flaky (~37% of recent runs fail — infra failures dominate; the
two code failures at the Wave-H/I boundary are closed; HEAD may show
`in_progress` or a false failure on CI). What remains to flip to end-to-end: **owner-gated
infra** (P2 CoreLink tenant provisioning — see
`docs/handoff/2026-06-08-corelink-p2-tenant-request.md` and the P2 ceiling
request `docs/handoff/2026-06-11-corelink-p2-ceiling-request.md`). The
disclosed live-infra seams (AC HTTP fleet-shared cache, runner box,
transparency log, live GitHub detect) remain hermetic-proof until P2.

Read first: `docs/whitepaper/hugit-v1.md` (product design) ·
`docs/product/product.md` (the product brief: ICPs, killers, positioning, pricing posture) ·
`docs/adr/` (0001 context envelope · 0002 HuGR identity) ·
`docs/interop.md` (the microscopic seam map: AC/CAS · runners · GitHub · githugr) ·
`docs/plan/decomposition.md` + `docs/plan/wp-contracts/` (the 67-WP register) ·
`docs/review/2026-06-07-roadmap-gap-build-campaign.md` (what's built) ·
`docs/strategy/campaign-3-llm-native-forge.md` (founding brief) · `docs/research/` ·
`docs/handoff/` (pending cross-repo work: P2 provisioning · identity rollout).

## Principles (decided, don't relitigate without the owner)

- **Don't deviate from git.** Names, CLI shape, mental model stay git-proximate.
  Every deviation costs human adoption AND LLM affinity. (This is why the
  product is "hugit", not a fantasy name.)
- **Embrace, don't assault.** Compat ladder: git wire protocol → landing layer
  riding ON GitHub → bounded bidirectional mirror → authoritative forge. A
  broken bridge kills trust instantly; never naive symmetric sync.
- **The wedge is the landing problem** (integration/merge for agent fleets),
  not authoring, not review prose.
- **Memoize by content, price flat.** Never usage-billing whiplash; never
  charge for the customer's own compute.
- **Zero debt, no loose ends, impeccable repo** (same owner mandate as
  CoreLink). Verify claims; never loosen rigor without an explicit waiver.
- **State the family in the correct tense.** Production-state claims about a
  sibling cite that repo at the time of writing (GA notes, runbooks) — never
  memory of a design. Cautionary tale: the cross-tenant-dedup overclaim
  (`../corelink-runners/docs/review/2026-06-09-cross-tenant-dedup-claim.md`).
- **Identity is decided (ADR-0002):** one **HuGR account** on CoreLink
  machinery (Clerk · org = tenant · PATs) behind a frozen contract; **a PAT
  never reaches a browser**. No new auth service without a forcing function.

## Relationship to the HuGR family

Same primitive stack, nothing built twice:
**HuGR → CoreLink { Cache (launch) · Runners (#1) · Workspaces (#2) } →
hugit (#3, here) → githugr (#4, the forge surface)**. The CAS, AC, manifests,
tenancy, and PAT auth live in corelink-server — hugit consumes them, it does
not fork them. **Do not let hugit work leak into CoreLink's launch route or
campaign #1/#2 critical paths.**

Two incubation repos are managed FROM hugit sessions under owner-approved
fence carve-outs: `../githugr` (campaign #4) and `../corelink-runners`
(campaign #1 — its `docs/spec/hugit-integration-contract.md` is frozen from
hugit's side; **amended to v1.2.0** 2026-06-11, WP-R6+WA4: §13 adds per-job
metrics emission + transcript capture hook obligations; §13.1 money field
renamed `cost_usd|f64` → `cost_usd_micros|u64` per the WA4 integer-micro-USD
contract amendment; the frozen v1.0 §0–§12 are otherwise unchanged).

⚠️ Sibling repos — corelink-server especially, but **also the carve-outs** —
have **other live sessions/worktrees**. Never assume sole ownership; check
status/log before acting. **Never `git commit --amend` or rewrite history in a
sibling**: another session's commit may have become HEAD between your commit
and your amend (it happened 2026-06-09; recovered via atomic ref
compare-and-swap). Fixup commits only; even in hugit, re-check `git log -1` is
yours immediately before any amend.

## The session fence (owner mandate 2026-06-05 — MECHANIZED)

It must be **impossible** for hugit work to cross other sessions' repos,
especially corelink-server. Enforcement is physical, not behavioral:

1. **`.claude/settings.json`** (this repo) carries `permissions.deny` rules
   AND a `PreToolUse` hook (`.claude/hooks/forbid-sibling-paths.py`) that
   **blocks every Edit/Write/NotebookEdit into a sibling HuGR project and
   every Bash command referencing one unless it is provably read-only**
   (fail-closed). Every session opened in this directory — and every
   subagent it spawns — inherits the fence automatically.
2. **Open hugit sessions IN `~/Documents/HuGR/hugit`** — never from a
   sibling project's directory (a session anchored elsewhere does not load
   this fence). The founding session was corelink-anchored by historical
   accident; do not repeat it.
3. The session fence (`.claude/settings.json`) is authoritative; the TechLead
   profile (`.techlead/profile`) lists a subset of the same `neverTouch` paths
   for fleet dispatch.
4. Read-only inspection of siblings (cat/grep/git log) is allowed — context
   is fine, mutation never is. Fence changes require explicit owner approval.

## Conventions

- Commits: `Signed-off-by:` trailer (DCO) + `Co-Authored-By: Claude …` trailer.
- English for all repo documents; lean, evidence-cited strategy docs (house
  style mirrors `corelink-server/marketing/expansion/`).
- Once code exists: branch → PR → merge, gates green before merge (inherit the
  CoreLink discipline). Until then, docs may land on `main`.

## Don't touch

Other projects share the parent dir (`corelink-server`, `hugr-wallet`,
`HuGR-Smith`, `HuGR-Arsenal`, `_worktrees/`, etc.). **Only work on hugit here.**
