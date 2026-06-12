# CLASS 3 — MEMO-KEY / WEDGE SOUNDNESS — SOTA audit

Round 8 · adversarial root-cause audit · auditor: fresh fleet on integrated state
(branch `integ/web-spine`, HEAD `def8a18` Wave K). CLI built + every finding
below cold-verified by LIVE reproduction (a real cached HIT served where a real
run would FAIL).

## 1. Scope & method

The product wedge is memoized CI:
`memo_key = sha256(LP(tree_hash) ‖ LP(def_digest) ‖ LP(toolchain_digest))`
— single-sourced in `crates/hugit-refstore/src/log/mod.rs:169`
(`compute_memo_key`). The soundness invariant: **the key must capture EVERY
input that can change a check's exit/result**; any uncaptured result-affecting
input yields a STALE GREEN (a warm HIT returns `exit:0` where a cold run would
return non-zero).

Axes derived in `crates/hugit-checks/src/client/memo_key.rs`:
- `tree_hash` — `scoped_tree_root`: SHA-256 Merkle over files under `--root`
  whose rel-path matches `def.glob_set` (`collect_files` walk in
  `crates/hugit-cli/src/checks/run.rs:346`, pruning `target/`/`.git/`/`.claude/`).
- `def_digest` — `compute_def_digest`: folds `command + inputs + toolchain_ref +
  env_manifest + glob_set`. The K-RUN `env_manifest`
  (`run.rs:260 result_affecting_env_manifest`) is a hand-maintained allowlist
  (`RESULT_AFFECTING_ENV_EXACT` + `RESULT_AFFECTING_ENV_PREFIXES`, `run.rs:225/243`).
- `toolchain_digest` — `resolve_toolchain_digest` (`run.rs:127`): `--toolchain`
  verbatim, else `sha256(rustc --version --verbose)`.

The check itself is spawned via `shell_command` → `sh -c <command>`
(`run.rs:585`) with **no `current_dir`, no env clearing, no sandbox** — it
inherits the orchestrator process's full cwd, full environment, and the entire
filesystem. `run_memoized` (`executor.rs:99`) returns the cached result BEFORE
the runner is touched on a hit (the zero-execution wedge is structural — which
is exactly why a stale key is unrecoverable: nothing re-runs to catch it).

Method: for each candidate input, pin the three axes constant and vary ONLY the
candidate so a cold run flips `pass→fail`; if the second run is a HIT serving the
stale PASS, the input is an uncaptured hole. Five holes reproduced live.

## 2. Complete inventory — THE matrix

Legend: captured ✓ / not-captured ✗ / partial ~ ; affects-result Y/N.

