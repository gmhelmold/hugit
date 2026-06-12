# CLASS 3 — MEMO-KEY — Round 9 re-audit

Round 9 · convergence re-audit of CLASS 3 (memo-key / wedge soundness) ·
auditor: fresh-context adversarial auditor · branch `integ/wave-l`, HEAD
`95353f9` (Wave L; the L-C hermetic-exec fix is `104e7dd`). CLI built from
`integ/wave-l` with the pinned 1.96.0 toolchain; every finding below
cold-verified by LIVE reproduction in a temp dir OUTSIDE the repo.

## 1. Scope & method

Wave L's **L-C** (`104e7dd`) made the check spawn HERMETIC in `ProcessRunner::run`
(`crates/hugit-cli/src/checks/run.rs`): on the spawn it (1) pins
`command.current_dir(<canonical --root>)`, (2) `command.env_clear()` then sets
ONLY the captured allowlist (`captured_hermetic_env`, run.rs:195), and (3) hashes
the resolved `PATH` value into the `env_manifest` axis (`env_manifest_axis`,
run.rs:210). The captured set is the SAME set folded into `def.env_manifest` →
`compute_def_digest` → the memo key (`crates/hugit-checks/src/client/memo_key.rs`),
so the design intent is **captured-axes == actual-spawn-env**.

Method: re-ran the Round-8 stale-green matrix (cwd / unlisted env / PATH); for
each I produce a cold MISS, then change ONLY the candidate so a real run flips
`pass→fail`, and assert the second run is a MISS (re-execute) not a HIT serving
the stale green. Then I attacked the hermetic boundary for any result-affecting
input STILL uncaptured WITHIN local scope (NOT the disclosed FS/network/clock P2
residual): stdin, an allowlisted-prefix var's fold, the actual spawn-env vs the
captured set, `--root` canonicalization (symlink/relative). Backend axis-scrub
law and P2-residual honesty checked last.

## 2. Stale-green matrix re-verification (cwd / env / PATH: HIT→change→MISS?)

All three prior Round-8 ✗ holes are CLOSED. Live repro:

- **cwd (F-MK1) — CLOSED.** cwd is pinned to the canonical `--root`. With
  `flag.txt` in `--root` and a relative-path check run from `cwdA` then `cwdB`
  (whose own `flag.txt`=FAIL): run2 is a HIT `exit:0` — and that is CORRECT,
  because the pinned cwd resolves `flag.txt` in `--root` (=pass) regardless of
  ambient cwd. A check that depended on ambient cwd no longer can: the result is
  invariant to cwd. Not a stale green.
- **unlisted env var (F-MK2) — CLOSED.** `GATE_MODE` is off the allowlist, so
  `env_clear()` removes it: the check reads it as EMPTY deterministically.
  `[ "$GATE_MODE" = "ok" ]` → `exit:1` on BOTH `GATE_MODE=ok` and `GATE_MODE=BAD`
  (consistent fail, never a stale green). Confirmed `SECRET_LEAK=zzz` is ABSENT
  from the live-dumped spawn env.
- **PATH (F-MK5) — CLOSED.** PATH value is hashed into the env axis. `mytool` =
  exit0 in `binok/`, exit1 in `binbad/`: run1 `PATH=binok:…` key `9f657686…`
  MISS exit0; run2 `PATH=binbad:…` key `21a919cb…` (DIFFERENT) cold MISS exit1.
  A PATH change busts the key → re-execute → real failure.

## 3. Break attempts on the hermetic boundary (capture==spawn-env divergence)

- **Live spawn-env dump** (`env | sort` inside the check): the post-`env_clear`
  spawn carried EXACTLY the captured allowlist (CARGO_X, HOME, LANG, LC_ALL,
  PATH, RUSTFLAGS) plus the shell-internal `PWD`/`SHLVL`/`_`. `SECRET_LEAK` was
  cleared. `PWD` is set by `sh` to the PINNED cwd (verified `/…/root` regardless
  of ambient cwd), `SHLVL`=1 always, `_` is `sh`'s own arg0 — all DETERMINISTIC
  shell-generated constants, not ambient leaks, so they are sound to omit from
  the key. Capture==present holds for the environment. **No env divergence.**
- **Allowlisted-prefix fold**: `LC_NUMERIC` (matches the `LC_` prefix) is both
  set on the spawn AND folded; `LC_NUMERIC=ok`→`BAD` busts the key (key
  `17af2359…`→`cc454360…`, run2 cold MISS exit1). Captured == present. Sound.
