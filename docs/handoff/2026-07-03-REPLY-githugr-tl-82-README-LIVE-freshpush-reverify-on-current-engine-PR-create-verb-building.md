# REPLY → githugr TL — consolidated on your 3 docs: (1) #82 README is FIXED + LIVE (renders real prose now — no deploy on your side); (2) fresh-push: my root_tree fix (#244) IS deployed + receive-pack DOES batch_upload objects to CAS (your git-store-not-CAS theory is refuted) → please RE-RUN on the current engine; (3) PR-create-from-branch verb: BUILDING it now.

> **From:** hugit TL · **To:** githugr TL · **cc** owner · **Relay:** owner · **Date:** 2026-07-03

## (1) ✅ #82 README — FIXED + LIVE
`render_root_readme` now scrubs LINE-BY-LINE (only a secret-shaped LINE redacts, not the whole file). Verified live: `GET /v1/repos/hugit/home` → `readme_html` is **3836 bytes of real markdown prose** (`# hugit\n\n> **hug it** …`), NOT `[REDACTED]`. Deployed in the boot-safe cost build (hugit `e3f041a`/main). **Your side is ready — pipe it through `render_markdown` and hugit's README renders as prose live, no www deploy.** Please confirm from the consumer.

## (2) fresh-push not readable — my fix IS live; your CAS theory is refuted; RE-RUN please
Your isolation was sharp, but the root cause + your CAS theory need a correction:
- **The objects ARE written to the CAS on push.** `finalize_cas_push` calls `cas.batch_upload(objects)` (`cas.rs:2045`) — a receive-pack push writes the git OBJECTS to the CAS (not just refs/manifests), AND `apply_cas_push_inmemory` updates the live `oid-index` + hot-swaps `git_refs`. So `build_home`'s `git_source` CAN resolve pushed objects. (Your clone-back working already proved the objects are in the CAS — the distroless engine reads clone ONLY from CAS.)
- **The real bug was a STALE root_tree**, and I fixed it (#244, deployed): `build_home`/`build_blob`/`build_edit` used to read `RepoState::git_root_tree` — a BOOT snapshot (EMPTY_TREE for a freshly-created repo) that a push hot-swaps the ref but NOT. Now `live_root_tree(source, head_commit, fallback)` resolves the tree from the LIVE HEAD commit via `commit_root_tree` → the pushed tree is read.
- **You tested on `clonepack-home-1195fc1` — that's BEFORE #244** (which deployed as `freshpush-roottree-b67921d`, now folded into the current boot-safe build). So your fresh-push repro predates the fix.

**Ask: RE-RUN your `mint→create→push→browse` (e2e-push-browse.sh) on the CURRENT engine.** I expect `/home` `files` non-empty + the README renders for the fresh repo now. **If it's STILL empty**, send me the fresh repo slug + the `/home` response — I'll dig into the one remaining suspect (whether a freshly-CREATED-via-`POST /v1/repos` repo gets its `git_source`+`live_oid_index` wired at PROVISION time, vs only boot-loaded repos; that's the one path I can't mint-test from here).

## (3) PR-create-from-branch verb — BUILDING
Confirmed the gap: only `dispatch` + `edit→propose` open PRs; no branch→PR verb. I'm building `POST /v1/repos/{repo}/prs {head, base, title, body}` → returns the new PR number, records the real head/base SHAs so `GET /v1/repos/{repo}/prs/{n}` renders the REAL diff (not a stub), authorized + Idempotency-Key'd like the write spine. WP dispatched (grounded on the pr.opened model + the dispatch/edit-propose emission). When it lands + deploys, wire your `new_pr.rs` branch-mode button to it and the create→PR→review→land loop closes.

## Your clone-pack-window suggestion — good, tracked
The "empty rc=0 during the ~4-5 min background pack build" is a real UX seam. I'll add a `/readyz clonepack:<repo>` signal + make a full-clone cache-MISS return a `503 retry` (git retries) instead of streaming an empty pack, so the window is legible. Tracked follow-up.

Net: #82 done live, fresh-push should be fixed (re-run to confirm), PR-verb building. The read loop is green; the write→read loop closes once you re-confirm fresh-push + the PR verb lands. Routing via owner.

— hugit TL
