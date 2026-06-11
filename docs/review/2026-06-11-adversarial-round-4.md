# Adversarial convergence — Round 4 verdict (fresh 7-agent fleet)

> 2026-06-11, after Wave G. Fresh-context refutation fleet (1 fable + 2 opus +
> 4 sonnet), no memory of Rounds 1–3, hugit @ main c13dabe. **Result: 7/7
> DO-NOT-SHIP — but the SPINE held a SEVENTH time** (auth routing, verify_chain
> on every read, hash framing, money, redaction ENGINE, tamper-detection +
> structural scrub confirmed GENUINE not theater, 4-altitude chain, cross-repo).
> Every finding is concentrated in ONE surface — the fast-built wedge EXECUTE
> path (`checks/run.rs`) + the scrub boundary's exemption + the proven
> projection — plus records lag. Several are CONSEQUENCES of Wave G's own fixes
> (the lesson repeats: a fix can open a hole). No new structural layer.

## Cluster A — the scrub boundary's exemption is exploitable (CODE)
- **WH-SCRUB [CRITICAL, fable+sonnet2, reproduced live]**: `porcelain::is_digest_key`
  exempts `*_digest`/`tree_hash`/`hash`/`commit`/`memo_key` from scrub BY KEY
  NAME. But `hugit check --toolchain <secret>` and `hugit verdict --tree-hash
  <secret>` route RAW user flags into those exempt fields → `ghp_`/JWT persists
  VERBATIM in the forever-log + the `.ac` cache. Same class as the Round-2 hex
  leak: an exemption is a hole. FIX (structural): exempt a digest field ONLY IF
  its VALUE is actually digest-shaped (64/40-hex or `sha256:`/`cas:` prefix);
  otherwise scrub it. Value-gated, not key-gated.
- **WH-IDENT [HIGH, fable, reproduced]**: identifier over-redaction — a 40-hex /
  high-entropy CAMPAIGN KEY is scrubbed to `[REDACTED]` → two distinct campaigns
  COLLAPSE to one key (silent data loss); and the scrub is ASYMMETRIC (open
  scrubs the key, abandon/close look up the raw key → a secret/entropy-shaped
  campaign is permanently unaddressable). FIX: identifier fields {campaign,
  intent_id, pr_id, run_id} are ADDRESSES not free text — exempt them from
  free-text scrub (WH-SCRUB) AND validate them at INPUT (reject empty +
  reject known-secret-prefix shapes with a structured error). Contract-coupled
  with WH-SCRUB: exempt-but-validated.

## Cluster B — the wedge EXECUTE path (checks/run.rs) → WH-CHECK
- **Lock-poison hang [SHIP-BLOCKER, opus2+fable]**: WG-CACHE's lock-before-decision
  holds the `.ac` lock ACROSS the (unbounded) execute. A hung/slow/orphaning
  command holds the lock and POISONS the log (every concurrent check → `ac_busy`,
  not reclaimed across 60s). Plus `child.kill()` kills only the direct child, not
  the process group → orphan grandchildren survive, wall-time bypasses the
  `--timeout-secs` ceiling. FIX: lock only the cache file ops (lookup/store),
  NOT the execute; kill the process GROUP (setsid/killpg); bound wall-time.
- **check non-idempotent / KPI fabrication [HIGH, opus2]**: `check --store` has no
  dedup — N identical runs append N `check.recorded` → inflates count + fabricates
  hit-rate (the one headline number). FIX: dedup on memo_key already-recorded →
  `already_recorded:true`, no double append.
- **symlink-cycle stack overflow [SHIP-BLOCKER, sonnet1]**: `collect_files` follows
  directory symlinks with no visited-set/depth guard → SIGSEGV on a workspace with
  a symlink loop (common in monorepos). FIX: visited-set or depth cap → structured
  error, never crash.
- **ad-hoc `--cmd` never memoizes [MED, opus2]**: a 2nd identical `--cmd` re-executes
  (cache_hit:false). FIX: ad-hoc checks memoize like built-ins.
- advisory: `as_object_mut().unwrap()` on json! literals (checks/mod.rs:307,
  run.rs:614) — unreachable but latent; convert to safe.

## Cluster C — projection truth → WH-PROVEN
- **REJECT verdict marks PROVEN [BLOCKING, opus1, reproduced]**: `ledger/mod.rs`
  sets `proven=true` for ANY `verdict.recorded`, never inspecting approve/reject.
  A REJECTED intent shows `proven:1` and `campaign close` SEALS over rejected work
  — the forever-log asserts rejected work is validated. Consequence of WG-COHERENCE.
  FIX: gate `proven` on `aggregate==approve`; surface the verdict outcome in
  `campaign show`.

## Cluster D — records lag + tracking (DOCS) → WH-DOCS
- CHANGELOG overclaims "queue show's verdict real" — it's null-disclosed; the
  verdict flows to `proven`, not the queue batch verdict. Correct the entry.
- Untracked deferral: the queue batch-verdict→queue-show projection (the half
  WG-COHERENCE flagged cross-module) has NO register entry → add **PS-6**.
- `toolchain-unprobed` fallback is an untracked cross-env false-hit vector → note it.
- CLAUDE.md/README "Wave G in progress" stale (complete); CI rate ~42%→~33% actual;
  p2-ceiling doc "two rounds" (§1) vs "three" (§0) contradiction + "adversarially
  hardened" unqualified while Round 4 ran; CHANGELOG names `scrub_payload` (callers
  use `scrub_to_canonical`).
- Missing e2e wedge-chain test (check→checks show hit-rate→verdict→proven as ONE
  binary-driven chain) → add it.

## Confirmed HELD (the spine — 7th independent confirmation)
Auth routing (every mutation through D14) · verify_chain on every read · hash
framing length-prefixed/collision-safe · money integer micro-USD checked_add ·
redaction ENGINE · tamper-detection GENUINE (re-executes, not a flag) · structural
scrub GENUINE (5 call sites drive the real binary) · PS-1 honest (local closed,
P2 live-AC tracked) · PS-2..5 real · 4-altitude chain test exists · cross-repo
contract v1.2.0 matches · cache self-hash unkeyed but HONESTLY disclosed (P2 HMAC).

## Wave H dispatch (disjoint by file; WH-SCRUB↔WH-IDENT contract-coupled)
- WH-SCRUB (opus): porcelain.rs — value-gate digest exemption + exempt identifier
  fields from free-text scrub. [CRITICAL #1, enables #2]
- WH-IDENT (sonnet): campaign/open.rs + intent/new.rs + pr input — validate
  identifiers at input (reject empty + secret-shaped). [#2, #7]
- WH-CHECK (opus): checks/** + hugit-checks — lock-only-cache + process-group kill,
  idempotency dedup, symlink-cycle guard, --cmd memoize, as_object_mut. [#3-6]
- WH-PROVEN (sonnet): ledger/mod.rs + campaign/show.rs + close.rs — gate proven on
  approve + surface outcome. [#8]
- WH-DOCS (sonnet): docs + PS-6 + e2e wedge-chain test. [#9 cluster]
Then Round 5: fresh fleet.
