# DIAGNOSIS + FIX-READY → owner (cc clw + githugr): the hugit-404 is CONFIRMED clw's mode-B (refless — hugit's `refs.json` absent/malformed in R2). The fix is a re-ingest of hugit into the engine's R2 (also closes #84 stale-hugit). Tooling + creds are READY. Requesting GO for the prod R2 write.

**From:** hugit TL · **Date:** 2026-07-06 · **Relay:** owner

## Confirmed (my independent probe, real-consumer rule)
- `www.githugr.com/r/hugit` → **404**; `/r/githugr` → 200; `www/` → 200. Reproduced githugr's finding from the public www (not engine-direct).
- clw's **mode-B (in-map but REFLESS)** is the grounded cause: hugit is in the map (`git_repos:2`, clonepack lists both) but boots with an empty `LiveRefs` → honest content 404. That happens when the durable **`refs.json` at `<tenant>/hugit/refs.json` is absent/unparseable at boot** (githugr serves because its manifest is present). Symptoms all fit: knows-the-repo, `cas_batch_read:ok`, `clonepack:idle`, persistent 10+ min (not warm-up = no-data).
- **Pre-existing, NOT B5** (githugr proved it reproduces on the pre-canary `645ba69` image; the B5 refresh only READS refs.json + is fail-safe on absent → keeps cache, so it neither caused nor masks this).
- **Regression window:** hugit WAS served (stale @#169 — the standing #84). So its `refs.json` existed and was dropped/corrupted since — most likely a partial/failed re-ingest attempt (the #84 work), not code.

## The fix (one operation, closes two things)
**Re-ingest hugit into the engine's R2** — `git-ingest <hugit-git-dir> hugit`: uploads the git objects to the CAS (`cas:rw`) + republishes the durable manifests `refs.json` + `oid-index.json` under `d863fafb/hugit/…`. This:
- **Fixes the refless 404** (fresh, complete, chain-verified `refs.json` → hugit boots with a populated ref set → serves 200), AND
- **Closes #84** (re-ingest hugit@main → the engine serves current hugit, not the stale @#169).
Then githugr **re-canaries #272** (boot-loads the restored hugit) → both repos 200 + `/readyz` clean → clw's two-key `≥2` ping → the smoke → sign-off. B5 resumes cleanly.

## Ready to execute — I have the pieces
- **Tooling:** `crates/hugit-serve/src/bin/git-ingest.rs` (the git→CAS+R2 manifest writer).
- **Creds (local, verified present — never printed):** `~/.hugit/secrets/corelink/{git-cas-rw-pat, ingest.env}` (the one-shot `cas:rw` + R2 RW grant; the standing engine cred is read-only and PUTs 403, so this is the right grant for the manifest writes).
- **git-dir:** the local hugit repo at current `main` (`fe26e1f` — the up-to-date tree, which ALSO closes the stale-@#169 gap).
- Target: tenant `d863fafb`, slug `hugit` — the SAME tenant that serves githugr, but a DISJOINT key prefix (`d863fafb/hugit/…` vs `d863fafb/githugr/…`), so the re-ingest cannot touch githugr's manifests.

## Why I'm asking for a GO before running it (not stalling — confirming a prod data write)
This is a **write to the prod R2 that serves the live engine** — a hard-to-reverse, outward-facing op (overwrites hugit's mutable manifests), same discipline as the staged prod deploys. #84 sanctions "re-ingest hugit@main," but not blindly-right-now. On your GO I run it carefully (verify the manifests land + parse, confirm no githugr collateral), then ping githugr to re-canary. If you prefer, I can first do a read-only verify of the current `refs.json` state before the write — say the word.

**Net:** confirmed mode-B refless; the fix is the hugit re-ingest (also closes #84); tooling + creds ready; requesting your GO for the prod R2 write, then I execute + coordinate the re-canary → B5 resumes.

Routing via owner.

— hugit TL
