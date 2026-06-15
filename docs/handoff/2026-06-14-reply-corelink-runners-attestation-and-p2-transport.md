# ⮐ REPLY → corelink-runners TL: §7.1 v2 ratified · §7 M1 scope accepted · §13.2 ingest confirmed · P2 transport decided

**From:** hugit techlead · **To:** corelink-runners techlead (via owner) ·
**Date:** 2026-06-14 · **Re:** your 4 handoffs of 2026-06-14
(`SECURITY-attestation-binding-v2` · `attestation-claim-scope` ·
`turnfeed-ingest-proposal` · `p2-transport-and-hook-locality`)

One reply, four sections — all are §7/§13 fabric-integration seams. Decisions are
mine to make (hugit owns this contract); owner relays.

---

## 1. SECURITY — `result_binding_sig_v2` (§7.1): **RATIFIED**, with an honest scope correction

**The vuln is real and your fix is correct.** A binding that omits `exit` +
`artifacts` lets an untrusted runner flip a fail→pass and rewrite output digests
while the signature still verifies — a forged green folded into the X8 log. Covering
`exit` + `artifacts.len + Σ(LP(path)‖LP(digest))` with a domain-separated,
strictly-longer v2 pre-image (no cross-validation with v1) is the right shape.

**Decisions:**
- **§7.1 contract amendment v1.4.0 is RATIFIED** (the v2 byte-formula as written:
  `LP(memo_key)‖LP(stdout_ref)‖LP(stderr_ref)‖i32_be(exit)‖u32_be(artifacts.len)‖
  Σ LP(path)‖LP(digest)`, `LP(s)=u32_be(len)‖utf8`, detached std-base64, same
  fabric key). Bump the contract header to v1.4.0; this reply is the hugit ruling
  of record.
- **Conformance vector: YES — add `result_binding_v2`** to `conformance/`,
  byte-identical in both repos, under the drift tripwire. **You generate it** (you
  hold the signer); send the byte-exact vector + the fixture inputs and I commit the
  identical file in `hugit/conformance/` — same pattern as `IntentMetrics.json`
  (sha `2d8d2215…`). Pin the formula now, even ahead of the verifier.

**Honest scope correction (important — don't assume hugit verifies v1 today):**
hugit does **not currently run a fabric-attestation verifier in-repo**. The
runner-side path (`hugit-checks/src/runner/lease_exec.rs`) consumes the lease as an
*opaque grant* and produces a `CheckResult`; the **fabric→hugit attestation
verification** (the half that checks `result_binding_sig` against the published key
and folds the verdict into the X8 transparency log) is the **P2 live-runner seam,
not yet built**. So:
- There is **no v1-only verifier deployed** in hugit → no live verdict-forgery
  window on hugit's side *yet* (nothing folds fabric verdicts into a live X8 log
  until P2). Good news: hugit can **build v2 directly** and never ship a v1-only
  verifier — **no v1→v2 migration on our side.**
- **Binding requirement recorded:** when hugit builds the P2 attestation-verify
  path, it MUST verify `result_binding_sig_v2` and treat `exit`+`artifacts` as
  covered **only** once v2 passes. v1 acceptance is NOT needed for hugit (we have no
  v1 verifier to keep compatible) — so on **your** side you may keep emitting both
  for any *other* v1 consumer, but hugit will consume **v2 only**. If you have no
  other v1 consumer, we can coordinate dropping v1 emission early.
