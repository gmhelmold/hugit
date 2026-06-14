# hugit → CoreLink: engine-storage R2 — ACK + access choice + credential request

**From:** hugit techlead · **To:** CoreLink techlead · **Via:** owner · **Date:** 2026-06-14 ·
**In response to:** `corelink-server` PR #265 `docs/handoff/2026-06-13-corelink-to-hugit-engine-storage.md`.

## ACK
Received + agreed on the boundaries: CoreLink owns the bucket + scoped credential +
key contract (`<tenant_id>/<repo>.json`, tenant_id = Clerk `publicMetadata.tenant_id`
UUID, no TDK); hugit owns the `<repo>.json` format + the snapshot write + the read
path in `state.rs`; githugr just displays. The dedicated `corelink-githugr-engine`
bucket (NOT the per-tenant CAS) is the launch read source; real CAS via seam-C
session-exchange + TDK stays P2. Runner/CI read path is phase-3 (checks stays
fixture). All correct.

## Access choice → **Option A (engine reads R2-S3 directly)**
hugit will read `<tenant_id>/<repo>.json` from `corelink-githugr-engine` directly
over the R2 S3 API, **not** via a fronting Worker. Rationale: the hugit workspace
already pins `hmac` + `sha2` + `ureq`, so **AWS SigV4 request signing is hand-rollable
with zero new dependency** (an HMAC-SHA256 key-derivation chain) — keeping the
supply-chain-strict, sync, async-free engine intact. This makes the engine
**self-contained end-to-end** (owns its read path, no proxy component to operate),
which is the cleaner headless-engine shape than a Worker hop. Option B's only
advantage (zero engine dep) is moot once the dep is zero anyway.

## What hugit will build (our lane — buildable now, live on credential)
1. A `LogSource` in `hugit-serve/state.rs`: `Local { dir }` (today) | `R2 { endpoint,
   bucket, key_id, secret }`. Given the authenticated `tenant_id` + `repo`, GET
   `<tenant_id>/<repo>.json` over S3 (SigV4). Fail-honest preserved: 404 → home page
   / honest defaults, never fabricated (already our contract); a transport/5xx fault
   → 503.
2. A snapshot export: `hugit` writes its canonical event-log `<repo>.json` to
   `<tenant_id>/<repo>.json` (we own the format) — the first real object that takes
   `engine.githugr.com` out of fixture.

Both are buildable against fixtures now; live activation needs the credential below.

## Credential request (owner-gated)
Please generate + send (via the owner's secure channel) the **read+write S3 key
scoped ONLY to `corelink-githugr-engine`** (least-privilege, NOT account-wide):
- `R2_ACCESS_KEY_ID`
- `R2_SECRET_ACCESS_KEY`
- `account_id` (for the endpoint `https://<account_id>.r2.cloudflarestorage.com`)

The engine takes these as env (`HUGIT_SERVE_R2_*`), alongside the existing
`HUGIT_SERVE_LOG_DIR` (which stays the default `Local` source for dev/test).

— routed via owner; no `path`/`git` dependency between repos.
