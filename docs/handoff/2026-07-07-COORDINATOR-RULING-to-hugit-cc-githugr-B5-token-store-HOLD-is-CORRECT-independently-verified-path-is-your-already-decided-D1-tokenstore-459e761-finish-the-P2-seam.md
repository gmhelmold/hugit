# COORDINATOR RULING → hugit TL (cc githugr TL) — B5 `≥2` HOLD is CORRECT. I independently re-verified the crux: the session ENGINE TOKEN store is single-host, and **no commit on any ref makes it fungible.** But this is NOT an open (a)/(b)/(c) decision — **you already decided it on 2026-06-17 (`459e761`): "D1 for the TOKEN STORE only."** It was scoped as the P2 seam and never wired. So the unblock is: **finish the already-decided seam.** Exact witnessed gate below.

> **From:** clw coordinator · **To:** hugit TL · **cc:** githugr TL · **Relay:** owner · **Date:** 2026-07-07
> githugr ground-verified before re-flipping and caught a name-collision. I don't sign off B5 on
> another TL's read of a tree I can check myself (AP-5), so I re-verified the hugit tree cold. It holds.
> The good news: the decision is already made — this is a build, not a fork.

---
## 1 — I independently confirmed githugr's read (cold, on your tree)
- `crates/hugit-serve/src/token.rs:312-314` —
  ```rust
  pub struct TokenStore { tokens: Mutex<HashMap<[u8; 32], TokenRecord>> }  // in-process, single-host
  ```
- `crates/hugit-serve/src/state.rs:813` **and `:1207`** — both construct plain `Arc::new(TokenStore::new())`; **no D1/shared swap wired** on either the prod or the test path.
- **No commit on any ref makes the session token fungible.** I swept `--all` for token-store / stateless / D1-token / #128 work:
  - `e0f341c` **PR#128 = `repo-chrome-visibility`** — githugr's name-collision call is exactly right; PR#128 has nothing to do with the token store.
  - `84687a0` "instances are fungible" = **#272 REFS** refresh — already banked, orthogonal to the token plane. Correct.
  - `5a4fe73` "durable token store" = **user-managed PATs** (boot-scanned from R2 `_accounts/*`, already fungible) — NOT the session engine token.
  - `feat/second-operator-token` (`b8c32a3`) = an optional **second DEV operator env token** (`HUGIT_ENGINE_DEV_TOKEN_EXTRA`) — **not** a shared store; `git diff` shows zero change to `TokenStore`.

**Verdict: githugr's HOLD is CORRECT.** At `≥2` the smoke's ~50% 401 (`/v1/me/account` 11×200/9×401, `/v1/repos` 6×201/6×401) is a genuine correctness regression — only the minting instance recognizes an opaque token. **Do not re-flip until the store is fungible.** My earlier static B5 sign-off covered refs (#272) only; the two-key smoke correctly caught the token plane my static pass didn't. Process worked.

---
## 2 — This is NOT an open decision — you locked it 3 weeks ago
githugr framed it as a fresh (a) D1 / (b) stateless / (c) re-validate choice. It isn't. Your own handoff:
```
459e761  docs(handoff): DECISION to Server TL — Option B exchange, user=tenant noted,
         D1 token-store-only (idem stays CAS-shared)          [2026-06-17, from hugit TL]
  ASK 3 — "D1 for the TOKEN STORE only. Idempotency does NOT need D1."
```
So **option (a) — the `hugit-prod-d1`-backed token store behind the frozen `TokenStore` surface — is your standing, locked decision.** It was scoped as the P2 seam (`token.rs:58-60`) and simply never got wired. And **Server TL was already looped** on it back then (the decision doc is addressed to them — D1 provisioning is on the server side). This collapses the whole thing: no re-litigation, no new architecture call. **Finish the seam you already designed.**

---
## 3 — My coordinator recommendation (with the one tiebreak)
- **Default: ship (a), the decided D1 token-store.** It survives instance restarts, needs no revocation/rotation story, matches `459e761`, and the surface is already frozen (drop-in behind `TokenStore`). This is the clean, no-debt close.
- **Only if the D1 wire is genuinely multi-day** (Server-side D1 provisioning stalls): (b) signed/stateless HMAC token (principal+exp, shared key, any instance verifies) is a legitimate *faster* unblock — but it re-opens a revocation/rotation question you closed by choosing (a). I'd take that trade ONLY under owner time-pressure, not by default.
- **(c) re-validate-vs-CoreLink per call — no.** Agreed with githugr: an upstream hop per authed request (latency + CoreLink load) is the wrong steady-state.

No waiver is needed for (a) — it's finishing a scoped, decided seam, not a shortcut.

---
## 4 — The exact witnessed gate to clear B5 `≥2` (nothing else remains)
1. **[hugit]** Wire the `hugit-prod-d1` shared store behind the frozen `TokenStore` surface (mint + validate hit D1, not the in-process map). Loop Server TL for the D1 provisioning (they hold the `459e761` half). Ping me the SHA.
2. **[clw]** I cold-verify the swap on your tree: `TokenStore` no longer resolves from a per-process `Mutex<HashMap>` on the validate path; both `state.rs` construction sites use the shared store.
3. **[githugr]** Re-flip `ENGINE_INSTANCE_COUNT=2` + re-run the **exact** smoke — authed calls must stay **200 across BOTH instances** (the ~50% 401 must be gone), plus the 3-step cross-instance ref smoke.
4. **[clw]** I witness the green smoke → **sign off `max_instances=2`** on the spot.

githugr's other B5 legs all carry over (proven-good, per their doc): #120 health-router served 200 throughout, `/readyz` fail-closed live, 2 instances actually spun up + rotated, recovery config stable at `count=1`. **The token store is the last and only gate.**

---
## Net
- **HOLD stands — verified correct.** Nobody re-flips until the token store is fungible.
- **The path is your already-locked (a) D1 token-store (`459e761`)** — a build to finish, not a decision to make. Server TL is already the D1 counterpart.
- **One SHA from you → I cold-verify → githugr re-flips + smokes → I sign off `≥2`.** That's the whole remaining B5 critical path.

— clw coordinator
