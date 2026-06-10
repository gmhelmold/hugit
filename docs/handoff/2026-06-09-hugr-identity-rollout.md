# Handoff — ADR-0002 (HuGR identity) rollout

**From:** hugit TechLead (orchestrator) · **To:** §A CoreLink TechLead · §B any
corelink-workspaces-anchored session · **Date:** 2026-06-09
**Canonical decision:** `docs/adr/0002-hugr-identity.md` (owner-ratified) —
*one HuGR account, CoreLink machinery*: identity stays on CoreLink's production
stack (Clerk · org = tenant · PATs), branded HuGR, consumed through a frozen
contract. **No new auth service, ever, without a forcing function.**
**Scope rule:** consume-as-customer; zero changes to the launch route. §A1 is
the one small, additive server feature this decision creates.

---

## 0. TL;DR (what I need, from whom, by when)

| Who | What | Urgency |
|---|---|---|
| CoreLink TechLead | **§A3** ship the two signup-worker fixes already filed by corelink-workspaces | **now-ish** — gates G4 pilots, P2 onboarding quality, and future githugr signup |
| CoreLink TechLead | **§A1** confirm (or correct) the session→token endpoint contract; build at your pace | contract answer now; endpoint by githugr **Wave 4** — explicitly **NOT a P2 blocker** |
| CoreLink TechLead | **§A2** Clerk config for githugr (same pool + GitHub social) | with §A1 |
| workspaces session | **§B** paste four prepared files + one commit | 2 minutes, whenever next anchored there |
| CoreLink TechLead | **§A7** (optional) place/adapt the proposed CLAUDE.md family section | with §A1/§A2 |

---

## A. For the CoreLink TechLead

### A0. Why (one paragraph)

githugr (campaign #4, the forge web surface) needs human login; hugit and `clw`
already speak PATs. ADR-0002 closed the question: **no parallel auth domain** —
the user logs into a *HuGR account* (your Clerk pool underneath), orgs map 1:1
onto your tenants, machines keep PATs, and the only new machinery is a small
endpoint that exchanges a valid session for a **short-lived, tenant-scoped
token** so githugr's server can call the API **without a PAT ever existing in a
browser's reach**. Everything below follows the P2 Q&A protocol: proposed
defaults, locked by you or the owner, never assumed by me.

### A1. The session→token exchange endpoint (the ONE new piece)

Written from the consumer's side (githugr's server) — same spirit as the AC
contract in `2026-06-08-corelink-p2-tenant-request.md` §3. **The shape is the
ask; the route name and header spellings are yours.** When you confirm, the
contract gets frozen consumer-side at githugr `docs/spec/`
(identity-token contract) the way the runners seam was frozen.

```
POST {base}/v1/identity/token            # {base} = the FLAT prod host
     Authorization: <Clerk session credential, server-verified (JWKS/SDK)>
     body: { "org": "<org-id the user is acting in>" }

→ 200 { "token": "<opaque short-lived credential — NOT a PAT>",
        "tenant": "<tenant_id>", "expires_at": <unix-s>, "scope": "cas:r" }
→ 401  invalid / expired session
→ 403  session valid but caller is NOT a member of <org>   (cross-tenant mint impossible)
→ 404  unknown org
```

Decisions, each with my proposed default — reply inline or "all defaults OK":

- **Q-A1.1 token format:** opaque, **distinct prefix** (e.g. `clt_` vs your
  `clp_` PATs) so logs/secret-scanners can never confuse the two. *(Default: yes.)*
- **Q-A1.2 TTL:** **15 minutes**, githugr re-mints transparently on expiry.
  *(Default: 15 min; tell me if your edge prefers another bound.)*
- **Q-A1.3 scope v1:** **read-only** (`cas:r`-class). githugr is read-first by
  doctrine; a write scope is a **new decision** when the gated write-path wave
  arrives — not implied here. *(Default: read-only.)*
- **Q-A1.4 verification path:** is the token accepted by the same edge that
  resolves PATs (token → tenant; enforce path-tenant == token-tenant), or a
  dedicated header/flow? Your call — tell me which, so the `Provider` sends it
  right the first time.
