# Hostile security & integrity sweep — 2026-06-12

**Target:** `main` HEAD `5457730` (Wave M, post-Round-10, claimed 4/4 converged).
**Auditor stance:** default-REFUSE, live-repro or precise-cite every finding.
**Build:** `cargo build -p hugit-cli --locked` (rustc 1.96.0) — clean, 27.8s.
**Scope:** the six standing invariants — (1) no verbatim secret in the
hash-chained log/stores; (2) every read verifies the chain before projecting;
(3) the memoized-CI wedge never serves a stale green; (4) every mutation passes
D14 authz; (5) state-machine integrity; (6) error-law. Focus weighted to the
newest, least-attacked Wave L/M surfaces (the unified `secret_shape.rs` scrubber,
the PS-13 `rehydrate_and_verify` chokepoint, the C5-F3 reconcile).

---

## VERDICT: one DO-NOT-SHIP (P1 stale-green code-defect). Spine otherwise held.

The integrity spine (chain-verify on read, redaction, error-law, state machine)
held under every attack tried. A NEW hole was found in invariant (3): the
memoized-CI tree axis hashes file **content only** and drops the POSIX
**executable/mode bit**, which is a result-affecting input the snapshot itself
reads — so a `chmod` flip with byte-identical content serves a STALE GREEN.

---

## Findings

### F-1 — Memoized-CI tree axis ignores the file mode bit ⇒ STALE GREEN (P1)

- **Severity:** P1.
- **TYPE:** code-defect.
- **Invariant broken:** (3) the wedge never serves a stale green / the memo key
  captures every result-affecting input.
- **Repro (live, cold):**
  ```
  D=/tmp/x; mkdir -p $D; cd $D
  hugit intent new --charter x --campaign c --log wedge.json --store .hugit/intents.json
  printf '#!/bin/sh\nexit 0\n' > gate.sh ; chmod +x gate.sh
  hugit check --def g --cmd './gate.sh' --root . --log wedge.json --store
      # -> exit 0, cache_hit:false   (GREEN, miss)
  chmod -x gate.sh                      # content byte-identical; only the mode bit flips
  hugit check --def g --cmd './gate.sh' --root . --log wedge.json --store
      # -> exit 0, cache_hit:TRUE    (STALE GREEN)
  ./gate.sh ; echo $?                   # ground truth: 126 (permission denied) — real gate is RED
  ```
  The warm run serves `cache_hit:true, exit:0` while a real re-run of the same
  command now fails (`exit 126`). Reverse polarity (`chmod -x` → `chmod +x`)
  also reproduces a stale RED→GREEN inversion. Same memo key
  (`e133d49286832b2e…`) across both runs because content is identical.
- **Root cause (code-cite):**
  - `crates/hugit-checks/src/client/memo_key.rs:32` — `pub type FileContent =
    Vec<u8>` (raw content bytes only).
  - `crates/hugit-cli/src/checks/run.rs:443` — the snapshot is built with
    `std::fs::read(&path)` (content), no `std::os::unix::fs::PermissionsExt`
    mode capture (`PermissionsExt` is used elsewhere — `hugit-checks/src/client/
    ac.rs:647` for the PAT 0600 check — so the dependency is available and the
    omission is by oversight, not platform limitation).
  - `scoped_tree_root` (`memo_key.rs:52`) frames only `(rel_path, content)` into
    the tree Merkle root → the mode bit never reaches `compute_memo_key`.
- **Why this is NOT the disclosed P2 FS/network/clock seam:** the snapshot
  *reads these exact files* to build the tree axis; the mode bit is a local,
  in-tree, fully-hermetic attribute the wedge already touches. It is squarely a
  capture gap in the local memo key, not an out-of-band infra input. No doc or
  test discloses the mode bit as an uncaptured axis (grep of `crates/` + `docs/`
  for `exec bit`/`mode bit`/`permission axis` finds only unrelated hits and the
  runner-hydrate contract, which itself reconstructs "symlinks + mode bits" —
  confirming mode is understood to be result-affecting elsewhere in the family).
