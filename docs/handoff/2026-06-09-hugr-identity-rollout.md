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
| workspaces session | **§B** paste two prepared files + one commit | 60 seconds, whenever next anchored there |

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

---

## B. For the corelink-workspaces session — two prepared files (60-second landing)

The hugit session fence (mechanized, fail-closed) correctly blocks this session
from writing into corelink-workspaces. The two files below are **final —
land verbatim**, then run §B3.

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

### B3. Landing commands (paste-ready; run from the workspaces checkout)

```sh
cd ~/Documents/HuGR/corelink-workspaces
mkdir -p docs/adr
"$EDITOR" docs/adr/0002-hugr-identity.md   # paste §B1 exactly (strip the outer fence)
"$EDITOR" docs/interop.md                  # paste §B2 exactly (strip the outer fence)
git add docs/adr/0002-hugr-identity.md docs/interop.md
git commit -F - <<'EOF'
docs: adopt ADR-0002 (HuGR identity) + family interop map

- docs/adr/0002-hugr-identity.md — thin companion adopting the canonical ADR
  (../hugit): clw credential model unchanged (tenant + PAT); user-facing copy
  says "HuGR account"; the signup bugs this repo filed sit on the ADR's
  account-creation route.
- docs/interop.md — the clw->CoreLink wire stated microscopically (FastCDC
  params, manifest v1, ref namespace, run-memo key, BLAKE3 verify, 429
  throttle) + the upward relationships (hugit, githugr, Runners M4).

Prepared in the hugit-anchored session (2026-06-09); landed here because the
hugit session fence (correctly) blocks cross-repo writes.

Signed-off-by: Gustavo Schneiter <gustavo@humangr.com>
Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>
EOF
git log -1 --stat   # DoD: exactly 2 files, ~150 insertions
```

---

## C. Rollout ledger (close a row only when its DoD passes)

| Repo | Artifact | DoD | State |
|---|---|---|---|
| hugit | canonical ADR · index row · `docs/interop.md` · this handoff | on `main` | ✅ |
| githugr | companion ADR · product.md §10.6 DECIDED · whitepaper §7 · `docs/interop.md` | on `master` | ✅ |
| corelink-runners | companion ADR · `docs/interop.md` | on `master` | ✅ |
| corelink-workspaces | §B1 + §B2 landed | §B3's `git log -1 --stat` shows the 2 files | ⏳ |
| corelink-server | §A3 fixes | fresh-signup 3-probe smoke green + derived region persisted | ⏳ |
| corelink-server | §A1 contract confirmed | inline answers or "all defaults OK" | ⏳ |
| corelink-server | §A1 endpoint + §A2 Clerk live | the five §A1 probes + §A2 DoD | ⏳ (Wave-4 horizon) |