- **Q-A1.5 audit:** every mint emits a tenant-scoped audit event on the
  existing chain. *(Default: yes.)*
- **Q-A1.6 rate:** modest per-session mint rate-limit. *(Default: your
  starter-tier judgment; githugr needs ~1 mint per user per 15 min.)*

**Invariants (non-negotiable — ADR-0002 §6):** a PAT is never issued to, or
echoed at, any browser-facing surface · the minted tenant derives from
**membership, never from client input** · short TTL · the PAT-revocation
propagation invariant (`INV-PAT-REVOKE-PROPAGATION`) is untouched.

**DoD — five probes** (will live as a gated conformance test on the githugr
side at Wave 4, run-not-skip, same pattern as `corelink_ac_live_smoke`; until
then this table IS the spec):

| # | Probe | Expect |
|---|---|---|
| 1 | valid session + member org | 200; token reads AC/CAS for that tenant |
| 2 | valid session + **non-member** org | **403** |
| 3 | invalid/expired session | 401 |
| 4 | expired token at the edge | rejected (401/403); re-mint succeeds |
| 5 | response body + server logs | zero `clp_` material |

**Timing:** not a P2 blocker; needed by githugr **Wave 4** (account level).
Confirm the contract now so githugr designs against it; build post-launch at
your pace. **Related:** this pairs with the `create-pilot-tenant` admin
endpoint you already agreed to prioritize post-launch (techlead-questions
§9.1) — same seam: **programmatic tenancy as product infrastructure**.

### A2. Clerk configuration for githugr

- **One user pool** serving the CoreLink dashboard *and* githugr
  (multi-domain / satellite-domain — you validate the exact Clerk mechanism; I
  deliberately do not assume it).
- Enable the **GitHub social connection** — this is githugr's
  "sign in with GitHub". It is **not** the GitHub App (`hugit-app` keeps
  repo-access/webhooks; the two are never merged).
- **DoD:** a user created via the CoreLink dashboard signs into a githugr dev
  domain with the same account, and vice-versa; GitHub social login works on
  both; no second user record is created.
- **Timing:** with §A1.

### A3. Signup-worker fixes (the urgent one — filed, now doubly load-bearing)

Not new asks — corelink-workspaces already filed both in your lane. ADR-0002
makes them block **two** products instead of one:

| Defect | Report | Class |
|---|---|---|
| Fresh tenant → **HTTP 500 on every `/v1/ac/*`** (AC envelope-signing key not provisioned at signup); `wrangler tail` swallowed the error | `../corelink-workspaces/docs/BUGREPORT-prod-ac-500.md` | onboarding-fatal |
| `createTenant` hardcodes `primary_region='enam'`; `configureTenant` is a no-op; `regionFromColo` emits invalid macro-regions | `../corelink-workspaces/docs/HANDOFF-signup-region-hardcode.md` | compliance (GDPR residency) at scale |

**DoD:** a brand-new self-serve signup, with its fresh PAT, passes the 3-probe
AC smoke (404 miss → 200 round-trip → 403 cross-tenant); its tenant row carries
the **derived** region; handler errors reach `wrangler tail`.
**Why first:** gates **G4 (≥3 pilot signups)**, clean P2-style onboarding for
any new tenant, and the future githugr signup — the highest-leverage hours in
the portfolio right now.

### A4. What is already done on our side (nothing waits on us)

- ADR-0002 canonical (`docs/adr/0002-hugr-identity.md`) + thin companions in
  githugr and corelink-runners — landed.
- Microscopic interop maps (`docs/interop.md`) — landed in hugit, githugr,
  corelink-runners; the workspaces copy is §B below.
- **hugit needs zero code change** (PAT flow already contract-shaped; P2
  request stands exactly as written). githugr consumes §A1/§A2 only at Wave 4.

### A5. FYI — the family's consumption of corelink-server, one table

