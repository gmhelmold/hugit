# hugit-serve production seams — buildable design (2026-06-14)

Pre-decision artifacts for the two production seams held out of the Wave-4 read
fan-out because they are judgment-heavy (architecture / security-critical
crypto), not clean parallel WPs. Both designs are evidence-cited against the
actual code + the frozen spec; every unverified claim is marked. Build these
centrally (they touch `server.rs`/`state.rs`), with a lead crypto/security audit
before merge.

---

## Seam A — SSE `GET /v1/repos/{repo}/events?since=<seq>` (spec §2)

**Threading reality (verified):** `tiny_http` uses an internal task-pool to *read*
connections, but `serve_on`'s `incoming_requests()` loop processes responses
**serially on one thread**. `request.respond(...)` blocks the loop until the body
is fully written. A long-lived stream therefore freezes the whole server.

**Decision: replay-then-close (buildable NOW); true live-tail is the genuine P2
seam.** The client (`../githugr/crates/githugr-live/src/events.rs`) already
handles clean closure: `Ok(None) → break`, reconnect from its `since` cursor.
For the current low-event-rate repos this is indistinguishable from live-tail
with a small polling lag.

**Frame format (verified `../githugr/.../tests/events.rs:53–54` + spec §2):**
```
id: <seq>\n
data: {"seq":<seq>,"kind":"<kind>","summary":"<scrubbed pt-BR>"}\n
\n
```
- `id:` value ≡ `data.seq` (pinned 2026-06-12). No `event:` line.
- Gap (since below retention floor): first frame `kind:"gap"`, `summary:"recarregue"`,
  `seq` = current head (`log.len()`).
- Heartbeat: `: hb\n\n` (SSE comment). In replay-then-close: a single trailing one.
- `Content-Type: text/event-stream` (client rejects otherwise).

**Redaction:** the `summary` field is payload-derived free text → MUST pass
`crate::fmt::scrub`. `kind`/`seq` are structural → no scrub.

**Plan:** new `handlers/events.rs` → `build_events(log, repo, since) -> Vec<u8>`;
`parse_since(url)` helper; SSE dispatch in `serve_on` BEFORE the standard
`(status, String)` path (different Content-Type + `Vec<u8>` body) via
`Response::from_reader(Cursor::new(bytes), Some(len))`; auth + `is_safe_repo_slug`
+ `load_verified` (the PS-13 chokepoint) first. Tests in
`tests/events_sse.rs`: replay-since-0/-N, gap-prepend, trailing-hb, summary-scrub,
empty-log, 401-no-bearer, 404-unknown/unsafe, content-type, id≡seq.

**Deferred P2:** true live-tail (needs per-connection threads or async server),
≤25s heartbeat on a quiet live stream, multi-client fan-out (CoreLink pub/sub),
native `Last-Event-ID` (client uses `?since=` instead — UNVERIFIED whether the
window uses native `EventSource`), retention trim.

**UNVERIFIED:** the exact payload field names per kind in `extract_summary` —
cross-check against how the `hugit-refstore` append verbs write each payload.

---

## Seam B — `POST /v1/token` (RFC-8693 exchange, spec §4) + request auth

**What exists (verified):** `auth.rs:20` `check_bearer` (SHA-256 + XOR-fold
constant-time compare) against `state.dev_token` (env `HUGIT_ENGINE_DEV_TOKEN`,
fail-closed). `server.rs:156` `dev_principal()` = hardcoded
`["orchestrator:hugit"]`; no principal derivation from the token yet. `state.rs`
doc already names this the deliberate Wave-1 stand-in for the ADR-0002/RFC-8693
Clerk-JWKS P2 seam.

**Crate decision (verified against `Cargo.lock`):** `jsonwebtoken 9.3.1` is
ALREADY in the lock (a `hugit-queue` dev-dep). Adding it as a regular dep to
`hugit-serve` = **zero lock delta**, no `cargo deny` multiple-versions hit.
JWKS fetch via `ureq` (already a dep). **Do NOT hand-roll RS256** (banked SigV4
lesson) and **do NOT add `rsa`/`rand 0.8` as a dev-dep** — it collides with the
in-tree `rand 0.9.4` under `multiple-versions = deny`. For tests: bake a
pre-generated 2048-bit RSA keypair as PEM constants + pre-computed JWKS `n`/`e`;
sign with `EncodingKey::from_rsa_pem` (no `rsa` dep needed).

**Validation spec (mirror githugr `clerk.rs`, fail-closed at each):** pin
`alg=RS256` on the untrusted header first (kills `alg=none` + HS256 confusion);
resolve `kid`→JWKS (refetch-once, fail-closed on unknown/fetch-fail); `decode`
with `validate_exp/nbf`, `set_issuer`, `set_required_spec_claims(["exp","iss"])`,
30s leeway; `azp` check (optional env now, P2-mandatory pending CoreLink TL value);
tenant from `publicMetadata.tenant_id` (authoritative) → `org_id` fallback → reject
if neither; `sub` → user; `auth_time`→`fresh_auth` (300s window, absent/future →
false). **NOTE:** Clerk on CoreLink does not currently emit `auth_time`, so
`fresh_auth` is always false today (step-up verbs deny) until frontend
re-verification — the engine mirrors this honestly.

**Engine-token design:** the exchange returns an OPAQUE 32-random-byte token
(not a JWT — the sync server has no async runtime and minting JWTs needs an RSA
signing key to manage). Stored as `Mutex<HashMap<sha256(token), TokenRecord{user,
org, fresh_auth, expires_at}>>`, 300s TTL, swept on mint; `lookup` SHA-256s the
presented token + constant-time compares (the existing pattern). **The raw token
and the subject_token are NEVER logged or echoed** (ADR-0002 §6.1 + banked
"PAT never logged"). Multi-instance shared store = P2 (same seam as the idem ledger).

**Auth gate (two-tier):** `token_store.lookup` → on hit return the real
`TokenRecord` principal; else `check_bearer(dev_token)` → `dev_principal()`; else
401. Distinguish `TOKEN_EXPIRED` (in-store-but-expired → client renews via
`/v1/token` + retries once) from `TOKEN_INVALID` (→ login).

**Files:** new `token.rs` (`ClerkValidator`+`JwksCache`+`TokenStore`+
`handle_token_exchange`+tests); `auth.rs` two-tier; `error.rs` `token_expired()`;
`state.rs` `token_cfg: Option<TokenConfig>` + `token_store: Arc<TokenStore>`;
`server.rs` `["v1","token"]` route (the ONLY no-Bearer route) + thread the real
principal/`fresh_auth` into the write dispatch; `main.rs` wire
`TokenConfig::from_env()`; `Cargo.toml` `jsonwebtoken = "=9.3.1"`.

**Tests (mock JWKS via in-process `tiny_http`, no `rsa` dep):** valid exchange,
expired/wrong-iss/missing-exp rejected, `alg=none` + HS256-confusion + tampered-sig
attacks rejected, fresh_auth true/false/absent/future, expired-engine-token →
TOKEN_EXPIRED, audience-mismatch + no-org + cross-tenant-mint blocked, step-up gate
uses `fresh_auth` from the record.

**Deferred P2:** live Clerk JWKS URL + issuer + mandatory `azp` (owner/CoreLink-TL
gated), shared multi-instance token store, real `fresh_auth=true`, git-layer
commit identity.

---

*Designs produced by a 2-agent research fan-out (read-only), lead-reviewed.
Source agents cited file:line evidence throughout; UNVERIFIED items flagged above.*