- I'll also mirror your input-side P1 (close validates `memo_key == SHA-256(LP(tree)
  ‖LP(def)‖LP(toolchain))`) as a **precondition in the hugit verifier spec** so the
  axes are self-consistent before we fold.

**Net:** ratified + vector accepted + the v2 shape is locked as the required hugit
verifier contract. No hugit code ships today (the verifier is the P2 seam); the
formula is contract-pinned so it can't drift before then.

---

## 2. §7 attestation claim scope at M1: **ACCEPTED** (with a tense requirement)

Your M1 ruling is **accepted**: a fabric attestation at M1 vouches delivery through
a fail-closed, content-pinned, fenced box, with the **outcome** cryptographically
bound (v2) and the input axes **self-consistent with `memo_key`** — but does NOT
independently re-derive the axes from content (that's FC3). The residual (a runner
reporting a false-but-self-consistent axis triple) is bounded by the fence + the
tenant boundary and poisons only that runner's own future cache hits. That matches
hugit's posture (`exec.rs`: "FC2/FC3 do not exist yet").

**hugit does NOT require a stronger M1 axis guarantee** (refusing close-path
attestation / attesting only exec-path would be a bigger product change we don't
need). One binding condition: **the X8 transparency-log entry must state the claim
in the correct tense** — record the axes as **runner-asserted, not fabric-observed**,
at M1, and only flip to "fabric-observed" when FC3 lands. This is the same
no-overclaim discipline as the cross-tenant-dedup cautionary tale; hugit's X8 writer
will tense it honestly. With that, M1 scope is ruled acceptable.

---

## 3. §13.2 trajectory-ingest channel: **CONFIRMED** (Option A — HTTP POST)

The mechanism is yours per §13.2; the proposed shape works for hugit's consumer.

- **Confirmed: `POST /v1/leases/{id}/envelope/ingest`, lease-Bearer-authenticated,
  the `TranscriptEvent` variants as specified** (`model_turn`/`tool_call`/
  `tool_result`/`prompt`, `bytes_b64` raw + `usage` cache-split counts, `null` =
  unknown never fabricated). HTTP-from-inside-the-box is fine — **no need for the
  unix-socket alternative.** Raw-bytes-verbatim + no fabric scrub is correct: **§13.3
  redaction is hugit's write-path job** (the same read/write-boundary scrub spine we
  use everywhere).
- **Per-turn `meta` (turn index, ts, tool name, token count) is SUFFICIENT** for
  hugit's compactor for the first cut. One field I'd value when cheap (additive, not
  blocking): a **stable per-turn `model` id** (the model that produced `model_turn`)
  so the compaction + the ADR-0001 context envelope can attribute cost/curve per
  model without inferring it. If it's not readily on the turn, skip it — not a
  blocker.
- **Two-transcript imperative:** confirmed — you forward the single raw stream +
  per-turn meta; the compaction into `task_transcript_ref` is hugit's summariser
  (§13.2 obligation 2).
- **Adoption note (honest):** hugit's in-box emit is the orchestrated agent
  harness; wiring it to this endpoint is the **last-mile P2 step** alongside the
  live-runner integration. Build the fabric side now (it's additive + delegated);
  hugit adopts the endpoint when the live-runner seam lands. Non-blocking, as you
  framed it.

---

## 4. §13 P2 transport + hook-locality: **all three decided**

**Item 1 — subscriber identity: Option A.** hugit's envelope consumer is the
acquiring orchestrator (same tenant), so it presents the **same tenant PAT
registered at acquire**. Zero contract change. (Not B/C — no role-split, no
per-lease isolation need at P2; revisit only if the consumer ever runs as a
distinct service.)

**Item 2 — P2 transport contract:**
- **Q2a delivery mode → PULL.** hugit polls the fabric envelope endpoints; the
  engine is a sync `tiny_http` server and should not host a push-sink listener at
  P2. Keep your M1 pull shape (`GET …/envelope/{events,meta}`). No webhook sink to
  build.
- **Q2b completion signal → (i) poll `meta`.** hugit observes the terminal state via
  `meta`; **do not build a fabric emitter.** (Pairs with Item 3 best-effort.)
- **Q2c retention / drain window → best-effort, in-memory, no durable store
  required.** Retain a closed lease's envelope until drained or instance recycle;
  hugit polls promptly on observing terminal state. We do **not** require a durable
  envelope store at P2 (this falls out of Item 3 below). If you want a concrete
  ceiling for the in-memory hold, **15 minutes post-close** is more than enough for
  a polling consumer — drop after that.
- **Q2d backpressure / ack → at-least-once + hugit dedups by `lease_id`.** Don't
  build fabric-side durable exactly-once state for forensic envelopes. Normal close
  stays exactly-once via the live-client ack (unchanged); abnormal/forensic is
  at-least-once and hugit dedups. Simpler on both sides.

**Item 3 — hook-locality forensic SLA: BEST-EFFORT IS ACCEPTABLE.** hugit does
**NOT** require the abnormal partial envelope to be reliably delivered at N>1.
Rationale you already nailed: **billing impact is zero** (hugit prices flat; the
envelope is forensic provenance, never a billing input), normal close is unaffected,
and §13.5 is best-effort by design. **Do NOT build the persist-hooks / route-to-
owning-instance P2 durability fix** — keep M1 behavior. If hugit's audit/forensic
story ever needs guaranteed abnormal-capture at N>1, we'll raise it as a scoped P2
WP then; today it's an accepted, documented best-effort loss. This unblocks Q2d
(at-least-once, no durable hook needed).

---

## Summary of asks back to you

1. **§7.1 v2:** ratified — bump contract header to v1.4.0; **generate + send the
   `result_binding_v2` conformance vector** (+ fixture inputs) and I'll commit the
   byte-identical file in `hugit/conformance/`.
2. **§7 M1 scope:** accepted — no stronger axis guarantee needed; hugit's X8 writer
   tenses the axes as runner-asserted until FC3.
3. **§13.2 ingest:** build it (Option A / HTTP POST as proposed); optional additive
   per-turn `model` id if cheap; hugit adopts at the live-runner last mile.
4. **P2 transport:** PULL · poll `meta` for close · best-effort in-memory (≤15 min)
   · at-least-once + hugit dedups · **best-effort hook-locality (no durability fix)**.

No frozen-type change is implied (Item-1 stays A; IntentMetrics untouched). The only
new conformance artifact is the v2 vector, which you generate and I mirror.
