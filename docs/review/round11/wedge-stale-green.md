# Round 11 — Memoized-CI wedge stale-green audit

**Auditor:** fresh-context hostile auditor (read-only on code).
**Target:** hugit engine, `main` HEAD `5b73a8b` (Wave O, WO-GLOBMISS).
**Single focus:** is the memoized-CI wedge stale-green class FINALLY dry (local scope)?
**Invariant under test:** *a warm HIT must NEVER serve a result a cold run wouldn't.*

## VERDICT: **NOT DRY** — a 4th local stale-green found, with live repro.

A built-in `clippy`/`test`/`fmt` gate reads a `.cargo/config.toml` (clippy/test)
or `rustfmt.toml` (fmt) from any **ancestor directory of `--root`**, but the
per-def tree-axis glob only matches those files **inside** `--root`
(`**/.cargo/config.toml`, `**/rustfmt.toml`). A parent-dir config that flips the
gate outcome leaves the memo key unchanged → a warm HIT serves a stale GREEN
where a cold run now FAILS. This is the **same class** as WO-GLOBMISS (a bounded,
known, result-affecting toolchain-config input the tree axis omits), not the
disclosed unbounded P2 fixture/outside-root seam — see "Classification" below.

---

## Scope & method

- Built `hugit-cli` once with the pinned 1.96.0 toolchain; reproduced with the
  binary only (no `cargo test --workspace`). Temp dirs under `/tmp/r11*`,
  outside the repo, removed after.
- Memo mechanics confirmed: cold run = `cache_hit:false`; an identical warm run
  over the same `--ac` = `cache_hit:true, local_executions:0`. A stale green is a
  `cache_hit:true, exit:0` served after the underlying input changed such that a
  fresh COLD run gives a different (failing) exit under the SAME `memo_key`.

## Axis inventory (captured ✓ / gap ✗)

| Axis | Captured? | Note |
|---|---|---|
| tree content (glob-scoped) | ✓ | `scoped_tree_root`, LP-framed, sorted |
| file MODE (N-1) | ✓ | folded via `frame_file_with_mode`; re-confirmed below |
| in-`--root` toolchain config (WO-GLOBMISS) | ✓ | rustfmt/clippy/.cargo/rust-toolchain globs; re-confirmed below |
| def body / command / inputs / glob_set | ✓ | `compute_def_digest` |
| env (allowlist + hashed PATH) | ✓ | `env_manifest` axis |
| cwd | ✓ | pinned to `--root` |
| stdin | ✓ | nulled |
| toolchain digest | ✓ (caveat) | `sha256(rustc --version --verbose)` — identity by version string, not binary bytes (narrow residual, below) |
| **ancestor-dir toolchain config (above `--root`)** | **✗ GAP** | cargo/rustfmt walk upward past `--root`; glob is `--root`-relative → **4th stale-green** |
| outside-root FS / unbounded fixtures / network / clock | ✗ (disclosed P2) | out of local scope by mandate |

## Re-confirmation of the two prior closes

**N-1 (file mode) — CLOSED.** Ad-hoc check `sh ./gate.sh` over `gate.sh`:
- warm re-run, unchanged → `cache_hit:true, exit:0, local_executions:0`.
- `chmod -x gate.sh` (same content) → key flips
  `1ce0737a…` → `0b649998…`, `cache_hit:false` (MISS). Mode change busts the key.

**WO-GLOBMISS (in-root config glob) — CLOSED.** `--def fmt` over a minimal crate:
- cold → key `af7c377b…`.
- add `rustfmt.toml` in `--root` → key `a17f0a95…`, MISS. In-root config change
  busts the key.

Both prior closes genuinely hold.

## The 4th stale-green — live repro (ancestor `.cargo/config.toml`)

Layout: `--root = /tmp/r11c/parent/proj`; the config lives one level UP at
`/tmp/r11c/parent/.cargo/config.toml` (an ancestor of `--root`, not inside it).
Source `src/main.rs` has an unused variable, so `-D warnings` denies it unless a
`rustflags` cap allows it.

1. **Cold, parent config caps lints** (`rustflags=["--cap-lints=allow"]`):
   `cache_hit:false, exit:0` (GREEN), key `f78881ae…`.
