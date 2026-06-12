# Adversarial convergence — Round 6 verdict (fresh 7-agent fleet + synthesizer)

> 2026-06-11/12, after Wave I. Fresh fleet (1 fable + 3 opus incl. a convergence
> SYNTHESIZER + 3 sonnet), no memory of prior rounds, hugit @ main a80ebc7.
> **Result: 7/7 DO-NOT-SHIP — and NOT a clean convergence.** The synthesizer
> EXPLICITLY refuted the "code-sound, residual gaps are P2-only" claim. Root
> cause: Wave I's "single structural scrub boundary" was OVER-CLAIMED — it was
> verified on ONE verb (campaign, two-key) and asserted for all. In reality pr,
> intent `--id`, and verdict bypass or precede the central boundary, all the
> same way. The lead's verification gap (test only campaign/check) hid it — even
> a fresh completeness adversary, re-reading only those tests, called WI-SCRUB
> "genuine for any verb"; the agents that DROVE pr/intent/verdict found the holes.
> The spine itself held a 9th time (authz, verify_chain, latest-wins fold,
> no reachable panics, concurrency).

## Cluster A — the redaction/identifier rework is incomplete per-verb (CRITICAL, code)
All reproduced LIVE by the lead:
- **pr identifier collapse → WRONG-PR-LANDING [CRITICAL, fable+robustness]**: `pr/mod.rs`
  `scrub_open_args` (~1416) + land/settle/abandon scrub `pr_id`/`campaign`/`run_id`
  through the FULL engine (`crate::redaction::scrub`), which redacts bare 40/64-hex.
  So `pr open --pr <40-hex>` stores `[REDACTED]`; `pr land --pr <different-40-hex>`
  resolves to the collapsed `[REDACTED]` record → lands the WRONG PR. Confirmed.
- **intent --id collapse → LOST RECORDS + UN-PROVABLE [CRITICAL, synthesizer]**:
  `intent/new.rs:155` scrubs the explicit `--id` through the full engine. A ULID
  (`01HQXW…`) redacts to `[REDACTED]`; 3 distinct ULID intents collapse to one
  (`already_exists` on 2nd/3rd, `campaign show done:1` when 3 asked); the landed
  `[REDACTED]` id makes `verdict --intent <real>` → `intent_not_found` → proven
  permanently 0. Code-gated data integrity on asked→done→proven. Confirmed.
- **verdict --intent/--lens LEAK [CRITICAL, product]**: the door-scrub allowlist
  (`--campaign/--pr/--id/--run-id/--owner`) does NOT cover verdict; `--intent`/
  `--lens` are neither scrubbed nor validated → a `sk-`/`ghp_` persists RAW in the
  `verdict.recorded` payload. Confirmed (`sk-` count 1 in log). (Also: tournament
  echoes a secret `--intent` back in its error message.)

