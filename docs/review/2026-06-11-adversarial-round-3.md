# Adversarial convergence — Round 3 verdict (fresh 7-agent fleet)

> 2026-06-11, after Waves E+F + the wedge wave. Fresh-context refutation fleet
> (1 fable + 2 opus + 4 sonnet), no memory of Rounds 1/2, hugit @ main 8b3c9c1.
> **Result: 7/7 DO-NOT-SHIP — but convergence is in sight.** Every adversary
> INDEPENDENTLY CONFIRMED the spine + all Wave E/F fixes HELD (hex leak, abandon
> deadlock, verify_chain, export unify, D14 ref-guard, ghost-record, hash chain,
> money, redaction engine, cross-repo). Zero new structural issue. The findings
> are TWO clusters: (A) the fast wedge wave needs the engine's hardening applied,
> (B) records lag the velocity. Wave G closes both → Round 4.

## Cluster A — wedge hardening (CODE)
- **WG-SCRUB [CRITICAL, 3 adversaries, reproduced live]**: the new write
  surfaces append user strings UNREDACTED to the forever-log — `ghp_`/JWT/
  conn-string in `check --def/--cmd/--pr/--principal`, `verdict --intent/--lens/
  --tree-hash`, `pr abandon --reason`, `pr open --campaign/--run-id/--principal`.
  Wave-E redaction was per-verb; the new verbs forgot. FIX = STRUCTURAL: scrub at
  the single porcelain append boundary so no verb can ever forget. (in flight)
- **WG-CACHE [CRITICAL, authz+fable]**: the `<log>.ac` file cache has no integrity
  check — editing it to flip exit→0 serves a FALSE GREEN laundered into the
  hash-chained log as authoritative. Plus the toolchain axis defaults to a
  CONSTANT (`local-toolchain`) → cross-toolchain false-hit. FIX: tamper-evident
  cache (self-hash verify on read → miss/re-execute), real toolchain digest, wire
  verify_hit. (checks/)
- **WG-CHECK-ROBUST [robustness+product]**: `--cmd 'sleep infinity'` hangs forever
  holding the log lock ≥120s (no timeout); `FileAc::lookup` has no lock (TOCTOU →
  double-exec + duplicate check.recorded); `hugit check` without `--store` breaks
  the log-not-found/exit-2 law (silent green on typo'd path); `--cmd` silently
  ignored for built-in defs; `check` vs `checks` one-letter footgun. (checks/)
- **WG-PR [robustness]**: two reachable `.expect()` panics in `pr open`/`land`
  (violate one-error-law → structured errors); a settled `pr.landed` PR still
  shows `queued` in queue show (projection doesn't leave the queue). (pr/)
- **WG-COHERENCE [product B2/B3/B4]**: `verdict --store` fabricates a verdict for
  a NONEXISTENT intent (no existence guard); `campaign show` shows `landed:1` but
  `proven:0` because `intent.landed` omits the `campaign` field so the ledger
  files it under "default"; `verdict.recorded` doesn't flow to `proven` or the
  queue batch verdict. FIX: existence guard, intent.landed carries campaign,
  verdict→proven + queue projection. (verdict/ + hugit-ledger + intent/)

## Cluster B — records lag (DOCS) → WG-DOCS
- PS-1 declared "closed" in CHANGELOG but the pending-seams register still says
  DEFERRED (protocol violation) — move PS-1 to the Closed table.
- CHANGELOG has NO Wave F entry (4 fix merges invisible in the human record).
- CLAUDE.md/README say "Wave F in progress" — Wave F + wedge are complete.
- interop.md says contract v1.1 — live is v1.2.0.
- "green by gate" / HEAD-CI / "~63% fail" wording stale (observed ~47%); the
  p2-ceiling doc cites "two rounds" while Round 3 is open.
- Test-quality: wcheck never tests a FAILING command (a memoized red→green mask
  would pass); saved_ms asserted non-null but =0 (use a measurable duration);
  add a tampered-log settle test.

## Confirmed HELD (the spine is sound — every adversary)
Hash chain + canonicalization · D14 guard on every mutation path · verify_chain
on every read · redaction ENGINE (gitleaks-class) · money integer micro-USD ·
WF-1..WF-AUTHZ + WF-CLI2 · package count 17 · PS-2/3/4/5 honest · cross-repo
(githugr-fixture builds clean, runner vectors byte-identical) · the wedge CORE
(cold→warm→bust, memo key 3-axis sound, "local TODAY / P2 fleet-shared" honest).

## Dispatch (partition)
WG-SCRUB (opus, structural redaction — all porcelain append sites) FIRST.
WG-DOCS (sonnet, records + test-quality) NOW, parallel (disjoint).
After WG-SCRUB merges → WG-CACHE (checks) ∥ WG-PR (pr) ∥ WG-COHERENCE
(verdict/ledger/intent) — disjoint by module. Then Round 4.
