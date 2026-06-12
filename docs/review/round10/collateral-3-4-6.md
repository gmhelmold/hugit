# Round 10 — Collateral sweep: Classes 3, 4, 6 vs Wave M

Auditor: fresh-context, read-only, default-refuse.
Branch: `integ/wave-m` · HEAD `e85bcf5`.
Wave M commits: M-2 `9301b56` (scrub unification) · M-1 `cf6977a` (read-path
chokepoint) · M-3 `72074e8` (intent atomicity) · M-INT `e85bcf5` (route
reconcile through chokepoint).
Build: `cargo build -p hugit-cli --locked` → clean (0.54 s, 0 warnings).
All repros live in `/tmp/hugit_r10/` outside the repo.

## 1. Scope & method

Wave M did NOT directly touch memo/AC/hermetic machinery, the authz door, or
the error-law envelope. This sweep confirms no collateral regression on:

- **Class 3** (MEMO-KEY/hermetic) — stdin null, PATH key axis, M-1's
  `checks/mod.rs` additions do not touch memo_key/AC/hermetic spawn.
- **Class 4** (AUTHZ) — raw `EventLog::append` door still `pub(crate)`;
  M-3's reconcile routes through `import_sidecar` → `append_authorized`.
- **Class 6** (ERROR-LAW) — M-1's `ChainLoadFault` maps to `chain_broken`
  exit-2; M-3's failure paths emit structured envelopes exit-2; clap envelope
  intact.

Method: targeted `grep` of changed files for collateral touches; live binary
repros for each confirming finding; `git show --stat` on each Wave M commit to
bound the change surface.

## 2. Class 3 — Hermetic/memo collateral check

**M-1's `checks/mod.rs` change is additive-only** for the memo path: it adds
`ChainLoadFault` + `rehydrate_and_verify` (a new read-path function) and
updates `load_event_log` to route through it. A `git show cf6977a --
crates/hugit-cli/src/checks/mod.rs` diff search for
`memo_key`/`compute_def_digest`/`AC`/`env_manifest`/`Stdio`/`env_clear`/`stdin`
returns zero hits. The hermetic spawn in `checks/run.rs` (`Stdio::null()` at
line 536, `env_clear()` + allowlist set) is UNTOUCHED by all four Wave M
commits.

**Live repro — stdin nulled (R9-C3 fix held):**
- Run1 `stdin=pass`: `cache_hit:false exit:1` key
  `ea5f4cbf1f17d599aaf2e21f118599554c468065271ab34a145b0c73d07c4d48`.
  (stdin is /dev/null → `read x` gets EOF → `[ "$x" = "pass" ]` → exit 1.)
- Run2 `stdin=FAIL`: `cache_hit:true exit:1` — SAME key, same result.
  No stale green possible: the check exit cannot be flipped by ambient stdin
  because stdin is always EOF.

The stdin null is deterministic, not a stale green. F-MK14-L (Round-9 close
`1e772a1`) is unaffected by Wave M.

**VERDICT: no collateral regression.**

## 3. Class 4 — Authz collateral check

**Raw door visibility:** `grep` for `pub fn append\b` in
`crates/hugit-refstore/src/log/mod.rs` returns `pub(crate) fn append` at
line 386. The Wave M commits do not modify `hugit-refstore`. Door is still
single and `pub(crate)`.

**M-3's reconcile uses guarded append only:**
- `reconcile_store_from_log` in `intent/new.rs` calls `import_sidecar` at
  lines 400–414 (the reconcile) and line 265 (the normal land path).
- `import_sidecar` (`hugit-refstore/src/intent/import/mod.rs:149`) calls
  `log.append_authorized(actor_class, Endpoint::Push, …)` — the D14-guarded
  door.
- A `grep` for raw `.append\b` calls in `crates/hugit-cli/src/` excluding
  comments and test modules returns zero hits.

**Live compile check:** the Wave M integration build (`e85bcf5`) is green;
no cross-crate raw append is reachable (compiler would reject it — raw door
is `pub(crate)`).

**VERDICT: no collateral regression.**

## 4. Class 6 — Error-law collateral check

**M-1 `ChainLoadFault` → exit-2 mapping:**
- `load_event_log` (checks/mod.rs:449) maps `ChainLoadFault::ChainBroken(e)`
  → `PorcelainError::new("chain_broken", …, "…")` — canonical envelope,
  exit 2 by the PorcelainError convention.
- Live repro: `hugit intent list --log <tampered.json> --store []` →
  `{"error":{"kind":"chain_broken",…}}` exit 2. Confirmed.

**M-3 failure paths:**
- Empty-charter guard: `PorcelainError::new("invalid_argument", …)` exit 2.
  Live repro: `hugit intent new --charter '   ' …` → exit 2 structured envelope.
- Reconcile encounters tampered `--log`: routes through `load_log_verified`
  → `crate::checks::rehydrate_and_verify` → `ChainLoadFault::ChainBroken`
  → `PorcelainError::new("chain_broken", …)` exit 2.
  Live repro: `hugit intent new --charter 'x' --log <tampered> --store []`
  → `{"error":{"kind":"chain_broken",…}}` exit 2. Confirmed.
- Store-save failure path: `PorcelainError::from_store` (structured, exit 2).
  No bare `eprintln!`/`process::exit(1)` in new.rs (grep clean).

**Clap envelope intact:**
- `hugit intent frobnicate` → `{"error":{"kind":"invalid_arguments",…}}` exit 2.
- `hugit intent new --bogus-flag` → `{"error":{"kind":"invalid_arguments",…}}` exit 2.
- No raw stderr output, no exit 1.

**VERDICT: no collateral regression.**

## 5. VERDICT per class

| Class | Converged? | Confirming repro |
|-------|-----------|-----------------|
| 3 (MEMO-KEY/hermetic) | **Y** | `stdin=pass\|FAIL` → both `cache_hit:false\|true exit:1` (EOF-deterministic, no stale green); M-1 diff touches zero memo/AC/spawn lines |
| 4 (AUTHZ) | **Y** | `pub(crate) fn append` at log/mod.rs:386 unchanged; M-3 reconcile calls `import_sidecar` → `append_authorized`; zero raw cross-crate appends (grep + compiler) |
| 6 (ERROR-LAW) | **Y** | Tampered log via `intent list` → `chain_broken` exit 2; M-3 reconcile with tampered log → `chain_broken` exit 2; empty charter → `invalid_argument` exit 2; clap envelope intact |

**No collateral regression found. All three classes remain CONVERGED on `integ/wave-m` HEAD `e85bcf5`.**