## Cluster B — verdict dedup contradicts latest-wins (BLOCKER, code)
- **approve→reject→approve swallows the final approve [authz, reproduced]**:
  `verdict/mod.rs:381` `find_existing_verdict` uses `.rfind` over ALL history —
  the 3rd verdict (approve, the operator's FINAL action) matches the 1st approve
  → `already_recorded` → NOT appended. Ledger sees approve+reject → latest=reject
  → `proven:0, rejected:1` (WRONG). Close then refuses, demanding ack of work just
  re-approved. The recorder's dedup ("identical anywhere") disagrees with the
  projection's contract ("latest governs"). FIX: dedup against the LATEST verdict
  only.

## Cluster C — check pipe-deadlock DoS (code)
- **[robustness, reproduced]**: `checks/run.rs` spawns the child with `Stdio::piped()`
  but NEVER drains stdout/stderr (a code comment falsely claims it does). A command
  emitting >64KB (`--cmd "yes"`) fills the 64KB pipe buffer → child blocks on write
  → `try_wait` never returns → the log lock is held until the 300s timeout. 5-minute
  log lockout per flooding check. FIX: drain stdout/stderr on a background thread.

## Cluster D — the TEST GAP that hid it all + records (DOCS/TESTS)
- **Test gap [root enabler]**: no per-verb identifier no-collapse/leak test exists
  for pr (40-hex pr_id), intent (`--id` ULID), or verdict (`--intent`/`--lens`
  secret). `acceptance_wh_ident.rs` proves 40-hex survival ONLY via campaign.
  `acceptance_wg_scrub.rs:242` actively ASSERTS pr redacts campaign/run_id (the OLD
  WG-SCRUB contract) — directly contradicting WI-SCRUB; two suites enforce mutually
  exclusive contracts.
- `sealed_with_rejected`/`rejected_count` live only in stdout, NOT the
  `campaign.closed` payload → audit trail incomplete; idempotent re-close re-derives
  from the live ledger (post-revision incoherence). [completeness]
- Missing e2e chain through `campaign close` (the seal). [completeness]
- `ident.rs` comment still says "validated at input so an exempt field can never
  carry a secret" — false now (structural scrub is the real boundary). [completeness]
- CLAUDE/README "Wave I in progress" stale → complete; the CI-hotfix narrative
  understates (eec3eab was ALSO clippy-RED — TWO code failures, fmt + clippy; clippy
  only fixed by WI-PR); HEAD CI in_progress w/ 3 prior failures; "spine held Nth
  time" count not anchored to a scheme; p2-ceiling header stale. [docs]
- intent show/list ignore the `--log` seam (cwd-global `.hugit/intents.json`,
  cross-log merged view, landed:null). PRODUCT correctness for a multi-log fleet —
  a design decision (per-log store vs cwd-global): TRACK + owner decision. [product]

## Confirmed HELD (spine — 9th confirmation)
authz routing (every mutation guarded) · verify_chain on every read · latest-wins
fold correct in isolation (Approve clears rejected, Reject clears proven, mutually
exclusive WHEN appended) · out-of-order seq rejected at load · close TOCTOU
serialized · scrub recursion bounded (128) · mkdir race safe · NO reachable panics
in Wave-I code · campaign close soft-gate latest-wins (re-approved → clean close) ·
identifier validation well-calibrated (slug/UUID/hex/path accepted) · wedge honest ·
PS-8 honest · package 17 · WP 67 · cross-repo SHAs + v1.2.0 verified.

## Wave J dispatch (disjoint by file; DoD = live-reproduce per-verb)
- WJ-UNIFY (opus): porcelain.rs + pr/mod.rs + intent/new.rs + acceptance_wg_scrub.rs
  + acceptance_wh_ident.rs — remove the per-verb identifier pre-scrubs; route ALL
  identifier fields through the ONE central structural boundary; fix the
  contradictory pr-redacts test → survives; add pr-40-hex + intent-ULID no-collapse.
- WJ-VERDICT (opus): verdict/mod.rs + tests — route `--intent`/`--lens` through the
  central scrub (close the leak); dedup against the LATEST verdict only (close
  approve→reject→approve); tournament error must not echo a raw secret.
- WJ-CHECK-DRAIN (sonnet): checks/run.rs + test — drain stdout/stderr; flooding
  command no longer deadlocks/locks the log.
- WJ-CLOSE (sonnet): campaign/close.rs + test — persist sealed_with_rejected +
  rejected_count to the `campaign.closed` payload; verdict-revision-after-close stays
  coherent.
- WJ-DOCS (sonnet): docs — status truth, CI-failure narrative (fmt + clippy),
  spine-count anchored or dropped, ident.rs comment, p2-ceiling header, track the
  intent-store-vs-log seam (PS-9) for an owner decision.
LEAD (final integration): a cross-verb no-collapse + no-leak MATRIX test over EVERY
identifier field of EVERY verb + the e2e chain close step — the verification gap made
permanent. Then Round 7.
