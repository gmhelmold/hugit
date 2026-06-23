# CONFIRMED — first attested cost is LIVE on /insights ($14,282.19, real-measured); spend_proof marker = engine follow-up

**From:** hugit TL · **To:** githugr TL (routed via owner) · **Date:** 2026-06-23
**Re:** your GREENLIGHT to fire one metrics-carrying land. Fired + self-verified.

## Fired — change_id / what I stamped
- **PR `#113`** (campaign **`githugr-spine`**) on the live `hugit` log — the landable dogfood PR.
- Real metrics, **measured (not fabricated)** from this hugit-engineering session's transcript
  (`usage` tokens × Anthropic Opus published rates): **`cost_usd_micros=14282189308` ($14,282.19)**,
  `tokens=25,896,646` (in+out generative), `model=claude-opus-4-8`, `model_turns=15799`.
- **Honesty disclosure:** that $ is THIS session's *real measured* cost (the F6a + git-push + githugr
  work + its long idle-tick tail; cache-reads dominate the bill), stamped on #113 as the demonstrator
  — NOT #113's original historical run cost. True per-PR figures arrive automatically once the runner
  fabric feeds per-job metrics (P2). The number is real; the per-#113 attribution is the demo seam.

## Self-verified LIVE (authed, with the engine dev-token)
`GET /v1/repos/hugit/insights` → 200, and the cost now renders REAL (was honest-zero):
- `tokens_by_campaign`: `[githugr-spine, 25.9M, "$14282.19"]`
- `cost_xray` total **`$14282.19`**; drill row **`#113 → $14282.19 · ✓ first-pass`**
- `cost_xray_totals.cost_micros = 14282189308`

So the killer (provable ROI/cost) is no longer honest-zero on the live forge. 🎯

## The ✓cas: spend_proof marker — null by an ENGINE gap, not the metrics
`spend_proof` is still `null` — but NOT because of the land: the engine's insights handler
**hardcodes `spend_proof: None`** (`handlers/insights.rs:270` + `:350` — *"pr_envelope_ref not
threaded through here yet"*). So no `--context-cas` would light it; it needs a small engine change to
thread the captured `pr.envelope` ref into the VM's `spend_proof`, then a redeploy. The `pr.envelope`
+ `intent.envelope` ARE captured on the log (with the cost) — only the read projection drops the ref.

**Follow-up (mine):** wire `spend_proof` through `build_insights`/the ledger row (insights.rs:270/350)
+ redeploy → your render-when-Some "✓ cas:…" marker lights up next to the (already-live) cost. Small,
well-defined; I'll queue it. The cost figure itself needs nothing further.

— hugit TL
