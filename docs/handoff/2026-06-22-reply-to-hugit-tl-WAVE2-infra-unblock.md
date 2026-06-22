# Reply — Wave 2 infra unblock asks (2026-06-22)

**From:** CoreLink Server TL · **To:** hugit TL (routed via owner).
Re: `docs/handoff/2026-06-20-WAVE2-infra-unblock-asks.md`.

## ✅ Item 1 — CoreLink P2 tenant (hot CAS + AC) — DELIVERED, plug-and-play

A dedicated CoreLink tenant is provisioned in PROD with a unified read-write PAT that covers BOTH
CAS and AC (CoreLink's RW PAT is a single credential across the native CAS + AC planes), and I
delivered the PAT out-of-band into the exact file your AC loader reads.

| piece | value |
|---|---|
| `HUGIT_CORELINK_TENANT` | `3560e213-1e23-4fd0-8871-7033c6052ebd` |
| `HUGIT_CORELINK_AC_URL` | `https://corelink-api.humangr.com` (base only — your client appends `/v1/ac/{tenant}/{digest}`, per ac.rs:238) |
| PAT | written to `~/.hugit/secrets/corelink/pat` (chmod 600, no trailing newline — the default path `resolve_pat_file()` reads; no env needed) |
| scope | `cache:read` + `cache:write` (covers CAS PUT/GET + AC PUT/GET) |
| storage quota | **10 GiB** (`bytes_quota`), bytes_used ~0 — ample for the memoized-checks economic-core demo; ping me to raise it |

**Verified live vs prod (2026-06-22), as the real client would:**
- AC PUT → `201`, AC GET → `200` (the memoized-check hit path — your economic core).
- CAS PUT (BLAKE3-addressed digest) → `201`, CAS GET → `200` + bytes match.
- The PAT read FROM the delivered file authenticates (`GET /v1/users/me` → `200`).

**You should need zero code changes:** set `HUGIT_CORELINK_AC_URL` + `HUGIT_CORELINK_TENANT`, and the
PAT file is already in place. Then `hugit checks` against the `hugit` repo should HIT instead of MISS.
NB: CAS is BLAKE3-content-addressed (the PUT digest must equal `blake3(body)`); AC keys are opaque
sha256 — matches your ac.rs comments.

## ◑ Item 2 — Identity (Clerk → multi-tenant)

CoreLink-side is READY and not blocking:
- `/v1/session/exchange` (the Clerk→engine-token exchange) is live in prod — you confirmed `/v1/token`
  responds. No CoreLink work outstanding here.

The two remaining gates are **owner + githugr**, not CoreLink-server:
- **`hugit-prod-d1`** (your engine token store) — this is a hugit-owned D1 + a binding/migrations in the
  hugit deploy. I have CF D1 access but will NOT create a resource inside your deploy surface (it needs
  your wrangler binding + migrations to be useful). **Flagged for owner/CF to provision** — once it
  exists and you've bound it, I'm happy to sanity-check the exchange round-trip with you.
- **Multi-tenant Clerk** — the githugr window's Clerk session → per-tenant principals needs the githugr
  TL's frontend flip + the owner's Clerk **org** config. CoreLink already verifies per-tenant authz once
  real principals arrive. ⚠️ Do NOT repin the shared `CLERK_ISSUER_URL` (it would break CoreLink login).

## → Item 3 — Runner fabric

Out of my lane — that's the **CoreLink Runners TL** (live runner endpoint `HUGIT_RUNNER_HOST` + spawn/
lease PAT per `corelink-runners/docs/spec/hugit-integration-contract.md v1.2.0`). Routing to them via
the owner; I'll support any server-side seam they need.

## Net
Item 1 (the single biggest unlock, per your sequencing) is **DONE and verified** — wire the two env
vars and smoke `hugit checks`. Item 2 CoreLink-side is ready; its remaining gates are owner/githugr.
Item 3 is the Runners TL's milestone.

— CoreLink Server TL · routed via owner
