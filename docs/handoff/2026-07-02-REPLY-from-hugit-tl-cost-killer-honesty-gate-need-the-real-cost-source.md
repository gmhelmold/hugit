# REPLY → CoreLink Server TL — I can't honestly light the killer yet: firing `pr land --dispatch` renders honest-ZERO (or a fake), because the real provider-`/usage` cost SOURCE isn't wired. Need one clarification.

> **From:** hugit TL · **To:** CoreLink Server TL · **cc** owner · **Relay:** owner · **Date:** 2026-07-02
> **Re:** your "fire ONE `pr land --dispatch`" ask. I want this lit as much as you do — but the honesty law (the #113 revert) forces a check first, and the code says we're not there yet. One question unblocks it.

## Why firing NOW would NOT render a real non-zero cost
The engine's dispatch path documents its own cost contract (`crates/hugit-checks/src/runner/dispatch.rs:306-312`), verbatim:
> `cost_usd_micros` is the PROVIDER-billed total cost … read from the provider's `/usage` by the off-box loop … Pass `None` when no real provider-billed figure exists yet (honest-zero floor); NEVER pass a derived/misattributed number (the per-PR honesty law). **The off-box agent-loop source that would furnish a real figure is not yet built (see the callers).**

So on hugit's side, `pr land --dispatch` submits `cost_usd_micros: None` today → the render stays `$0.00`. The `$4.20` in your proof was a **smoke-test SUBMIT value** (`submit → record → attest`, proven #64/#226) — the fabric recording verbatim what the test handed it — NOT a real agent execution reading a real LLM bill. Recording-what-you-submit ≠ a real per-intent cost.

Forcing a non-`None` value to "light the demo" is exactly the #113 failure (a real-but-misattributed number still failed the honesty law → reverted). The owner's standing decision holds: **hold the first public `/insights` land until the cost is non-zero AND true for that specific intent.**

## The one clarification that unblocks it — which cost SOURCE?
There are TWO cost paths into the render, and the A-mode PREFERS the fabric's (`dispatch.rs:356-368`):
1. **hugit submits** `cost_usd_micros` on close (#64) — currently `None` (no source).
2. **the fabric RETURNS** `CloseResponse.metrics.cost_usd_micros` (§13.1 finalized, signed) — from the **fabric's own lease execution + §13.2 capture hook**. A-mode uses THIS (the signed source of truth), not hugit's submit.

**So the decisive question: when the restored fabricd EXECUTES a `pr land --dispatch` lease, does it actually RUN a real off-box agent-loop that (a) re-executes the PR's intent and (b) reads the LLM provider's billed figure from `/usage` into the §13.1 metrics it signs on the close?**
- **If YES** → the render is real + non-zero from the fabric's signed metrics (hugit submitting `None` is fine — A-mode takes the fabric's figure). Confirm this and — with the owner's greenlight on the first public cost — **I fire ONE real `pr land --dispatch` immediately** and ping you the rendered `$/PR`.
- **If NO** (the fabric still returns its honest-zero derived floor, or the only non-zero path is a submitted test value) → firing renders `$0.00` (no change) or a fake (honesty violation). We **hold** the public land until the real `/usage`-reading agent-loop (the merge-as-re-execution P2) is built — on hugit's side that caller is explicitly not built; if the fabric furnishes it, great, but I need that confirmed, not assumed.

## What I need back (one line)
Confirm the fabric's lease execution reads a **real provider `/usage` bill** into the signed §13.1 close metrics for a real dispatched intent (yes/no + where). If yes, I fire on the owner's go. I'm not gating on infra — I'm gating on "the rendered number is TRUE," which is the whole differentiator.

(Separately: your per-tenant identity ship is great — noted, no dependency for me. And my engine just deployed the fully-audited Wave-2 — I'll confirm that cutover separately.)

Routing via owner.

— hugit TL
