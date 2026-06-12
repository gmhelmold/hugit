# Adversarial Round 7 — consolidated verdict

**Date:** 2026-06-12 · **Base:** `main` HEAD `bd95162` (Wave J + WK-AC merged & pushed)
**Fleet:** 7 fresh-context read-only adversaries (3 opus + 4 sonnet), default-refuse mandate.
**Result: 7/7 DO-NOT-SHIP.** Unlike Rounds 1–6 (which skewed to honesty-gaps once the
spine held), Round 7 surfaced **multiple confirmed CODE defects**, several
orchestrator-verified by live reproduction.

## Confirmed findings (lead cold-verified ✅ = reproduced live)

| # | Surface | Finding | Sev | Type | Verify |
|---|---|---|---|---|---|
| 1 | Secret-at-rest | **`cas:` exemption leaks a PAT verbatim.** `verdict --tree-hash "cas:ghp_…"` stores the token raw in the forever-log — any `cas:`-prefixed value is blanket-exempted from the detector (`porcelain.rs` `is_content_address_ref` / `redact.rs:404`), bypassing the WH-SCRUB value-gate. `--tree-hash` has no door; the Wave J matrix never tests it. Pre-existing since WH-SCRUB (`1ff2fcf`). **Exemption-is-a-hole #5.** | **P0** | code-defect | ✅ PAT verbatim count=1 |
| 2 | Event-log chain | **`hugit why` skips `verify_chain`.** `run_why` (`main.rs:154`) reads via `read_log()` (JSON-only) and projects a tampered log as authoritative provenance. **PS-8 line 273 explicitly claims "verify_chain continues to run on every read"** → false for `why`. Found independently by 2 agents. | **P0** | code-defect + honesty-gap | ✅ 0 verify_chain in why/; PS-8 claim present |
| 3 | Verdict resolution | **Lens-substitution launders a rejection.** `verdict --lens security reject` then `verdict --lens security2 approve` → latest-wins flips the projection to `proven:1 rejected:0` — the recorded reject is erased from the projection. (Agent also claimed `campaign close` then seals `sealed_with_rejected:false`; in the lead repro `close` exited 2 — the projection erasure is confirmed, the clean-seal is setup-dependent.) | **P1** | integrity-bypass | ✅ projection proven:1/rejected:0 |
| 4 | Wedge / memo | **env_manifest stale-green (missed-miss).** Check spawns `sh -c` inheriting the full ambient env, but `env_manifest` is hardcoded empty (`run.rs:196`); the `CheckDef` contract HAS the axis and the engine folds it — the verb never populates it. A result-changing env/`RUSTFLAGS` change does NOT bust the memo key → stale green, 0 execution. | **P1** | code-defect | agent repro (FOO=green→red) |
| 5 | D14 authz | **`hugit export` raw `.append()` bypasses the guard.** `run_export` (`main.rs:409`) builds an EventLog via raw `append()` for every event in a user file — arbitrary `kind`/`principal_chain`/`payload` (e.g. forged `pr.opened`) enters the export corpus unguarded. | **P1** | code-defect | agent file:line |
| 6 | Error-law / gate | **`ac_busy`→`ac_error` taxonomy collapse → flaky gate.** `map_exec_error` (`run.rs:1150`) flattens every `ExecError::Ac` (incl. the retryable `ac_busy` lock-exhaustion) to `kind:"ac_error"`. Under `--workspace` parallel load the retry budget exhausts → terminal `ac_error`, and `concurrent_checks_do_not_double_record_even_if_both_execute` (asserts `ac_busy`/`log_busy`) fails non-deterministically. **The CLAUDE.md "green by the LOCAL gate, verified by real exit code" claim is not deterministically true.** | **P1** | code-defect | ✅ code-read; agent repro (test exit 101); lead's earlier full run passed (load-luck) |
| 7 | Error-law | `pr open/land/abandon` I/O-fault path (`pr/cli.rs:467` `io_fail`) emits bare `hugit: error:` on stderr + exit 1, not the structured `{error:{kind,message,fix}}` JSON exit 2. | P2 | code-defect | agent file:line |
| 8 | Consistency | `intent new --log` two-phase commit is non-atomic: `--store` saved before the `--log` append; if the append fails, store/log diverge permanently (`intent/new.rs:251`). | P2 | code-defect | agent file:line |
| 9 | Secret-at-rest | Unprefixed high-entropy credential in an **identifier** field survives verbatim (identifier fields skip the entropy scan by design — indistinguishable from a legit address). Matrix only feeds PREFIXED specimens → the class is implied-covered but untested. **Needs an owner decision (accept as physics, or add an entropy heuristic with false-positive risk).** | P2 | accepted-seam? | agent repro |
| 10 | Honesty | CLAUDE.md status prose stale ("Wave J in progress, Round 7 pending") vs HEAD past Wave J + WK-AC. pending-seams register IS current. | P3 | honesty-gap | ✅ |

