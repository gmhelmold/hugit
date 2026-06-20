# Adversarial pre-open-source audit — 11-agent sweep (2026-06-20)

5 haiku + 4 sonnet + 2 opus, read-only, "malicious" brief. Consolidated + deduped by the hugit TL.
**Headline:** the *code* is genuinely well-hardened (no copyleft deps, no committed secret values, no
SSRF/shell-injection/path-traversal, 404-no-oracle holds, constant-time token compare). The real blockers
to going public are **recon data in source+docs+git-history**, a **proprietary LICENSE**, and a batch of
**authz/redaction/DoS** fixes. Going public is NOT a flip — it's this list first.

## GOOD (verified clean)
- **Deps Apache-2.0-publishable**: 238 transitive deps, ZERO GPL/AGPL/LGPL/SSPL copyleft; `cargo deny check` PASSED; zero RUSTSEC advisories.
- **No secret VALUES in source** — all via `std::env::var`. gitleaks history hits were all test fixtures.
- **No exploitable** SSRF (token.rs/cas.rs), shell-injection (`git` via `arg()`, not shell), `../` path-traversal (resolve_blob_at_path rejects `..`), idempotency-replay, cross-tenant SSE/read oracle. 404-no-oracle holds.

## MUST-FIX BEFORE PUBLIC (CRIT/HIGH)

### A. Recon data exposed (the #1 blocker) — in source, docs, AND git history
- **[CRIT] Runner box IP `91.99.11.196`** in `crates/hugit-invariants/x6/lib.rs:61` (compiled into the binary!) + `docs/plan/provisioning-day0.md:57` + `docs/handoff/2026-06-08-*` (≥9 places). A direct SSH/attack target.
- **[CRIT] Tenant UUIDs**: `d863fafb-…` (CAS tenant) in handoff docs; `ee30f7ba-…` (Clerk owner_tenant) in **`engine-snapshots/hugit.json`** (committed) + handoffs.
- **[HIGH] Secret-file paths** (`~/.hugit/secrets/corelink/pat`) + GitHub App ID `3975152` + Clerk dev host `welcomed-eft-86.clerk.accounts.dev` in provisioning/handoff docs.
- **[HIGH] `crates/hugit-mirror/src/outbound/auth.rs:78`** hardcodes the github-app secret path (env-source it).
- **[HIGH] `docs/handoff/` (~88 files)** + `docs/plan/provisioning-day0.md` + `.claude/` + `.techlead/` = internal infra/topology/process map. **Exclude the whole `docs/handoff/` dir from the public repo.**
- **Git history**: these values are in PAST commits → a clean public release needs `git filter-repo` or a fresh/orphan history, not just deleting files.

### B. License (being decided) — current `LICENSE` is **proprietary / all-rights-reserved** → must replace.

### C. Authz / integrity holes
- **[HIGH] D14 is a no-op on the serve write boundary** — `write_undo/land/policy/verdict.rs` pass a HARDCODED principal class, never the authenticated caller → the "Human-only undo / Orchestrator-only land" matrix is decorative on the live API. Fix: derive class from the authenticated principal_chain, fail-closed.
- **[HIGH] `Caller::Unknown` can read public repos** (`hugit-serve/src/authz.rs:127-133`) — any structurally-valid non-standard bearer clears read on public repos (audit log, PRs). Fix: Unknown → 401.
- **[MED] dev-token = full operator bypass** — a single shared static secret grants total authority; go-live must require the Clerk exchange + dev-token disabled (not silent fallback).
- **[LOW] serve `verdict` has no adversarial-diversity** yet renders `adversarial:true` — false integrity signal.

### D. Redaction misses (secrets can land in the log/history un-scrubbed)
- **[HIGH] DigitalOcean `dop_v1_…` tokens** leak every redaction surface (not in KNOWN_PREFIXES; entropy below threshold). One-line fix: add prefix.
- **[HIGH] Policy pre-commit secrets gate uses structural-only** (`hugit-policy/src/gates/secrets.rs:42`) → high-entropy unprefixed secrets (AWS secret key) pass the gate into the log + git history. Fix: route through `redact::apply` and compare.
- **[MED] 32-char bare hex** + missing keyword prefixes (`auth_token`/`access_token`/`private_key`) + `Bearer\t` bypass.

### E. DoS / robustness
- **[HIGH] Symlink blobs served verbatim** (`hugit-proto/.../pack/mod.rs:345`) — leaks fs paths, bypasses scrub. Reject mode 120000.
- **[MED] No blob size cap** (multi-GB → RAM), **no search scan cap** (O(N) per query), **unbounded Idempotency-Key** (1MB → R2 bloat). Single-threaded `tiny_http` → each is a full-server stall.
- **[HIGH] `.expect()` on poisoned mutexes on REQUEST paths** — `hugit-app/src/webhook.rs:190/196/270` (webhook = untrusted input) + `hugit-checks/src/client/ac.rs:199/203` → a panic crashes the service.

### F. Honesty (bites once outsiders verify)
- **[HIGH] README** "build complete / Wave J / 17-package / all 67" — stale (8 days), wrong count (19), overclaims completeness.
- **[HIGH] CLAUDE.md** "symbol outline not wired / `[]` default / still W6" — STALE; symbols ARE wired (`blob.rs:80`).
- **[MED] product/whitepaper** "lights live at P2" / cross-repo status stated as hugit facts; **strategy stats** cite truncated/blog URLs.

### G. Repo hygiene
- **[MED] `.gitignore`** does NOT cover `.env`/`*.pem`/`*.key`/`wrangler.toml`/`ingest.env` → an accidental `git add .` could commit a real secret. Add them + a pre-commit secret hook.
- **[LOW] `gustavo@humangr.com`** in `gen_fixtures.rs` (compiled bin) + ADR examples → use `owner@example.com`.
- **[LOW]** Missing SECURITY.md / CONTRIBUTING.md / CODE_OF_CONDUCT.

## Decomposition (fix waves — fleet-buildable, disjoint)
- **W0-pub (must-do before public):** A (sanitize + history scrub) · B (license) · F (README/CLAUDE honesty) · G (.gitignore + governance files).
- **W1-sec (security, parallel):** C (D14 wiring + Unknown→401) · D (redaction prefixes/entropy gate) · E (symlink reject + caps + mutex-poison handling).
- All disjoint by crate/file → fan out ≤6 concurrent, contract-freeze the D14 class helper first.