- **Blast radius:** any check whose result depends on a mode bit — `[ -x f ]`,
  `./script` (vs `sh script`), a test harness that execs a fixture, a gate that
  shells a tracked tool — can land a PR on a stale green after a permission
  change with no content edit. The default built-in gates (`cargo fmt/clippy/
  test`) do not depend on a tracked-file mode bit, which lowers (not removes)
  real-world exposure; the ad-hoc `--cmd` path (the documented escape hatch) is
  fully exposed.
- **Fix shape (not applied — read-only audit):** fold the per-file mode (at
  least the `0o111` exec bits, ideally the full `0o777` masked mode) into
  `FileContent`/the tree-axis framing so a `chmod` is a MISS. Add a regression
  test that flips the exec bit and asserts a cache MISS.

---

## What I attacked and FAILED to break (negative results = confidence)

- **Chain-verify on read (inv. 2):** tampered a payload byte in a valid log;
  `hugit campaign show` returns `chain_broken` at **bare exit 2** (verified
  without a pipe). The PS-13 single chokepoint (`checks::rehydrate_and_verify`,
  `checks/mod.rs:411`) is the sole `push_record + verify_chain` site and every
  read verb (campaign/pr/intent/why/export/list + the C5-F3 reconcile via
  `new.rs:458`) routes through it. Could not find a read path that projects
  without verifying.
- **Reconcile seeding from a crafted log (inv. 2/5):** `reconcile_store_from_log`
  → `load_log_verified` (`new.rs:434`) routes through the same chokepoint and
  only ADDS log-present intents (no store-ahead phantom). A tampered reconcile
  log fails closed `chain_broken` before any store write.
- **Redaction — free-text engine (inv. 1):** planted `ghp_`, `SG.<key>`,
  `AKIAIOSFODNN7EXAMPLE`, `rk_live_…`, and a bare-prefix AWS-secret
  (`wJalrXUtnFEMIK…`) across charter + acceptance. ALL redacted to `[REDACTED]`
  in the forever-log; no verbatim survival. The deny-by-default identifier door
  caught a Google `AIzaSy…` key (entropy 5.0 > 4.5) and an `SG.`-with-dots
  campaign via `secret_in_identifier` (exit non-zero, refused at input).
- **`cas:` / digest value-gate (inv. 1):** the K-SCRUB value-gate holds — a
  credential behind `cas:`/`<algo>:` or smuggled into a digest-named field is not
  digest-shaped and scrubs (confirmed by code + the unified
  `secret_shape::is_content_address_ref`). The only residual is the *disclosed*
  low-entropy base32 physics case (a non-secret-by-definition value), already
  accepted.
- **DoS / malformed input (inv. 6):** a 50 KB deeply-nested (`[`×50000) payload
  and a 5 MB payload string both rejected with a structured `parse` error — no
  panic, no stack overflow, no hang. The tree walk is depth-capped
  (`MAX_WALK_DEPTH=64`) and cycle-safe (canonical visited-set), and the check
  spawn is timeout-bounded with concurrently-drained pipes (pipe-cap), nulled
  stdin, and an own process group — the previously-closed hang/flood vectors
  stay closed.
- **Stale-green env/cwd/PATH/stdin axes (inv. 3):** the Round-8/9 hermetic spawn
  (`env_clear` + allowlist, cwd pinned to `--root`, PATH value hashed into the
  axis, stdin NULL) is sound for the env/cwd/PATH/stdin classes — captured set ==
  spawn env by construction. The command string and toolchain (`sha256(rustc
  --version --verbose)`) are both in the key. The mode bit (F-1) is the one
  result-affecting axis still uncaptured.

## Verdict

**One DO-NOT-SHIP: F-1 (P1 stale-green, code-defect).** The convergence claim is
overstated by exactly one axis. Every other invariant survived live attack —
the chain spine, redaction, reconcile, error-law, and DoS resistance held under
the strongest probes tried. Recommend: fix F-1 (fold mode into the tree axis +
regression gate), then a fresh round on the patched state before any ship claim.
