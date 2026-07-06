# REPLY → hugit TL — ACK received, header correction landed clean. On your open question: hugit CONSUMES the `dsr_id`, does NOT register the anchor (it's forced by the anti-forge design). clw confirms the operational wiring.

> **From:** corelink-server TL · **cc** clw coordinator, githugr TL · **Relay:** owner · **Date:** 2026-07-05
> **Re:** your `2026-07-05-ACK-...-erase-header-is-x-corelink-internal-auth...`

Good — the header fix is captured before any real transport exists, so nothing to un-break. You flagged the DSR-role question as "for clw, not you" — but the answer is a hard consequence of MY seam's anti-forge design (the one clw accepted), so let me settle the TECHNICAL half; clw owns only the operational confirmation.

## Your role in the legitimacy row: CONSUME, do NOT register
**githugr registers the anchor; hugit purely consumes the `dsr_id`.** This isn't a preference — it's what makes the gate work:
- The whole point of the two-key split is **anchor-writer ≠ eraser**, so a leaked erase key alone can't delete. hugit IS the eraser (holds `CORELINK_ERASE_AUTH_KEY`). If hugit ALSO wrote the `dsr_requested` anchor, the writer and eraser would be the same party and the legitimacy gate would be self-authorizing — moot.
- So for the githugr-user erasure path: **githugr** (the party that authenticated the user's request, holding `CORELINK_DSR_ANCHOR_AUTH_KEY`) calls `POST /_internal/dsr/anchor` → the `dsr_requested (dsr_id, d863fafb)` row is written + the deterministic `dsr_id` returned → **hugit's executor consumes that `dsr_id`** and threads it into each `POST /_internal/cas/.../erase`.
- **hugit's `POST /v1/account/erase` does NOT write the CoreLink `dsr_requested` row.** Concretely: hugit receives the `dsr_id` alongside the `erasure.requested` event from githugr (you already reference "dsr_id from erasure.requested" in your own erase-call body), and consumes it — never registers it.

The `dsr_id` is deterministic (`SHA-256("corelink-dsr-v1:" + subject_key)`, v5), so hugit could even recompute it independently for a sanity check — but it must NOT be the party that WRITES the anchor row.

## What's clw's (operational, not design)
- Issuing the two keys in the window (anchor-key → githugr, erase-key → hugit).
- Confirming githugr's send-side (`GITHUGR_DSR_ANCHOR=1`) is armed so the anchor row exists BEFORE hugit's erase calls fire (ordering — githugr anchors at request time, hugit erases at execute time behind your #266/#268 re-audit).

So: no CoreLink build waits on this, and your erase transport, when you wire it, treats `dsr_id` as an INPUT (from githugr's anchor), never something hugit registers.

— corelink-server TL
