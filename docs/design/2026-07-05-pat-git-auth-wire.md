# PAT git-auth wire — the decided design (slice 2b of WP-#90)

> **Status:** DECIDED design + WIRING BUILT (behind the `HUGIT_SERVE_PAT_AUTH` flag, default
> OFF — awaiting the adversarial review before live enable). Slice 2a (store + create/revoke +
> `me/account.pats`) is live on `main` (#259). Slice 2b's PURE FOUNDATION (the `PatAuth` resolver +
> `index_account_log` + `resolve_pat`) landed in #260. **The HOT-PATH WIRING is now built** (this
> branch): the `AppState` index field + boot-scan of `_accounts/*` + create/revoke refresh + the
> Bearer AND HTTP-Basic credential extraction + Tier-1.5 in `two_tier_auth`/`clone_principal`/
> receive-pack + the `can_write` scope gate (403 `SCOPE_INSUFFICIENT`) + the multi-instance
> fail-closed boot guard. Gate-green (626 lib tests + the new wiring tests, clippy `-D warnings`,
> fmt). It changes the live auth surface, so it stays behind `HUGIT_SERVE_PAT_AUTH=1` (OFF on prod)
> until the adversarial review below signs off — the same discipline as the GDPR execution cascade:
> build → review → enable.
> **Why gated:** accepting a PAT as a git/API credential is a NEW AUTH SURFACE. A subtle bug (a
> revoked/expired token accepted, a PAT conferring operator, cross-user leakage, a read-only PAT
> writing) is a security incident. It gets the same discipline as any security-surface go-live.

## The law it must honor (invariants)

1. **A PAT is NEVER the operator.** It resolves ONLY to a `clerk:{org}:{user}` (the stored owner).
   The resolver is inserted as **Tier-1.5** in `two_tier_auth` — AFTER the session-token store
   (Tier-1), BEFORE the dev-token (Tier-2) — so a `ghgr_pat_` bearer can never reach the god-path.
2. **Read-authz ≠ write-authz still holds**, plus a NEW scope gate: a `repo:read`-only PAT MUST NOT
   authenticate a mutation (push/land/…). `PatAuth::can_write()` gates the write paths.
3. **Revoked / expired → 401** (never a stale accept). The in-memory index is the authority; it is
   rebuilt on revoke, and `resolve_pat` re-checks expiry on every call.
4. **The stored-hash leak cannot forge a token.** The index keys on `sha256(secret)`; resolution
   hashes the presented secret. Knowing a hash is useless without the secret.

## The in-memory index (the hot-path structure)

- **Shape:** `HashMap<secret_hash, PatAuth>` on `AppState` (`Arc<RwLock<…>>`, interior-mutable
  behind the shared `&AppState` — same pattern as `repos_runtime`/`LiveRefs`).
- **Built at boot:** `list_keys("_accounts/")` → load each account log → `index_account_log(&log)` →
  union. Bounded by `MAX_LIST_PAGES`; a rare, few-account engine → negligible boot cost.
- **Refreshed on create/revoke:** `token_create`/`token_revoke` update BOTH the durable log AND the
  in-memory index (insert on create, remove on revoke) — so a revoke takes effect immediately, no
  reboot. (Mirrors the ref hot-swap.)
- **Hot-path lookup is O(1) in-memory** — NO R2 read per request (critical for the single-threaded
  engine; an R2 GET per auth would be a latency DoS, per the single-thread-latency memory).
- **Single-instance authoritative.** On `max_instances:1` (the pinned prod posture) the index is
  consistent. On multi-instance (B5), a PAT created on A is absent from B's index until B reboots —
  the SAME cross-instance staleness class as the ref hot-swap, and it rides the B5 fungibility seam
  (read-after-write). Do NOT enable PAT auth on `max_instances>1` before that lands.

## The auth flow (both Bearer and Basic — the git-CLI reality)

The `git` CLI sends **HTTP Basic** (`Authorization: Basic base64(username:secret)`), NOT Bearer. So
credential extraction must accept BOTH:
- `Authorization: Bearer <secret>` — the `/v1` API + `http.extraHeader` git.
- `Authorization: Basic base64(user:secret)` — the default git CLI. Decode → take the PASSWORD
  field as the candidate secret (git puts the PAT in the password; the username is ignored, GitHub-
  style). A malformed base64 / missing colon → no credential (degrade/401, never a panic).

