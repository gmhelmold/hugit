# Round 11 — six-class hostile sweep (fresh context)

- **Target:** hugit engine (`crates/`), `main` HEAD `5b73a8b` (Wave O).
- **Mandate:** broad sweep for any NEW P0/P1 the prior rounds missed across the
  six root-cause classes. Five classes in depth (redaction, read-path, authz/D14,
  state-machine, error-law/concurrency) + a LIGHT wedge pass (the wedge
  stale-green class is a sibling auditor's job). READ-ONLY; default-REFUSE.
- **Method:** built `hugit-cli` (`cargo build -p hugit-cli --locked`, exit 0),
  then live-reproduced attacks with the debug binary against scratch repos
  OUTSIDE the tree (`/tmp/r11scratch`, cleaned). Every claim below is either a
  live repro or a precise code-cite. No edits, no commits.

## VERDICT

**No DO-NOT-SHIP. No new P0/P1 code hole in any of the five in-depth classes.**
One **P2 / honesty** doc-overclaim found in the *policy CI secrets gate* (not on
any at-rest path — see Class 1). The integrity spine held under every attack.

---

## Class 1 — REDACTION / secret-at-rest — HELD (one P2 honesty doc note)

The unified detector (`hugit-ledger::secret_shape`) is the single source for
both the free-text engine (`redact.rs`, threshold 4.0 + bare-hex exemption) and
the identifier door (`is_safe_identifier_shape`, threshold 4.5, PS-14 hybrid).
The CLI write boundary (`porcelain::scrub_payload`) re-evaluates the scrub mode
per `(key,value)` with deny-by-default FreeText, Structural for identifier keys,
and value-gated Verbatim for digest keys.

Attacks (all redacted at-rest, byte-scanned in both `--store` and `--log`):
- PAT (`ghp_…`) in `--charter` free text → `[REDACTED]` wholesale at rest.
- High-entropy 35-char base64 blob + `postgres://u:pass@h` conn-string +
  `cas:ghp_…` smuggle, all in one charter → all redacted (the `cas:` value-gate,
  `secret_shape::is_content_address_ref`:261, correctly refuses the smuggle).
- 40-char dense AWS-style key as `--run-id` (identifier door, Structural mode) →
  rejected at input with `secret_in_identifier`/exit-2 (the entropy gate at
  `secret_shape.rs:404` fires even in Structural mode).
- `clp_` PAT as `--run-id` → same `secret_in_identifier` rejection.

No regression from the N-4 unification was found in any field of any verb tested.

**P2 / honesty (NOT a leak):** `hugit-policy/src/gates/secrets.rs` module doc
(lines 9–13) claims the gate covers the "entropy scan". `eval` (line 42) calls
ONLY `is_structural_secret`, which by design EXCLUDES detector-5 (the entropy
scan). Conn-string + keyword-context ARE covered (they live inside
`is_structural_secret`), but a bare high-entropy unprefixed key in a changed
source file would PASS this gate. This is the standalone CI advisory gate — it is
**not wired into any at-rest write path** (verified: the write paths use
`scrub_to_canonical`/`scrub_payload`/`redact::apply`, which DO run the entropy
scan). So it is a documentation overclaim, not a forever-log leak. Severity P2,
type honesty.

## Class 2 — READ-PATH — HELD

Single chokepoint `checks::rehydrate_and_verify` (`checks/mod.rs:411`) →
`hugit_refstore::verify_chain`; `load_event_log` routes every flow read through
it. Live tamper (flip a payload byte, leave hashes) on the canonical
`[EventRecord,…]` array:
- `export`, `checks show`, `queue show`, `pr show`, `campaign show` → all
  `chain_broken`/exit-2, none projected a tampered log.
- `why` uses its own wrapper shape and also fails closed (`parse_log`/exit-2).
The R7 `why`/`export`-skip-verify holes are confirmed closed.
Residual is the known PS-8 honesty seam (tamper-EVIDENT, not tamper-PROOF: an
attacker who controls the file can re-chain). No code regression.

## Class 3 — AUTHZ / D14 — HELD

Raw `EventLog::append` is `pub(crate)` (`refstore/log/mod.rs:386`). The only
cross-crate doors are `append_authorized` (matrix-guarded, audits denials) and
the typed closed-kind `append_external_change` shim (can only emit
`ref.update`/`ref.delete` — a compile-time fact). Mirror sync routes through
`append_authorized` (`mirror/sync/engine.rs:477`); proto external/store paths
use the typed shim and fail closed on empty `principal_chain`
(`external/mod.rs:164`, `MissingAttribution`). `export` no longer raw-appends —
it reads via `load_event_log` (K-CHAIN, `main.rs:415`). No verb-reachable raw
append.

## Class 4 — STATE-MACHINE — HELD

- Lens-laundering (`--lens security --result reject --lens security --result
  approve`) → rejected at the door (`duplicate_lens`/exit-2, `verdict/mod.rs:206`)
  AND the ledger fold is reject-sticky; aggregate is APPROVE only if ALL lenses
  approve (`aggregate_verdict`:426) — a mixed panel returns `reject` (live-confirmed).
- Sealed = terminal: after `campaign close`, live attempts to `verdict`,
  `pr open`, and `intent new` on the sealed campaign were ALL refused with
  `campaign_sealed`/exit-2 via the shared `seal_guard::guard_not_sealed`
  chokepoint (C5-F2). No post-seal append path found.

## Class 5 — ERROR-LAW / CONCURRENCY — HELD

- Adversarial JSON (malformed, 50k-deep nesting, empty file, array-of-numbers,
  incomplete records) → all structured `parse_log`/exit-2, **no panic**, on
  `checks show`/`export`/`why`. Clean read = exit-0.
- Concurrency: 8 and 6 parallel `intent new` to the same log → file lock
  serialized; one winner landed, losers got structured retryable
  `store_busy`/exit-2 (no silent drop), the chain verified intact afterward
  (no torn write). No `ac_busy`→`ac_error` collapse on this path.
- AC taxonomy preserved: `AcError::Busy` (retryable) distinct from terminal
  variants (`checks/client/ac.rs:29`, K-RUN intact).

## Class 6 — WEDGE (light) — sane

`hugit check`/`verdict`/`campaign close` dispatch end-to-end and produce honest
JSON projections (ledger counts, aggregate). Deep coverage deferred to the
dedicated wedge auditor per mandate.

---

## What I attacked and FAILED to break (confidence)

- Secret survival at-rest across charter/id/run-id/tree-hash, including the
  `cas:` smuggle, high-entropy non-prefixed keys, and conn-strings — all caught.
- Read-path projection of a tampered chain on six verbs — all fail closed.
- Verdict lens-laundering and post-seal mutation on three verbs — all refused.
- Panic / torn-write under malformed input and parallel writers — none observed.

High confidence the five in-depth classes hold at `5b73a8b`. The only finding is
the P2 documentation overclaim in the policy CI secrets gate (Class 1), which is
not on the redaction-at-rest spine and carries no shipping risk.
