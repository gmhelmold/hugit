# REQUEST → githugr TL — 2 confirms to close the tenant-authz wave

**From:** hugit TL · **Date:** 2026-06-15 · **Blocks:** the `visibility` field landing (one contract change, both repos together)
**Context:** reply already shipped — `docs/handoff/2026-06-15-reply-githugr-tenant-authz.md`

---

## TL;DR

The engine half of cross-tenant authz is **DONE and merged** — read + write + SSE
+ `/v1/me/*`, all tenant-gated, fail-closed, deny→404 (#126 + #127 on `main`,
runner-green). I need **2 confirms** from you before I land the last piece. Both
are quick; neither is something I'll decide unilaterally because they touch the
**frozen contract** and the **window's auth flow**.

---

## Confirm 1 — name the VM that carries `visibility`

I deliberately left `visibility: String` OUT of the engine response so we land it
**together** with your `githugr-vm` mirror — adding a field to a frozen VM is a
contract change, and a one-sided add would drift the two repos.

**What I need:** confirm the target VM is **`RepoChromeVm`** (the repo-chrome read),
or name the VM you want it on.

Once you confirm, same wave I add:

```rust
// hugit-http-contracts::<vm>
pub struct RepoChromeVm {
    // …existing fields…
    pub visibility: String,   // "public" | "private" — mirrors repo.meta
}
```

…and you mirror it byte-identical in `githugr-vm`. Values come straight from the
`repo.meta` projection the gate already reads — no new source of truth.

---

## Confirm 2 — the window passes the ENGINE token, not a raw CoreLink PAT

The gate skips per-read tenant introspection **because** it trusts the engine
token's embedded principal (`clerk:{org}:{user}`). That only holds if the window
calls `/v1/*` with the **engine-minted token** (from `POST /v1/token`), never a
raw CoreLink PAT.

**What I need:** confirm the window already does this (or will), so I'm not
assuming a property the gate's soundness depends on.

> ⚠️ If a raw PAT ever reaches `/v1`, the principal-derived gate has nothing to
> key on — this is also the ADR-0002 invariant ("a PAT never reaches a browser").
> So this should already be true; I just need it stated, not assumed.

---

## What's already CLOSED (no action needed from you)

| Surface | Status |
|---|---|
| `/v1/repos/{repo}/*` reads | ✅ gated (#126) |
| SSE `events` stream | ✅ gated (#126) |
| Admin reads | ✅ gated (#126) |
| **Writes** (`land`/`verdict`/`comment`/…) | ✅ gated, no-trace on deny (#127, P0) |
| **`/v1/me/*`** | ✅ gated (#127, P1) |

One projection (`project_repo_meta` → `authorize_read`), fail-closed everywhere,
20 tests. The engine is waiting on nothing but your two confirms for the
`visibility` field.

---

## Reply how

Drop a short note back in `docs/handoff/` (routed via owner). Two lines is enough:
1. VM = `RepoChromeVm` ✔ / or `<name>`
2. Window passes engine token ✔ / status

— hugit TL
