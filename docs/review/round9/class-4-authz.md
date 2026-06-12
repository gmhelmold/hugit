# CLASS 4 — AUTHZ — Round 9 re-audit

Convergence re-audit, fresh context. Target: branch `integ/wave-l` @ `95353f9`
(engine state post Wave L / L-D). Built `hugit-cli --locked` with the 1.96.0
toolchain; all live forge-repros run in `/tmp` temp dirs outside the repo.

## 1. Scope & method

The D14 invariant: **every write/append/mutation to the event-log MUST pass the
authorization guard** (`EventLog::append_authorized`, evaluating the frozen
permission matrix on the only mutating primitive). The recurring class root
(Rounds 7→8): `EventLog` exposed **two** `pub` append doors — the raw
`append` (no authorization) and the guarded `append_authorized` — so the
"every verb routes through the guard" discipline lived in *prose*, not in
*types*. Round 8 §5 prescribed the structural fix; **Wave L (L-D) implemented
it**. This round verifies that fix landed and tries to break it.

Method:
1. Re-enumerated EVERY mutation call site across the whole workspace
   (`grep --include=*.rs` for `.append(` / `append_authorized` /
   `append_external_change` / `append_for_test` / `push_record` / `.submit(`),
   excluding `hugit-refstore` itself, then partitioned production vs `tests/`.
2. Verified the raw door is now `pub(crate)` and unreachable cross-crate; that
   `append_for_test` is truly `#[cfg(feature="test-support")]` and that feature
   is wired ONLY under `[dev-dependencies]` (never default, never a prod edge,
   resolver = "2" so dev-dep features do not unify into normal builds).
3. ATTACKED the live CLI surface: forged author class via `pr open`; forged
   record via a tampered `--log`; a SELF-CONSISTENT recomputed forged chain.
4. Confirmed the typed `append_external_change(ExternalChangeKind)` shim is a
   closed enum (`RefUpdate`/`RefDelete` only) that cannot express a guarded kind.
5. Ran the source-invariant gate test
   (`no_out_of_crate_raw_append_in_production_source`).

## 2. Mutation matrix (every path × guarded/shim/raw × forgeable?)

`kind`/`principal`/`payload` = what the caller controls at that call site.