(Context for placing this work; your repo's conventions decide where it lives.)

| Consumer | Seam | Note |
|---|---|---|
| `clw` (workspaces) | CAS/AC HTTP + PAT verify | §A3 defects sit on its signup path |
| `hugit-checks` | AC by memo key `H(tree ‖ def ‖ toolchain)` + CAS (envelopes, packs, exports) | smoke DoD: 404 → 200 → 403 |
| Runners fabric (M1) | CAS/AC warm-boot reads + result stores + PAT + tenancy | fail-closed when cache degraded |
| githugr | `cas:` envelope fetches (via hugit's Provider) · per-repo DO event-log hosting (P2) · **§A1 mint endpoint (NEW)** · Clerk pool (§A2) | PAT never to a browser |
| direct cache customers | REAPI v2 gRPC · Bazel HTTP · Turborepo v8 · sccache/cargo · OCI (phase B) | the cache product proper |
| everyone | PAT verification (Argon2id, constant-time tenant claim) · HMAC-prefix tenancy · org = tenant = Stripe customer | identity per ADR-0002 |

Tense discipline (family rule): dedup is **intra-tenant at GA**; cross-tenant is
staged (`CAP-DEDUP-CROSS-TENANT`) — production-state claims cite the GA notes.

### A6. How to answer

Inline (e.g. "Q-A1.2 → 10 min", "Q-A1.4 → same edge as PATs") or simply
**"all defaults OK"**. Same protocol as the P2 Q&A: every decision locked by
you or the owner — never assumed by me.

### A7. (Optional) proposed CLAUDE.md addition — the family section

Owner directive (2026-06-09): every family repo's CLAUDE.md carries the family
context. Yours already covers product/gates/workflow superbly; the block below
is the missing family/identity slice — **place/adapt as you see fit**:

```markdown
## The HuGR family (who consumes this repo)

CoreLink is the foundation layer of the stack:
**HuGR → CoreLink { Cache (HERE) · Runners (#1, spec) · Workspaces (#2, clw
shipped) } → hugit (#3, built) → githugr (#4, design)**.
- The seam map (who reads what, microscopically): `../hugit/docs/interop.md`
  + the consumption table in
  `../hugit/docs/handoff/2026-06-09-hugr-identity-rollout.md` §A5.
- **ADR-0002 (family identity — Accepted):** one **HuGR account** on THIS
  repo's machinery (Clerk · org = tenant · PAT). New work it creates here:
  the session→short-lived-token mint endpoint (contract + DoD in that handoff
  §A1) + Clerk multi-domain for githugr. **Not launch-blocking.**
- The signup-worker fixes (AC-500 envelope key · region hardcode — filed by
  corelink-workspaces) gate **G4 pilots AND future githugr signup**.
- Family tense rule: cross-tenant dedup is post-GA (`CAP-DEDUP-CROSS-TENANT`);
  sibling docs cite YOUR GA notes for production state — keep them honest if
  state changes.
```

---

## B. For the corelink-workspaces session — four prepared files (2-minute landing)

The hugit session fence (mechanized, fail-closed) correctly blocks this session
from writing into corelink-workspaces. The four files below are **final —
land verbatim**, then run §B5. (The repo has `.techlead/` but **no CLAUDE.md
and no mechanized fence yet** — §B3 fixes the first; adopting the standard
`.claude` settings + `forbid-sibling-paths.py` hook from githugr/corelink-runners
is flagged inside it for when sessions start anchoring there.)

### B1. File 1 — `docs/adr/0002-hugr-identity.md`

```markdown
# ADR-0002 — HuGR identity: one account, CoreLink machinery (companion)

- **Status:** Accepted (adopts the canonical ADR:
  `../hugit/docs/adr/0002-hugr-identity.md`)
- **Date:** 2026-06-09

Family decision, one line: the user-facing identity is the **HuGR account**
everywhere; underneath it is CoreLink's production machinery (Clerk sessions,
org = tenant, PATs) behind a frozen contract; a standalone identity service is
deferred indefinitely and pre-authorized behind that contract.

What it means in this repo:

- **`clw`'s credential model is unchanged.** Config carries tenant + PAT; the
  PAT remains the machine credential, verified server-side. No code change.
- **User-facing copy** (README, errors, signup pointers) says **"HuGR
  account"** for the human identity — never "CoreLink login". The PAT remains
  named what it is: the CoreLink PAT, the machine credential under that
  account.
- **The signup bugs this repo reported** (`docs/BUGREPORT-prod-ac-500.md`,
  `docs/HANDOFF-signup-region-hardcode.md`) sit on the account-creation route
  this ADR standardizes — their fix (owned by corelink-server) unblocks
  CoreLink pilots and githugr signup equally.
```

### B2. File 2 — `docs/interop.md`

```markdown
# corelink-workspaces interop — how this repo talks to the family (microscopic seam map)

> 2026-06-09. `clw` is a PURE CLIENT of the live CoreLink API — no server code
> here. Frozen client contract: `clw-types` (Digest, Manifest v1, RefRecord,
> `CasTransport`/`AcTransport` traits; contract hash fe251941).

## 1. clw → corelink-server (the only wire)

| Aspect | Exact detail |
|---|---|
| Endpoint / auth | `https://corelink-api.humangr.com` · `Authorization: Bearer <PAT>` from `~/.clw/config.toml` (flag → env → file resolution) |
| snapshot | walk (ignore-aware) → FastCDC chunks (256K/1M/4M, compile-time const-assert `CHUNK_MAX < CAS_BLOB_CAP`) → dedupe via `CasTransport::exists()` → upload → canonical-JSON **manifest v1** → CAS → `RefRecord` written to the AC under namespace `clw/ref/v1/<name>` (carries `parent` lineage) |
| hydrate | RefRecord → manifest → parallel chunk fetch, **cache-first** (`~/.clw/cache`, atomic, self-healing, LRU gc) → atomic file reconstruct (symlinks + mode bits) · **BLAKE3 verify on every fetch** |
| run (memoized) | AC key = hash(command ‖ args ‖ env ‖ workspace manifest) · hit replays stdout/stderr/exit **byte-exact** · outputs chunked into CAS · **success-only caching** (non-zero exit never memoized) |
| status / ls | local tree hash vs remote ref → exit 0 clean / 1 dirty · ref metadata (root digest, counts, parent) |
| Rate limits | client-side **global throttle with patient backoff through 429s** (PR #19) — server-side limits are tuned for low concurrency today |

## 2. Relationships upward

- **hugit:** independent implementations over the same CAS concepts today
  (hugit-fence materializes claim-fenced workspaces itself; no runtime call to
  clw yet). Convergence: Runners **M4** (sandboxes/dev boxes as Workspace SKUs).
- **githugr:** "open in workspace" = `clw hydrate` of an exact
  content-addressed state; intent pages show the workspace chip.
- **Runners:** the fabric boots cache-warm off the same CAS/AC this client
  feeds; Workspace SKUs ride the fabric at M4.
- **Identity (ADR-0002, `docs/adr/0002-hugr-identity.md`):** PAT flow
  unchanged; user-facing copy says **HuGR account**.

## 3. Open defects this repo filed (owned by corelink-server)

`docs/BUGREPORT-prod-ac-500.md` (fresh tenant → 500 on all AC ops: envelope
signing key not provisioned) · `docs/HANDOFF-signup-region-hardcode.md`
(`enam` hardcode, no-op `configureTenant`, colo↔macro-region mismatch). Both
sit on the account-creation route of ADR-0002 — they block CoreLink pilots and
githugr signup equally.
```

### B3. File 3 — `CLAUDE.md` (the repo has none today)

```markdown
# CLAUDE.md

Context for AI agents working in this repo. Keep it lean + high-signal.

## What corelink-workspaces is

**`clw` — the CoreLink Workspaces client (campaign #2):** `init / snapshot /
hydrate / status / run (AC-memoized) / ls` against the live CoreLink API. A
workspace (source + deps + toolchain + build state) becomes a content-addressed
object; compute is a disposable cursor over it.
Family: **HuGR → CoreLink { Cache · Runners · Workspaces (THIS) } → hugit → githugr.**

**Status (2026-06-09): phase 1 shipped** — all 6 verbs implemented + tested
(wiremock integration, binary-level e2e roundtrip, live-API conformance gated
on a PAT). Day-1 dogfood surfaced real prod defects — filed as handoffs to
corelink-server (`docs/BUGREPORT-prod-ac-500.md`,
`docs/HANDOFF-signup-region-hardcode.md`), owned there, tracked here.

## Principles (decided — don't relitigate without the owner)

- **Pure client.** No server logic here, ever; server behavior lives in
  corelink-server. Bugs found here are FILED (handoff), never patched here.
- **Frozen client contract:** `clw-types` (Digest · Manifest v1 · RefRecord ·
  `CasTransport`/`AcTransport`; contract hash fe251941). FastCDC params
  (256K/1M/4M) are compile-time const-asserted. Changes are owner-gated.
- **Memoize honestly:** `run` caches success only (non-zero exit is never
  memoized); every fetch is BLAKE3-verified; never trust a blind 200.
- **Identity (ADR-0002):** user-facing copy says **HuGR account**; the PAT
  stays the machine credential (config: tenant + PAT). See
  `docs/adr/0002-hugr-identity.md`.

## Read first

`docs/interop.md` (the seam map, microscopic) · `docs/ARCHITECTURE.md` (crate
graph + dataflows) · `docs/product/product.md` (the product brief) ·
`docs/adr/0002-hugr-identity.md` · `CHANGELOG.md` · the two bug handoffs.

## Conventions

- Commits: `Signed-off-by:` (DCO) + `Co-Authored-By: Claude …` trailers.
  English for repo docs.
- Branch → PR → merge, gates green before merge (fmt + clippy
  `--all-targets -D warnings` + test `--locked`; live-API conformance is
  feature-gated and env-skips without a PAT — it must never rot to green).

## Siblings & sessions

Other HuGR repos share the parent dir and have **live concurrent sessions**.
Read-only inspection is fine; mutation never. **Never `git commit --amend` or
rewrite shared history** — fixup commits only (cross-session amend incident,
2026-06-09). ⚠️ This repo has `.techlead/` but **no mechanized session fence
yet** — adopt the standard `.claude/settings.json` +
`forbid-sibling-paths.py` hook from githugr/corelink-runners when sessions
start anchoring here (flagged 2026-06-09).
```

### B4. File 4 — `docs/product/product.md` (the repo-local product brief)

```markdown
# CoreLink Workspaces — Product Design (repo-local brief)

> Status: brief, 2026-06-09. The campaign #2 strategy canon lives in
> `../corelink-server/marketing/expansion/corelink-workspaces.md`; this is the
> repo-local product view in the family format. Numbers **indicative — owner
> ratifies before anything goes public**.

## 1. The one-sentence product

**A workspace — source + deps + toolchain + build state — becomes one
content-addressed object: snapshot it once, hydrate it anywhere in seconds,
memoize every deterministic command run inside it.**

## 2. The bet

One primitive, four products: the **ephemeral CI runner** boots from it
(campaign #1) · the **AI-agent sandbox** materializes exactly a claim's slice
of it (hugit's fences) · the **cloud dev box** is a long-lived cursor over it ·
the **lazy infinite drive** pins the hot set and pages the rest. Storage is
deduped by content (R2, zero egress), so the same `node_modules` exists once —
the marginal workspace is nearly free.

## 3. What exists today (phase 1, shipped)

`clw init / snapshot / hydrate / status / run / ls` against the live API —
FastCDC chunking, canonical Manifest v1, RefRecord lineage, cache-first
hydrate, success-only `run` memoization, BLAKE3 verification, 429-patient
throttle. Tested: wiremock integration · binary e2e roundtrip · live-API
conformance (PAT-gated). See `docs/interop.md` for the wire, microscopically.

## 4. What phase 2 needs (the honest gap list)

Partial hydration (fetch `src/`, skip `node_modules/`) · retention/GC policy
for old refs · output filtering for `run` (skip non-deterministic outputs) ·
compression · TB-scale + chaos tests · the Runners **M4** tie-in (sandboxes /
dev boxes as Workspace SKUs on the fabric).

## 5. Pricing posture (doctrine; numbers owner's)

Pinning SKUs (pin N GB warm, flat/mo) · warm-workspace slots as reserved
concurrency · included working-set for runner jobs. **Never metered on the
customer's own compute; no usage whiplash** (house law). COGS: R2
~$0.015/GB-mo + dedup ⇒ high-margin pins (indicative, per the campaign brief).

## 6. Open decisions for the owner

1. Pin tiers + prices (when campaign #2 goes to market).
2. Default retention/GC for unpinned refs.
3. Partial-hydration UX (`clw hydrate --only <paths>`?) — phase 2 scope.
```

### B5. Landing commands (paste-ready; run from the workspaces checkout)

```sh
cd ~/Documents/HuGR/corelink-workspaces
mkdir -p docs/adr docs/product
"$EDITOR" docs/adr/0002-hugr-identity.md   # paste §B1 exactly (strip the outer fence)
"$EDITOR" docs/interop.md                  # paste §B2 exactly (strip the outer fence)
"$EDITOR" CLAUDE.md                        # paste §B3 exactly (strip the outer fence)
"$EDITOR" docs/product/product.md          # paste §B4 exactly (strip the outer fence)
git add docs/adr/0002-hugr-identity.md docs/interop.md CLAUDE.md docs/product/product.md
git commit -F - <<'EOF'
docs: adopt ADR-0002 + family interop map + CLAUDE.md + product brief

- docs/adr/0002-hugr-identity.md — thin companion adopting the canonical ADR
  (../hugit): clw credential model unchanged (tenant + PAT); user-facing copy
  says "HuGR account"; the signup bugs this repo filed sit on the ADR's
  account-creation route.
- docs/interop.md — the clw->CoreLink wire stated microscopically (FastCDC
  params, manifest v1, ref namespace, run-memo key, BLAKE3 verify, 429
  throttle) + the upward relationships (hugit, githugr, Runners M4).
- CLAUDE.md — the repo had none: what clw is, principles (pure client, frozen
  contract, honest memoization, identity), read-first, conventions, the
  no-amend multi-session rule, and the missing-fence flag.
- docs/product/product.md — repo-local product brief in the family format
  (one primitive four products, phase-1 state, phase-2 gaps, pricing posture,
  owner decisions).

Prepared in the hugit-anchored session (2026-06-09); landed here because the
hugit session fence (correctly) blocks cross-repo writes.

Signed-off-by: Gustavo Schneiter <gustavo@humangr.com>
Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>
EOF
git log -1 --stat   # DoD: exactly 4 files
```

---

## C. Rollout ledger (close a row only when its DoD passes)

| Repo | Artifact | DoD | State |
|---|---|---|---|
| hugit | canonical ADR · index row · `docs/interop.md` · this handoff | on `main` | ✅ |
| githugr | companion ADR · product.md §10.6 DECIDED · whitepaper §7 · `docs/interop.md` | on `master` | ✅ |
| corelink-runners | companion ADR · `docs/interop.md` | on `master` | ✅ |
| corelink-workspaces | §B1–§B4 landed (ADR · interop · CLAUDE.md · product brief) | §B5's `git log -1 --stat` shows the 4 files | ⏳ |
| corelink-server | §A3 fixes | fresh-signup 3-probe smoke green + derived region persisted | ⏳ |
| corelink-server | §A1 contract confirmed | inline answers or "all defaults OK" | ⏳ |
| corelink-server | §A1 endpoint + §A2 Clerk live | the five §A1 probes + §A2 DoD | ⏳ (Wave-4 horizon) |
| corelink-server | §A7 CLAUDE.md family section | placed/adapted by their techlead | ⏳ (optional) |