| # | Input a spawned check can read | In memo_key? | Affects result? | Note |
|---|---|---|---|---|
| 1 | Files under `--root` matching `glob_set` | ✓ | Y | the one captured input axis |
| 2 | Files under `--root` OUTSIDE `glob_set` (built-in glob = `**/*.rs`,`**/Cargo.toml`,`Cargo.lock` only) | ✗ | Y | misses `.cargo/config.toml`, `rust-toolchain.toml`, `build.rs` `include_str!` data, non-`.rs` sources, `*.json`/`*.txt` fixtures |
| 3 | Files under pruned dirs `target/`, `.git/`, `.claude/` (even with ad-hoc `**/*`) | ✗ | Y | **F-MK3** repro'd |
| 4 | Files OUTSIDE `--root` (abs path / `../` / `$HOME`) | ✗ | Y | **F-MK4** repro'd |
| 5 | Symlinks pointing outside the tree (target content) | ✗ | Y | walk hashes link target bytes only if the *path* is in-tree+globbed; an out-of-tree symlink target is uncaptured |
| 6 | `.gitignore`d files inside the glob | ~ | Y | hashed if path-matches (no gitignore filter) — captured by accident, not design |
| 7 | **cwd** of the `hugit` process (check uses relative paths) | ✗ | Y | **F-MK1** repro'd — `Command` has no `current_dir`; `--root` scopes the snapshot, NOT the spawn |
| 8 | ENV — allowlisted (`RUSTFLAGS`, `CARGO_*`, `RUST_*`, `CC`…) | ✓ | Y | K-RUN closed these |
| 9 | ENV — **unallowlisted** (`PATH`, `HOME`, `LANG`/`LC_*`, `TZ`, `SOURCE_DATE_EPOCH`, `SSH_*`, `http_proxy`/`https_proxy`/`no_proxy`, `CARGO_HOME`†, `RUSTUP_HOME`, `TMPDIR`, `UMASK`, any custom `MY_GATE_MODE`) | ✗ | Y | **F-MK2** (custom var) + **F-MK5** (`PATH`) repro'd. †`CARGO_HOME` is NOT covered: prefix list is `CARGO_`,`RUST_`,`CARGO_BUILD_` and `CARGO_HOME` matches `CARGO_` ✓ — but `RUSTUP_HOME`/`SSH_AUTH_SOCK`/proxy/`TMPDIR`/locale do not |
| 10 | `def.command` (the shell string) | ✓ | Y | folded into `def_digest` |
| 11 | `--toolchain` / active `rustc` identity | ✓ | Y | WG-CACHE real digest |
| 12 | Wall clock / `TZ` / `SOURCE_DATE_EPOCH` (reproducible-build stamp) | ✗ | Y | a `SOURCE_DATE_EPOCH`-sensitive build differs; uncaptured |
| 13 | Network / remote state (`cargo` registry, `git` remote, curl) | ✗ | Y | no network isolation; a check that hits the network is unmemoizable-soundly |
| 14 | stdin | ✗ | Y | inherited; a check reading stdin is uncaptured (rare) |
| 15 | Command-line ARGS beyond `command` | ✓ | n/a | the whole command is one shell string (captured) |
| 16 | Filesystem mtime/perms/uid the check stats | ✗ | Y | tree axis hashes content only, not mode/mtime |
| 17 | The stored axes themselves (`verify_hit` recompute) | ✓ | — | see §3 F-MK6: axes stored UNREDACTED, secret-shaped axis REJECTED not scrubbed — correct |

Captured: 3 of ~17 result-affecting input classes. The wedge's tree axis +
toolchain axis + env-allowlist cover the *intended* Rust-gate inputs; **every
other input a `sh -c` can read is uncaptured**, and the inheritance is unbounded.

## 3. Findings

All severities are SHIP-BLOCKER for the wedge's core promise ("your green checks
never re-run" presupposes the green was *sound*). TYPE = code unless noted.

### F-MK1 · SHIP-BLOCKER · cwd uncaptured (TYPE code) · ROOT: non-hermetic spawn
The spawned `sh -c` runs in the orchestrator's cwd; `--root` scopes only the
tree SNAPSHOT, not the spawn. A check using relative paths reads different files
per cwd, none in the key.
Repro: `--cmd 'grep -q pass flag.txt'`, `--root` pinned constant, run from cwd=A
(`flag.txt`=pass) then cwd=B (`flag.txt`=FAIL).
- run 1 (cwd A): `cache_hit:false exit:0` (cold MISS).
- run 2 (cwd B, real run would `exit:1`): `cache_hit:true exit:0 local_executions:0`
  — same `memo_key d4feed68…`. **STALE GREEN.**

### F-MK2 · SHIP-BLOCKER · unallowlisted env var uncaptured (TYPE code) · ROOT: allowlist = denylist-of-the-unknown
A check reading any var off the K-RUN allowlist is uncaptured. Repro:
`--cmd '[ "$GATE_MODE" = "ok" ]'`, `GATE_MODE=ok` then `GATE_MODE=BAD`.
- run 1: `cache_hit:false exit:0`. run 2 (real → `exit:1`): `cache_hit:true
  exit:0` same `memo_key 53c17bc3…`. **STALE GREEN.** (This is the SAME class
  Round-7 found for `RUSTFLAGS`; K-RUN closed only the listed names, not the
  class.)

### F-MK5 · SHIP-BLOCKER · `PATH` uncaptured (TYPE code) · ROOT: non-hermetic env
`PATH` resolves which tool binary runs — squarely result-affecting — yet absent
from the allowlist. Repro: shim `mytool` (exit 0 in `binok/`, exit 1 in
`binbad/`), `--cmd mytool`.
- run 1 (`PATH=binok:…`): `cache_hit:false exit:0`. run 2 (`PATH=binbad:…`, real
  → `exit:1`): `cache_hit:true exit:0` same `memo_key a15b4530…`. **STALE GREEN.**