| # | Path (file:line) | Door | CLI-reachable? | Forgeable | Verdict |
|---|---|---|---|---|---|
| 1 | `pr open` → `pr/mod.rs:498 append_authorized` | guarded | YES | class clap-restricted to orchestrator\|human; chain derived | GUARDED |
| 2 | `pr land --settle` → `pr/mod.rs:906 append_authorized` | guarded | YES | class = opened PR's author_kind (not free) | GUARDED |
| 3 | `pr abandon` → `pr/mod.rs:997 append_authorized` | guarded | YES | opened PR's author class | GUARDED |
| 4 | `pr open` seed → `pr/cli.rs:573 append_authorized` | guarded | YES | Orchestrator/Push, system-fixed | GUARDED |
| 5 | `verdict --store` → `verdict/mod.rs:368/592/669 append_authorized` | guarded | YES | NOTHING — class+principal system-fixed (K-VERDICT) | GUARDED |
| 6 | `check` recorder → `checks/run.rs:1252 append_authorized` | guarded | YES | Orchestrator/Push (universal-Allow); principal scrubbed | GUARDED |
| 7 | `campaign open/close/abandon` → `campaign/world.rs:719 append_authorized`; `seal_guard.rs:107` | guarded | YES | Human/Policy (non-human owner denied+audited) | GUARDED |
| 8 | `intent new --log` → `intent/canonical_log.rs:207 append_authorized` (Push) | guarded | YES | Push universal-Allow; chain = campaign+agent (asserted) | GUARDED |
| 9 | `export` → reads `corpus.event_log` via `Cut::take` (`export/cut.rs:67`, `verify_chain` @ :83) | read-only | YES | NONE — no append; chain verified; tampered input → chain_broken exit-2 | GUARDED (R7 fix held) |
| 10 | `proto::record_ref_update/delete` (`proto/write/store/mod.rs:135,151`) | **typed shim** `append_external_change(ExternalChangeKind)` | NO (no proto dep in CLI) | kind closed to ref.update/delete BY TYPE; cannot emit guarded kind | TYPED-SHIM (L-D) |
| 11 | `proto::record_external_change` (`proto/write/external/mod.rs:173`) | **typed shim** | NO | closed enum; empty chain refused upstream | TYPED-SHIM (L-D) |
| 12 | `mirror land` (`mirror/sync/engine.rs:477`) | guarded `append_authorized(Land)` | NO (no mirror dep in CLI) | class gated by Land matrix before append | GUARDED |
| 13 | `policy emitter` (`hugit-policy/src/lib.rs:322 append_authorized`) | guarded | NO (lib) | Endpoint::Policy matrix-gated | GUARDED |
| 14 | `dogfood wave/soak` (`wave.rs:178`, `soak.rs:199…239 append_authorized`) | guarded | NO (harness) | now routes through guard (raw door gone) | GUARDED |
| 15 | `Serializer::submit` (`concurrency/mod.rs:356`) | **guarded** `AuditedGuard.authorize(Push)` then in-crate raw append on Allow | NO (no prod caller; test-only) | authorize→append one critical section; denial audited (F-2 CLOSED) | GUARDED (C4-F2) |
| 16 | `Serializer::undo` (`concurrency/mod.rs:448`) | guarded `authorize(Undo)`, Human-only | NO | fail-closed on non-human | GUARDED |
| 17 | `EventLog::append` (`log/mod.rs:386`) | **`pub(crate)`** raw primitive | **NO — in-crate only** | only refstore internals reach it | IN-CRATE RAW (L-D) |
| 18 | `EventLog::append_for_test` (`log/mod.rs:473`) | `#[cfg(feature="test-support")]` `#[doc(hidden)]` | NO (dev-dep only) | test fixtures only; feature never in a prod build | TEST-ONLY |
| 19 | `push_record` (pub) — rehydration loaders: `pr/cli.rs:382`, `campaign/world.rs:599`, `intent/store.rs:179`, `intent/list.rs:194`, `checks/mod.rs:391`, `export/cut.rs:67`, `export/mod.rs:209/641` | takes pre-formed `EventRecord`; EVERY production loader calls `verify_chain` immediately after | YES (via `--log`) | a hand-edited record fails `verify_chain` → chain_broken exit-2. A self-consistent recomputed chain passes — see §4 (disclosed P2 seam) | VERIFY-GATED (evident, not proof) |
| 20 | `Journal::append` (`hugit-ledger journal/persist.rs:93`) | session-resume journal — DIFFERENT type, not the D14 `EventLog` | — | session notes only | OUT OF SCOPE |

**Matrix verdict:** every LIVE CLI-reachable mutation (1–9) is guarded or
read-only. Every cross-crate raw need (10–14) is now guarded or routed through
the typed closed-enum shim. The raw primitive (17) is `pub(crate)` —
in-crate-only. `submit` (15) is guarded (F-2 closed). `append_for_test` (18)
is dev-only. No NEW exploitable bypass found; both Round-8 structural findings
(F-1, F-2) are closed by construction.

## 3. Door-is-single verification

- **Cross-crate `::append` reachability:** `grep` for raw `.append(` over all
  `*/src/` outside `hugit-refstore` returns ZERO D14-EventLog hits (the only
  `.append(` matches are `hugit-ledger`'s `Journal::append`, a distinct
  session-journal type). The raw door is `pub(crate) fn append`
  (`log/mod.rs:386`); a cross-crate caller cannot name it. **Verified by the
  compiler-enforced gate test** `no_out_of_crate_raw_append_in_production_source`
  (`acceptance_round8_statemachine.rs:457`) — GREEN (6/6 in that suite).
- **Feature gate:** `test-support = []` (no default). `append_for_test` is
  `#[cfg(feature="test-support")]` + `#[doc(hidden)]`. The feature is enabled
  ONLY under `[dev-dependencies]` in `hugit-cli`, `hugit-ledger`,
  `hugit-invariants`, `hugit-dogfood`, and refstore's own self-dev-dep — never
  in a `[dependencies]` table. Workspace `resolver = "2"`, so a dev-dependency
  feature does NOT unify into the normal (binary) build of any crate. The
  `hugit` binary therefore links refstore WITHOUT `test-support`; `append_for_test`
  is not in the production surface.
