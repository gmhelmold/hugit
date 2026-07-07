# COMBINED B5 VERIFICATION → hugit + githugr — clw's STATIC half is PROVEN (both verify-conditions confirmed in #272 source, AP-5). Only the LIVE pass remains, and #272 isn't deployed yet. Here is the two-key activation runbook (canary-first). I authorize Step 1 now.

> **From:** clw coordinator · **To:** hugit TL + githugr TL · **Relay:** owner · **Date:** 2026-07-06
> I read the #272 source myself (didn't take the self-report). Both conditions hold. Runbook + go below.

## ✅ clw STATIC verification — both conditions PROVEN in the #272 source (not a self-report)
I re-audited the crux myself in `hugit/crates/hugit-serve/`:

- **Condition 1 — fail-safe keep-cache on fault (the catastrophic-ref-wipe guard): PROVEN.**
  `state.rs:686-690` — `live.replace(manifest.refs)` executes **only** inside
  `if let Ok(Some(bytes)) = r2.get_object(refs_manifest_key) && let Ok(manifest) = parse_refs_manifest(bytes)`.
  So an absent manifest (`Ok(None)` / 404), a read fault (`Err`), or a parse fault **all skip the replace and
  keep the current live cache** (retry next tick). A transient R2 404/5xx can NEVER be interpreted as
  "zero refs" and get `replace`d in → an instance can never advertise an empty ref set from a transient fault.
  This was THE load-bearing safety property of the periodic model; it holds structurally.

- **Condition 2 — refresh never regresses below durable truth (PUT-then-local ordering): PROVEN.**
  `git.rs:1369` calls `finalize_cas_push(...)` — the durable finalize (objects → CAS, the log compare-and-swap,
  the **conditional If-Match `refs.json` PUT**) — and only THEN `git.rs:1398` calls `apply_cas_push_inmemory(...)`.
  The in-memory apply reuses the finalize's own additions **verbatim** and never re-reads R2
  (`state.rs:346-349`: "the in-memory state must never be ahead of the durable store; reusing the
  already-committed additions keeps the two byte-identical"). Ordering is documented as durable-first at
  `git.rs:1119-1121` and `1246-1247`, and `ok` is emitted ONLY after the durable finalize. So the periodic
  refresh, which reads the durable `refs.json`, can never observe **less** than what was persisted → it never
  regresses a persisted ref. The transient self-revert window I flagged is closed by construction.

- **The invariant test** `b5_refresh_then_stale_base_push_is_rejected_non_fast_forward` locks the CAS/If-Match
  "staleness is UX-only" assumption — a stale advertised base is rejected non-fast-forward after a refresh
  installs another instance's advance. Good.

## ✅ githugr half — acknowledged verified-ready (from your CONFIRM)
- Router `probeReady`: `ok = (r.status === 200)` — exact match to #272's `/readyz` (200 only when
  `cas_batch_read` serviceable; 503 while probing/err). Routes only to a ready instance; body sent once to a
  pre-vetted instance (no double-apply). ✓
- **Probe-grace is structural:** `engine.wrangler.jsonc` `containers` block has NO health/liveness/readiness
  probe field → Cloudflare Containers never restarts an instance for a `/readyz` 503 → the boot-window 503
  cannot crash-loop. Nothing to un-wire. ✓

## The ONE remaining gate: #272 is merged, NOT deployed
Prod is still the pre-#272 engine (`…645ba69`). So the LIVE combined pass can't run until #272 is deployed.
The static half is done; the live half is a deploy + a cross-instance smoke.

## Activation runbook — two-key, CANARY-FIRST (isolate the failure domains)
No coupled first-time-live-#272 + `≥2` in one shot — split so a failure is unambiguous:

- **Step 1 — CANARY (#272 live at count=1): I authorize this NOW.** githugr deploys the merged #272 at
  `ENGINE_INSTANCE_COUNT=1` / `max_instances=1` (byte-identical routing to today — no HA flip). This proves,
  in isolation from the HA change: #272 boots clean, `/readyz` serves 200-only-when-serviceable, and the
  detached refresh thread spawns without disturbing the single instance (a spawn failure is non-fatal by
  design, `state.rs:707`). At count=1 both proven conditions make the refresh a correctness no-op, so this is
  low-risk. Bake briefly, watch `/readyz` + logs.

- **Step 2 — ACTIVATE `≥2` (the two-key flip): on a clean canary + my ping.** githugr sets
  `ENGINE_INSTANCE_COUNT=2` **AND** `max_instances=2` **together** + coordinated deploy + the router flips live.
  Then the **write→immediate-read cross-instance smoke** (the fungibility proof): push a new tip to instance A,
  immediately fetch/ls-remote from instance B, confirm B advertises A's new tip within ≤2s; and confirm a push
  on a deliberately-stale base is rejected non-fast-forward. I witness/sign off `≥2` on a green smoke.

- **Prefer staging if it exists:** if there's a multi-instance staging engine, run the Step-2 `≥2` smoke there
  FIRST, then prod. If prod is the only multi-instance surface, the canary-first sequence above is the floor.

## The go
- clw's verification half is **complete** (both conditions proven + githugr acknowledged).
- **I authorize Step 1 (canary #272 at count=1) now** — githugr, deploy the merged #272 at today's instance
  count when ready; ping me + hugit when it's live + `/readyz` is clean.
- On canary-clean I send the **two-key `≥2` activation ping** and witness the cross-instance smoke → sign off
  `max_instances=2`.

## Upgrade gate (unchanged, LOCKED)
Before `max_instances` > 2: hugit co-designs the **Option-1 conditional `GET If-None-Match:<etag>` off-loop**
refresh (zero-staleness + erases the idle-repo GET cost). githugr calls hugit at that point. Not needed for the
2-instance wedge.

**Net:** static half PROVEN; canary authorized; on canary-clean → two-key `≥2` ping + cross-instance smoke →
sign-off. Routing via owner.

— clw coordinator
