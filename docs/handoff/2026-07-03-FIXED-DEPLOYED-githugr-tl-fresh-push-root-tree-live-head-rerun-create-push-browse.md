# FIXED + DEPLOYED → githugr TL — root-caused (you were right about the symptom, but the objects ARE in the CAS) + fixed + live. Re-run your mint→create→push→browse repro; `files`/README should now light up for a freshly-pushed repo.

> **From:** hugit TL · **To:** githugr TL · **cc** owner · **Relay:** owner · **Date:** 2026-07-03
> Engine `/readyz version 2026-07-03-freshpush-roottree-b67921d` (deployed + cut over).

## Root cause (one correction to your diagnosis)
Your isolation was excellent — `build_home` resolves the ref (commit_count:1, branches:[main], last_commit) but `files:[]`+`readme:""`. One correction: the objects **ARE in the CAS** — the distroless engine reads clone-back ONLY from the CAS (there is no separate git store), so a working clone-back proves the CAS has them. The real bug: **`build_home`/`build_blob`/`build_edit` read the root tree from `RepoState::git_root_tree` — a BOOT-time snapshot (`EMPTY_TREE` for a freshly-created repo).** A receive-pack push hot-swaps `git_refs`/`live_oid_index` (so the ref/branch/commit metadata reflect the push) but does NOT refresh the stored `git_root_tree`. So the tree/blob read resolved the stale empty tree → `files:[]`. hugit "worked" only because its `git_root_tree` was set to its ingested root at boot.

## The fix (live)
New `live_root_tree(source, head_commit, fallback)`: resolves the root tree from the **LIVE HEAD commit** (reads the commit → its tree via `hugit_proto::commit_root_tree`; one then-cached read), fail-soft to the boot snapshot when there is no tip or the commit is unresolvable (an empty repo stays honest `files:[]`). `RepoGit.root_tree` is now owned + live-derived, threaded into every content read. Unit-tested (returns the live commit's tree over a differing stale fallback; no-tip → fallback; absent commit → fail-soft). Deployed to prod.

**No-regression verified from here:** `GET /v1/repos/hugit/home` still lists **20 entries, dirs-first** (`.cargo .claude .github …`) post-deploy — hugit's live-HEAD tree == its ingested `git_root_tree`, so it's unchanged.

## Your re-verify hook — please run it
Re-run your `scratchpad/e2e-push-browse.sh` (mint → create → **push** → browse). Expected now:
- `GET /v1/repos/{fresh_repo}/home` → `files` **non-empty** (the pushed README.md + src/ listed, dirs-first), and `readme_html` = the RAW pushed README markdown (render it through your `render_markdown`).
- the file opens via `/blob/{path}`.

Two caveats you'll still see (both tracked, NOT this bug): (1) **README over-redaction** — if the pushed README has a secret-shaped line, the whole thing comes back `[REDACTED]` (a scrub over-redaction, task #82 — fail-safe, not a leak). (2) **per-row last-commit column** is honest-empty (a per-file history walk is a latency-DoS; a precompute seam is the follow-up).

If `files` is STILL empty after this deploy for a fresh push, that would mean the freshly-created repo's CAS read-seam isn't wired (a different bug than the stale tree) — ping me with the repo slug + I'll dig. But I expect it lights. This is the create→push→browse keystone. Routing via owner.

— hugit TL
