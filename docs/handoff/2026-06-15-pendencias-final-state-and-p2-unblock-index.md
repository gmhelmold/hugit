# Pendências — final state + the P2-unblock index (2026-06-15)

> Single actionable view: every tracked seam is now either **CLOSED** (code/docs
> shipped) or has **one crisp owner action** that unblocks it. Detail lives in the
> register (`docs/plan/2026-06-11-pending-seams.md`) and the cited handoffs — this
> index does NOT duplicate it, it routes.

## A. CLOSED — in-control, shipped (no owner action)

| Seam | What closed it | Commit/PR |
|---|---|---|
| **PS-1** | recorder verbs (wedge) | wedge wave |
| **PS-6** | `queue show` real batch verdict + implicated_pr (shared `Ledger`, agrees with `campaign show`) | PR #121 |
| **PS-9** | `intent new --log` owning-log + truthful global view | (prior) |
| **PS-10** | AC axis guard hoisted to the shared `ActionCache` trait (all 3 backends) | PR #121 |
| **PS-11** | `hugit check --env-axis` opt-in | (prior) |
| **PS-12 (advisory tools)** | runner provisions `cargo-deny`/`cargo-audit` | Wave L |
| **PS-12b (registry-corruption half)** | `ci.yml` CARGO_HOME isolation (`$HOME/.cargo-hugit-ci`) — kills the cross-repo registry race by construction | main `0bc6aab` |
| **PS-13** | single `verify_chain` chokepoint + build-failing source-invariant | Wave M |
| **PS-14** | deny-by-default identifier scrub (hybrid hex pin) | Round 9 / Wave M |
| **PS-15 F-2** | error JSON `kind`-first (shared ordered builder) | PR #121 |
| **PS-15 PERF F5** | found ALREADY verify-once-per-invocation (honest correction; alloc micro-shave lead-deferred) | PR #121 |
| **PS-17** | memo-key snapshot read caps peak memory | (prior) |
| **PS-4 (hugit-side)** | interop.md §8 frames `HUGIT_RUNNER_HOST` as the intentional seam | PR #122 |
| **PS-7 (docs-acceptance)** | interop.md §8 `--toolchain <digest>` fleet requirement | PR #122 |
| **PS-18 (most of it)** | Waves 1–5b shipped 20 reads + SSE + 9 writes + token + R2 source | #111–#120 |

## B. OPEN — owner/infra-gated. Each needs ONE provisioning action.

The common root: hugit consumes CoreLink's machinery; the **P2 CoreLink tenant +
live creds** are the single forcing function behind nearly all of these. Provision
that and most close in one motion.

| Seam | The ONE unblock action (owner) | Governing handoff |
|---|---|---|
| **PS-2** — `--author-kind` authn binding | Provision the **Clerk-backed HuGR identity session** (ADR-0002) so author-kind binds to an authenticated principal, not a CLI flag | `2026-06-09-hugr-identity-rollout.md` · ADR-0002 |
| **PS-3** — cold-tier R2 erasure | Grant **R2 + D1 access on the P2 tenant** so the production `ColdStore` adapter gets a real `erase` (X7/X12 re-target off the in-memory toy) | `2026-06-08-corelink-p2-tenant-request.md` |
| **PS-8** — event-log server-side crypto | Stand up the **per-repo DO event-log (Seam D) + transparency log (Seam E)** so a competent local rewrite is server-rejected (local `verify_chain` stays the tamper-EVIDENT half) | `2026-06-11-corelink-p2-ceiling-request.md` (Seams D/E) |
| **PS-12b** — remaining CPU/IO contention | **Dedicated runner capacity** for hugit (the box is shared with many sibling runners; isolation killed corruption, but a sibling storm still SLOWS CI) | `2026-06-08-p2-go-live-runbook.md` |
| **PS-18 — live auth** | Provide the **live Clerk JWKS URL + mandatory `azp`** (+ frontend `auth_time` for `fresh_auth`); the JWT-validation CODE is already built (`token.rs`) | `2026-06-13-hugit-serve-deploy-handoff.md` · `2026-06-14-…-production-seams-design.md` |
| **PS-18 — R2 write cred** | The standing R2 cred is read-only; provide a **write cred** for live writes (reads already work) | `2026-06-14-request-r2-credential-from-corelink.md` |
| **PS-18 — CoreLink CAS / GitHub-mirror / fleet KPIs** | These need the **live AC (Seam A)** + a **GitHub App** — same P2 tenant family | `2026-06-11-corelink-p2-ceiling-request.md` |

## C. owner PRODUCT DECISIONS — DECIDED 2026-06-15 (no in-control work pending)

| Item | DECISION (owner, 2026-06-15) | Consequence |
|---|---|---|
| **PS-18 — ~12 git-layer/identity reads** (blob/compare/edit/org/profile/account/…) | **KEEP honest-default fixture** | the window's hybrid provider serves them client-side; a conscious decided posture, NOT a hollow shell. No engine work; revisit only if a real git-tree reader is pulled. |
| **PS-5** — CI fork-guard | **stay PRIVATE for now** | accepted-LOW holds; the threat-model comment + fix-path are already in `ci.yml`/`dco.yml`. The `pull_request_target` hardening is built ONLY if/when the repo goes public. |
| **Owner/business dashboard** (users · countries · usage volume · growth) | **PARKED — revisit post-launch** | it's a separate surface for a different audience (the founder, across all forges), and needs a user-analytics/telemetry pipeline (signup/usage events + geo-IP) that does not exist, plus actual users (pre-launch). When pulled, it's its own design + data-pipeline project — see the operator-admin handoff §6. |
| **PS-4 — sibling rename** | doc-title "hugit-runner" → "CoreLink runner" inside `../corelink-runners` | routed to the corelink-runners owner (session fence forbids hugit mutating a sibling). Not a hugit decision. |

## D. Accepted-by-physics (no action, tracked for audit completeness)

AR-1..AR-5 (check double-exec window, `kill(1)` portability, no log rate-limit,
orphan grandchildren, unprefixed-entropy-in-identifier), the PS-14 low-entropy
base32 residual, and the toolchain-version-string vs binary-bytes edge. All
documented in the register; none is a live hole.

---

**Bottom line:** everything in hugit's control is closed and SOTA. The open set is
exactly the P2 CoreLink tenant + live creds (one provisioning motion unblocks
PS-2/3/8/18-auth/CAS/KPIs), a dedicated runner (PS-12b tail), and two owner product
calls (the git-layer reads, the sibling rename). No code/docs debt remains on the
hugit side.
