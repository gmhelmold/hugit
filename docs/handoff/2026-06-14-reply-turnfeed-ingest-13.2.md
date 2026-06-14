# → corelink-runners TL: §13.2 turn-feed ingest — shape ACCEPTED, prefer the socket, +1 field

**From:** hugit TL · **To:** corelink-runners TL · **Via:** owner · **Date:** 2026-06-14 ·
**Re:** `corelink-runners/docs/handoff/2026-06-14-hugit-turnfeed-ingest-proposal.md`
(PROPOSAL — your §13.2 mechanism). Non-blocking; here's hugit's position.

## 1. The wire shape is ACCEPTED — it maps cleanly to ADR-0001 + IntentMetrics §13.4

The `TranscriptEvent` variants (`model_turn`/`tool_call`/`tool_result`/`prompt`, each
`bytes_b64` + optional `usage` + `busy_ms`) are exactly what the
`hugit-ledger::envelope` producer + the IntentMetrics fold need:
- raw `bytes_b64` (verbatim, unscrubbed — §13.3 redaction is OUR write-path job, agreed)
  → `raw_transcript_ref`; the summariser compacts → `task_transcript_ref`.
- `usage` cache-split token counts → `IntentMetrics.tokens` (`null` = unknown, never
  fabricated — correct).
- `busy_ms` per event → `active_ms`; event counts → `model_turns` / `tool_calls`;
  the close signal's `wall_ms`/`active_ms`/`capture_incomplete` → the rest.

One derivation note (not a blocker): `cost_usd_micros` (§13.1, WA4 integer-micro-USD)
is runner-computed at close from pricing × `usage`, not carried per ingest event —
confirm that's still the close-path source and the ingest stream stays cost-free
(raw + usage only). That keeps pricing authority on the fabric, where it belongs.

## 2. Prefer the **unix-domain socket** (mounted), not HTTP — security, your call on mechanism

You own the §13.2 mechanism, so this is a recommendation, not a demand: for an
**untrusted-compute box**, requiring outbound HTTP for telemetry adds a network egress
surface (a potential exfil channel) and a reason to punch the box's network open. A
**unix-domain socket mounted into the box** (the mount = the auth/scope boundary, no
egress, no in-box Bearer to leak) is the cleaner fit for the threat model and is
strictly simpler for the in-box emitter (write to an fd). If the socket is awkward
fabric-side, the lease-auth HTTP endpoint is acceptable — but the socket is hugit's
preference. Either way the EVENT SHAPE above is unchanged.

## 3. Per-turn meta — sufficient, with ONE addition: the model id

Your surfaced meta (turn index, ts, tool name, token count) covers the compactor and
most of IntentMetrics. The one field to add: **the model identifier per
`model_turn`** (e.g. in `usage` or the event meta). `ContextEnvelope.authorship.model`
must be REAL per-turn (a fleet job can switch models mid-trajectory); without it that
field is either fabricated or lost. If the model API response you already capture
carries the model string, surface it — that closes the gap.

## 4. The in-box emitter is a coordinated production seam (flagging, not blocking)

The WRITE half — the in-box agent loop actually streaming these events — is the
production envelope-capture integration (today hugit captures the trajectory
**in-process** only, in the `hugit-dogfood` envelope leg / `TrajectoryRecorder`; the
real worker-agent-in-the-box emitter is not yet built). That emitter lives at the
agent-runtime↔runner boundary, so its build is a coordination point (likely alongside
the P2 AC/runner seam going live) — owner-routed when we wire it. Your fabric side
(endpoint/socket + box injection + hook-write) is the right half to build now; hugit's
adoption is the last mile, exactly as you framed it. No counter-proposal blocks you —
build it; the shape + socket-preference + model-id are hugit's inputs.

— routed via owner; no `path`/`git` coupling; frozen contract unchanged (§13.2 delegates the channel to the runner).
