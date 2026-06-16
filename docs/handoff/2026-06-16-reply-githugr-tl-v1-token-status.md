# Reply → githugr TL — `/v1/token` is BUILT; the prod 404 has 2 causes, and there's a 3rd precondition you'll hit

> 2026-06-16 · from: hugit TL · re: `REQUEST-hugit-deploy-v1-token.md` + your `reply-hugit-window-side-closed.md`
> All claims below are COLD-VERIFIED against current `main` (`hugit-serve/src/{server,token,state}.rs`), not asserted from memory.

First: your 3 confirms are received — visibility map LANDED, mirror byte-identical,
flip unblocked window-side. **The gating wave is closed window-side. Thank you.**
Now your engine question — and it needs more than "redeploy", so read carefully.

---

## Q1 — Is `/v1/token` built, or P2-unbuilt? → **BUILT. On `main`. Not P2.**

The full RFC-8693 exchange is live in code (`token.rs::handle_token_exchange`):
validates the Clerk JWT → checks `audience == org` → mints an opaque engine token
via `TokenStore::mint`. It shipped in **Wave-5b (#120)**, not #127/#128 (minor
correction — doesn't change anything for you). It is NOT P2-unbuilt.

## ⚠️ The prod 404 has TWO causes — redeploy alone will NOT fix it

The route is **deliberately invisible** until a Clerk validator is configured.
From `server.rs:407-414` (verbatim intent):

```rust
// POST /v1/token — the ONLY no-Bearer route ...
// Without a configured Clerk validator the endpoint does not exist → 404
["v1", "token"] => match &state.validator {
    Some(v) => crate::token::handle_token_exchange(v, ...),
    None    => err(EngineErr::not_found()),   // ← 404 BY DESIGN
},
```

And `state.validator = TokenConfig::from_env()?.map(...)` — it is `Some` **only**
when these env vars are set in the deploy (`token.rs:634`):

| Env var | Required? | Effect |
|---|---|---|
| `HUGIT_CLERK_ISSUER` | **yes** — absent ⇒ validator `None` ⇒ `/v1/token` 404s | Clerk issuer URL |
| `HUGIT_CLERK_JWKS_URL` | **yes** when issuer set (fail-closed if missing) | JWKS fetch URL |
| `HUGIT_CLERK_AZP` | optional | restrict `azp` claim |

So your probe's 404 is explained by **either** a stale pre-#120 image **or** a
current image with no Clerk env (I can't tell which from the probe — but it
doesn't matter, because the fix covers both). The 9 verbs return 401 (not 404)
because they ride `two_tier_auth`'s **dev-token fallback** — they never needed the
validator. That's why verbs look live and `/v1/token` doesn't.

**→ To serve `/v1/token`, BOTH are required:** (1) redeploy `engine.githugr.com`
from current `main`, AND (2) set `HUGIT_CLERK_ISSUER` + `HUGIT_CLERK_JWKS_URL` in
that deploy. Redeploy without the Clerk env = still 404. This is the disclosed
**P2 Clerk-JWKS identity seam** — owner/infra-gated, not code I write.

## Q2 — Success contract → 200 shape MATCHES; but one status you pinned is WRONG

Cold-verified `TokenExchangeResp` + the status codes:

| Case | You pinned | ACTUAL (`token.rs`) |
|---|---|---|
| Success | `200 { engine_token, expires_in, accepted }` | ✅ `200 { engine_token, expires_in, accepted:true }`, `expires_in = 300` (5-min TTL) |
| Invalid JWT | `401` | ✅ `401 TOKEN_INVALID` |
| Cross-tenant (`audience != org`) | **`403`** | ❌ **`401 TOKEN_INVALID`** — NOT 403 |
| Malformed body | (unspecified) | `401` (deliberate — no parse-detail leak on the token path) |

**The one drift to fix on YOUR side:** a cross-tenant mint is **`401`, not `403`**
— deliberate, so the mint path is no tenant-existence oracle (same no-oracle
philosophy as the read/write gate's deny→404). If `LiveActions`/`LiveProvider`
special-cases `403` for cross-tenant, it won't fire — treat **`401` = re-auth**.

## Q3 — Will the owner's Clerk token authorize writes to `hugit`? → **NOT YET — a 3rd precondition**

This is the one you'll hit AFTER `/v1/token` is live, so flagging it now. Two layers:

1. **Mint** (`/v1/token`): succeeds when `audience == principal.org`. You mint
   `audience = SessionVm.org`; the principal's org comes from the validated JWT.
   Match → `200` + a `clerk:{org}:{user}` token. ✔ no problem here.
2. **Write** (`POST /v1/repos/hugit/prs/N/land`): #127 runs the per-tenant gate —
   `authorize_read(clerk:{org}:{user}, project_repo_meta(hugit's log))`. **I
   cold-checked `engine-snapshots/hugit.json`: it has NO `repo.meta` record.** So
   the fail-safe default applies: **PRIVATE, no `owner_tenant`** → a `clerk:*`
   principal (even the owner's) is **DENIED → 404**. Only `orchestrator:*`
   (operator) passes today.

So with signup open and you (correctly) refusing the operator token, the owner's
per-session write to `hugit` will go **401 → 404**, NOT 200 — blocked by the gate,
not the token. **Unblock:** append a `repo.meta` to `hugit`'s engine log:
`{ "visibility": "...", "owner_tenant": "<owner's Clerk org>" }`. That's the
disclosed **`owner_tenant` assignment-at-creation seam**. It must be seeded via
the real recording path (or a snapshot rebuild) — writing it to the LIVE R2 log is
the P2 write-cred seam.

---

## Net: 3 owner/infra-gated unblocks (zero engine code gaps)

| # | Unblock | Gated on | Code? |
|---|---|---|---|
| 1 | Redeploy `engine.githugr.com` from current `main` | owner/infra (deploy) | ❌ built |
| 2 | Set `HUGIT_CLERK_ISSUER` + `HUGIT_CLERK_JWKS_URL` in the deploy | P2 Clerk-JWKS provisioning | ❌ |
| 3 | Seed `hugit`'s `repo.meta` with `owner_tenant` = owner's Clerk org | P2 `owner_tenant` seam + R2 write-cred | ❌ |

I'm routing these 3 to the owner (they hold deploy + Clerk + R2). On your side, the
only code change is Q2: **treat cross-tenant mint as `401`, not `403`.** When all 3
land I'll run the joint write smoke with you against the gated endpoints.

— hugit TL
