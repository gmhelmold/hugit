# Adversarial convergence — Round 1 verdict (7-agent fleet)

> 2026-06-11. Owner method ([[adversarial-convergence-loop]]): prove ship-
> readiness by refutation, not assertion. Fleet of 7 (1 fable + 2 opus + 4
> sonnet), inverted mandate (refute "ship it / SOTA"; uncertain = DO-NOT-SHIP),
> fresh context, read-only, hugit @ main `0b94bcb`. **Result: 7/7 DO-NOT-SHIP.**
> The lead's "prato zerado" was premature. This is the fix list for Wave E;
> Round 2 re-audits with a fresh fleet.

## Cleared (verified real, NOT pendencies)
- **No hard cross-repo break**: `githugr-fixture` (the real path-dep consumer)
  compiles all targets + 28/28 parity green against current hugit. The 1.2.0
  money change did NOT break the downstream build. (fable)
- **Money 1.2.0 clean in code**: integer micro-USD end-to-end, zero off-by-10^6,
  zero stale `cost_usd`. (fable + regression)
- **WA1/WA2/WA3/WA4 are real, not theater** — each verified through the live
  binary (envelope scrub, subagent-refusal, erase-not-absent, hand-pin). (fable)
- **D14 porcelain wiring + lock seam + full cycle**: clean. (regression)
- HEAD (`0b94bcb`) passed remote CI green (run 27329035014) — CI not doomed. (CI)

## Confirmed pendencies → Wave E

### TIER 1 — security/integrity (blocking)
- **P-REDACT-SURFACE [HIGH, 2 agents converged]**: redaction is envelope-only.
  The porcelain RE-CREATES S1/S2: `intent new` persists a real `ghp_` PAT
  verbatim into `.hugit/intents.json` AND the hash-chained event-log payload
  (forever, unredactable post-hash); `intent show/list`, `campaign list`,
  `campaign open` echo emit raw secrets while `why`/ledger redact. One
  redaction law across ALL read + write surfaces. (fable F1/F2, sec H1-H3)
- **P-GUARD-PATHS [HIGH, 2 converged]**: the D14 guard is wired only on CLI
  porcelain. Two production raw-`append` bypasses of guarded verbs survive:
  `hugit-mirror sync/engine.rs:423` (`intent.landed`/Land) and
  `hugit-refstore undo/mod.rs:171` (`undo`/Human-only) — both named by
  `append`'s own doc as MUST-route-through-`append_authorized`. (sec H5, fable F3)
- **P-REDACT-ENGINE [HIGH]**: detector misses connection strings
  (`postgres://u:p@h` shredded under the 20-char floor), low-entropy long
  secrets, `files_read[].hash` (verbatim); `sk-` over-redacts; the write-path
  and export-path redactors diverged. SOTA = gitleaks-class corpus. (sec H1-H4)

### TIER 2 — my own waves' inconsistency (blocking, gate-missed)
- **P-PR-LAW [HIGH, 2 converged + lead-confirmed]**: `pr show/list/land/abandon`
  emit plaintext/exit-1 on bad `--log` (WB0 law says JSON/exit-2) and skip
  `verify_chain` (siblings call it). (regression R1, wedge F4)
- **P-CAMPAIGN-EMPTY [HIGH, 2 converged]**: `campaign show/list` read a missing
  `--log` as silent empty-world/exit-0 — the exact P5/B4 finding the plan
  claimed closed. `checks`/`queue` do it right. (regression R2, wedge F3)

### TIER 3 — wedge honesty
- **P-WEDGE-HOLLOW [HIGH, 2 converged]**: `checks show` returns all-null on
  every real agent log — no verb appends `check.recorded` (`hugit check` is
  reserved/undispatched); same for `queue` verdict (`hugit verdict` reserved)
  and `pr.landed` (no producer). The CHANGELOG markets "real hit-rate
  aggregation" = overclaim. Honest fix: downgrade the claim + track the
  recorder-verb work (genuine new surface, not a patch). (wedge F1/F5, complete F2)

### TIER 4 — records/process (cheap, but rigor-compact debt)
- **P-CONTRACT-PROP [MED]**: 1.2.0 break not propagated — corelink-runners
  integration spec + hugit R6 draft still pin `cost_usd|f64` / "1.1.0 next
  additive"; githugr delivery manifest still says "build against 1.1.0".
  §12 change-protocol amendment skipped. (fable F4/F5)
- **P-CHANGELOG-TRUTH [MED]**: package count (wrote 17→16, was 18→17),
  "(audit closed)" false (D2 transplant-naming open), "handoffs" plural (one
  marked); "green by gate" blanket wording vs red intermediate commits. (docs, CI)
- **P-DEFERRALS [MED]**: recorder-seam + `--author-kind` authn-binding are
  vapor-deferred (commit-message prose, no findable tracked item). (complete F2/F3)
- **P-SELF-LAUNDER [MED]**: only ContextEnvelope hand-pinned; 14 frozen
  contracts still generator-verified. Pin the security-critical ones. (complete F1)
- **P-TEST-GAPS [MED]**: no single e2e 4-altitude chain (capture→cold→rollup→
  show); mid-write cold-store failure uninjected; rollup untested at scale.
  (complete F4/F5/F6)
- **P-TRANSPLANT-NAMING [LOW]**: `HUGIT_RUNNER_HOST` + `hugit-runner` doc title
  inside the corelink-runner product (audit D2, never done). (arch, docs)
- **P-RUNNER-SEC [LOW-MED]**: self-hosted runner on the dev box, `pull_request`
  trigger, no fork guard (medium for a private repo). (CI)

## Wave E dispatch (partition by owned files)
- **E-REDACT** (sonnet, hugit-ledger/redact.rs): SOTA engine. Lands FIRST.
- **E-CLI** (opus, hugit-cli/**): redaction parity (read+write) using the
  hardened engine + pr/campaign error-law + missing-log + why format. After E-REDACT.
- **E-GUARD** (opus, hugit-mirror sync + hugit-refstore undo/authz): route the
  two bypasses through append_authorized. Now.
- **E-PINS** (sonnet, hugit-contracts/tests): hand-pin security-critical types. Now.
- **E-TESTS** (sonnet, hugit-ledger+dogfood tests): the 3 gap tests. Now.
- **E-DOCS** (sonnet, docs ×3 repos): contract-1.2.0 propagation + CHANGELOG
  truth + deferral tracking + wedge-claim downgrade + green-by-gate wording +
  transplant naming + runner-sec note. Now.
Then Round 2: fresh 7-agent fleet.
