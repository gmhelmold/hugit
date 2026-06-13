# Wedge stale-green re-audit — sibling tree/metadata axes after the N-1 mode-bit fix

**Auditor stance:** hostile, default-REFUSE. Single focus: the memoized-CI wedge
and its STALE-GREEN class. Prior P0 (N / N-1): the tree axis dropped the POSIX
mode bit, so a `chmod -x` served a cached green. That is fixed. This sweep asks:
is the class FULLY closed, or is a SIBLING file-metadata / tree axis still
uncaptured?

**VERDICT: NOT CLOSED.** A sibling stale-green reproduces LIVE on `main`
(HEAD `e50603d`, branch `integ/wave-o`): the **glob-miss** axis. The built-in
check defs (`fmt`/`clippy`/`test`) scope their tree axis to
`["**/*.rs", "**/Cargo.toml", "Cargo.lock"]`, but the built-in gate commands
read result-affecting files OUTSIDE that set (`rustfmt.toml`, `clippy.toml`,
`.cargo/config.toml`, `rust-toolchain.toml`, …). Such a file changes the gate's
real outcome while leaving the memo key unchanged → a warm HIT serves a green
where a cold run now FAILS.

---

## 1 Scope & method

- READ-ONLY on code. Build/repro per brief
  (`cargo build -p hugit-cli --locked`, toolchain 1.96.0). All temp trees under
  `/tmp/`, outside the repo.
- Code read: the full memo-key derivation and tree snapshot —
  `crates/hugit-cli/src/checks/run.rs` (snapshot_tree / collect_files / file_mode
  / builtin_glob_set / resolve_def / ProcessRunner), and
  `crates/hugit-checks/src/client/memo_key.rs`
  (frame_file_with_mode / scoped_tree_root / compute_def_digest / derive_memo_key)
  + `client/glob.rs` (matches_any).
