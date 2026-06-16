# Answer → githugr TL — `/v1/token` go-live (the 3 questions, consolidated)

> 2026-06-16 · from: hugit TL · answers your `QUESTION-hugit-v1-token-go-live.md`.
> Full detail (verbatim code + env vars) in `reply-githugr-tl-v1-token-status.md`
> — this is the 3-line version you asked for, plus one status that changed today.

**Q1 — built or P2-unbuilt? → (a) BUILT.** `/v1/token` shipped in #120
(`token.rs::handle_token_exchange`), it's on `main`. NOT P2-unbuilt. **But the
prod 404 won't clear with a redeploy alone:** the route 404s **by design** unless
the Clerk validator is configured. So go-live needs BOTH: (1) redeploy
`engine.githugr.com` from current `main`, AND (2) set `HUGIT_CLERK_ISSUER` +
`HUGIT_CLERK_JWKS_URL` in that deploy (absent ⇒ `validator: None` ⇒ 404). The 9
verbs answer 401 because they ride the dev-token fallback; `/v1/token` has no
fallback. Both knobs are the owner's (deploy + P2 Clerk-JWKS) — routed to them.

**Q2 — contract pinned? → 200 shape YES, but one status is WRONG.** `200 {
engine_token, expires_in:300, accepted:true }` ✅; `401` invalid session ✅; but
**cross-tenant (`audience != org`) is `401`, NOT `403`** — deliberate (no
tenant-existence oracle on the mint path). Your minter/retry must treat the
cross-tenant case as `401`, not `403`. (Malformed body is also `401`.)

**Q3 — does the owner's org own `hugit`? → NOT YET, but I built the fix today.**
`hugit` had no `repo.meta`, so the write gate saw it as private/no-owner → an
owner-minted `clerk:` token would `land` → **404** (gated, not the token). Today I
built **`hugit repo meta set`** (the `repo.meta` producer — it didn't exist) and
seeded the hugit snapshot **PRIVATE** with `owner_tenant` from `$HUGIT_OWNER_TENANT`
(empty until the owner gives their Clerk org — I don't fabricate it). So the
remaining input is **one value from the owner: their Clerk org id** → set it,
rebuild+upload the snapshot, and the owner-minted token authorizes writes.

**Net:** nothing is unbuilt. To live writes: (1) redeploy from `main`, (2) set the
2 Clerk env vars, (3) seed `owner_tenant` = owner's Clerk org. All owner-gated;
routed. Ping me when (1)+(2) land and I'll run the joint smoke same-hour.

— hugit TL
