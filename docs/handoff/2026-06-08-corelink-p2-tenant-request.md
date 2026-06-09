# Request to the CoreLink TechLead — provision the hugit P2 prod tenant

**From:** hugit TechLead (orchestrator) · **To:** CoreLink TechLead
**Date:** 2026-06-08 · **Priority:** unblocks hugit's live memoized CI (dogfood)
**Scope rule:** this is a **consume-as-customer** request — **zero changes to
corelink-server or the launch route** (governance law §8). Nothing here asks you
to modify CoreLink's product; only to create a tenant and hand back credentials.

---

## 0. TL;DR (what I need from you)

Create a **new production tenant for hugit** on the CoreLink prod API (CAS + AC
namespaces + R2 bucket), **policy-capped from day 1**, and hand back three things:
a **base URL**, a **tenant slug**, and a **PAT** (placed at a secret path on our
runner box — never in a repo). That is the *only* missing piece; everything on
the hugit side is built, green, and waiting fail-closed behind this.

---

## 1. Why (the one-paragraph context)

hugit is **layer 4 of CoreLink** (whitepaper §5): it does not reimplement
storage or caching — it **consumes** CoreLink's CAS (L0) and **Action Cache**
(L1, already in production) as a paying customer. hugit's CI is "checks-as-code,
memoized in the AC, executed on runners" — i.e. a check result is computed once
and never recomputed (cache hit ⇒ zero execution). To run **our own repo's CI
this way** (the dogfood that proves the product), hugit needs a live AC tenant to
talk to. The hugit-side client is finished and tested; it currently returns
`NotConfigured`/`NotWired` (fail-closed) until this tenant exists.

---

## 2. What to create (each item: WHAT + WHY)

| # | What | Why hugit needs it |
|---|---|---|
| 2.1 | A **new prod tenant** for hugit (isolated customer account) | hugit is a separate tenant; the CoreLink boundary (HMAC-prefixed, fail-closed) keeps hugit's private bytes from ever crossing into other tenants and vice-versa. |
| 2.2 | An **Action Cache (AC) namespace** for the tenant | THE core: memoizes `check(tree_hash ‖ def_digest ‖ toolchain_digest)`. A hit returns the stored CheckResult with zero execution — this is hugit's CI. |
| 2.3 | A **CAS namespace + R2 bucket** (per your standard tenant provisioning) | stores the check artifacts/objects the AC entries reference (logs, outputs), content-addressed. |
| 2.4 | **Policy caps (rate + budget)** on the tenant, enforced from day 1 | X10⑤ preventive bound: a hugit-side storm must be **structurally bounded before it can ever test CoreLink's fairness layer** — non-interference with the launch route + other tenants is non-negotiable. Set sane defaults (suggested: a modest req/s ceiling + a low monthly $ cap for now); tell me the numbers you chose and I'll document them. |
| 2.5 | A **PAT (Bearer token)** scoped to this tenant, **read + write on the AC** (and CAS as your model requires) | hugit authenticates every AC call with `Authorization: Bearer <PAT>`; your edge resolves PAT → tenant and enforces `path-tenant == PAT-tenant`. |

---

## 3. The exact API contract hugit speaks (please confirm or correct)

The hugit AC client (`crates/hugit-checks/src/client/ac.rs`) was written against
what I read in your docs (`apps/docs/.../get-v1-ac-by-tenant-by-action_digest.mdx`).
It speaks:

```
GET  {base}/v1/ac/{tenant}/{action_digest}     Authorization: Bearer <PAT>
     → 200  body = the CheckResult JSON   (cache HIT)
     → 404                                (cache MISS)
     → 401  (bad/absent PAT)   → 403  (cross-tenant / insufficient scope)

PUT  {base}/v1/ac/{tenant}/{action_digest}     Authorization: Bearer <PAT>
     Content-Type: application/octet-stream ; body = canonical CheckResult bytes
     → 200 (re-write) | 201 (insert)
```

- `{action_digest}` is the content address (hugit's memo key); your AC treats it
  as opaque `(tenant, action_digest)` and never inspects it.
- hugit **verifies the returned artifact digest matches the requested key** on a
  hit — it never trusts a blind 200 (so a mismatched record is rejected, not used).

**ASK:** if any route / header / status-code / body shape differs from the live
API, send me the authoritative contract and I'll align the client (it's behind a
trait — a one-file change). I'd rather match your real API than assume.

---

## 4. Hard constraints (please respect)

1. **Zero corelink-server changes.** Consume-as-customer only (governance law §8).
2. **Tenant isolated** from CoreLink's own runners / launch route (X6/X10 boundary —
   separate from the launch path; this tenant must not share fate with launch).
3. **Boundary, non-negotiable:** public-deterministic artifacts may share across
   tenants; **private bytes never cross**. (Inherited from CoreLink; stated so we're
   explicit.)
4. **Caps before keys** — please set the policy caps (2.4) *before* issuing the PAT,
   so the tenant is bounded the instant it can be used.

---

## 5. Delivery (what to hand back + exactly where)