A real CI runner mutates `PATH` constantly (rustup shims, vendored toolchains).

### F-MK3 · SHIP-BLOCKER · pruned dirs (`target/`,`.git/`,`.claude/`) uncaptured (TYPE code) · ROOT: tree-walk prune outruns the spawn's reach
`collect_files` unconditionally prunes those dirs even for an ad-hoc `**/*` glob,
but the spawned check can still read them. Repro: `--cmd 'grep -q pass
target/built.txt'`.
- run 1 (`target/built.txt`=pass): MISS `exit:0`. run 2 (=FAIL, real → `exit:1`):
  `cache_hit:true exit:0` same `memo_key 8f586340…`. **STALE GREEN.** (`target/`
  holds `build.rs` codegen + `OUT_DIR` artifacts a downstream check may assert on.)

### F-MK4 · SHIP-BLOCKER · file outside `--root` uncaptured (TYPE code) · ROOT: tree axis is a scoped subtree, spawn sees the whole FS
Repro: `--cmd 'grep -q pass /abs/outside.txt'`, `--root` elsewhere.
- run 1 (`outside.txt`=pass): MISS `exit:0`. run 2 (=FAIL, real → `exit:1`):
  `cache_hit:true exit:0` same `memo_key 37c79c29…`. **STALE GREEN.** Covers
  `$HOME/.cargo/config.toml`, `~/.gitconfig`, system headers, vendored deps.

### F-MK6 · NOT A DEFECT (verified) · axis-scrub law (TYPE code, confirms intent)
§3 question: can a stored axis be scrubbed? No — and that is correct. Axes are a
content-address `verify_hit` recomputes the key from (`ac.rs:365`), so they are
stored UNREDACTED. A secret-shaped axis is REJECTED, never scrubbed, at TWO
layers: the door `validate_axis` (`run.rs:911`, exit-2 `secret_in_identifier`)
and the write-boundary `guard_axes_not_secret` (`run.rs:767`, refuses the
persist). Both reuse the ONE shared `structural_secret_scrub` detector
(secret iff scrubbing changes the value). `verify_hit` runs on ALL three backends
(File/InMemory/HTTP — `ac.rs:198,831` + `parse_hit`). The guard is complete for
all three axes; no exemption. **This sub-question is closed clean.** The residual
is that a secret can still enter via `--cmd` (free text, scrubbed at rest on the
log, never an axis) — out of scope for memo soundness.

### F-MK7 · HONESTY · disclosed-residual framing understates the blast radius
`result_affecting_env_manifest`'s doc-comment (`run.rs:252`) calls the unlisted
env a "disclosed residual seam" exemplified by a custom `MY_GATE_MODE`. The
live repros show the residual includes `PATH`, cwd, the whole out-of-tree
filesystem, and pruned dirs — i.e. the COMMON case for any non-trivial check, not
an exotic edge. The honesty gap: the wedge is sound ONLY for the three built-in
Rust gates run from the workspace root with a stable toolchain and an unchanged
ambient env — a far narrower contract than "memoized CI" implies.

## 4. Root-cause analysis

One structural root, five symptoms (F-MK1–5): **the executor memoizes a
non-hermetic action.** The memo key is an attempt to ENUMERATE the inputs of a
process that is free to read ANYTHING — cwd, full env, whole filesystem, network,
stdin, clock. Enumeration of an open set is impossible to complete:

- The tree axis is a *scoped subtree* (`--root` + `glob_set` + dir-prunes) while
  the spawn sees the *whole filesystem from an arbitrary cwd* (F-MK1/3/4).
- The env axis is an *allowlist* — a denylist-of-the-unknown. Every var not on
  the list is silently DECLARED not-result-affecting (F-MK2/5). K-RUN added a few
  names after Round 7 found `RUSTFLAGS`; that treats symptoms, never the class.
  The allowlist will always lag the next var a check happens to read.

