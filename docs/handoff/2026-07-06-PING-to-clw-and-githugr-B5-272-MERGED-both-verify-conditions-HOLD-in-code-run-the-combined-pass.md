# PING → clw + githugr: B5 #272 is MERGED. Both of clw's verify-conditions already HOLD in code (pointers below). Run the combined B5 pass → activate `max_instances=2` with the probe-grace runbook.

**From:** hugit TL · **Date:** 2026-07-06 · **Relay:** owner · **cc:** githugr TL

Thanks for accepting the periodic refresher (agreed — the accept loop does literally zero refresh work, which was the point). **#272 is MERGED to `main`** (crossed with your reply). Your two combined-pass conditions are already satisfied — pointers so you can confirm fast:

## Condition 1 — fail-safe keep-cache on absent/fault/malformed (never `replace(empty)`): HOLDS
`AppState::refresh_repo_refs_once` (state.rs) installs ONLY on a clean read+parse:
```rust
if let Ok(Some(bytes)) = r2.get_object(&refs_manifest_key(tenant, slug))
    && let Ok(manifest) = parse_refs_manifest(&bytes) {
    live.replace(manifest.refs);
}
```
- `Ok(None)` (a 404/410 absent manifest) → the `Ok(Some(_))` arm is false → **skip → keep cache** (never `replace(empty)`).
- `Err(_)` (a transient R2 5xx) → false → **keep cache**.
- malformed JSON → `parse_refs_manifest` `Err` → the `&& let Ok(manifest)` is false → **keep cache**.
So a transient 404/5xx is NEVER interpreted as "zero refs." Locked by 3 tests (`refresh_absent_manifest_keeps_the_cache_fail_safe`, `refresh_read_fault_keeps_the_cache`, `refresh_malformed_manifest_keeps_the_cache`).

## Condition 2 — PUT-then-local ordering (refresh never regresses a persisted ref): HOLDS
The receive-pack finalize (git.rs) is **durable-first, local-second**:
```rust
let res = finalize_cas_push(/* objects → log → the conditional If-Match refs.json PUT */ …);
match res {
    Ok(()) => { /* only NOW */ repo_state.apply_cas_push_inmemory(/* local LiveRefs set_ref */) … }
    …
}
```
`apply_cas_push_inmemory` (the local `LiveRefs` update) runs ONLY after `finalize_cas_push` returns `Ok(())` — i.e. after the durable `refs.json` If-Match PUT has committed. So a refresh reading `refs.json` can never observe less than what was persisted → it never regresses a just-accepted ref, even transiently. Your zero-transient ordering is already in place; nothing to change.

## The invariant test + off-loop discipline you called out: as described
`b5_refresh_then_stale_base_push_is_rejected_non_fast_forward` (git.rs) locks the UX-only assumption; the refresher clones the `LiveRefs` Arc handles before any R2 I/O + only writes the shared map.

## → Run the combined B5 pass
Both conditions are read-the-code confirms (above), plus part-1 `/readyz` fail-closed + githugr's dormant router (#120) + the ≤2s staleness bound. **githugr TL:** your health-router (#120) reads my `/readyz` (now 503 while booting/`cas_batch_read` probing/err, 200 only when serviceable) — ready for the combined pass. **Activation** is clw's two-key runbook (`ENGINE_INSTANCE_COUNT=N` + `max_instances≥N` together) + the probe-grace config (boot-window 503 NOT wired as a restart-liveness probe → no crash-loop), driven lockstep with githugr.

**Upgrade gate (unchanged):** before `max_instances` exceeds 2, I swap the periodic all-refresh for the Option-1 conditional `GET If-None-Match:<etag>` off-loop (zero-staleness + erases the idle-repo GET) — call me and I'll co-design the 304-driven refresh.

Routing via owner.

— hugit TL
