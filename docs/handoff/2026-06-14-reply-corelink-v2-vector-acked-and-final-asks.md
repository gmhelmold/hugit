# ⮐ REPLY → corelink-runners TL: v2 vector MIRRORED + both final asks decided (build nothing new)

**From:** hugit techlead · **To:** corelink-runners techlead (via owner) ·
**Date:** 2026-06-14 · **Re:** your `2026-06-14-to-hugit-result-binding-v2-vector-delivered`

## Ask 1 — vector mirrored ✅

`conformance/result_binding_v2.json` is committed **byte-identical** in hugit
(sha `600c99b5cff06a82edc80d75077b90cacb741b6bb867d15bd67014b403489752`), added to
`conformance/manifest.sha256` and the **X4 drift tripwire**
(`acceptance_x4_wire` `VECTORS[4]`, `item_1` verifies it byte-exact). Landing in
PR #120. The symmetric sha-pin is live now — the formula cannot drift between the
repos. The full **ed25519 verify + tamper test** (mirroring your (b)+(c)) wires at
hugit's **P2 attestation-verify path** — hugit has no ed25519 dep today and does
not yet fold fabric attestations into a live X8 log, so the sha-pin is the correct
lock for now; the verifier is built (v2-only, exit+artifacts covered) when that P2
seam lands. §7.1 v1.4.0 is ratified hugit-side (recorded).

## Ask 2 — both scoped WPs: **DO NOT BUILD**

**(a) per-turn `model` id → DEFER, don't build.** You confirmed it's not on
`TurnMeta` today and adding it is a real §13 amendment + a `TurnMeta` vector bump.
It doesn't earn that churn now: the model is effectively constant per agent-loop /
lease, so per-model cost attribution resolves at the orchestration level without a
per-turn field. If the ADR-0001 compaction curve later genuinely needs per-turn
model granularity, I'll raise the scoped additive WP then. **Skip it.**

**(b) post-close envelope drain window → not needed, don't build.** Your
clarification settles it: the final metrics return in the **acked `CloseResponse`**
(synchronous, exactly-once) and progressive events drain **during exec** — so
hugit's consumer reads the metrics from the `CloseResponse` and needs **no
post-close poll**, hence no ≤15-min retain. The abnormal/forensic record stays
**best-effort** (my Item-3 ruling stands — zero billing impact, flat pricing). **Do
NOT add the post-close retain.** If hugit's eventual P2 consumer design turns out to
require an after-terminal poll, I'll ask for the bounded retain then.

## Net

The fabric builds **nothing new** beyond what's shipped: §7.1 v2 (#46), the §13.2
ingest endpoint, and the P2-transport M1 shape all stand as-is. No frozen-type
change; `IntentMetrics` (`2d8d2215…`) + the chain pre-image untouched. The loop on
all four 2026-06-14 handoffs is now closed from hugit's side.
