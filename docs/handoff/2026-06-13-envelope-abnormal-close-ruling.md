# → corelink-runners: §13 RULING — abnormal-close envelope = Option B (best-effort partial flush)

**From:** hugit techlead (via owner relay) · **Date:** 2026-06-13 ·
**Owner ratification:** ✅ Gustavo, 2026-06-13 ·
**Re:** your question `corelink-runners/docs/handoff/2026-06-13-hugit-envelope-flush-on-abnormal-close.md` ·
**Governs:** the frozen integration contract `docs/spec/hugit-integration-contract.md` §13 (envelope emission) — this adds **§13.5 (abnormal-close emission)** as a hugit-side amendment; §0–§12 and the §13.4 `IntentMetrics` vector are **untouched**.

---

## 1. The ruling: **Option B — best-effort partial flush.** Do NOT drop.

On **Expired** (deadline reaper) and **Crashed** (crash sweep) termination, the fabric
MUST flush whatever the `CaptureHook` accumulated, as a partial envelope, **explicitly
marked incomplete**. Dropping (Option A) is rejected.

**Why (hugit's model, so you don't have to guess):**
- **No billing risk.** hugit prices **flat, never by usage** — partial `IntentMetrics`
  are NEVER billed. The "incomplete metrics might be billed" worry does not exist on our
  side, so the main argument for dropping evaporates.
- **Forensic provenance is the point.** A crashed long-running agent job accumulated
  REAL signal (tokens, tool calls, a partial trajectory: "the agent did X, Y, Z and died
  at tool-call 40"). For an LLM-native forge that is exactly the fleet-debugging gold we
  want; silently dropping it is lost provenance.
- **Honesty is preserved by marking, not by hiding.** hugit's honest-null law forbids
  presenting incomplete data AS IF complete — it does NOT forbid emitting clearly-labeled
  partial data. The explicit markers below make a partial envelope unmistakable for a
  clean close, so no consumer can over-trust it.

## 2. The wire shape (what an abnormal envelope carries)

Reuse the EXISTING normal-close envelope payload — **no new envelope type, no change to
the frozen §13.4 `IntentMetrics` vector** (it is `deny_unknown_fields`; the close metadata
is WRAPPER-level, never inside `IntentMetrics`). The abnormal envelope is the normal one
plus two close-metadata fields:

| field | normal | abnormal |
|---|---|---|
| `IntentMetrics` (the frozen §13.4 vector) | full | **partial** — whatever the hook captured at termination (byte-shape identical; values just reflect partial progress) |
| trajectory (if captured) | full | partial — as captured |
| `close_reason` | `"normal"` | **`"expired"`** or **`"crashed"`** |
| `capture_incomplete` | `false` | **`true`** — yes, the existing flag fires on the reaped/crashed paths (today it never produces a close response at all; make it fire) |

`close_reason` is a small closed enum (`normal | expired | crashed`) at the
close/envelope-metadata level — extend your existing `JobClose`/close-status type, don't
touch `IntentMetrics`. If you already have a close-status enum, add the two variants there.

## 3. Delivery & ack semantics (the abnormal contract)

- **Fire-and-forget, NO exactly-once ack.** There is no live client/lease to run the
  `JobClose` ack handshake against (the lease is already torn down). The fabric attempts
  delivery **once** to the envelope endpoint after teardown→transition→`record_slot`; on
  failure it is **dropped + logged** (the envelope is forensic, not authoritative — a lost
  partial is acceptable, a blocked teardown is not). Teardown MUST NOT wait on delivery.
- **Idempotency / dedup by `lease_id`.** A lease emits at most one terminal envelope.
  The ledger transition is atomic and mutually exclusive (`Held→Closed` vs
  `Held→Expired|Crashed`), so a normal close and an abnormal flush cannot both fire for the
  same lease; the `lease_id` dedup at the endpoint is belt-and-suspenders, and a `normal`
  close always supersedes a partial if they ever race.
- **Redaction is identical — no exemption.** The partial envelope goes through the SAME
  write-path redaction as a normal one (the trajectory may carry secrets). An abnormal
  path is NOT a redaction exemption ("an exemption is a hole").

## 4. Scope / who does what

- **corelink-runners (now):** add the `close_abnormal` flush call on the Expired + Crashed
  paths in `reaper.rs` (your `TODO(envelope)`), with the markers + fire-and-forget delivery
  above. Bounded WP, as you noted.
- **hugit (at P2):** hugit consumes the abnormal envelope when the **live envelope
  transport** is wired (the P2 fabric-API seam, `interop.md` §2). **No hugit code change is
  required now** — the consumer-side handling of `close_reason`/`capture_incomplete` rides
  the P2 envelope-transport WP, and `capture_incomplete:true` envelopes are stored as
  forensic provenance, never as billable/authoritative.

## 5. What we need back

Nothing blocking. Implement §13.5 per the above; ping hugit (via owner) only if the
fire-and-forget/no-ack contract or the `close_reason` enum placement reads differently than
your `reaper.rs`/`JobClose` shape assumes. We'll mirror §13.5 into our contract copy.

— routed via owner; no `path`/`git` dependency between repos.
