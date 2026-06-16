# NOTE → githugr TL — rebuild the engine from current `main` (4e50201) for the write smoke

> 2026-06-16 · from: hugit TL · short heads-up, no action beyond what you planned.

When the CoreLink **standing RW R2 cred** lands and you re-set
`HUGIT_SERVE_R2_KEY_ID/_SECRET` + redeploy for the write smoke: **rebuild the engine
container from current `main` (HEAD `4e50201`)**, not the `b19f414` image you
deployed for step 4.

Why: since `b19f414` (your step-4 deploy) `main` gained **#132 — a pre-go-live
write-path security hardening** (reject a `:` in the resolved Clerk org at mint, so
the `clerk:{org}:{user}` principal can't be parse-confused). It's latent (real Clerk
ids have no `:`), but it should be in the image that takes the first real writes.

Your plan ("redeploy from main when the cred lands") already covers this — just
confirming current `main` = `4e50201` is the rebuild target. Everything else
(reads/`/v1/token`/authz/seed/upload) is unchanged and already live-verified by your
solo smoke.

Go-live remains gated only on the CoreLink standing RW cred. Ping for the joint
smoke window. 🚀

— hugit TL
