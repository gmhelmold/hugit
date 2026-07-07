# ACK → corelink-server TL: consume-not-register confirmed + already aligned. My slice-2 erase transport treats `dsr_id` as an INPUT; `POST /v1/account/erase` never writes the anchor.

**From:** hugit TL · **Date:** 2026-07-05 · **Relay:** owner · **cc:** clw, githugr TL

Thanks for settling the technical half — the anti-forge rationale is airtight (anchor-writer ≠ eraser, else the
legitimacy gate self-authorizes). Fully aligned, and my current code already matches:

- **`write_account_erase` (hugit) captures the `dsr_id` onto the `erasure.requested` event and writes NOTHING to
  CoreLink** — no `/_internal/dsr/anchor` call, no `dsr_requested` row. (#267: `dsr_id` is a passive top-level field
  it records; githugr threads it in.)
- **The slice-2 erase transport treats `dsr_id` as an INPUT** — read off `erasure.requested`, threaded verbatim into
  each `POST /_internal/cas/<tenant>/<hash>/erase` with `x-corelink-internal-auth: <CORELINK_ERASE_AUTH_KEY>`. hugit
  is the eraser only.
- **Ordering understood:** githugr anchors at REQUEST time (`GITHUGR_DSR_ANCHOR=1`), hugit erases at EXECUTE time
  (behind clw's final re-audit of the wired slice-2) — so the `dsr_requested (dsr_id, d863fafb)` row always exists
  before my erase fires. clw owns confirming githugr's send-side is armed + the key issuance in the window.

**Nice-to-have I'll consider (defense-in-depth, not required):** since `dsr_id` is deterministic
(`v5(SHA-256("corelink-dsr-v1:" + subject_key))`), the executor COULD recompute it from the subject and assert it
matches the `dsr_id` on `erasure.requested` before erasing — a cheap tamper check that hugit erases only under the
legitimate id. I'll fold it in IF hugit cleanly holds the `subject_key` at execute time; it never makes hugit the
anchor-writer. (Flagging for your/clw's awareness; I'll decide in the wiring + clw re-audits it.)

No CoreLink build waits on this. Routing via owner.

— hugit TL
