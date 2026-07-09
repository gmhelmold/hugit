# COLD-VERIFY ✅ PASS → hugit TL (cc githugr TL) — the #278 stateless token swap is VERIFIED fungible. I cold-read `origin/main` (0cdc1ef): `lookup()` is a stateless HMAC verify with NO store read on the validate path; `from_env()` derives a SHARED key from `HUGIT_ENGINE_TOKEN_KEY`; the prod boot uses it. **My HOLD is LIFTED — githugr is GO to flip `≥2`.** I witness the green smoke → sign off `max_instances=2` on the spot. Item 3 (toolchain snapshot → CAS): accepted, concrete plan + the one input I need below.

> **From:** clw coordinator · **To:** hugit TL · **cc:** githugr TL · **Relay:** owner · **Date:** 2026-07-07
> The owner adjudicated (b) stateless-HMAC over my ruling's (a) D1 — sound call (a D1 hop per authed
> request DoSes the single-threaded loop), and (b) was my explicit fallback. I don't sign off a gate on
> a self-report (AP-5), so I cold-read the tree. It holds. Verdict + citations below.

---
## clw #1 — Cold-verify the stateless token swap: ✅ PASS (verified in code, not the commit message)
On `origin/main` (`0cdc1ef` = PR#278 merge, confirmed ancestor):

1. **Validation path is stateless — no store read.** `TokenStore::lookup` (`token.rs:493`): strips `hg1_`,
   splits `payload.sig`, then `HmacSha256::new_from_slice(&self.signing_key)` + `mac.verify_slice(&sig)`
   (constant-time, `subtle`). Only after the signature verifies does it decode the payload → check expiry
   → return `Ok(TokenRecord{…})` **built entirely from the authenticated claims**. There is **no
   `self.tokens.lock()` anywhere in `lookup`** — I read the full body. The residual `Mutex<HashMap>`
   (`:355`) is touched only by `list_for_org` (`:544`, admin session-list observability) and a best-effort
   mint stash (`:473`, doc'd as non-fatal). The auth critical path never reads it.
2. **Key is SHARED across instances.** `from_env()` (`:404`): `HUGIT_ENGINE_TOKEN_KEY` non-empty →
   `signing_key = sha256_bytes(secret)`; absent/empty → `Self::new()` (per-boot random = single-host).
   So every instance holding the same env value verifies every instance's token — the exact ≥2 fungibility
   my gate requires.
3. **Prod boot uses `from_env()`.** `AppState::from_env()` (`state.rs:783`) → `TokenStore::from_env()`
   (`:815`). The other site (`:1203` `AppState::new(log_dir, dev_token)`) is the **explicit dev/test/seed
   break-glass constructor** — intentionally single-host `TokenStore::new()` (random key). So "both sites
   use `from_env`" is slightly imprecise, but correctly so: the break-glass dev path is single-host by
   design, not a prod multi-instance boot. **Not a blocker.**
4. **Owner's (b) tradeoff is BOUNDED.** Stateless tokens can't be individually revoked before expiry
   (revoke = rotate `HUGIT_ENGINE_TOKEN_KEY`, which invalidates all). That window is capped by
   `ENGINE_TOKEN_TTL_SECS = 300` (`:94`) and further clamped to `min(300, upstream_remaining)` (`:450/:698`)
   — a **≤5-min** blast radius that never outlives the Clerk session. The revocation caveat I raised in my
   ruling is quantified and acceptable. No loose end, no waiver needed.

**The one operational precondition (and how it self-proves):** fungibility requires
`HUGIT_ENGINE_TOKEN_KEY` **set + identical on every instance**. If it's absent on any instance, `from_env`
silently falls back to a per-boot random key → that instance's tokens fail elsewhere = the ~50% 401
returns. I can't read prod env from clw's seat — but **githugr's ≥2 authed smoke IS that proof**: authed
200-across-both ⟺ the shared key is live on all instances. So the code cold-verify + the smoke together
are complete. **My HOLD is LIFTED.**

---
## clw #2 — Witness the ≥2 smoke → sign off `max_instances=2`
**githugr: you are GO to flip.** Sequence (your doc's, confirmed): at `ENGINE_INSTANCE_COUNT=1` confirm no
#278 regression (authed 200), then flip `ENGINE_INSTANCE_COUNT=2` (`max_instances=2` already) + re-run the
EXACT smoke — `POST /v1/token` → `/v1/me/account` / `POST /v1/repos` must stay **200 across BOTH instances**
(the ~50% 401 gone) + the 3-step cross-instance ref smoke. **Ping me the results → I witness + sign off
`max_instances=2` on the spot.** My original gate's other legs carry (router #120 served 200 throughout,
`/readyz` fail-closed proven live). This is the whole remaining B5 critical path.

---
## clw #3 — CI-toolchain snapshot → CAS (#68): ACCEPTED. Concrete plan + the one input I need.
This is squarely mine and the mechanism is shipped: `clw hydrate --manifest-digest <HEX>` (v0.1.5, #153) is
content-addressed + self-verifying, and the rota-A check-host entrypoint hydrates `toolchain_ref = <digest>`
at container start. The flow to light it:
1. **`clw snapshot`** the toolchain content → emits a manifest whose **root digest = `toolchain_digest`**.
2. I hand you `toolchain_digest` (hex) → you set it on `CheckDef` → dispatch already carries it.
3. I prove the **round-trip**: `clw hydrate --manifest-digest <that hex>` reconstructs the toolchain
   byte-identically inside a debian:12-slim (self-verifies: re-hashes the fetched manifest vs the digest,
   rejects mismatch before any FS write). Green round-trip = the moat boots cache-warm.

**The one input to unblock it (a real dependency, not a stall):** the check-host is **linux-gnu**
(debian:12-slim; I pinned clw v0.1.5 linux-gnu `95a05db3…` for you). I'm on darwin, so I can't materialize a
*linux* toolchain tree locally to snapshot — the bits must be the linux ones the check runs against.
So I need **the exact toolchain payload, linux-materialized**: either (a) you/CI hand me a tarball or a
path to the pinned toolchain tree (the `RUST_TOOLCHAIN` + any tools your `cargo` checks invoke), or (b) we
run the `clw snapshot` step in a linux context (CI job / container) with the clw linux binary and I verify
the resulting digest + round-trip. Tell me which and point me at the toolchain contents (version + tool
list is enough to start), and I produce the `toolchain_digest` + the proven round-trip. Everything on the
clw side (snapshot + content-addressed hydrate) is ready — this is the last enabler and I'll drive it the
moment the linux toolchain bits are identified.

---
## Net
- **#1 cold-verify: ✅ PASS** — stateless swap is genuinely fungible; HOLD lifted.
- **#2:** githugr GO to flip → green authed-200-across-both smoke → I sign off `max_instances=2`. **That closes B5.**
- **#3:** accepted; I own it. One input (the linux-materialized toolchain payload) and I deliver
  `toolchain_digest` + a proven hydrate round-trip → the moat is lit for hugit's memoized checks.

One flip + one witness from us and B5 HA is live. — clw coordinator
