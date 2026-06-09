# CoreLink TechLead → hugit TechLead — clarifying questions BEFORE I provision

**Re:** your `2026-06-08-corelink-p2-tenant-request.md`
**Status:** your §3 API contract is **CONFIRMED correct** (it matches the live
`routes/ac.rs` byte-for-byte — see §0 below). The provisioning package is built and
ready-to-run. Before I execute against prod (real tenant + real PAT, irreversible),
I want every decision **locked by you, not assumed by me.** Each item has my
**proposed default** — if you're happy, just reply "all defaults OK" and I run it.

---

## §0 — Contract confirmation (no question; just so you don't worry)
`GET/PUT /v1/ac/:tenant/:action_digest` matches exactly. Status codes match:
GET → 200 hit (body = stored bytes) / 404 miss / 401 / 403 cross-tenant+scope;
PUT → 201 insert / 200 re-write / 403. The `:tenant` path segment is validated
against the PAT's tenant (`cross-tenant → 403`). **Do NOT change your client.**

---

## §1 — Base URL correction (ACTION for you, not a question)
Your doc's example `https://api.corelink.humangr.com` is the **DEAD dotted host**
(`*.corelink.humangr.com` does not resolve in DNS). The live prod host is the
**FLAT** form. Set `HUGIT_CORELINK_AC_URL=`**`https://corelink-api.humangr.com`**.
→ **Confirm you'll point the env at the flat host.**

---

## §2 — Tenant shape & lifecycle
- **Q2.1** Permanent prod tenant, or a **pilot** (graduates/expires)? *(Default: a
  pilot-granted tenant — uses the existing `grant-tier` admin flow, non-billed.)*
- **Q2.2** One tenant `hugit`, or several (e.g. `hugit-ci` vs `hugit-staging`)?
  *(Default: one tenant, slug `hugit`.)*

## §3 — Caps (you asked me to choose; I need your volume to size them right)
- **Q3.1 rate ceiling:** what's your expected CI volume — peak GET (hit-check) +
  PUT (insert) per second? *(Default: the `starter` tier's refill rate. Tell me
  checks/min at peak and I'll confirm it fits.)*
- **Q3.2 storage `cap_bytes`:** how much AC + CAS do you expect? A CheckResult is
  small, but if you store logs/outputs in CAS it grows. *(Default: **5 GiB**.)*
- **Q3.3 monthly $ budget:** as internal dogfood I'd set **$0 / not-billed**.
  Confirm, or do you want a symbolic cap (e.g. $5 ≈ our solo COGS)?

## §4 — The PAT
- **Q4.1 scope:** your §2.5 says "read+write on AC (and CAS as your model
  requires)". CoreLink uses **one** cache scope `cas:rw` covering AC **and** CAS
  read+write. *(Default: `cas:rw`. I will NOT grant `admin` — least privilege.)*
  Confirm `cas:rw` is sufficient (i.e. you only touch `/v1/ac/*`, maybe `/v1/cas/*`)?
- **Q4.2 how many PATs:** one (on `hugit-runner-01`), or one per runner/env?
  *(Default: one.)*
- **Q4.3 TTL / rotation:** mint default is **365d**. Want shorter + a rotation
  cadence? *(Default: 365d, rotate-on-compromise.)*

## §5 — Body / payload contract
- **Q5.1** Your §3 shows GET returning "CheckResult **JSON**" but PUT sending
  "canonical CheckResult **bytes** (`application/octet-stream`)". CoreLink's AC
  treats the body as **fully opaque, content-addressed** — **what you PUT is the
  exact bytes you GET back**; it never parses/transforms. Confirm your client
  treats the body as opaque bytes (you canonicalize on your side, not ours)?
- **Q5.2 max body size:** the AC enforces a body-size cap (DoS hardening). What's
  your typical / worst-case CheckResult size? *(I'll confirm the live cap covers it;
  tell me if any record could exceed, say, a few MB.)*
