# Wave 2 — infra unblock asks (consolidated, 2026-06-20)

**From:** hugit TL · **To:** owner (relay) → CoreLink Server TL + CoreLink Runners TL.
Current state: engine LIVE + security-hardened (`cc4eed0`), lazy git-from-CAS, `/v1` read+write + the
wedge porcelain (`hugit checks`/`queue show`) all live for the `hugit` repo. The code for everything
below is BUILT (hermetic) — these are infra/provisioning gates, not code.

## 1. CoreLink P2 tenant — hot CAS + AC live (CoreLink Server TL)
Unblocks the economic thesis (memoized checks at near-zero cost, global dedup) — today the AC client
(`crates/hugit-checks/src/client/ac.rs`) is a real `ureq` client with no live tenant, so every check MISSES.
**Need:** a P2 tenant namespace + a `cas:rw`/`ac` PAT scoped to it. The AC client is plug-and-play once
the PAT + tenant id exist (one env var). Prior ask: `docs/handoff/2026-06-08-corelink-p2-tenant-request.md` —
this supersedes it with the engine now live. **Deliverable to me:** tenant id + PAT (out-of-band, into
`~/.hugit/secrets/corelink/`); I wire + smoke.

## 2. Identity — finish the Clerk → multi-tenant path (owner + CoreLink)
The `/v1/token` Clerk→engine-token exchange is WIRED + live (`HUGIT_SESSION_EXCHANGE_URL` set; the CoreLink
`/v1/session/exchange` endpoint is live; `/v1/token` responds, not 404). **Remaining:**
- **`hugit-prod-d1`** (the engine token store) — provision the D1 (owner/CF). Until then the in-process token
  store is per-instance only.
- **Multi-tenant Clerk** — the githugr window's Clerk session → real per-tenant principals (githugr TL's
  frontend flip + the owner's Clerk org config). Then the engine's per-tenant authz gates real users.
**Deliverable:** `hugit-prod-d1` provisioned + the Clerk org/JWKS confirmed; I verify the authed read/write path.

## 3. Runner fabric live (CoreLink Runners TL)
Unblocks real check EXECUTION, merge-as-re-execution dispatch, and the landing queue EXECUTE (today
`write_dispatch` records the demand but never spawns; `land` is reserved). The runner code is in
`../corelink-runners`; hugit holds only the consumer seam + the frozen conformance contract.
**Need:** a live runner endpoint (`HUGIT_RUNNER_HOST`) + the spawn/lease PAT, per the frozen
`corelink-runners/docs/spec/hugit-integration-contract.md v1.2.0`. **Deliverable:** the endpoint + PAT;
I wire `HUGIT_RUNNER_HOST` + run the conformance probes.

## Sequencing (highest leverage first)
P2 tenant (item 1) is the single biggest unlock — it makes the economic core (memoized CI) a live demo,
and is the cheapest (one PAT). Then identity (item 2) for real users, then the runner fabric (item 3) for
EXECUTE. Items 1+2 are days; item 3 is a corelink-runners go-live milestone.

— hugit TL · routed via owner
