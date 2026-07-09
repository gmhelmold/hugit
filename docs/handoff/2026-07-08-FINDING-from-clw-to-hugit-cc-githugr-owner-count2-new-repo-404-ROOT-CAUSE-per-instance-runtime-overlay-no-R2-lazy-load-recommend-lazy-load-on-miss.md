# FINDING → hugit TL (cc githugr, owner) — the count≥2 new-repo 404: ROOT-CAUSED in source. `POST /v1/repos` inserts the repo into ONLY the handling instance's in-memory `repos_runtime` overlay; other instances have no entry AND `repo_state` does not lazy-load from R2 → they 404 `git-receive-pack`/`upload-pack` until reboot. #272 refs-refresh can't cover it (it reloads refs for LOADED repos, never adds the repo SET). Recommended fix: **lazy-load-on-miss** (load the durable repo from R2 into the overlay before 404ing). Refs (#272) + token (#278) fungibility are unaffected — this is the repo-EXISTENCE plane.

> **From:** clw coordinator · **To:** hugit TL · **cc:** githugr TL, owner · **Relay:** owner · **Date:** 2026-07-08
> Owner asked me to look at githugr's count=2 new-repo git-registration lag. Root-caused below (verified in
> `hugit` source, not inferred).

## Root cause (confirmed)
`AppState::repo_state(repo)` (`state.rs:1248`) resolves a repo in exactly two places, then 404s:
```rust
pub fn repo_state(&self, repo: &str) -> Option<&RepoState> {
    if let Some(r) = self.repos.get(repo) { return Some(r); }      // 1. IMMUTABLE boot set
    self.repos_runtime.read()…​.get(repo).copied()                 // 2. per-instance runtime overlay
}                                                                  // else None → git-service 404
```
- `repos_runtime` (`state.rs:32`, `Arc<RwLock<HashMap<String, &'static RepoState>>>`) is **per-instance,
  in-memory.** `POST /v1/repos` inserts into the overlay of **whichever instance handled the create** — that
  instance serves the repo immediately (correct at count=1). No other instance gets the entry.
- **There is no lazy-load from R2 on a miss** — `repo_state` returns `None` → the git-service path 404s.
  Even though the repo's durable log lives in R2 (`<tenant_id>/<repo>.json`), a non-creating instance never
  consults it for a git-service existence check.
- Your own doc-comment names it (`state.rs:29-32`): a provisioned repo's LOG re-loads from durable `source`,
  but "its git seam is gone until the `HUGIT_SERVE_CAS_REPO` list is updated — the runtime-repo-set" is not
  propagated.

**So at count≥2:** create routes to instance A (A's overlay ← repo; A serves it) → the immediate
`git-receive-pack`/`upload-pack` routes to instance B → B's `repo_state` misses both maps → **404**, until B
reboots (which reloads the boot set). Exactly githugr's observation (`gdpr-verify-9c68b6` only became
git-serviceable after the window; `/v1` reads were 200 because those hit a different path).

## Why #272 doesn't cover it (and this isn't a refs bug)
`refresh_repo_refs_once` (`state.rs:712`) + the refresh thread (`:731`, every `REFS_REFRESH_INTERVAL_MS`)
reload `refs.json` for **every LOADED repo** — i.e. repos already in an instance's map. A repo absent from
the instance's maps is not "loaded," so the refresh never discovers it. #272 keeps *existing* repos' refs
fungible (verified, works); it is not a repo-SET propagation mechanism. This is a distinct plane:
**repo-existence**, not refs.

## Recommended fix — lazy-load-on-miss (window-free)
Make R2 the source of truth and the overlay a cache: in `repo_state` (or the git-service resolution), on a
miss, attempt to **load the durable repo from R2** (`<tenant>/<repo>.json` + reconstruct its git seam the same
way `POST /v1/repos` mints it) and insert into `repos_runtime`, before 404ing. Then ANY instance serves ANY
durably-created repo on demand — no propagation window, no polling.
- Must uphold the existing invariants: **404-no-oracle** for a genuinely-absent/private repo (a load-miss on
  R2 → the same honest 404, no existence leak); safe-slug gate before any R2 fetch; and the **receive-pack
  write seam** must be reconstructable on the lazy path (the create mints it from the CAS handles every
  instance holds — re-mint identically on load).
- **Alternative (simpler, but windowed):** a periodic repo-set refresh (enumerate R2 `<tenant>/*.json`, add
  new slugs to the overlay every N s) — same shape as #272 but for the catalog. Has a propagation window +
  polling cost; lazy-load-on-miss avoids both. Your seam, your call.

## Severity / scope (for the owner's go-live read)
- **Real, but bounded:** create→**immediate**-push (a common first-user forge flow) is unreliable at count≥2
  until the new repo propagates/reboots. The repo is durable in R2 (no data loss); it becomes serviceable
  once any instance loads it. `/v1` reads (dashboard, me/*) are fine throughout.
- **Latent until B5:** never manifested at count=1 (same instance creates + serves). The `≥2` flip activated
  it; my B5 smoke exercised *existing-repo* cross-instance push (refs converged) but not *create-then-push*,
  so it didn't surface there. Refs + token fungibility (what B5 signed) are unaffected.
- **Recommendation: fix before WIDE go-live** (lazy-load-on-miss). Not a beta/GDPR-verify blocker (githugr
  used a propagated repo). I'll re-verify the fix with a create→push→clone cross-instance smoke at ≥2.

Ping me the fix PR (lazy-load or refresh) → I re-audit + re-run the ≥2 create→push smoke that should have been
in the original B5 pass.

— clw coordinator
