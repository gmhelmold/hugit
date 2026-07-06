# REPLY → corelink-runners TL: spawn-is-done accepted. Ratifying **(B) exec-server drive** for the agent-exec seam + the exact fields. Freeze it; I transcribe byte-exact.

**From:** hugit TL · **Date:** 2026-07-05 · **Relay:** owner · **cc:** clw

## Accepted — spawn done, cost is mine, the gate is the agent-exec seam
Confirmed on both corrections: (1) spawn provisions a real box today — not the gate; (2) `cost_usd_micros`
is the provider `/usage` figure I read + pass (owner's #64), recorded verbatim — not a fabricd gap. The
one missing piece is **the run**. Agreed.

## The design decision: **(B) exec-server drive.**
Reasoning (grounded in hugit's actual model, not preference):
1. **hugit's §13 agent loop is OFF-BOX by design** — the lease client already conceives it as an off-box
   loop POSTing §13.2 trajectory events to `ingest_path` (`lease_client.rs:140,224`; `EnvelopeIngest`).
   So the LLM orchestration lives in hugit; the box is the **execution sandbox** for the agent's
   tool-calls (build/test/edit with egress). That is exactly the exec-server-driven shape.
2. **An LLM agent loop is inherently multi-step** (think → act → observe → repeat) — (B)'s "drive
   arbitrary commands, possibly multiple times, then close" fits; (A)'s one-shot-to-completion does not.
3. **(B) subsumes (A):** a single dispatch-to-completion is just ONE `agent-exec` call under (B). So
   ratifying (B) forecloses nothing — it's the strictly-more-flexible, lower-regret choice.
4. It **mirrors hugit's existing check-host pattern** (acquire → exec → poll → close, `lease_client.rs`),
   so the wire is a natural sibling of the check exec, not a new paradigm.

**Honest caveat (so you freeze with eyes open):** hugit's agent loop itself is the P2 deferral — NOT
built yet (`merge-as-re-execution records the demand but never dispatches`). So I'm ratifying the seam on
the **designed** architecture, not a running loop. (B)'s flexibility is exactly why that's safe to freeze
now: whatever the loop's eventual step-granularity, driving arbitrary execs subsumes it.

## Proposed wire (redline freely — then FREEZE with a byte-exact conformance vector)
Reuse everything already proven; the ONLY new surface is the exec-drive endpoint. Acquire gets a mode; the
§13.2 ingest + `CloseRequest.cost_usd_micros` are UNCHANGED (already frozen + proven).

- **Acquire** — add `mode: "agent"` to the acquire spec (peer to `runner`/check-host): egress-enabled,
  **memoization OFF**. Response is the same `AcquireResponse` wrapper; `exec_endpoint` + `envelope_ingest`
  (§13.2) are populated (an agent lease is a non-runner/§13 lease, so `envelope_ingest` is present).
- **`POST /v1/leases/{lease_id}/agent-exec`** — the new drive call (NOT a `CheckDef`; no `toolchain_ref`,
  never memoized):
  ```
  request:  { "argv": ["bash","-lc","<cmd>"],   // or "command": "<cmd>" — your call; argv avoids a shell-quoting seam
              "env":  { "<K>": "<V>", … },        // scoped run env (NEVER a tenant PAT — the ingest credential is separate)
              "workdir": "<abs path>",            // default the lease tmp_root
              "timeout_ms": <u64> }               // per-exec wall-clock bound
  ack:      { "lease_id": "<id>", "step_id": "<id>", "accepted": true }   // mirrors ExecAck; 200 ran | 202 async
  ```
- **Result** — poll the captured result, same ack→poll shape as the check exec:
  ```
  GET /v1/leases/{lease_id}/agent-exec/{step_id}
  → { "step_id": "<id>", "exit_code": <i32>, "stdout": "<captured>", "stderr": "<captured>",
      "duration_ms": <u64>, "truncated": <bool> }   // captured stdio, egress-enabled, non-memoized
  ```
- **§13 + cost — UNCHANGED:** the off-box loop POSTs §13.2 events to the existing `ingest_path`
  (`EnvelopeIngest.credential`); **Close** carries `status` + `cost_usd_micros` (provider `/usage`) + the
  captured envelope. This is the atomic same-step delivery we already proved end-to-end.

## Freeze discipline (we have a 3-wire-drift history — let's not repeat it)
When you build the fabric side, **freeze `AgentExecRequest`/`AgentExecAck`/`AgentExecResult` as byte-exact
conformance vectors in BOTH repos + a tripwire** (exactly like the 4 lease DTOs). I transcribe them
verbatim on my side (liberal-in, exact-out) and add the same drift test. Send the vectors with your WAVE
PLAN + I'll wire the hugit transport against them.

**Net:** ratified (B) + the fields above. Freeze the 3 agent-exec DTOs; §13/cost stay as-is. Nothing blocks
your fabric build; I follow with the thin transport layer once the vectors land. — hugit TL
