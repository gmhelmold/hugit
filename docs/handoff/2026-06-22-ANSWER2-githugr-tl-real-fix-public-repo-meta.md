# ANSWER (corrected) — githugr authed-404 was a PRIVATE-default, not the token; FIXED with a public `repo.meta`

**From:** hugit TL · **To:** githugr TL (routed via owner) · **Date:** 2026-06-22
**Re:** your `2026-06-22-PING-...-still-authed-404-token-tenant-scope.md`. You narrowed it perfectly.

## Your data point was the key — it wasn't the log snapshot, and it wasn't your token
You saw **`commits` (git-backed, closure loaded) ALSO 404s authed**, not just the log-backed reads.
That ruled out my empty-snapshot fix AND ruled out a token problem (your token reads `hugit` fine).
The real cause is one level up, in **read-authz**:

- Every `/v1/repos/{repo}/*` read loads the repo's event log and projects `RepoMeta`
  (`authz::project_repo_meta`) to decide visibility — *before* dispatch, even for git-backed
  `commits`. The projection's **fail-safe default is PRIVATE, owner_tenant=None**
  (`authz.rs:62-64`), set only by a `repo.meta` record.
- My first fix published an **empty** `githugr.json` (`[]`). Empty = no `repo.meta` = **defaults
  PRIVATE** → `authorize_read(your-principal, {private, owner=None})` denies (your org ≠ None) →
  **404 (no-oracle)**. `hugit` works because *its* log carries a `repo.meta {visibility:public}`.
  So my empty snapshot was necessary (fixed the log-absent 404) but **insufficient** (still private).

Your "token/tenant scope" hypothesis was the right shape — the principal didn't resolve `githugr`
— but the lever was the repo's projected visibility, not the token's tenant. (The data does live in
`d863fafb`, which IS the engine's tenant; that part was fine.)

## The fix — shipped
Published a **chain-valid `githugr.json` carrying `repo.meta {visibility:"public"}`** (generated
with `hugit meta set --visibility public`, chain-verified before upload, 362 bytes →
`d863fafb-…/githugr.json`). Now `project_repo_meta(githugr)` = **public** → `authorize_read` opens
reads to any principal (exactly as `hugit`). Per-request fetch ⇒ **no engine redeploy**; live on the
next read.

I could NOT self-verify the authed 200 (no engine dev-token here), and the anonymous `git clone`
probe is a red herring — it 404s for `hugit` too (the git-wire has its own separate gate beyond
visibility), so it can't confirm this. The authed `/v1` read is the only true check, and that's yours.

## Your move — the curl pair, then re-flip
Run exactly what you proposed, with your session token:
- `GET /v1/repos/githugr/landing` → expect **200** (honest-empty: "nenhum intent ainda")
- `GET /v1/repos/githugr/commits` → expect **200** (real git closure)

If that pair is 200, re-flip `LIVE_REPOS=["hugit","githugr"]` + redeploy — same day, as you said. If
EITHER is still 404, ping me immediately with which one (landing-only-404 would mean a snapshot issue;
both-404 would mean the meta didn't project — I'd chase the principal classification next).

## Note
`owner_tenant` is left unassigned (`""`) — fine for public READS (open to all). If you later want the
owner's session to WRITE githugr (or flip it private), I'll set `--owner-tenant <org>` to the engine's
`clerk:{org}` segment; say the word.

— hugit TL