- The invariant under test:
  `H(tree_root ‖ def_digest ‖ toolchain ‖ env ‖ …)` must capture EVERY
  result-affecting input; a warm HIT must NEVER serve a green a cold run would
  not. For each candidate axis: either a LIVE stale-green repro (cold green/key
  K → mutate the axis so a real run differs → warm still serves K's green) OR a
  proof it is captured / cannot affect a local-scope result.

The tree axis is built per file as
`frame_file_with_mode(mode, content)` = `MODE_TAG ‖ LP(u32_le mode&0o7777) ‖ LP(content)`,
inserted into a `BTreeMap<rel_path, framed>`; `scoped_tree_root` hashes
`u32_be(count) ‖ (LP(path) ‖ LP(framed))…` over the glob-MATCHED, sorted subset.
`collect_files` uses `path.is_dir()`, `std::fs::read(&path)`, and
`std::fs::metadata(path)` — all of which FOLLOW symlinks — and prunes
`target` / `.git` / `.claude`.

## 2 The tree / metadata axis inventory

| # | Axis | Captured? | Can affect result? | Status |
|---|------|-----------|--------------------|--------|
| 1 | File CONTENT (matched path) | ✓ `LP(content)` | yes | captured |
| 2 | POSIX MODE — exec bit | ✓ `mode & 0o7777` (N-1) | yes | captured (P0 fix verified) |
| 2b| POSIX MODE — setuid/setgid/sticky | ✓ `0o7777` folds all three | yes (rare) | captured |
| 3 | Symlink target swap, target IN-glob | ✓ read follows → content changes | yes | captured |
| 3b| Symlink-DIR → outside-root, content swap | ✓ walk follows, records under in-root rel path | yes | captured (stronger than P2 residual) |
| 3c| file → dangling symlink | ✓ `read` fails → entry dropped → key changes | yes | captured |
| 4 | File-type: file ↔ dir at matched path | ✓ entry drops / children appear | yes | captured |
| 5 | Empty-file PRESENT vs ABSENT | ✓ count + framed-entry differ | yes | captured |
| 6 | mtime-only touch (no content/mode change) | n/a (not folded) | NO | correct (not result-affecting) → HIT |
| 7 | Ordering / determinism | ✓ `BTreeMap` sort + LP framing | — | canonical: no false miss, no boundary collision |
| **8** | **GLOB-MISS: a result-affecting file the gate reads that the glob_set does NOT match** | **✗** | **yes** | **HOLE — stale green (see §3, §4)** |
| 9 | Files outside `--root` read by absolute path (no in-root symlink), network, clock | ✗ (DISCLOSED P2) | yes | out of local scope — not a regression |

## 3 Attack repros

All on HEAD `e50603d`. Cold = key absent; warm = byte-identical re-derivation.

**(1) Symlinks — CAPTURED.** `linked.txt → real_green.txt` (`grep -q GREEN
linked.txt`), cold exit 0 key `06d5c0fa…`. Re-point to `real_red.txt`; warm key
`f8bc6f84…`, exit 1 — MISS (`std::fs::read` follows the link, so the framed
content changed). Symlink-DIR to an OUTSIDE-root dir (`ext → $OUT`, read
`ext/payload.rs`): mutating the outside target's content also busted the key
(`eed4516b…` → `0ed0d52f…`) — the walk follows the dir link and records the
target under the in-root rel path. file → dangling symlink: read fails, entry
drops, key changes. Symlinks are fully captured.

**(2) GLOB-MISS — STALE GREEN (LIVE).** Built-in `--def fmt`
(glob `**/*.rs|**/Cargo.toml|Cargo.lock`) over a minimal crate, no `rustfmt.toml`:

- COLD A: `exit 0`, `cache_hit:false`, key `ce53cef787a4`, stored.
- Add `rustfmt.toml` (`max_width = 1`) → the file is now unformatted under the
  new rule. GROUND TRUTH: a direct `cargo fmt --all --check` returns `exit 1`.
- WARM B: same `--def fmt` → **`cache_hit:true`, `exit 0`, key `ce53cef787a4`** —
  a green served where the real gate FAILS.

`rustfmt.toml` is a genuine, first-class input to `cargo fmt --check`, but it
matches none of the three built-in glob patterns, so it is invisible to the tree
axis. The same hole confirmed for `.cargo/config.toml` under `--def clippy`:
adding `.cargo/config.toml` (`[build] rustflags = ["-Dwarnings"]`) left the key
unchanged (`f345f38e…` → `f345f38e…`, HIT) — `.cargo/config.toml` is not pruned
but is not glob-matched, so it is uncaptured for clippy/test too. (The clippy
repro's cached verdict happened to be a fail in the throwaway crate, but the
key-collision is identical to the fmt green-then-red case.)

**(3) File-type swap — CAPTURED.** `thing` is a file (`test -f thing`) cold exit
0 key `38fee2c0…`; `rm thing && mkdir thing` → warm exit 1 key `a37beaa6…`
(MISS: the file entry dropped, the dir's children appeared).

**(4) Empty-vs-absent — CAPTURED.** `marker` empty file present (key
`44bedf39…`, exit 0) vs removed (key `908638ea…`, exit 1) — MISS.

**(5) Mode completeness — CAPTURED.** `file_mode` folds `meta.mode() & 0o7777`
— the FULL perm set plus setuid/setgid/sticky, not just exec. P0 regression
re-checked: `chmod +x gate.sh` (key `505aab8f…`, exit 0) → `chmod -x` (key
`9b459c96…`, exit 126) — MISS. The N-1 fix is intact and complete on the
sub-bits.

**(6) Ordering/determinism — CAPTURED.** Identical tree → identical key → HIT
(`793182653ff9` twice across processes). mtime-only `touch` → still HIT (mtime
is correctly NOT result-affecting). `scoped_tree_root` sorts via `BTreeMap` and
length-prefixes every field, so two identical trees never false-miss and two
different trees never collide on a path/content boundary.

## 4 Findings

| ID | Sev | TYPE | Repro |
|----|-----|------|-------|
| **WO-GLOBMISS** | **HIGH (ship-blocker, same class as the N-1 P0)** | CODE — incomplete tree axis | `--def fmt`; cold green; add `rustfmt.toml(max_width=1)`; warm HIT serves `exit 0` while `cargo fmt --check` is `exit 1`. (§3.2) |

**Root cause.** `builtin_glob_set()` =
`["**/*.rs", "**/Cargo.toml", "Cargo.lock"]` is narrower than the real input set
of the built-in gate commands. `cargo fmt/clippy/test` are influenced by, at
least: `rustfmt.toml` / `.rustfmt.toml`, `clippy.toml` / `.clippy.toml`,
`.cargo/config.toml` (and `.cargo/config`), `rust-toolchain` /
`rust-toolchain.toml`, `.cargo/audit.toml`, and any file pulled in by
`include_str!` / `include_bytes!` / `build.rs`-emitted paths that is not a
`.rs`/`Cargo.toml`. Each is a result-affecting input that the tree axis does not
see → a warm HIT can serve a stale green when one of them changes the gate's
verdict. This is the SAME failure SHAPE as the N-1 mode P0 (a real,
result-affecting per-file input the tree axis omitted), on a sibling axis the
mode fix did not touch.

**Why it is a true stale-green, not a benign scope choice.** The ad-hoc path is
honest: it globs `**/*`, so any in-root edit is a MISS (verified: editing
`config.txt` under an ad-hoc def busted the key). The BUILT-IN path is the one
that ships the wedge's headline value (`fmt`/`clippy`/`test`), and its narrow
glob is presented as "scopes the tree axis so an edit inside the source tree is
a MISS" (run.rs doc on `builtin_glob_set`) — but it silently drops the gate's
own config inputs, which are exactly the files a developer edits to change a
gate's outcome.

**Scope of the disclosed P2 residual is unchanged and not in play here.** The
hole is a file INSIDE `--root` that the gate reads — not the outside-root /
network / clock seam. The symlink-to-outside-root case (§3.1) actually pulls
outside content back INTO the captured set, so it is captured; the P2 residual
remains only for direct absolute-path reads with no in-root link.

**Suggested remediation direction (not applied — read-only sweep):** either
(a) broaden the built-in glob to include the toolchain/lint/cargo config files
(`**/rustfmt.toml`, `**/.rustfmt.toml`, `**/clippy.toml`, `**/.clippy.toml`,
`**/.cargo/config.toml`, `**/.cargo/config`, `rust-toolchain`,
`rust-toolchain.toml`), or (b) make the built-in defs hermetic the same way env
was made hermetic — neutralize ambient config the gate would read that is not in
the captured tree (e.g. pin/clear `CARGO_HOME`-relative config discovery). (a)
is the minimal close that matches the existing tree-axis design; note glob
breadth must be kept honest against the `target`/`.git`/`.claude` prune list and
the state-file exclusions. A per-built-in input MATRIX (mirroring the per-verb
secret matrix) would be the durable guard against the NEXT missed config file.

## 5 VERDICT

**NOT CLOSED.** Mode (incl. setuid/setgid/sticky), symlinks (in-glob,
outside-root-dir, dangling), file-type swaps, empty-vs-absent, and
ordering/determinism are all captured — the N-1 mode fix and the surrounding
tree axis are sound on those. But a SIBLING axis the mode fix did not cover —
the **glob-miss** between the built-in `glob_set` and the built-in gates' real
input set (`rustfmt.toml` et al.) — reproduces a live stale green: a warm HIT
serving `exit 0` where a cold `cargo fmt --check` is `exit 1`. Same class as the
P0 that motivated this sweep. The stale-green class is NOT fully closed.

Strongest stale-green built: `--def fmt`, cold green, add
`rustfmt.toml(max_width=1)`, warm HIT still `exit:0` while ground-truth
`cargo fmt --all --check` = `exit 1` (§3.2).