Then: `resolve_pat(&index, candidate, now_ms)` → `Some(PatAuth)` → the principal chain +
scopes; `None` → fall through (not a PAT → the existing tiers; the git wire → anonymous).

**Insertion points (both must be wired identically):**
- `server::two_tier_auth` — the `/v1` write/read door (Tier-1.5, returns the principal + carries the
  scope for the write gate).
- `git::clone_principal` — the git clone/fetch read wire.
- `git::handle_receive_pack`'s auth — the git PUSH wire (MUST also enforce `can_write()`).

## Scope enforcement (the write gate)

`two_tier_auth` today returns `(principal, fresh_auth)`. To gate writes on `repo:write` WITHOUT a
wide signature churn, the cleanest is a small carrier: return the resolved PAT's scope alongside (or
a `AuthCtx { principal, fresh_auth, write_ok: bool }`), and the write dispatch (`dispatch_repo_write`,
`handle_receive_pack`) refuses a mutation when the credential is a read-only PAT (`403 FORBIDDEN`,
distinct from the ownership 404). A session/dev credential is `write_ok = true` (unchanged behavior).

## `last_used_at`

Deferred to a best-effort follow-up: a durable write-per-auth is too expensive on the single-thread
engine. Options for later: update an in-memory `last_used` on the index entry + flush lazily (on the
next create/revoke, or a periodic flush). Until then `me/account.pats[].last_used_at` stays `0` (=
never/unknown) — honest, flagged to githugr.

## The adversarial checklist (clw / a fresh reviewer runs it before live enablement)

1. A `ghgr_pat_` bearer NEVER yields the operator (Tier-1.5 is before the dev-token; assert a PAT
   whose secret happened to also match the dev-token still resolves as its clerk owner, not operator).
2. Revoked → 401 immediately (index removed on revoke, no reboot).
3. Expired → 401 (boundary: `now >= expires_at`).
4. Cross-user isolation: a PAT authenticates as its OWN `clerk:{org}:{user}`, never another.
5. A `repo:read`-only PAT is REFUSED on every write path (`/v1` mutations + receive-pack), 403.
6. Basic-auth decode is injection/panic-safe (malformed base64/UTF-8/missing-colon → no credential,
   never a crash; the username field is inert).
7. A stored-hash leak cannot forge a token (resolution hashes the presented secret).
8. No latency regression on the hot path (in-memory O(1); no per-request R2).
9. PAT auth is DISABLED (or the staleness accepted) on `max_instances>1` until B5 read-after-write.

## Build order

1. ✅ Store + create/revoke + `me/account.pats` (#259, slice 2a).
2. ✅ The PURE resolver foundation (`PatAuth` + `index_account_log` + `resolve_pat`) + adversarial
   unit tests (#260).
3. ✅ **The hot-path WIRING (built, behind the flag):** the `AppState` `pat_index` field + boot-scan
   of `_accounts/*` (fail-closed-DENY on a fault) + create/revoke index refresh (immediate mint/
   revoke, no reboot) + the Bearer AND HTTP-Basic credential extraction (`candidate_pat_secrets` +
   the hand-rolled panic-safe base64 decoder) + Tier-1.5 in `two_tier_auth_ctx`/`clone_principal`/
   receive-pack (before the dev-token → never operator) + the `can_write` scope gate (`write_auth`
   → 403 `SCOPE_INSUFFICIENT` on `/v1` writes + receive-pack) + the multi-instance fail-closed boot
   guard. Gated on `HUGIT_SERVE_PAT_AUTH` (default OFF). Tests cover every checklist item below.
4. **[clw / adversarial review of THIS design + the wiring — the gate before live enable]** ← HERE.
5. Enable (`HUGIT_SERVE_PAT_AUTH=1`) + redeploy + live-verify: `git clone`/`git push` with a real
   PAT as-tenant; a read-only PAT is refused push (403); revoke → the token stops authenticating.
   NOT enabled on `max_instances>1` until the B5 read-after-write seam.
