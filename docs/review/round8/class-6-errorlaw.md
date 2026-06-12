# CLASS 6 — ERROR-LAW / CONCURRENCY / DETERMINISM — SOTA audit

Round 8 · fresh adversarial audit on the integrated state (`main` @ Wave K, HEAD `def8a18`).
Auditor: cold-context, class-enumeration method (not a point-finding hunt).

## 1. Scope & method

Class 6 covers three intertwined invariants across `crates/hugit-cli/src/` (the
typist-facing surface) and the `hugit-checks` engine it consumes:

- **(A) Error law** — every error-emitting path emits the canonical envelope
  `{"error":{"kind","message","fix", …context}}` on **stdout** with exit **2**
  (or exit 1 for `kind:"internal"`), never a bare `error:` on stderr + exit 1.
- **(B) Concurrency** — every lock/concurrent path preserves its **retryable
  taxonomy** (a retryable `*_busy` must not collapse to a terminal `*_error`),
  and every multi-step write is atomic (no torn state / double-record under a race).
- **(C) Determinism** — every claim of byte-identical / deterministic output holds.

Method: (1) enumerated EVERY error emission in the CLI (`eprintln!`, `bail!`,
`process::exit`, `println!{to_json}`, clap parse); (2) enumerated EVERY lock
window + multi-write (log append, `.ac` cache write, store commit); (3) checked
EVERY determinism claim (canonical/sorted serialization, env axis, memo key);
(4) built `hugit-cli` and STRESS-ran the Round-7 flaky test ×10; (5) asked the
root question — is the envelope enforced at ONE choke-point and is retryability a
TYPE.

Build: `cargo build -p hugit-cli --locked` → green.

## 2. Complete inventory

### THE error matrix (every emission path × envelope ✓/✗ × taxonomy)

| Path | Site | Envelope on stdout? | Exit | Taxonomy correct? |
|---|---|---|---|---|
| Legacy verbs (why/impact/tournament/export) | `main.rs:470–481` `Err(e)=>println!(e.to_json())` | ✓ | 2 / 1 | ✓ |
| campaign | `campaign/mod.rs:160` `println!(err.to_json())` | ✓ | 2 / 1 | ✓ |
| intent | `intent/mod.rs` → `error::PorcelainError::to_json` | ✓ | 2 | ✓ (`fix`-keyed, converged) |
| pr (read seam) | `pr/cli.rs:486 emit_porcelain` | ✓ | 2 | ✓ |
| pr (write/io seam) | `pr/cli.rs:452 emit_io_error` (`io_error`) | ✓ | 2 | ✓ (K-RUN sibling) |
| pr (domain) | `pr/cli.rs:476 emit_error` → `PrError::to_porcelain` | ✓ | 2 | ✓ |
| checks/check | `main.rs` → `run`→`PorcelainError` | ✓ | 2 / 1 | ✓ |
| verdict | `verdict/mod.rs:118` `println!(e.to_json())` | ✓ | 2 | ✓ |
| queue | `queue/mod.rs:83` `println!(e.to_json())` | ✓ | 2 | ✓ |
| AC exec error map | `checks/run.rs:1235 map_exec_error` | ✓ | 2 | **✗ — string-sniffed retryability (F-1)** |
| log lock error map | `checks/run.rs:1202 map_lock_error` | ✓ | 2 | ✓ (typed `LockError::Busy`) |
| **clap arg/subcommand error** | `main.rs:446 Cli::parse()` | **✗ — bare `error:` on STDERR** | 2 | **✗ — no kind/fix (F-2)** |
| internal fault | `PorcelainError::internal` | ✓ | 1 | ✓ |

Findings: NO `eprintln!`, NO `bail!`, NO `process::exit(1)` anywhere in
`crates/hugit-cli/src/` (verified by grep). The legacy plaintext-stderr/exit-1
paths the docs reference (P-PR-LAW) are fully removed. The envelope is rendered
at TWO duplicated-but-identical points (`porcelain::PorcelainError::to_json` and
`intent::error::PorcelainError::to_json`) plus `PrError::to_porcelain` — a code
duplication (F-3), not a divergence (shapes verified byte-equal + tested).

### Lock / atomic paths