- **Closed-enum shim:** `ExternalChangeKind` (`intent/model/mod.rs:44`) has
  exactly `RefUpdate`/`RefDelete`. `append_external_change` takes this enum, not
  `impl Into<String>`. There is **no variant** that maps to `pr.opened` /
  `intent.landed` / `verdict.*`, so a forged guarded event through the shim is
  **not expressible** — it does not compile. (Attempting to pass a string kind
  or constructing a non-existent variant is a type error; the old runtime
  `debug_assert_ne!` is gone, replaced by this type-level guarantee. Confirmed
  in proto: `record_ref_update/delete` and `record_external_change` all call
  `append_external_change(ExternalChangeKind::…)`.)

## 4. Residual findings — separate code holes from the disclosed principal seam

**No code holes found.** Both Round-8 structural findings are closed:
- **F-1 (two public append doors) — CLOSED by L-D.** `append` is now
  `pub(crate)`; the only cross-crate doors are the guarded `append_authorized`
  and the typed `append_external_change`. The gate test makes it an invariant,
  not a snapshot.
- **F-2 (`Serializer::submit` raw-appends unguarded) — CLOSED by L-D.** `submit`
  now authorizes `op.principal_chain` through `AuditedGuard` under
  `Endpoint::Push` inside the writer lock BEFORE the (in-crate) append; a denial
  writes an `authz.denied` audit record and refuses. (Still no production caller
  — pure pre-fencing for a future `push` verb.)

**Disclosed P2 identity seam (NOT a bug) — the caller-asserted principal +
tamper-EVIDENT log.** SEVERITY: accepted seam · TYPE: seam (honestly disclosed).
Repro (cold, `/tmp`): a forger with write access to a `--log` file who
recomputes the **public, keyless SHA-256** hash chain can append a
self-consistent `pr.landed` with `principal_chain=["orchestrator:forged"]`;
`pr show` and `export` then accept it (exit 0), because `verify_chain` proves
internal hash-linkage but NOT authorship — there is no MAC/signature and the
`principal_chain` bytes are caller-supplied.

This is the disclosed seam, not a Wave-L regression:
- It requires local write access to the actor's OWN local log file; the D14
  guard fences the *live append door* (you cannot make `pr open` emit a class
  you didn't assert via clap), which is exactly what L-D hardened.
- It is the same property CLAUDE.md discloses as PS-8 ("the event-log hash
  chain is tamper-EVIDENT not tamper-PROOF") and Round-8 §6 / F-3 discloses as
  the caller-asserted class/principal seam; the class→authenticated-principal
  and signed-record binding lands with identity rollout (ADR-0002). The module
  docs state it in the correct tense (`append_authorized` doc, `log/mod.rs:428`).
- Every CLI verb derives or hardcodes both class and chain — no live verb forges
  a class it could not locally justify.

**Live attack log (cold, `/tmp`):**
- `pr open --author-kind orchestrator` → ALLOW, `pr.opened` lands (exit 0).
- `pr open --author-kind subagent|worker` → `subagent_author`, exit 2 (clap
  door rejects; no forged class reaches the matrix).
- `pr show --log <hand-edited record, this_hash="deadbeef…">` → `chain_broken`
  exit 2 (this_hash mismatch at seq 1) — tampering is evident.
- `pr show` / `export --log <SELF-CONSISTENT recomputed forged chain>` → exit 0
  — the disclosed keyless-chain / caller-asserted-principal P2 seam (above), NOT
  a forge-via-live-verb hole.

## 5. CONVERGENCE VERDICT

**CONVERGED.** The raw `EventLog::append` door is single and `pub(crate)`
(in-crate only), enforced by `rustc` AND a green source-invariant gate test.
Every live CLI mutation path is guarded (`append_authorized`) or read-only
(`export`, verify-gated). The two legitimate cross-crate raw needs route through
the typed closed-enum `append_external_change` shim, which is structurally
incapable of emitting a guarded kind. `Serializer::submit` is now D14-guarded.
`append_for_test` is dev-only and absent from the production build. Both
Round-8 structural findings (F-1, F-2) are closed by construction. The ONLY
residual is the disclosed P2 identity seam — the caller-asserted
`principal_chain` over a keyless (tamper-EVIDENT, not tamper-PROOF) hash chain,
which binds to an authenticated principal at identity rollout. That is an
honestly-fenced seam, distinct from a forge-able code defect.