## Accepted-seam / negative results (no action)
- **principal_chain caller-asserted** (P2 identity rollout — disclosed).
- **Competent local rewriter forges the unkeyed chain** — correctly scoped by PS-8 (the fix is the P2 server-side keyed seam).
- **Cache poisoning by hand-written `.ac`** — stopped only at the local-trust boundary; documented (`run.rs:627`); P2 HMAC seam. (Blast radius compounds #1/#4.)
- No tamper-PROOF overclaim found; 17-package count + 4+13 crate split verified; P2 seams honestly fenced.

## Infra (not a code defect, but a red gate)
- **CI `gates/deny` step exit 127** on the push (run 27422619640): `cargo-deny` not installed on the self-hosted runner (same class as `cargo-audit` absent locally). The runner cannot run the full advisory gate. Track as an infra seam.

## Lead accountability
The orchestrator pushed `bd95162` asserting "100% green by real exit code." Honest correction:
`fmt`/`clippy`/`cargo deny check`/`cargo test --workspace` all passed **locally on one run**
(real exit 0) — but (a) the workspace test is **load-flaky** (finding #6) and determinism was
not stress-established before the green claim; (b) `cargo audit` is not installed locally and
`cargo-deny` is absent on the runner (CI red); (c) the matrix's coverage hole (#1/#9) meant a
PRE-EXISTING `cas:` PAT-leak vector was not caught before the push. **Round 7 should have run
BEFORE the push, not after.** The push did not introduce #1 (pre-existing since Wave H), but the
green claim was over-stated. → Wave K remediates; re-gate must STRESS the flaky path and run
Round 8 before any further push.

## → Wave K (remediation clusters)
- **K-SCRUB** — value-gate the `cas:`/content-address exemption (run the ONE detector on the value); door `--tree-hash` + digest fields; extend the matrix (cas:<secret>, --tree-hash, queue/tournament payloads, unprefixed-entropy decision).
- **K-CHAIN** — `verify_chain` on `run_why` (+ audit ALL read verbs); fix/realign PS-8 line 273; tamper test for `why`.
- **K-VERDICT** — reject must not be launderable by a novel-lens approve (resolution rule: per-intent, reject-sticky OR latest-complete-panel — owner/lead decision); post-close append guard.
- **K-WEDGE** — pin or hermetically reset the execution env so a result-changing env change busts the memo key (populate `env_manifest` or keyed allowlist).
- **K-AUTHZ** — `export` through `append_authorized` (or fence it as a non-authoritative dump).
- **K-ERRLAW** — preserve `ac_busy`/`log_busy` retryable kinds in `map_exec_error` (fixes the flaky gate); `pr` `io_fail` → structured JSON exit 2; `intent new` log/store atomicity.
- **K-DOCS/INFRA** — CLAUDE.md currency; runner `cargo-deny`/`cargo-audit` install (infra seam).
