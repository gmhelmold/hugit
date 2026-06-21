# Convergence re-audit before the public flip (2026-06-21)

10-agent brutal re-audit (6 haiku + 3 sonnet + 1 opus). **Prior fixes HELD; but two SHOWSTOPPERS for
going public surfaced, plus redaction leaks + public-set cleanup.** (1 agent — HTTP/git-wire surface —
still running; fold its findings in.)

## ✅ PRIOR FIXES VERIFIED HELD
- **D14 authz** enforced on the write boundary (chain-derived class, fail-closed) + the ownership gate
  excludes workers before the verb body. **`Caller::Unknown` read-deny** held. **dev-token** constant-time, fail-closed.
- **Mutex-poison** recovery held (webhook/ac); **no request-path panics/unwraps** anywhere (incl. Wave-1 code).
- **Deps Apache-2.0-publishable** (cargo-deny clean, no copyleft, no RUSTSEC; new `gix-hash` is MIT/Apache).
- **README honesty** fix held. **LICENSE** Apache-2.0 correct, license fields set, no junk.

## 🔴 SHOWSTOPPERS — MUST FIX before the public flip
1. **[CRIT] Going public un-gates the admin/audit control-plane.** `/v1/repos/{repo}/audit`, `/erasure`,
   `/admin/overview` are gated ONLY by read-visibility (`authorize_read`), NOT `is_operator`
   (`hugit-serve/src/server.rs:~809-826`). The documented path to enable anonymous `git clone` is to set
   the `hugit` repo `visibility:public` — the instant it's public, ANY anonymous caller can
   `GET /v1/repos/hugit/audit` → full authz/denial timeline, principal chains, record hashes, erasure
   decisions, admin overview. **The public-clone switch and the admin plane share one gate.** Fix: gate
   audit/erasure/admin/overview with `is_operator(principal)` → 404 for non-operators regardless of visibility.
2. **[HIGH] CI fork-injection on the self-hosted runner.** With the repo public + `pull_request` trigger +
   the (just-added) self-hosted macOS runner, a forked PR runs arbitrary code on OUR runner box. The
   workflow's own comment flags this. Fix BEFORE public: `pull_request` → `pull_request_target` with a
   fork-guard (`if: github.event.pull_request.head.repo.fork == false`) / environment approval.

## 🟠 HIGH — redaction leaks (secrets can escape to a served/public surface)
3. **[HIGH] `avatar_class` leaks a pre-scrub author token** — `fmt.rs:82-89` `classify_avatar` runs on the
   RAW author and returns its first token unscrubbed into `/v1/.../commits`. A PAT in an author string rides out. Fix: scrub.
4. **[HIGH] Bearer case-sensitivity** — `secret_shape.rs:161` matches literal `Bearer`; `bearer`/`BEARER` bypass. Fix: case-fold.
5. **[MED] Branch names + `blob.tree` filenames + export refs not scrubbed** — a secret in a branch name /
   filename / ref leaks (commits.rs/new_pr.rs branches, `blob.rs:116-133` sidebar names, `export/mod.rs:197` refs). Fix: scrub each.
6. **[LOW] Missing prefixes** — `glpat-` (GitLab), uppercase-hex edge, newline-split connection strings.

## 🟡 Public-set cleanup (before the squash-public tree)
- **Recon residuals in the WILL-BE-PUBLIC set:** `~/.hugit/secrets/` paths in source docstrings + tests +
  CHANGELOG; tenant `ee30f7ba` in `cas.rs`/`state.rs` TESTS; `corelink-api.humangr.com` + `humangr-labs/hugit`
  in tests; **`Gustavo Schneiter`** full name in `profile.rs`/`org.rs`/`account.rs` test fixtures; `test@humangr.com`;
  `hugit-runner-01` in `docs/interop.md` (interop is EXCLUDED, so n/a). Sanitize the in-public ones.
- **`.gitleaks.toml` allowlist** for the redaction test-vectors (ghp_/AKIA examples) so GitHub secret-scanning doesn't fire.
- **R2 error → client** (`state.rs:602-609`) leaks the credential model; make it generic.
- **SECURITY.md** needs a real contact; **README** wants a license badge + a top-level quick-start.
- Approach **B (squash-fresh public repo)** ALREADY solves the "241 internal files in git history" blocker
  (no history is published) — confirms B was the right call vs an in-place flip.

## Plan
- **Wave A (blockers — before ANY public push):** #1 admin-plane operator gate · #2 CI fork-guard.
- **Wave B (redaction, parallel):** #3 avatar · #4 bearer-case · #5 branch/tree/ref scrub · #6 prefixes.
- **Wave C (public-set cleanup):** sanitize the in-public recon/name residuals + `.gitleaks.toml` + R2-error + SECURITY/README.
- Then rebuild the squash-public tree → re-verify recon-zero → owner push-trigger.
