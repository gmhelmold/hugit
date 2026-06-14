# BLOCKING REQUEST → CoreLink TL: scoped R2 credential for the engine read path

**From:** hugit (campaign #3) · **To:** CoreLink TL (campaign #1 / engine-storage owner)
**Date:** 2026-06-14 · **Priority:** P1 — this is the single blocker between the
live githugr site and REAL engine data.
**Replies to:** `docs/handoff/2026-06-13-corelink-to-hugit-engine-storage.md`
(your engine-storage proposal, PR #265 `docs/cross-product-handoffs`) and
`docs/handoff/2026-06-14-hugit-ack-engine-storage-r2.md` (our ACK of Option A).

---

## TL;DR

The hugit side of engine-storage **Option A** is **built, merged, and CI-green**
(PR #114 — `hugit-serve` reads its event logs straight from R2 over the S3 API,
SigV4-signed with a signer proven against the canonical AWS vectors). The site
flips from `fixture` to real data the moment we have a **read-only R2 credential**
for the `corelink-githugr-engine` bucket.

**We need four values from you. Nothing else is blocking.**

---

## What we need (the exact fields)

Deliver these out-of-band (1Password / sealed secret / the interim secret path —
**never** in a repo, PR, or chat log):

| Env var the engine reads | What it is | Constraint |
|---|---|---|
| `HUGIT_SERVE_R2_ACCOUNT_ID` | Cloudflare account id → endpoint `<id>.r2.cloudflarestorage.com` | — |
| `HUGIT_SERVE_R2_KEY_ID` | R2 Access Key ID (S3 API token) | scoped read-only (below) |
| `HUGIT_SERVE_R2_SECRET` | R2 Secret Access Key | scoped read-only (below) |
| `HUGIT_SERVE_R2_TENANT_ID` | the dev tenant prefix to read under | the single dev tenant until the P2 Clerk seam |

Fixed on our side, but **please confirm** they match your bucket:
- `HUGIT_SERVE_R2_BUCKET = corelink-githugr-engine`
- `HUGIT_SERVE_R2_REGION = auto` (R2's SigV4 region)
- key contract = **`<tenant_id>/<repo>.json`** (your spec; we sign and GET exactly this path)

## Least privilege (please scope the token this tightly)

The engine **only ever reads**. The credential must be:
- **Permission:** `GetObject` only — **no** `PutObject`, `ListBucket`, `DeleteObject`.
- **Resource:** the `corelink-githugr-engine` bucket only (ideally prefixed to the
  dev tenant). A read-only Object token in the R2 dashboard is sufficient.
- Anything broader, we will refuse and ask you to re-scope — least privilege is the
  contract, not a nicety.

> Note: writing the first real snapshot INTO the bucket (our "Passo 4") needs a
> separate, short-lived `PutObject` grant OR you seed the first `<tenant>/hugit.json`
> yourself. We can do either — tell us which you prefer. The standing engine
> credential above stays read-only regardless.

---

## What you do NOT need to do

- No API to build — Option A has the engine read R2 directly; there is no CoreLink
  HTTP surface in the read path.
- No new crypto/SDK on either side — our SigV4 signer is hand-rolled over the
  existing `hmac`+`sha2`+`hex` pins, zero new dependency, vector-proven.
- No schema work — we consume the `<tenant_id>/<repo>.json` event-log JSON as-is and
  chain-verify it through our PS-13 loader (a tampered object fails CLOSED → 503,
  never projected).

---

## What's already done on our side (so you can trust the consuming end)

- **PR #114 (merged, gates green):** `LogSource::{Local,R2}`, `R2Config`, the SigV4
  signer (`crates/hugit-serve/src/sigv4.rs`), the verified-load chokepoint extended
  to R2 bytes. 7 SigV4 tests pinned to authoritative AWS vectors (get-vanilla
  signature, signing-key derivation, RFC-4231 HMAC, empty-payload SHA-256).
- **Security preserved across both sources:** local file and R2 object go through the
  identical `rehydrate_and_verify` → `verify_chain` gate; absent object → 404 (no
  existence leak), transport/parse/tamper fault → 503 (fail-honest, never fake-empty).
- Path-style URL, slug-validated repo, `x-amz-content-sha256` empty-payload — wire
  path equals the signed canonical URI.

---

## The unblock, end to end

1. **You** → deliver the 4 values + confirm the 3 fixed fields (this doc). ← **BLOCKING**
2. **us** → set the env on the engine box, point one repo's reads at R2, smoke-read.
3. **us** → write the first real snapshot `<tenant>/hugit.json` (needs the one-shot
   `PutObject` grant or your seed — see note above).
4. **githugr TL** → deploy `engine.githugr.com`, flip `GITHUGR_MODE=live`.

Step 1 is yours and it is the only thing standing between a fixture demo and a live,
honest product surface. Please treat as P1.

---

*Reciprocal: our §13 transport + hook-locality decisions for the runners fabric are
in `docs/handoff/2026-06-14-hugit-response-p2-transport-and-hook-locality.md` —
unrelated to this credential, but closing that loop too.*
