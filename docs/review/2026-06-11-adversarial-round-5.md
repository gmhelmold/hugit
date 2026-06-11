# Adversarial convergence — Round 5 verdict (fresh 7-agent fleet + synthesizer)

> 2026-06-11, after Wave H. Fresh fleet (1 fable + 3 opus incl. a CONVERGENCE
> SYNTHESIZER + 3 sonnet), no memory of Rounds 1–4, hugit @ main 443ff1b.
> **Result: 7/7 DO-NOT-SHIP.** The spine held an 8th time (append/D14,
> verify_chain, hash framing, money, concurrency double-exec benign). Findings
> cluster in: the identifier-redaction coupling (CRITICAL, the exemption-is-a-hole
> pattern AGAIN), forge state-machine coherence under realistic multi-step agent
> workflows (a class the single-step rounds missed), one DEEP honesty insight
> from the synthesizer, and records/test-hygiene + a real fmt-red HEAD (hotfixed).

## Cluster A — identifier-redaction coupling is broken (CRITICAL, code) → WI-SCRUB
The Wave-H "identifiers exempt-from-scrub + validated-at-input" contract fails two ways:
- **Validator WEAKER than the engine [fable]**: `ident.rs`'s reject-prefix list omits
  Slack (`xoxb-`), CoreLink (`clp_`), `Bearer`; uses `starts_with` not substring;
  no whitespace trim. A Slack token in `campaign open --campaign` → VERBATIM in the
  forever-log (reproduced).
- **Validator NOT CALLED on every write path [sonnet1]**: `check --pr <secret>` →
  `pr_id` is identifier-exempt from scrub but the check verb never validates it →
  the credential persists verbatim (run.rs:808 comment claims it can't — it does).
- **Open-ended prefix list [sonnet2-#12]**: any unknown secret format passes.
FIX (structural, single-source-of-truth): identifier fields get the engine's
STRUCTURAL secret detectors (prefix/connection-string/JWT/PEM) at the scrub
boundary, exempt ONLY bare-hex/entropy (addresses survive). No verb can leak a
secret-in-identifier; the input validator becomes UX, not the security boundary.

## Cluster B — forge state-machine coherence (code) → WI-PR + WI-PROVEN2
- **proven-revision [opus1-authz]**: `ledger` folds verdicts as a monotonic OR —
  approve-then-REJECT leaves `proven:1, rejected:1` (latest is reject). No
  resolution. FIX: reject-vetoes-proven (or latest-wins); proven/rejected mutually
  exclusive. → WI-PROVEN2 (ledger).
- **campaign close doesn't gate [opus2 + sonnet2-#1]**: `close` blocks only on
  in-flight PRs; seals `closed:true` with `proven:0, rejected:1`. FIX: close
  surfaces/guards rejected work, never silently seals it. → WI-PROVEN2 (close).
- **pr land non-idempotent on terminal [opus2]**: re-running `pr land` on a settled
  (landed) PR re-appends `pr.queued` after `pr.landed` (post-terminal corruption,
  `already_queued:false` lies). FIX: terminal landed → idempotent `already_landed`.
  → WI-PR.
- **pr open doesn't validate intent existence [opus2]**: a phantom `--intent` is
  accepted (verdict rejects it later — un-provable PR). FIX: validate like verdict.
  → WI-PR.
- **intent new bootstrap crash [opus2]**: `intent new` on default `.hugit/` store
  crashes `store_error` when `.hugit/` absent, despite help claiming bootstrap.
  FIX: create the dir. → WI-PR.

## Cluster C — the log is tamper-EVIDENT, not tamper-PROOF (HONESTY) → WI-HONESTY
**[opus3 SYNTHESIZER — the single strongest blocker]**: the event-log integrity is
an UNKEYED SHA-256 chain (public, frozen byte-exact). `verify_chain` only re-derives
the same public function. A COMPETENT rewriter (recomputes the chain forward — the
exact in-scope actor: a fleet agent with write access to the shared `--log`) forges
contents that pass verify_chain: `reject→approve` (rejected shown as proven),
failing check→green. A naive byte-flip IS caught (the prior waves' win); a competent
rewrite is not. This is PHYSICS for a local file (no local crypto stops the local
writer) — the real authentication is SERVER-SIDE (P2: CoreLink append-only event-log
+ transparency log). The DEFECT is an HONESTY gap: the Action Cache layer HONESTLY
discloses this exact limitation (run.rs:540, "P2 HMAC seam"); the EVENT LOG asserts
"tamper-evident … the log holds" (p2-ceiling:24,56) WITHOUT the caveat and with no
tracked seam. FIX (honesty, not local-crypto-theater): track PS-8 (event-log
cryptographic authentication = P2 server-side seam, peer of the AC HMAC + the
transparency log); scope every "tamper-evident/holds" claim to "detects partial
corruption; authentication against a competent rewriter is P2 server-side"; align
CLAUDE/README/ceiling to the AC's honest tense.

## Cluster D — records + test-hygiene (DOCS/TESTS) → WI-HONESTY + WI-TESTS
- **fmt-red HEAD [sonnet3, CRITICAL] — HOTFIXED** (eec3eab): acceptance_wh_ident.rs:58
  was not rustfmt-canonical; remote CI failed on HEAD; a prior local check misread
  tail's exit through a pipe and discarded the fix. Now `cargo fmt --all --check`=0.
- CI rate ~35% not ~26%; "(not code)" misleading (HEAD failure WAS fmt-code) [sonnet3].
- CLAUDE/README "Wave H in progress" stale → complete; p2-ceiling "Wave H being
  hardened" stale; Wave G CHANGELOG "no double-exec" contradicted by Wave H [sonnet2,3].
- Track accepted risks in the register: double-exec window, kill(1)-portability,
  no log rate-limit/quota, orphan-grandchild accumulation [sonnet2].
- TEST THEATER to kill [sonnet2]: the concurrent no-poison test passes with OR
  without the fix (never proves both threads executed); the symlink test doesn't
  assert the cycle-guard fired; the chain test skips hit_rate>0 in the dedup branch.
  → WI-TESTS.

## Confirmed HELD (the spine — 8th confirmation)
append/D14 routing · verify_chain on every read (partial-tamper) · hash framing
collision-safe · money integer micro-USD · concurrency double-exec benign for log
integrity (12-way proven) · check idempotency dedup · AC verify_hit + honest
self-hash disclosure · identifier validation NOT too strict (slug/UUID/hex/path
accepted) · package count 17 · 67 WP · conformance vectors byte-identical · no
cross-repo Cargo dep.

## Wave I dispatch (disjoint by file)
- WI-SCRUB (opus): porcelain.rs — structural-secret scrub for identifier fields
  (engine detectors, exempt bare-hex). [Cluster A CRITICAL]
- WI-PR (opus): pr/** + intent/new.rs — land idempotent-on-terminal, pr-open intent
  existence, intent-new bootstrap. [Cluster B pr]
- WI-PROVEN2 (sonnet): ledger/mod.rs + campaign/close.rs — verdict-revision
  resolution + close gating/surfacing. [Cluster B proven]
- WI-HONESTY (sonnet): docs + pending-seams — PS-8 log-auth seam, scope the
  tamper claims, status truth, accepted-risk tracking, ceiling alignment. [C + D]
- WI-TESTS (sonnet): acceptance_wcheck.rs — kill the 3 theater tests. [D]
Then Round 6: the convergence candidate. The synthesizer drew the line at "the
spine holds against everything except the unauthenticated-log rewrite (a P2
seam)" — once that is honestly tracked, the convergence condition (code-sound +
P2 honestly scoped) is met.