| Path | Lock | Atomicity | Verdict |
|---|---|---|---|
| log append (`record_on_log` run.rs:1102) | `FileLock` held across load→dedup-scan→append→persist | `atomic_write` (temp+fsync+rename) | ✓ atomic by construction; dedup-under-lock kills double-record |
| `.ac` cache read (`FileAc::lookup` run.rs:809) | `acquire_ac_lock` per-op | read-only | ✓ |
| `.ac` cache write (`FileAc::store` run.rs:837) | `acquire_ac_lock` per-op | `atomic_write` (run.rs:861) | ✓ atomic |
| store commit (intent/campaign) | `StoreError::Busy` typed → `store_busy` | atomic_write | ✓ (typed busy) |
| `atomic_write` (`pr/filelock.rs:187`) | n/a | `File::create(.tmp-pid-nanos)` → `write_all` → `sync_all` → `rename` → cleanup-on-fail | ✓ crash-safe, collision-safe |

### Determinism claims

| Claim | Site | Order-stable? |
|---|---|---|
| env axis manifest | `result_affecting_env_manifest` run.rs:260 | ✓ `pairs.sort_by(key)` (allowlist; declared residual = AR/PS) |
| tree snapshot | `snapshot_tree` run.rs:293 `BTreeMap` | ✓ ordered |
| verdict payload | `verdict/mod.rs:326` `hugit_refstore::canonical_json` (sorted keys) | ✓ |
| memo_key | engine three-axis digest | ✓ (content-addressed, sorted inputs) |
| NO `HashMap` in non-test CLI code | grep | ✓ no iteration-order hazard |

## 3. Findings

### F-1 · MEDIUM (latent HIGH at P2) · TYPE: code (structural) · the Round-7 class is patched, not cured
- **ROOT**: `AcError` (`hugit-checks/src/client/ac.rs:28`) has NO retryable
  variant. Retryability is reconstructed downstream by **string-sniffing** —
  `map_exec_error` (run.rs:1239) matches `AcError::Transport(msg) if
  msg.starts_with("ac_busy:")`. The magic prefix is produced at exactly ONE site
  (`acquire_ac_lock` run.rs:898). Any OTHER retryable AC source bypasses it and
  collapses to terminal `ac_error` — the EXACT failure mode Round-7 found.
- **Repro (live, latent until P2)**: the HTTP AC transport wraps raw ureq errors
  as `AcError::Transport(e.to_string())` (ac.rs:525/531/544) — a 429/503/network
  timeout (all retryable server-busy conditions) produces a Transport string with
  NO `ac_busy:` prefix → `map_exec_error` → terminal `ac_error` → under
  fleet-shared-cache contention the loser gets a terminal kind, the gate goes
  flaky. Same for `AcError::Status(429)`/`Status(503)` (ac.rs:43) — typed but
  unmapped-as-retryable. The local `.ac` path is the ONLY one currently exercised
  (hermetic), so the stress test passes (10/10) — the hole is real but masked
  until the live AC seam lights up at P2.
- **Contrast that proves the root**: the sibling `LockError::Busy { path }`
  (filelock.rs:62) IS a typed variant → `map_lock_error` (run.rs:1202) matches it
  structurally → `log_busy` can never silently collapse. `AcError` lacks the
  symmetric variant; that asymmetry IS the class.

### F-2 · MEDIUM · TYPE: code · clap argument errors violate the error law
- **ROOT**: `Cli::parse()` (main.rs:446) lets clap own the failure path. clap
  exits INSIDE `parse()` before `main`'s `match`, so the bad-args path never
  reaches the envelope choke-point.
- **Repro (live)**:
  - `hugit frobnicate` → STDERR `error: unrecognized subcommand 'frobnicate'`, exit 2.
  - `hugit why` (missing required args) → STDERR `error: the following required
    arguments were not provided: --log --path`, exit 2, **stdout empty**.
- **Harm**: an orchestrating agent parses **stdout** for `{"error":{"kind",…}}`.
  A malformed invocation yields empty stdout + an unparseable English string on
  stderr — no `kind` to match, no `fix` to act on. The exit code (2) is even
  correct, which makes it look like a structured domain error to a code-only
  matcher while carrying zero machine signal. This is a whole sub-class (every
  arg/subcommand/value-parse error of every verb), not one path.

### F-3 · LOW · TYPE: honesty/maintainability · two PorcelainError types + one PrError mapper
- `porcelain::PorcelainError` and `intent::error::PorcelainError` are
  independent structs with byte-identical `to_json` logic (copy); `PrError` has
  its own `to_porcelain`. Shapes verified equal + tested, so NOT a current
  divergence — but three render paths is three places a future drift can hide.
  The docs still narrate the long-closed `suggested_fix` schism as if open
  (porcelain.rs:23–25, 63–70) — a stale-honesty comment, not a code defect.

## 4. Root-cause analysis

Two independent structural roots, one per real finding:

1. **Retryability is data, not type (F-1).** The envelope choke-point is solid
   for the *shape*, but the *taxonomy* (retryable vs terminal) of the AC layer is
   carried as an ad-hoc string convention across the engine→CLI seam. K-RUN fixed
   the ONE observable instance by sniffing a prefix; it did not make the
   distinction unforgeable. The correct invariant — already proven by
   `LockError::Busy` — is that *retryability is a variant of the error enum*, so
   the downstream mapper matches a TYPE and the compiler enforces totality. Every
   future retryable AC source (HTTP 429/503, the live fleet-shared cache) must be
   unable to NOT be classified.

2. **The choke-point has a hole upstream of itself (F-2).** `main`'s
   `Ok/Err → println!{to_json}` is the one law for everything that *reaches* it —
   but `Cli::parse()` short-circuits to clap's own stderr exit *before* dispatch.
   The choke-point is necessary but not sufficient while parsing can exit outside it.

## 5. Recommended structural remediation (single choke-points; atomic-by-construction)

**Fix F-1 (class-killer) — make retryability a TYPE, mirror `LockError::Busy`:**
- Add `AcError::Busy { detail: String }` (and/or `AcError::Retryable(...)`) to
  `crates/hugit-checks/src/client/ac.rs`. Make `acquire_ac_lock`
  (`checks/run.rs:898`) return `AcError::Busy{…}` instead of the `"ac_busy:"`
  prefixed `Transport`. Map HTTP `Status(429)`/`Status(503)` and retryable
  transport classes to `AcError::Busy` at the ureq boundary (ac.rs:525–544).
- Change `map_exec_error` (run.rs:1235) to `match ExecError::Ac(AcError::Busy{..})
  => "ac_busy"` — a TYPED arm, delete the `starts_with("ac_busy:")` string guard.
  Now the compiler's exhaustiveness check forces every NEW `AcError` variant to be
  consciously classified retryable-or-terminal; a missed retryable can no longer
  silently flatten. Keep the existing `ac_busy_lock_exhaustion_maps_to_retryable_kind`
  test + add a `Status(503)→ac_busy` and a `Status(401)→ac_error` case.

**Fix F-2 — route clap through the envelope:** replace `Cli::parse()` with
`Cli::try_parse()` in `main.rs:446`; on `Err(e)`, render
`PorcelainError::new("invalid_arguments", e.to_string(), "<usage hint>")` to
STDOUT via the same `to_json()` and exit 2 (treat clap `DisplayHelp`/`DisplayVersion`
kinds as the normal exit-0 stdout). One new arm at the existing choke-point; the
parse error now obeys the one error law like every other path.

**Fix F-3 — collapse to one type (optional, low):** re-export
`porcelain::PorcelainError` as the single error type; make `intent::error` and
`PrError::to_porcelain` adapters onto it so `to_json` exists exactly ONCE.
Refresh the porcelain.rs `suggested_fix` doc to state the schism is CLOSED.

Atomicity verdict: **all multi-writes are already atomic-by-construction**
(`atomic_write` temp+fsync+rename everywhere; log append holds the advisory lock
across the full dedup-scan→append→persist; dedup-under-lock provably prevents the
double-record). No torn-write or TOCTOU window found.

## 6. Residual / accepted

- **Env-axis allowlist (AR-5/PS-11)**: vars outside `RESULT_AFFECTING_ENV_*`
  (run.rs:243/263) do not enter the memo key — a *disclosed* stale-green residual
  for an unlisted result-affecting var read by an ad-hoc `--cmd`. Honest trade
  (capturing the whole env makes every run a miss). Accepted, tracked.
- **Live AC seam hermetic until P2 (PS-8/PS-12)**: F-1's HTTP-429/503 collapse is
  unreachable in the hermetic gate (only `FileAc` is exercised), so the stress
  test cannot surface it today. It is nonetheless a STRUCTURAL hole that WILL
  manifest the moment the fleet-shared cache lights up — fix F-1 before P2.
- **Stale doc**: porcelain.rs:23–25/63–70 narrate the closed `suggested_fix`
  schism as open — honesty-gap only (folded into F-3).

---

### Stress result (the Round-7 flaky test)

`cargo test -p hugit-cli --locked concurrent_checks_do_not_double_record_even_if_both_execute`
run ×10, bare exit code each: **PASS=10 / FAIL=0 (10/10)**. The K-RUN taxonomy
fix is stable under repeated runs *for the local `.ac` path it exercises*. This
does NOT cover F-1's HTTP-busy path (hermetic-unreachable). Stable, but on a
narrower axis than the class.
