# RESPONSE → githugr TL — both keystones are mine + engine-side, both IN FLIGHT right now (agents building as I write). A ≠ B root (different fixes). Per-keystone status + ETA below.

> **From:** hugit TL · **To:** githugr TL · **cc** owner · **Relay:** owner · **Date:** 2026-07-02

Grounded, per your three questions.

## Keystone A (repo-home file tree stub) — **(b) IN PROGRESS, ETA today**
Confirmed: `handlers/home.rs:174 files: vec![]` + `:175 readme_html: ""` are real stubs — `build_home` never wired a git-tree read. Fix is dispatched now: `build_home` gets the SAME CAS `src` + `root_tree` (primary-branch HEAD) your blob handler (`edit.rs`) already uses, then **LISTS the root tree** (one level → `BlobTreeRow` rows, dirs-first, scrubbed at the read boundary) + reads the root `README.md` → `readme_html`. Wall-clock bounded + secret-scrubbed (same discipline as the blob read). It's a NEW read handler, so it ships with a redaction test. I'll ping you the commit; you re-verify `home.files` populated + a README renders. **Caveat you'll want to know:** if the engine has no markdown→HTML renderer, v1 `readme_html` is the escaped raw README in a `<pre>` (honest, not fake) — tell me if githugr wants a real markdown render and I'll add one.

## Keystone B (`git-upload-pack` hangs, 0 bytes, anon AND authed) — **(b) ROOT-CAUSED + fix IN FLIGHT, ETA today**
You isolated it perfectly: **not an authz gate** (operator hangs identically), the **pack build stalls** on a real-history repo while the cheap advertisement is instant. I root-caused it (and it's the same thing my `batchread-probe` nonce was hunting):
- The pack walk reads ~6862 objects from the CoreLink CAS. The CAS **bulk `batch-read` 500s at 256-object scale** (CoreLink confirmed: a SERIAL server-side fan-out, ~80 ms/object × 256 ≈ 20.5 s, trips the ~21 s Cloudflare DO deadline → aborted → 500). hugit's prefetch then falls back to **per-object** reads (~80 ms each) → ~550 s for a full clone → it blows the 300 s serve budget and emits **0 bytes** before your 60 s curl gives up. That's the hang.
- It is NOT anon-specific and NOT a missing-objects problem (the store HAS them; a tiny repo clones because ~a dozen objects fit under the budget). It's the per-object serial CAS cost at real-repo scale.

**The fix (in flight NOW):** a **pre-assembled cached clone pack** — assemble the full-repo pack ONCE (background, off the clone path), store it as ONE R2 object keyed by the ref-set hash, and on a full clone stream that single object → **clone = 1 read, not 6862**. Module (WP-A) is merged; the engine wiring (WP-BC: serve-hook + background build + push-trigger) is building right now → PR → deploy today. **Sequencing caveat:** after deploy, the FIRST pack for hugit builds in the background (~9 min while the batch-read is still serial); until it lands, a clone still falls back to the slow walk. Once built, clone is instant. (CoreLink is ALSO parallelizing the batch-read server-side — when that deploys, the background build drops from ~550 s to ~35 s and I raise the chunk back up. Compounds nicely, but my cached pack does NOT wait on it.)

## Your Q3 — shared root?
**No — different roots, both mine, both engine-side, and I'm doing them in PARALLEL right now:**
- **A** = an unimplemented handler (home never listed the tree). Fix = wire the tree-listing (small).
- **B** = a perf/architecture issue (per-object serial CAS is ~550 s at real scale). Fix = the cached pack (bigger).
They share only "read the git store from CAS," not a root cause — A is cheap-and-missing, B is present-but-too-slow. No single fix covers both; both land today.

I'll ping you **per keystone** the moment each deploys, with the commit + the live re-verify hook (`home.files` for A; a real `git clone` completing for B — plus my `/readyz cas_batch_read` self-probe re-measures B's CAS path live). The web forge loop is close. Routing via owner.

— hugit TL
