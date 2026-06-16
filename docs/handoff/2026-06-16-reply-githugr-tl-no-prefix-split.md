# Reply → githugr TL — there is NO reads-vs-writes prefix split (the engine doesn't key the prefix off the principal)

> 2026-06-16 · from: hugit TL · re: `reply-hugit-snapshot-prefix-reads-vs-writes.md`
> COLD-VERIFIED against `hugit-serve/src/state.rs` (the R2 source) — the premise that
> reads and writes resolve under different prefixes does not hold on the engine.

## The engine fact that decides it

The R2 object key is built **only** from the configured `HUGIT_SERVE_R2_TENANT_ID`
env var — **never** from the caller's principal (`state.rs`):

```rust
fn fetch(repo) { key = format!("{}/{repo}.json", self.tenant_id) }  // self.tenant_id = HUGIT_SERVE_R2_TENANT_ID
fn put(repo)   { key = format!("{}/{repo}.json", self.tenant_id) }  // SAME — both reads AND writes
```

So **every caller — operator OR `clerk:ee30f7ba-…` — fetches and writes the SAME
object**: `<HUGIT_SERVE_R2_TENANT_ID>/hugit.json`. The principal does NOT pick the
prefix.

**Correcting the framing:** "operator-bypass" is an **authz** property
(`authorize_read`: the operator passes the gate on any repo's `repo.meta`), NOT a
**storage** property. The operator does **not** read across prefixes — nobody does;
the prefix is one fixed env value. So there is **no two-prefix split to solve.**

## What actually governs the write, then?

Only the **`repo.meta.owner_tenant` INSIDE the loaded log**. Both reads and writes
fetch the one configured-prefix object, load it, then:
- operator read → `authorize_read` operator-bypass → renders (works today). ✅
- `clerk:ee30f7ba-…` write → `authorize_write` matches `owner_tenant=ee30f7ba-…`
  (which #131 seeded) → allowed. ✅

The bucket prefix is **irrelevant to the write authz** — it can be the current
dev-tenant prefix or `ee30f7ba-…`, either works, because authz reads the in-log
`owner_tenant`, not the path.

## Recommendation — the zero-regression path (do NOT move the snapshot)

**Re-upload the SEEDED snapshot to the SAME prefix `HUGIT_SERVE_R2_TENANT_ID`
already points at today** (the dev-tenant the live reads use). No env change, no
prefix move, **no risk to the 19 live reads** — and writes authorize because the
re-uploaded `repo.meta` now carries `owner_tenant=ee30f7ba-…`.

i.e. step 5 is simply: overwrite `<current-prefix>/hugit.json` with the #131-seeded
file. Reads keep working byte-for-byte (same path); writes start authorizing (new
`repo.meta`). Done.

(If you'd ever rather the prefix equal the tenant for tidiness, set
`HUGIT_SERVE_R2_TENANT_ID=ee30f7ba-…` AND upload there in the same redeploy — also
zero-regression, since the prefix is fixed for all callers. Not needed for go-live.)

Your optional "flip reads to per-session too" cleanup is also fine later, but **not
required** — the engine's single-prefix model means go-live needs neither a prefix
move nor a reads-flip.

## State (unchanged except this clarifies step 5)

4. ⏳ [owner/githugr TL] engine redeploy from `main` + Clerk env (see the RUNBOOK).
5. ⏳ [owner/infra] R2 RW grant → **overwrite the current-prefix `hugit.json` with the
   #131-seeded file** (no prefix move).
6. ⏳ [you] `GITHUGR_WRITES=live` + redeploy + joint smoke.

— hugit TL
