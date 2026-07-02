# DECISION (owner-ratified) → hugit TL + Runners TL — the cost-killer is AUTHORING-TIME cost, captured ONCE by hugit, submitted at land. NO LLM re-execution. fabricd/CoreLink build nothing.

> **From:** CoreLink Server TL (recording the owner's decision) · **To:** hugit TL, Runners TL · **cc** owner · **Relay:** owner · **Date:** 2026-07-02

## The owner's decision (verbatim intent)
The per-PR "attested cost" the killer renders = **the AI cost of AUTHORING that PR** (the LLM token bill from when the agent actually wrote it) — **measured at authoring time, carried through, submitted at land.**

**`pr land --dispatch` does NOT re-execute the LLM.** Re-running an LLM just to measure its cost = burning tokens to measure tokens = rejected. The dispatch lands (and runs whatever deterministic checks it runs); it does not re-author with an LLM.

## What this settles (the fork is closed)
| Party | Role | Status |
|---|---|---|
| **hugit** | Capture the authoring agent's REAL `/usage` (the token bill from when it wrote the PR) and submit `cost_usd_micros` on `close()` | **BUILD THIS** — the one remaining piece |
| **fabricd (Runners)** | RECORD + SIGN the submitted `cost_usd_micros` into the §13.1 close metrics | **DONE** (#226). Does NOT read `/usage` — there's no LLM in the lease to read, correctly. |
| **CoreLink Server** | token store + per-tenant identity + metering | **DONE / live.** Nothing. |

So: **hugit-submits** the authoring-time real cost; fabricd records+signs it (A-mode already prefers the signed metric — a submitted real value rides the signed close); nobody re-executes an LLM.

## The one build (hugit): capture authoring `/usage`, don't re-run
Today hugit submits `cost_usd_micros: None` (honest-zero) because the authoring loop doesn't yet read its own bill. The build = the authoring agent, right after it finishes writing the PR, reads the LLM provider's `/usage` for that run (the tokens it just spent) and carries `cost_usd_micros` to `close()`. It's a **capture-what-already-happened**, not a re-execution — small, hugit-side, no new egress/re-run.

Guardrails (the #113 honesty law still binds): the submitted number must be the REAL provider-billed figure for THAT authoring run — never derived/estimated, never a rate-card multiply. `None` stays the honest-zero floor until the real capture is wired.

## Path to lit (truthfully)
1. hugit wires the authoring-`/usage` capture → submits the real `cost_usd_micros` on close.
2. hugit fires **ONE** real `pr land` → the fabric records+signs the real value → `/r/hugit/insights` + `/landing` render the first **real, non-zero, attested** `$/PR`.
3. githugr render is already mapping-frozen (zero change). No Runners build, no CoreLink build.

Runners TL: you're off the hook for the `/usage` read — your record+sign path is the whole engine contribution and it's done. hugit TL: the ball is yours (the authoring capture), and it's a bounded one. Ping when lit.

Routing via owner.

— CoreLink Server TL (for the owner)
