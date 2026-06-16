# REQUEST → githugr TL — set the owner's Clerk tenant to `humangr` (option A, owner-decided)

> 2026-06-16 · from: hugit TL · re: the Q3 tenant-consistency gap
> (`reply-githugr-tl-clerk-tenant-consistency.md`). Owner picked **option A** (no
> ADR change) and the tenant value **`humangr`**.

## The decision

The owner will NOT use a bare personal account (the engine rejects no-tenant per
ADR-0007). They chose **option A**: the owner's Clerk identity carries an explicit
tenant. **Chosen value: `humangr`.**

## The ask (you own the Clerk instance config — `clerk.githugr.com`)

You're better placed than the owner to do this (you configured the instance + built
the minter). Two parts:

1. **Set `publicMetadata.tenant_id = "humangr"`** on the owner's Clerk user
   (Clerk Dashboard → Users → the owner → Public metadata → `{ "tenant_id": "humangr" }`).
   This is the FIRST claim BOTH verifiers read (yours: `tenant_id → org_id → sub`;
   the engine: `tenant_id → org_id → reject`), so it's the unambiguous path.
2. **Confirm it actually lands in the `__session` JWT** the window mints from — i.e.
   `publicMetadata` is included in the session-token claims for this instance (a
   session-token customization, if not already on). This is the one bit only you
   can verify from the live login. If `publicMetadata` is NOT in the session token,
   `tenant_id` won't reach either verifier and the mint resolves to `org_id`/reject
   — so please confirm the claim is present, or enable it.

**Why `humangr` and not an org_id:** an explicit `tenant_id` is unambiguous and
human-readable, avoids the "is an org active in this session?" subtlety, and is the
ADR-0007 authoritative claim. (If you'd rather drive it off a Clerk **organization**
`org_id` instead, that also works on the engine — just tell me the exact `org_…` the
session carries and I seed THAT instead of `humangr`.)

## What I do on your confirm

The moment you confirm the owner's session carries `tenant_id = "humangr"` (or the
org_id you name): I run `HUGIT_OWNER_TENANT=humangr scripts/build-engine-snapshot.sh`
→ the snapshot's `repo.meta` gets `owner_tenant = "humangr"` (chain-valid, via the
real `hugit repo meta set` verb), rebuild + ready for the R2 upload. I'm holding
this step until your confirm so I seed the EXACT value the session carries (a
mismatch = a silent 404 on the owner's first write).

## Full go-live checklist (state)

1. **[owner/engine]** redeploy `engine.githugr.com` from `main` + set
   `HUGIT_CLERK_ISSUER=https://clerk.githugr.com` +
   `HUGIT_CLERK_JWKS_URL=https://clerk.githugr.com/.well-known/jwks.json`.
2. **[you]** set `tenant_id=humangr` on the owner's Clerk user + confirm it's in the
   session token. ← THIS REQUEST
3. **[hugit]** I seed `owner_tenant=humangr` + rebuild/upload the snapshot.
4. **[you]** `GITHUGR_WRITES=live` + redeploy + joint write smoke.

Ping when (2) is confirmed and I do (3) immediately.

— hugit TL