Hand back three values; the first two are non-secret (tell me in chat / the
ticket), the third is the secret and goes on the **runner box only**:

| Value | Example | Where it goes on the hugit side |
|---|---|---|
| **Base URL** | `https://api.corelink.humangr.com` | env `HUGIT_CORELINK_AC_URL` (non-secret) |
| **Tenant slug** | `hugit` | env `HUGIT_CORELINK_TENANT` (non-secret) |
| **PAT** (Bearer) | `clp_…` | file `~/.hugit/secrets/corelink/pat` on `hugit-runner-01` (91.99.11.196), **mode 600, outside every repo** — mirrors the P1 GitHub-App secret pattern. **Never** put it in a repo, a commit, an env dump, or a log. |

(The runner box `hugit-runner-01` is already provisioned and online — P3 done. The
PAT lives there because that's where checks execute + where the hugit secrets
broker, C5b, will later take over from flat-file storage.)

**Placing the PAT on the box, with correct perms** (run on `hugit-runner-01`; do
NOT echo the PAT into shell history — paste it into the editor or use a here-doc
from a secure source):

```sh
install -d -m 700 ~/.hugit/secrets/corelink            # dir, owner-only
umask 177 && printf '%s' '<PAT>' > ~/.hugit/secrets/corelink/pat   # file mode 600
chmod 600 ~/.hugit/secrets/corelink/pat                 # belt-and-suspenders
# verify (prints the byte length only, never the value):
wc -c < ~/.hugit/secrets/corelink/pat
```

The loader trims a trailing newline, so `printf` (no newline) or a normal editor
save both work.

---

## 6. Definition of done (how we both confirm it works)

Once you've delivered the three values, the hugit side runs a **smoke test** that
proves the wiring end-to-end without polluting the cache:

1. `GET /v1/ac/{tenant}/{random-unused-digest}` with the PAT → expect **404
   (miss)**, NOT 401 → proves the PAT is valid + routing + tenant resolution work.
2. `PUT` a throwaway CheckResult then `GET` it → expect **200 (hit)** with the same
   bytes → proves read+write round-trip + memoization.
3. A deliberate cross-tenant probe (a digest under a different tenant slug) → expect
   **403** → proves isolation.

All three probes run in **one gated test** — `corelink_ac_live_smoke` in
`crates/hugit-checks/tests/corelink_ac_smoke.rs`. The exact command (run from the
runner box `hugit-runner-01`, where the PAT file lives):

```sh
export HUGIT_CORELINK_AC_URL="<base url you gave me>"
export HUGIT_CORELINK_TENANT="<tenant slug you gave me>"
# PAT already at ~/.hugit/secrets/corelink/pat (mode 600) — see §5
cargo test -p hugit-checks --test corelink_ac_smoke -- --nocapture
```

- **Success** = the test runs and the three probes pass (404 miss → 200 round-trip
  hit → 403 cross-tenant). That is P2 done; I flip the seams to live and run hugit's
  first self-hosted CI.
- **Before provisioning** (env/PAT absent) the same test **cleanly SKIPS** with a
  printed reason — so it can never go falsely green, and you can confirm right now
  that nothing is waiting on the hugit side. (I run this; just telling you what it
  does so the DoD is transparent.)

---

## 7. What is ALREADY done on the hugit side (so you know nothing is waiting on me)

- `HttpAcClient` (`crates/hugit-checks/src/client/ac.rs`) speaks the §3 contract
  behind a `HttpTransport` trait; the content-address guard (`verify_hit`) rejects a
  200 whose record keys to a different `action_digest`; fail-closed (`NotWired`) when
  unconfigured. Hermetically tested via a recording mock transport (`tests/ac_http.rs`).
- A config **loader** `corelink_ac_from_env()` / `HttpAcClient::from_runtime()` reads
  **exactly** these (the names are FROZEN consts in `ac.rs` — `ENV_AC_URL`,
  `ENV_TENANT`, `ENV_PAT`, `ENV_PAT_FILE`, `DEFAULT_PAT_FILE_REL`, so this doc and the
  code cannot drift):
  - `HUGIT_CORELINK_AC_URL`, `HUGIT_CORELINK_TENANT`,
  - PAT from `~/.hugit/secrets/corelink/pat` (preferred) → else env `HUGIT_CORELINK_PAT`
    (override the file path with `HUGIT_CORELINK_PAT_FILE` for tests only).
  It returns `NotConfigured` naming **which** piece is missing; the PAT is never
  rendered in `Debug`/`Display`/errors (covered by a redaction test).
- The **smoke test** `corelink_ac_live_smoke` (`tests/corelink_ac_smoke.rs`) is wired
  and **gated** to run only when the env + PAT are all present — otherwise it SKIPS
  with a printed reason (so it can't rot to green before you deliver).
- Everything merges to `main` green by local cold-verify (fmt + clippy
  `--workspace --all-targets --locked -D warnings` + test `--workspace --locked` +
  `cargo audit`); GitHub Actions cloud CI is quota-paused on our side — unrelated to
  this request.

When the three values land, this goes live with **no further hugit code change** —
just configuration. Ping me and I'll run §6 and report.