2. **Mutate the parent config** to `rustflags=[]` (no cap). A fresh COLD run
   (clean `--ac`) now → `cache_hit:false, exit:101` (FAIL) — **same key
   `f78881ae…`**. The key did not move though the real outcome did.
3. **Warm re-run** over the original `--ac` after the mutation → `cache_hit:true,
   exit:0` (GREEN), key `f78881ae…`. **STALE GREEN served.**

Cargo-level confirmation (no hugit in the loop): from `proj`,
`cargo clippy … -- -D warnings` **passes** with the parent cap present and
**fails** ("could not compile … due to 1 previous error") with it removed — so
the ancestor config is unambiguously result-affecting and outside `--root`.

The `fmt` variant is the same class: rustfmt honors a `rustfmt.toml` in an
ancestor of the formatted tree (verified `cargo fmt --all --check` changes its
wrapping rules with/without a parent `max_width=1` `rustfmt.toml`), and the
`**/rustfmt.toml` glob is likewise `--root`-relative.

### Severity: P0 (correctness — stale green = the wedge's core invariant)
A green CI verdict can be memoized and re-served after the build configuration
that produced it changed to a failing one — exactly the failure WO-GLOBMISS
claimed to close, on the ancestor-path axis the glob fix did not reach.

### Classification: local glob-scope gap, NOT the disclosed P2 seam
The WO-GLOBMISS doc scopes the residual P2 seam to the **unbounded** case (a
`test` reading an arbitrary fixture / `include_*!` / `build.rs`-emitted path) and
to runner-rootfs whole-FS confinement. This finding is neither: cargo's and
rustfmt's **ancestor config search is a well-known, bounded, deterministic** part
of the toolchain contract — the very property that justified capturing
`.cargo/config.toml`/`rustfmt.toml` in the first place. WO-GLOBMISS captured them
but only `--root`-relative, missing the ancestor walk. It is the SAME bounded
class, so it is closable locally.

### Suggested fix shape (for the lead — not applied; read-only audit)
Capture the bounded ancestor chain as additional tree-axis inputs: walk from
`--root` up to the filesystem root (or the cargo "workspace/home" stop) and fold
any `.cargo/config.toml` / `.cargo/config` (clippy/test) and `rustfmt.toml` /
`.rustfmt.toml` (fmt) found above `--root` into the key under a stable
ancestor-relative path label. (`rust-toolchain[.toml]` searches upward too and
shares the gap.) `CARGO_HOME`'s `config.toml` is a further ancestor-of-sorts;
`CARGO_HOME`/`RUSTUP_HOME`/`HOME` are env-captured, but the *contents* of
`$CARGO_HOME/config.toml` are not — same gap, broader root. Bound the walk so the
hit-rate is preserved.

## Other axes attacked — held / residual

- **Edition in `Cargo.toml`** — captured (`**/Cargo.toml` matches root + nested;
  `**/` matches the empty prefix, verified in `glob.rs::double_star_crosses_segments`).
  A `[workspace.lints]`/`[lints]` in the root `Cargo.toml` is captured for the
  same reason. HELD.
- **Toolchain identity** — `default_toolchain_digest` hashes
  `rustc --version --verbose` *output*, not the binary bytes. Two distinct rustc
  builds with byte-identical verbose output (release+commit-hash+date+host+LLVM)
  would collide → a theoretical false HIT. Practically distinct (the commit hash
  differs); recorded as a **narrow residual**, no live repro. A RUSTUP override
  file (`rust-toolchain[.toml]`) inside `--root` IS globbed (clippy/test); an
  ancestor one shares the 4th-hole gap above.
- **Directory-mode fold** — only FILE modes are folded; a parent dir `chmod 000`
  makes `read_dir` fail → that subtree is silently dropped from the snapshot,
  which CHANGES the tree_root (a MISS) — fail-safe, not a stale green. No repro.
- **Byte-identical trees hashing differently / different trees colliding** —
  framing is canonical (count-prefix + LP path + LP framed-content, sorted);
  no collision or false-miss found. HELD.

---

## Bottom line
Three prior holes (env / mode / in-root config-glob) are genuinely closed. A
**fourth** remains in the SAME bounded toolchain-config class: configuration the
gate reads from **above** `--root` is invisible to the `--root`-relative globs, so
a parent-dir `.cargo/config.toml` / `rustfmt.toml` change is a stale green. Live
P0 repro above. The local scope is **NOT** closed.
