# CLAUDE.md

Context for AI agents working in this repo. Keep it lean + high-signal.

## What hugit is

The **git-compatible, LLM-native forge** — CoreLink expansion campaign #3.
VCS + merge + CI designed for orchestrated agent fleets, built on CoreLink's
production CAS (Cloudflare Workers/R2/D1/DO). Founded 2026-06-05.

## Status — honest delivery reality (double-checked 2026-06-17)

> This **supersedes the prior maximalist "67/67 built · complete · converged"
> narrative**, which conflated *test-green-hermetically* and *PR-merged* with
> *delivered live*. Full grounded, double-checked audit (44-agent read-only
> sweep vs the whitepaper + 67-WP decomposition):
> `docs/review/2026-06-17-honest-delivery-audit-double-checked.md`. Read it
> before making any "done"/"complete"/"live" claim.

**What this repo IS:** a **19-package** Rust workspace (hugit-app + {ui,exit,sidecar}
sub-crates + feature crates + `hugit-http-contracts` + `hugit-serve`; `hugit-web`
migrated to `../githugr` 2026-06-10; `hugit-runner` transferred to
`../corelink-runners` 2026-06-10 — hugit is git+forge, compute is campaign #1; the
seam is the byte-identical `conformance/` wire contract, no git dep either way).
It implements the **logic** of all 67 decomposition WPs + the `/v1` HTTP backend
the githugr window reads. The engine logic + the **Squad-X platform invariants**
are real, hermetically tested (real Ed25519/SHA-256 crypto), and have held every
adversarial round (1–13) + a SOTA sweep. The integrity spine is genuinely solid.

**What is NOT delivered ("built" ≠ "live" — the honest gap):**
- **No live git wire protocol.** `hugit-proto` has full clone/fetch/push logic, but
  nothing serves it — `git clone` against the deployed engine returns **404**.
- **Substrate P2-gated / transferred.** CoreLink hot CAS + AC = a real `ureq` client
  with no live tenant; the runner fabric → `corelink-runners`; cold-store
  (`UnwiredColdStore`) persists no transcript blobs; **merge-as-re-execution records
  the demand but never dispatches an agent** (intentional P2 deferral).
- **One deployed network surface:** `hugit-serve` (`/v1` + `/readyz`). NO git endpoint,
  NO runner endpoint.
- **Identity = dev-token stub** live; the Clerk→engine-token exchange is code-complete
  (the CoreLink exchange endpoint is live), gated on the deploy env (`HUGIT_SESSION_EXCHANGE_URL`)
  + a stale deployed image + `hugit-prod-d1`.
- **Absent entirely:** the semantic index (not even a WP). 10+ CLI verbs reserved but
  unimplemented (approve/reject/undo/policy/ws/ctx/dispatch/fleet/journal/diag).

**What IS genuinely live (don't under-claim it either):** the `/v1` read+write API
against the ONE `hugit` launch repo — 11/20 reads serve real chain-verified R2 data;
the 9 POST verbs are code-complete + R2-CAS-persisted (proven against prod R2),
`authz`-gated (the one deployed security boundary, 404-no-oracle); `hugit check`/
`verdict` are real EXECUTE paths; `hugit export` is a real zero-dependency exit-proof;
SSE replay-then-close. **Magnitude: ~15–20% live for the `/v1` API on the hugit repo;
single-digit % for a full multi-tenant end-to-end forge.**

**Critical path to a usable single-tenant forge (biggest → smallest):** deploy the
current `main` image (+ set `HUGIT_SESSION_EXCHANGE_URL` = the live
`corelink-api.humangr.com/v1/session/exchange`) → CoreLink P2 tenant (hot CAS+AC) →
live git wire serving (the file-content/CAS seam, which also unblocks blob/edit/symbol)
→ runner fabric live → GitHub App + live mirror → multi-tenant Clerk + `hugit-prod-d1`.
Most are owner/infra-gated, not "a few PRs". Per-capability status table + tracked
seams: the audit doc above.

**Gate + CI:** `main` green by the local gate (fmt + clippy `--workspace --all-targets
--locked -D warnings` + test `--workspace --locked` + `cargo deny`) AND runner-verified
per code push (docs-only pushes skip CI via `paths-ignore`). The single self-hosted
runner is contention-flaky (~37% of runs fail on infra); HEAD may show a false failure.
**Never claim green without a concluded `mergeStateStatus=CLEAN` + both checks SUCCESS —
never asserted by a watcher's exit.**

Read first: `docs/review/2026-06-17-honest-delivery-audit-double-checked.md` (the TRUE
state — what's live vs hermetic vs absent) · `docs/whitepaper/hugit-v1.md` (product design) ·
`docs/product/product.md` (the product brief: ICPs, killers, positioning, pricing posture) ·
`docs/adr/` (0001 context envelope · 0002 HuGR identity) ·
`docs/interop.md` (the microscopic seam map: AC/CAS · runners · GitHub · githugr) ·
`docs/plan/decomposition.md` + `docs/plan/wp-contracts/` (the 67-WP register — the LOGIC spec,
not a delivery claim) · `docs/strategy/campaign-3-llm-native-forge.md` (founding brief) ·
`docs/research/` · `docs/handoff/` (pending cross-repo work: P2 provisioning · identity rollout).

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
- **"Built" ≠ "delivered".** A PR merged + the gate green means the LOGIC passes
  tests hermetically — NOT that it is served, wired to real data, or live. State
  the scope explicitly; verify with a live probe before any "live"/"done" claim.
  (The over-claim that eroded trust 2026-06-17 — see the audit doc.)
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
status/log before acting. A relayed sibling handoff may live on that repo's
`main` or another branch, NOT its checked-out tree — `git show <branch>:path`
before claiming it's missing. **Never `git commit --amend` or rewrite history in
a sibling**: another session's commit may have become HEAD between your commit
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
