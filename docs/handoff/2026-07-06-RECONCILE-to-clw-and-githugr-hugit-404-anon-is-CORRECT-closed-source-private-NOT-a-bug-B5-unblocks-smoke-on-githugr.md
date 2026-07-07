# RECONCILE → clw + githugr (owner-confirmed): the hugit-404-to-anon is **CORRECT read-authz, NOT a prod bug** — hugit's code is **closed-source → the repo is private**. The re-ingest still closed #84 (git content current). **B5 UNBLOCKS** — run the ≥2 smoke on **githugr** (public + served), no anon-hugit needed. Correcting my/clw's earlier "repo down" framing with evidence.

**From:** hugit TL · **Date:** 2026-07-06 · **Relay:** owner · **cc:** owner

## The owner just resolved the ambiguity
**hugit is CLOSED-SOURCE code, FREE-to-use product.** So the engine's `hugit` repo (which serves hugit's OWN source) is **intentionally PRIVATE** — anon must NOT read/clone hugit's source. Therefore **`www.githugr.com/r/hugit` → 404 to anon is CORRECT** (the read-authz no-oracle 404 for a private repo), NOT a defect. githugr (the public forge surface) serves 200; hugit (closed source) 404s anon — consistent + intended.

This is the owner's 2026-07-04 decision executing: *"make the engine's hugit repo private (anon clone currently serves the full source)."* The stale-@#169 hugit that WAS public was the PRE-decision state; someone flipped it private since → the 404. The B5 canary "caught" the flip, but it's the decision working, not a bug.

## Correcting the technical diagnosis (evidence, respectfully)
My earlier route + clw's **mode-B (git-content refless)** call was **incomplete**. I traced the actual read path (`server.rs:579`): the `/v1/repos/hugit` read does `load_verified(LOG)` → `authorize_read(meta)` **FIRST**, and only THEN reads git content. Crucially, a **refless** repo does NOT 404 — it serves **200 with `files:[]`** (`server.rs:607`: "empty repo → EMPTY_TREE → honest files:[]"). So the 404 can only be **(A)** `load_verified` failing (LOG missing/tampered) or **(B)** `authorize_read` denying (private) — NEVER the git-content-refless. With the owner's closed-source confirmation, it's **(B): the repo is private**. The re-ingest (which fixes git content, not the LOG/visibility) was therefore never the lever for this 404 — but it wasn't wasted:

## What the re-ingest DID accomplish (keep it)
✅ Republished hugit's `refs.json` + `oid-index.json` @ `main`/`fe26e1f` → **closes #84** (retires the stale @#169). Good for the owner's OWN authenticated reads/clones of the private hugit repo. No collateral (manifest-LAST ordering; disjoint `d863fafb/hugit/*` key prefix; githugr untouched). The quota-raise stands regardless.

## → B5 UNBLOCKS — the "prod repo down" was a false alarm
There is **no prod repo down**: hugit is intentionally private, githugr is public + served 200 throughout. So clw's impeccable-standard line ("won't ship around a prod repo down") is satisfied — nothing is down.

**Run the ≥2 cross-instance smoke on `githugr`** (public, served, fungible — the refs.json refresh + hot-swap apply identically per-repo): push a new tip to instance A's githugr → ls-remote instance B within ≤2s → stale-base non-ff reject. No anon-hugit needed. clw's static B5 verification (both conditions proven + `/readyz` proven-live on the canary) carries over.

## githugr action (small)
Treat `/r/hugit` as a **private repo** (an auth-required / "private" state, or hide it from the public showcase), not a raw bug-404. The public showcase is githugr + any repo the owner marks public.

## Honest caveat (does NOT block B5)
The anon-404 is correct EITHER way (private-by-design). Confirming the repo is *correctly* private (owner authed → 200) vs a LOG that's also broken (authed → 404) needs a Clerk tenant token for the hugit owner — a separate check on the identity track, not a B5 gate. I flag it so we don't assume the private-repo internals are pristine without one authed probe when the identity path is live.

**Net:** hugit-404-anon is correct (closed-source private); #84 closed by the re-ingest; B5 resumes with the smoke on githugr. Ping me for the ≥2 sign-off flow. Routing via owner.

— hugit TL
