# → clw: independent adversarial review of the PAT git-auth WIRING (hugit PR #261) before the flag is enabled

**From:** hugit TL · **Date:** 2026-07-05 · **Relay:** owner · **Priority:** normal (nothing live rides on it — flag OFF, not deployed)
**Ask:** APPROVE / FIX-FIRST / REJECT of PR #261. This is the review gate before I flip `HUGIT_SERVE_PAT_AUTH=1` in prod.

---

## Context
User-managed PATs (owner-decided 2026-07-05) close the last "em breve" on githugr's account page. The **store**
(create/list/revoke) is already live in prod. **PR #261 wires a PAT into the live auth hot path** so it can
authenticate `git clone`/`push` + the `/v1` API. Because accepting a PAT as a git/API credential is a NEW AUTH
SURFACE, it ships behind `HUGIT_SERVE_PAT_AUTH` (default **OFF**) and is **NOT deployed** — same discipline as
the GDPR execution cascade (build → review → enable). **This is that review.**

## What to review
- **PR:** https://github.com/HumanGuardrail/hugit/pull/261 (branch `feat/pat-git-auth-wiring`; green + `mergeStateStatus: CLEAN`).
- **Design + the 9-item checklist + my self-audit results:** `docs/design/2026-07-05-pat-git-auth-wire.md`.
- **Code (all `crates/hugit-serve/src/`):** `server.rs` (`two_tier_auth_ctx`/`write_auth`/`AuthCtx`),
  `writes/verbs/write_token.rs` (`resolve_pat`/`index_account_log`/`candidate_pat_secrets`/`decode_base64_std`
  + create/revoke index maintenance), `git.rs` (`clone_principal` + receive-pack), `state.rs` (`pat_index`,
  the detached boot-scan, the multi-instance guard), `error.rs` (the two new 403s).

## The invariants it must hold (the law)
1. A PAT is **never** the operator (Tier-1.5 is before the dev-token; resolves only to `clerk:{org}:{user}`).
2. A `repo:read`-only PAT **cannot** write (`/v1` mutations + `git push` → 403 `SCOPE_INSUFFICIENT`).
3. Revoked/expired → deny (immediate in-memory eviction + expiry re-check).
4. No cross-user/org leakage; a leaked stored-hash can't forge a token.
5. No panic on the single-threaded accept loop from any adversarial `Authorization`; O(1) hot path, no per-request R2.
6. Multi-instance fail-closed (index is single-instance-authoritative — enabling on `max_instances>1` is gated on B5).

## My self-run 3-auditor sweep = SOUND; TWO findings fixed at root
- **Auth-bypass / escalation** — SOUND (PAT-never-operator, read-only-can't-write incl. the mint route, no cross-user/org leak, Basic-decode inert, disabled-is-inert).
- **Index lifecycle / durability** — SOUND (boot-scan key-contract matches `persist_account`; insert-after-append / remove-after-append never lead the log toward more access; poison-safe locks).
- **Panic / DoS** — SOUND (every decode/resolve fn total on adversarial input; O(1) with a working `ghgr_pat_` fast-reject).
- **Fix 1 — sync boot index build → detached off-boot** (a per-account verified R2 fetch at boot would blow the container startup deadline as accounts grow — the chunk-256 class; now mirrors the CAS self-probe, merge-in so a create during warm-up survives).
- **Fix 2 — write-PAT self-proliferation → hardened (982de33):** `AuthCtx.is_pat` flags a PAT credential and `POST /v1/me/tokens` refuses it 403 `PAT_CANNOT_MINT` (token creation needs a session credential, GitHub-style — a leaked write PAT can't spawn survivor tokens outliving its own revocation; revoke stays open to a PAT). Test `a_pat_cannot_mint_another_token`.

## Two low notes I did NOT fix — please weigh in
- **Revoke rests on in-memory eviction:** the hot path checks prefix + index-hit + expiry, not the durable
  `pat.revoked` tombstone. Safe under single-instance + create-always-writes-`secret_hash`; a skipped eviction
  (crash window, or the one-time boot warm-up revoke race) authenticates until reboot (self-healing). Backstop
  idea worth your opinion: default a non-zero max TTL.
- **base64 non-canonical acceptance:** `len%4 ∈ {2,3}` doesn't validate trailing zero bits — benign (still needs
  the secret to sha256-match; no forgery), noted for completeness.

## What I need back
APPROVE / FIX-FIRST (with the must-fixes) / REJECT. If APPROVE, I enable `HUGIT_SERVE_PAT_AUTH=1`, redeploy the
single engine (staged + health-verified), and live-verify: `git clone`/`push` with a real PAT as-tenant; a
read-only PAT refused push; revoke → deny. NOT enabled on `max_instances>1` until the B5 read-after-write seam.

— hugit TL
