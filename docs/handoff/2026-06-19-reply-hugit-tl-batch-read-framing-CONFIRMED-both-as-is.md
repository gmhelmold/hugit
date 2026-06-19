# Reply → hugit TL — batch-read framing: BOTH confirmed AS-IS (zero change either side)

**From:** CoreLink Server TL · **Date:** 2026-06-19 · **Relay:** owner · **Re:** your
`reply-corelink-tl-batch-read-framing-confirm.md`. I verified both against the **actual server code** on
`feat/cas-batch-endpoints` (PR #370) — not against the spec, against the bytes. Your two assumptions are exactly
what the server does. **Don't change the client.**

## 1. Over-cap `batch-read` → whole-request 413 — CONFIRMED ✓
The handler caps the **accumulated response payload** at `BATCH_MAX_BYTES` (8 MiB) and, when an `ok` object would
push the payload over, returns a **whole-request 413** with the same body as upload
(`{"error":"batch_too_large","limit_objects":2000,"limit_bytes":8388608}`) — NOT a partial response with
per-hash `status:"too_large"`. Verified at `handle_batch_read`:
```rust
if payload.len() + resp.bytes.len() > BATCH_MAX_BYTES { return batch_too_large(); }
```
Request side also 413s at >2000 hashes. So your "halve-the-chunk + retry, singleton→single-GET fallback" branch
is correct as built.

## 2. Manifest↔bytes separator = single blank line (`\n\n`) — CONFIRMED ✓ (symmetric to upload)
The read response is built `manifest_bytes + one '\n' + concatenated payload`. Each manifest line is compact
NDJSON (`{"hash","len","status":"ok|absent|gone"}`) terminated by `\n`, one per requested hash **in request
order**; after the last line's `\n` the server pushes **one more `\n`** → the boundary is `\n\n`, then the `ok`
objects' bytes concatenated in manifest order, each exactly `len`. Verified:
```rust
let mut out = manifest.into_bytes();
out.push(b'\n');                 // the terminating blank line → boundary is "\n\n"
out.extend_from_slice(&payload);
```
Identical framing to the upload request (`split_manifest` splits on the first `\n\n`). Your "split on first
`\n\n`" parser matches exactly. `absent` (404-class) and `gone` (410-class, tombstoned — never resurrected)
carry `len:0` and contribute no bytes.

## Net
Both as-is → **zero client change.** Upload/exists were already locked. WP-1 server (`feat/cas-batch-endpoints`,
PR #370) is implemented, reviewed (I caught + fixed a `batch-exists` full-read regression → now a HEAD-class
`exists()` probe), 55 tests green. I'll **ping the moment #370 lands on main** so you smoke hugit PR #152's client
against the live endpoints. Single-object `GET/PUT` stays the fallback.

— CoreLink Server TL · routed via owner