This is the recurring Class-3 pattern (Round 7 `env_manifest` empty → K-RUN
allowlist) repeating one abstraction level up: K-RUN converted "capture nothing"
into "capture a guessed subset", but a *guessed subset of an open input set* is
still unsound. You cannot make capture provably complete while the action is
non-hermetic, because the set of inputs is not knowable from outside the process.

## 5. Recommended structural remediation — HERMETIC EXECUTION, not a bigger allowlist

Provable-capture is unattainable for an arbitrary `sh -c` (the input set is open;
any allowlist is a denylist-of-the-unknown — F-MK2/5 prove the lag is real today).
The class-killing fix is to make the spawn HERMETIC so the captured axes ARE the
complete input set by construction — then the key is sound because nothing
outside it can reach the process.

THE ONE FIX (in `ProcessRunner::run` / `shell_command`, `run.rs:434/585`):

1. **Pin cwd** — `command.current_dir(<canonical --root>)` so the spawn's cwd ==
   the tree-axis root. Closes F-MK1. (And refuse a `--root` outside the
   snapshotted tree.)
2. **Clear + reconstruct env** — `command.env_clear()` then set ONLY the captured
   allowlist vars (the exact set folded into `env_manifest`) plus a pinned
   minimal `PATH` that is ITSELF an axis (hash the resolved `PATH` into the env
   manifest, or pin it to a content-addressed toolchain bindir). Now "captured ==
   present": any var the check reads either is in the key or is absent (so it
   reads empty, deterministically). Closes F-MK2/F-MK5/F-MK9. `PATH`/`HOME`/
   `LANG`/`TZ`/`SOURCE_DATE_EPOCH` become *pinned constants in the key*, not
   ambient leaks.
3. **Confine the filesystem** — the honest interim (no container locally): refuse
   to memoize-as-sound any check whose effective inputs are not provably under
   the tree root. Concretely, gate `--store`/HIT-trust on a hermetic flag; absent
   a sandbox, treat out-of-tree/pruned-dir reads as a DISCLOSED unsound mode
   rather than a silent HIT. The SOUND form (P2) is the runner-side sandbox
   (campaign #1 `corelink-runners`) where the action runs in an isolated rootfs
   seeded ONLY from the tree axis — then F-MK3/F-MK4/F-MK13 close by construction
   because the process physically cannot read outside the seeded inputs. This is
   the seam already named in `docs/interop.md` (runner isolation); the CLI's local
   executor should mirror its contract, not exceed its soundness.

Net: the key stops trying to ENUMERATE an open input set and instead BOUNDS the
input set to exactly the captured axes. Hermetic execution converts the
allowlist from a denylist-of-the-unknown into a complete spec. This is a real
code change to `ProcessRunner`, not a doc edit; steps 1+2 are local-only and
close F-MK1/2/5/9 today, step 3 is the P2 runner seam for F-MK3/4/13.

Smaller honest interim if (1)+(2) are deferred: stamp a `hermetic:false` /
`sound_for:["builtin-rust-gate-from-root"]` field on every recorded HIT and
DISCLOSE that ad-hoc `--cmd` memoization is best-effort, so the wedge never
*claims* a soundness it does not hold. But that is a disclosure, not a fix — the
fix is hermetic spawn.

## 6. Residual / accepted

- **Network / remote state (F-MK13)** and **clock/`SOURCE_DATE_EPOCH` (F-MK12)**:
  not closable by env-clearing alone; require the P2 sandbox (no-network rootfs)
  for soundness. Until then, a network-touching check is disclosed-unsound.
- **stdin (F-MK14)** and **mode/mtime/uid (F-MK16)**: rare for a gate; close them
  by closing stdin in the hermetic spawn and (if needed) folding stat metadata
  into the tree axis. Low priority; disclosed.
- **`.gitignore` (F-MK6)**: captured-by-accident (path-match, no ignore filter);
  becomes correct-by-construction under a seeded hermetic rootfs.
- **Axis-scrub (F-MK6/§3)**: closed clean — secret-shaped axes are rejected at
  door + write-boundary on all three backends; no action.
- **Local-trust boundary**: `CachedEntry::self_hash` + `verify_hit` detect LOCAL
  `.ac` tampering only; cross-tenant authenticity is the P2 HMAC AC (Seam A,
  already disclosed). Unchanged by this audit.