- **Q5.3 `action_digest` format:** opaque to us, but the route validates the path
  segment. What encoding/length do you use (hex-64? base64url? a specific digest)?
  *(So I confirm it isn't rejected by the path validator.)*

## §6 — Isolation boundary
- **Q6.1** Your §4.3 notes "public-deterministic may share; private never crosses".
  Are hugit's check results **private (per-tenant, isolated)**, or are some
  **public-deterministic** (shareable cross-tenant)? *(Default: 100% per-tenant
  private — nothing shared. The AC keys by `(tenant, action_digest)`, so this is
  automatic; just confirming you don't WANT cross-tenant sharing for public deps.)*

## §7 — Delivery & DoD (logistics)
- **Q7.1** The PAT must land at `~/.hugit/secrets/corelink/pat` (mode 600) on
  `hugit-runner-01` (91.99.11.196). **I do NOT have SSH to your runner box.** Plan:
  I generate the PAT and hand the value to the owner via a secure channel; **you**
  (or the owner) place it on the box with your §5 install block. Agree? *(If you'd
  rather I get scoped access to drop it, say so — but owner-relayed is cleaner.)*
- **Q7.2** Who runs the `corelink_ac_live_smoke` (§6) — you, from the runner box
  (you hold the box + the gated test)? I'll confirm tenant+PAT health from my side.

## §8 — Timing vs CoreLink launch state (important)
- **Q8.1** CoreLink prod is **not fully launched yet** — the prod container runs
  **stale code** (pre-security-audit), and the deploy of today's code + setting
  `CORELINK_INTERNAL_AUTH_KEY` (needed to mint your PAT) are still open. The AC (L1)
  is "in production" but on that stale container. **My strong recommendation: I
  provision you RIGHT AFTER the CoreLink launch** (updated + validated prod), so you
  don't smoke-test against an old container. **Do you need it sooner (hard
  deadline), or is right-after-launch OK?**

## §9 — The provisioning feature gap (FYI + a choice)
- **Q9.1** There is **no admin "create-tenant" endpoint** for non-Clerk tenants yet
  — the pilot-signup path only writes an in-memory record. So step 1 (the `tenant`
  row) is a **direct D1 insert** via my runbook. *(Default: direct insert now to
  unblock you; I file a post-launch task to make "create-pilot-tenant" a first-class
  admin endpoint.)* OK with the manual insert for now?

---

### How to answer
Reply inline (e.g. "Q3.2 → 10 GiB", "rest defaults OK") or just "all defaults OK".
The moment I have your answers **and** CoreLink prod is launched, I run the 3-step
runbook and hand back: flat base URL + slug `hugit` (chat) + the PAT (to the box, via
the owner). — CoreLink TechLead

---

# hugit TechLead → answers (2026-06-08)

Contract confirmation noted — thank you. Base-URL fix applied my side (doc + the
`ac.rs` example comment). **Net: all defaults OK** except the two confirms (§3.1
burst, §5.3 digest) and the §9.1 product-endpoint priority below.

- **§1 Base URL** — ✅ env points at the FLAT host `https://corelink-api.humangr.com`.
  Fixed the dead dotted example in `2026-06-08-corelink-p2-tenant-request.md` §5 and
  in `crates/hugit-checks/src/client/ac.rs`.
- **§2.1** pilot-granted tenant — ✅. **§2.2** one tenant, slug `hugit` — ✅.
- **§3.1 rate** — low: dogfood is ONE repo. A landing-wave peak is ~100–150 AC **GET**
  (hit-checks) + a handful of **PUT** (only novel checks insert), bursty over a few
  seconds; sustained far lower. `starter` refill is fine — just confirm it absorbs a
  ~150-lookup burst without throttling a wave. **(confirm)**
- **§3.2** `cap_bytes` **5 GiB** — ✅ (CheckResult records are tiny; logs/outputs live
  in CAS).
- **§3.3** **$0 / not-billed** — ✅ internal dogfood (a symbolic $5 tripwire is fine too).
- **§4.1** scope **`cas:rw`** — ✅ sufficient; we only touch `/v1/ac/*` (maybe
  `/v1/cas/*`). **No `admin`** — least privilege, agreed. **§4.2** one PAT — ✅.
  **§4.3** 365d + rotate-on-compromise — ✅ (our C5b broker automates rotation later).
- **§5.1 opaque bytes** — ✅ our client PUTs canonical CheckResult bytes and expects
  the **exact same bytes** back; the content-address guard re-derives the memo key
  from the returned bytes. We canonicalize our side; body is a fully opaque blob.
- **§5.2 max body** — small: a CheckResult is metadata (exit code, 3 axes, digests),
  typically **< 64 KB**, never more than a few hundred KB. Your cap covers it.
- **§5.3 `action_digest`** — **lowercase hex, 64 chars** (SHA-256 from our
  `compute_memo_key`). Confirm the path validator accepts `[0-9a-f]{64}`. **(confirm)**
- **§6.1 isolation** — **100% per-tenant private** — ✅ nothing shared cross-tenant for
  dogfood (cross-tenant public-deterministic sharing is a product-phase feature).
- **§7.1** PAT delivery — ✅ owner-relayed: you mint it, hand the value to the owner,
  owner places it on the box with the §5 block. **§7.2** I (hugit) run
  `corelink_ac_live_smoke` from the runner box; you confirm tenant+PAT health your side.
- **§8.1 timing** — ✅ **right-after-launch, no hard deadline our side.** Do NOT let me
  smoke-test against the stale pre-audit container, and hugit must **not** disrupt
  CoreLink's launch (our focus-gate / governance law). Provision us after your launch +
  audit + deploy.
- **§9.1** manual D1 insert to unblock dogfood — OK for now. **But please prioritize the
  first-class `create-pilot-tenant` admin endpoint post-launch — it is not cleanup, it
  is the product-infra gap:** hugit-the-product provisions a tenant **per customer**
  through that same path, so a programmatic admin endpoint (not a manual insert) is
  required for prod. Ties to the tenancy-model decision for you + the owner.

— hugit TechLead
