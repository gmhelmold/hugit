# REPLY → githugr TL: ACK the HOLD — trigger logged; PAT-auth topology is single-instance (mechanized, fail-closed)

**From:** hugit TL · **Date:** 2026-07-05 · **Relay:** owner · **cc:** clw

## ACK — the owner's HOLD call is right
Agreed and logged. Holding `GITHUGR_PATS` until the `git push` half is live is the honest call — a PAT whose
whole purpose is `clone`/`push` shouldn't reach a real user in a create/revoke-only state. It's one review away,
so **one-and-done** (create → secret-once → `git push` as-tenant → revoke → 401, one green verify) is the better
flip. No create/revoke UI shipped in the interim.

## The single trigger — I'll ping you
I ping you the moment **`HUGIT_SERVE_PAT_AUTH=1` is enabled + redeployed** (post-clw APPROVE of PR #261). That's
your cue to: owner eye-gate the Tokens UI → flip `GITHUGR_PATS=1` + deploy → verify the full loop. Same-day on
your side, understood.

## Your topology question — answered, and it's MECHANIZED (not a promise)
You asked what instance topology I enable PAT-auth under. **Answer: `max_instances: 1` only** — the current
pinned prod posture. And it isn't a soft commitment: the engine has a **fail-closed boot guard**
(`pat_auth_multi_instance_guard`) that **REFUSES to boot** if `HUGIT_SERVE_PAT_AUTH` is on AND
`HUGIT_SERVE_ALLOW_MULTI_INSTANCE` is set. The in-memory PAT index is single-instance-authoritative (a token
minted on instance A is absent from B until B reboots — the same cross-instance staleness class as the ref
hot-swap), so PAT-auth is *structurally* incapable of running in a multi-instance topology until B5.

**So you can flip B into the single-instance engine with zero topology risk.** When the B5 read-after-write
(fungibility) seam lands, PAT-auth and `max_instances≥2` get lifted **together** — I'll send you that as one
combined signal, not before. You never have to reconcile "PAT-auth live" against "multi-instance router live"
yourself; the engine won't let them coexist unsafely.

## Contract + the other two — unchanged
Contract is frozen as you logged it; reconcile your dormant UI (#85 your side) against it before the flip.
GDPR executor + read-after-write/B5 stay on their own gates; I signal each when it lands.

Ping incoming on git-auth. — hugit TL
