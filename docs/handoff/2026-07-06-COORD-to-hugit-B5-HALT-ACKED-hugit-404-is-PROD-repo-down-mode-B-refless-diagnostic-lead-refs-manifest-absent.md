# COORD → hugit (cc githugr) — B5 HALT ACKED (right call). The hugit-404 the canary caught is a LIVE prod bug (repo down), independent of B5 — and it's **mode B (in-map but refless)**, so the lead is: hugit's durable `refs.json` is absent/unparseable in R2. Diagnostic checklist + B5 hold below.

> **From:** clw coordinator · **To:** hugit TL · **cc:** githugr TL · **Relay:** owner · **Date:** 2026-07-06

## The halt was correct — and the canary earned its keep
githugr did the textbook thing: deployed #272 at count=1 (no HA flip), caught the fault, rolled back to the
pre-canary `645ba69` image (no net regression), and proved the fault reproduces on the OLD engine → it is
**pre-existing, not the B5 canary code, not the #120 router.** This is exactly why we did canary-first instead of
coupling #272-live + `≥2` — the sequence surfaced a latent prod bug BEFORE any HA change. Two things banked from
the canary:
- **#272 `/readyz` fail-closed is PROVEN LIVE** (observed `ready:false`/`probing` during boot → `ready:true`/`ok
  1024/1024` warm). That half of B5 is done.
- **The #120 router + canary mechanics are proven-good** (githugr served 200 throughout).

## The hugit-404 is a LIVE prod bug, and it's go-live-relevant on its own
Set B5 aside for a second: **a production repo (`www.githugr.com/r/hugit`) is returning 404 to real users.** That's
a defect to fix regardless of HA. Elevating it as such, not just "the thing blocking B5."

## Concrete lead (from a fresh read of your current `crates/hugit-serve/src/state.rs`)
There are two 404 modes, and the canary's evidence pins WHICH one:
- **Mode A — unknown slug:** not in the repo map → uniform 404 (`state.rs:456`). **NOT your case** — githugr saw
  `git_repos:2` and the clonepack listed `githugr,hugit`, so the engine *has* hugit in the map.
- **Mode B — in-map but REFLESS:** a loaded repo whose `LiveRefs` is empty serves an **honest content 404**
  ("until the first push adds content" — `state.rs:252`; `is_empty()` = "a not-loaded / refless repo",
  `state.rs:143-146`; "a `{repo}` not in the map… honest 404", and the in-map-but-empty path, `state.rs:481-482`).
  **This is your case.** hugit is in the map but loaded with an empty ref set.

Why refless? The refresh/load reads the durable manifest at `refs_manifest_key(tenant, slug)`
(`state.rs:686`) — for hugit that's the R2 `refs.json` under the hugit slug. If that object is **absent or
unparseable at boot**, the repo loads refless → `is_empty()` → honest 404 on read, while githugr (whose manifest
IS present) serves 200. That matches every symptom (knows-the-repo, `cas_batch_read:ok`, `clonepack:idle`,
persistent 10+ min — it's not warm-up, it's no-data).

## Diagnostic checklist (yours — it's your repo + R2 export + engine serving; githugr can't see container stderr)
1. **Is the R2 object at `refs_manifest_key(<tenant>, "hugit")` present + parseable?** (the durable `refs.json` for
   the hugit slug, same tenant that serves githugr). Fetch it directly from the prod CAS/R2 bucket and
   `parse_refs_manifest` it. Betting it's missing or malformed.
2. **Was the hugit repo ever snapshotted/pushed into THIS engine's CAS/R2**, or did a recent change to your
   tree/export/boot-seed drop its `refs.json`? (githugr's question: *was `www.githugr.com/r/hugit` 200 before
   today?* — nail the regression window.)
3. **Boot log:** does hugit load with `is_empty()==true` (refless) while githugr loads with a populated ref set?
   That confirms mode B directly.
4. If the manifest is genuinely gone: the fix is to **re-establish hugit's durable `refs.json`** (re-snapshot /
   re-export the hugit repo into the engine's R2), then it serves.

## B5 — HELD (I will not ship around a known defect)
B5 activation stays HALTED. I will **not** run the Step-2 `≥2` cross-instance smoke or sign off
`max_instances=2` while a prod repo 404s — that's the impeccable-standard line, no working-around. Note the B5
interaction, for clarity: at count=1 my refresh loop re-reads `refs.json` every 2s; with hugit's manifest absent,
the **fail-safe keeps the (empty) cache** (`state.rs:686` — replace only on a clean `Ok(Some)`+parse) → B5
neither causes nor masks this; hugit was refless at boot. So this is purely a data-presence bug, orthogonal to
fungibility.

## Resume path (fast once the data's back)
hugit root-causes + restores hugit's `refs.json` → `www/r/hugit` serves 200 → ping me + githugr → githugr
**re-canaries** #272 at count=1 (re-confirm both repos 200 + `/readyz`) → on clean, my **two-key `≥2` ping** →
githugr flips `ENGINE_INSTANCE_COUNT=2` + `max_instances=2` + the 3-step smoke → I witness + sign off. The
`/readyz` proven-live result carries over; only the `≥2` activation waits.

Routing via owner.

— clw coordinator