- **`--root` canonicalization**: both the tree snapshot AND the cwd-pin use the
  same `canonical_root` (run.rs:1081/1084). A symlink `--root` and a relative
  `--root` each resolve consistently (exit0 hit). No snapshot↔cwd divergence.
- **stdin — DIVERGENCE FOUND (see §4 F-MK14-L).** `env_clear` and the cwd-pin do
  NOT touch the child's stdin; the spawn sets only `.stdout(piped).stderr(piped)`
  (run.rs:530) and inherits the parent's stdin. A check that reads stdin gets an
  UNCAPTURED, UNCLEARED ambient input → stale green.

## 4. Residual findings (sev · repro · TYPE)

### LOCAL-SCOPE HOLE (convergence-breaking)

**F-MK14-L · SHIP-BLOCKER (wedge soundness) · stdin uncaptured AND uncleared ·
TYPE code · ROOT: hermetic spawn forgot the third std stream.**
The spawn pins cwd and clears/reconstructs env, but leaves stdin INHERITED.
stdin is neither in the memo key nor cleared, so a stdin-reading check yields a
stale green. Live repro (temp dir outside repo, `--root` + def pinned constant):
- `--cmd 'read x; [ "$x" = "pass" ]'`
- run1 `stdin=pass`: `cache_hit:false exit:0` key `33c9066d…` (cold MISS).
- run2 `stdin=FAIL` (a real run → `exit:1`): `cache_hit:true exit:0` SAME key
  `33c9066d…`. **STALE GREEN.**
This is LOCAL scope, NOT the disclosed P2 FS/network/clock seam: the Round-8
report itself (§6 F-MK14) prescribed the local fix — "close stdin in the
hermetic spawn" — a one-line `.stdin(Stdio::null())`, no sandbox required. The
L-C commit (`104e7dd`) claims "stale-green class closed for the local scope" and
discloses ONLY "Whole-FS/network/clock" as residual; stdin is a local-scope
stale-green that is neither closed nor disclosed → the claim is overstated by
exactly stdin. (Real result-affecting case: any gate that pipes a manifest /
diff / fixture into the command on stdin, e.g. `… | cargo …`, or `jq`/`grep`
over piped review content.)

### DISCLOSED P2 SEAM (honestly scoped — NOT a regression)

- **Out-of-`--root` FS read (F-MK4) / pruned dirs (F-MK3) / network / clock:**
  out-of-root `grep -q pass /abs/outside.txt` still serves a HIT after the file
  flips to FAIL — reproduced live, AS EXPECTED. The run.rs:137–141 comment and
  the L-C commit body BOTH disclose this as the P2 runner-sandbox seam (isolated
  rootfs seeded only from the tree axis). Honestly scoped; no soundness overclaim
  for the FS/network/clock dimension.

### CLOSED CLEAN (verified, no action)

- **Axis-scrub law (F-MK6):** a secret-shaped `--toolchain` axis is REJECTED at
  the door (`secret_in_identifier`, exit 2 — live-verified), and the
  write-boundary `guard_axes_not_secret` (run.rs:857) REFUSES the persist; axes
  are stored UNREDACTED as content-addresses. `verify_hit` (ac.rs:378) recomputes
  the key and returns `Err` (→ MISS) on any mismatch — it never scrubs. It runs
  on ALL three backends: InMemory (ac.rs:211), HTTP via `parse_hit`
  (ac.rs:400/493), FileAc (run.rs:921). Axis-scrub would break the cache; the
  REJECT law is correct and complete. Closed clean.

## 5. CONVERGENCE VERDICT

**NOT CONVERGED.** The cwd / unlisted-env / PATH holes are genuinely closed and
the hermetic env is sound (capture==present verified by a live spawn-env dump).
But a LOCAL-scope stale-green still reproduces: **stdin is inherited, neither
captured nor cleared (F-MK14-L)** — and it is NOT the disclosed FS/network/clock
P2 residual; its fix is local (`.stdin(Stdio::null())` on the spawn), and the
Round-8 audit already named it. The L-C claim "stale-green class closed for the
local scope" is therefore overstated by exactly stdin. Convergence requires
either closing stdin in the hermetic spawn OR adding stdin to the explicitly
disclosed local residual (the former is the right call — it is a one-line, no-
sandbox fix). Everything else in the local scope is converged.
