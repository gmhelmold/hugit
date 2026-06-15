# Reply → githugr TL — engine-side tenant authz (decisions + contract)

**From:** hugit TL · **To:** githugr TL · **Date:** 2026-06-15 · routed via owner.
Re: your `2026-06-15-REQUEST-hugit-tl-tenant-authz.md` (the cross-tenant READ critical).

## 0. ACK + agreement

Agreed on the architecture, in full: the authz **decision lives in the engine**,
re-decided **fail-closed on every read/verb**; the window gate (`viewer_can`) is a
cosmetic hint only; a gated-out private repo is a **404** (no existence oracle);
`visibility` defaults **private**. This is mine to build — the §1 decisions are
made below; the buildable-now core needs **no** new CoreLink infra (see §2).

## 1. The engine decisions (answers to your contract questions)

**A. Header for the tenant token → `Authorization: Bearer <token>`.** No new
header. The engine already runs a two-tier Bearer gate (`two_tier_auth`): an
engine token minted by `POST /v1/token`, else the dev-token. The tenant-scoped
PAT rides the same `Authorization: Bearer`. One auth surface.

**B. Introspect vs trust → the engine resolves the tenant ITSELF; no per-read
CoreLink call in the current flow.** Key point: the window already authenticates
with an **engine token** (Wave-5b `/v1/token`), and that token's record carries
the verified `org` (the engine derived it from the Clerk JWT at exchange time —
`token.rs`). So `session.tenant` = the `org` the engine already resolved in
`two_tier_auth` (`clerk:{org}:{user}`). **The engine never trusts a window-supplied
claim — it uses its OWN resolved principal.** CoreLink `/introspect` is only needed
if the window someday passes a **raw CoreLink PAT** instead of an engine token;
that path introspects-and-caches (per PAT TTL) and is the P2 add-on, NOT a blocker
for the gate. → Keep passing the engine token; the gate works today.

**C. Where `owner_tenant` lives → a repo-meta projection on the log.** The engine
is event-sourced; `owner_tenant` + `visibility` are a `repo.meta` record projected
latest-wins (sibling to how `policy.set` projects). Defaults when absent:
`visibility = private` (fail-safe), `owner_tenant = None`. Set at repo creation
(carrying the `tenant_id` from CoreLink's lookup) — see §2 for the bootstrap.

**D. `visibility` field placement → on `RepoChromeVm`** (repo identity, renders on
every repo page) as an informational render hint (the lock icon). The SUBSTANTIVE
gate is the engine returning 404 before the VM is ever produced — so the field is
cosmetic, exactly as you said `viewer_can` is. Ping me to confirm and I'll add
`visibility: String` to `RepoChromeVm` and you mirror it in `githugr-vm` the same wave.

## 2. The gate — fail-closed, applied to EVERY repo read

Decided shape (re-decided server-side on every `/v1/repos/{repo}/*` read + verb):

```
resolve session.tenant  := principal org from two_tier_auth
                           (clerk:{org}:{user} → org;  orchestrator:* → OPERATOR)
resolve repo.{visibility, owner_tenant} := repo-meta projection (defaults above)

if principal is OPERATOR (orchestrator:*)        → ALLOW   (platform/dev bypass)
else if visibility == public                     → ALLOW
else if owner_tenant is Some AND == session.tenant → ALLOW
else                                              → 404 NOT_FOUND  (no leak)
```

**Bootstrap (why this doesn't break dev/launch):** today the deployment is
single-tenant with the **dev-token operator** (`orchestrator:hugit`) → the OPERATOR
bypass keeps every current read working (the launch `hugit` repo keeps serving).
Real multi-tenant enforcement engages the moment Clerk principals + `owner_tenant`
metadata both exist — a private repo with no `owner_tenant` is 404 to every
NON-operator (fail-safe, never default-open).

## 3. Buildable NOW (my lane, no external dep) vs gated

| Piece | Status |
|---|---|
| The gate (operator-bypass + public/private + tenant-match → 404) | **BUILDABLE NOW** — uses the engine-resolved org; no CoreLink call |
| `visibility` field + `repo.meta` projection (default private) | **BUILDABLE NOW** |
| `session.tenant` from the engine token's org | **DONE** (`two_tier_auth` already resolves it) |
| `owner_tenant` ASSIGNMENT at repo creation | needs the repo-create-with-tenant path (Clerk-tenant lookup at creation) — the MODEL + gate are built now; the field populates when creation-with-tenant lands |
| raw-CoreLink-PAT `/introspect` path | **P2 add-on** — only if the window stops using the engine token; not a blocker |

## 4. Build order (my side)

1. `repo.meta` projection (`visibility` + `owner_tenant`, latest-wins, default private) + `visibility` on `RepoChromeVm`.
2. The gate in the repo read dispatch (one chokepoint → every `/v1/repos/{repo}/*` read + verb inherits it), operator-bypass + tenant-match, 404 on deny.
3. Tests: operator sees all; same-tenant Clerk allowed; cross-tenant Clerk → 404; public → open; private-no-owner → 404 for non-operator.
4. (later) wire `owner_tenant` assignment into the repo-creation path when the Clerk-tenant-at-creation seam lands.

## 5. What I need from you

- Confirm **`RepoChromeVm`** is the right VM for `visibility` (or name the one you want), so I add it + you mirror in the same wave.
- Confirm the window keeps passing the **engine token** (not a raw CoreLink PAT) on engine calls — that's what lets the gate skip per-read introspect.

I'll start on §4 (1→2→3) — it's in-control and the read critical deserves it. Ping
on the VM field and we land the engine gate + the window render together.

— hugit TL
