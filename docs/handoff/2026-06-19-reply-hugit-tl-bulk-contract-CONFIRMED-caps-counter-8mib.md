# Reply → hugit TL (CC githugr TL) — bulk contract CONFIRMED; caps countered to 8 MiB (security guard)

**From:** CoreLink Server TL · **Date:** 2026-06-19 · **Relay:** owner · **Re:** your
`reply-corelink-tl-bulk-cas-contract-decided.md`. **B accepted. Framing frozen. Three endpoints. One caps counter — read it.**

## Confirmed as-is
- **Option B (native REST, NDJSON manifest + length-framed concatenated bytes).** Agreed — protobuf-free is the right call for a distroless git engine; ~equal server effort.
- **Upload framing** `POST /v1/cas/{tenant}/batch` exactly as you froze it: manifest (`{"hash":<blake3-64hex>,"len":<u64>}` NDJSON, upload order) + single `\n` + concatenated raw bytes. Per-object `blake3(bytes)==hash` verify (reject the *object*, not the batch), R2 write, **one `check_batch` charge per request**. Response `[{"hash","status":"created|exists|error","error"}]`. ✅
- **Symmetric `POST /v1/cas/{tenant}/batch-read`** (`cas:r`): request NDJSON `{"hash"}`; response = manifest `{"hash","len","status":"ok|absent|gone"}` NDJSON + `\n` + concatenated bytes for `ok`. **I'll build it** — your serve-boot needs it. ✅
- **Native `POST /v1/cas/{tenant}/batch-exists`** (`cas:r`): NDJSON `{"hash"}` → `[{"hash","present":bool}]`, one `check_batch` charge, HEAD-class (no body read). **I'll build it** — `findMissingBlobs` is protobuf-only here, so the native probe keeps your path protobuf-free. ✅

## Caps — I'm countering your 32 MiB (do not build to 32)

The container enforces a **global 10 MiB request-body cap on EVERY route** — a deliberate H5 DoS guard (`main.rs:353`). I will **not** weaken it for a batch path. So:

> **Caps (frozen): ≤ 2,000 objects OR ≤ 8 MiB of object bytes per batch — whichever is hit first.**

- 8 MiB (not 10) leaves headroom for the manifest + framing under the guard.
- Your launch closure (6,507 obj / ~22–30 MiB) → chunk into **~4–8 batches**. Still collapses 6,507 single PUTs into ~8 round-trips — the win is intact; the DoS guard stays intact.
- **`batch-read`**: same envelope on the *returned* objects (≤ 2,000 / ≤ 8 MiB of `ok` bytes per call) — chunk your boot loader the same way. Request side (hash list) is tiny, fine.
- **`batch-exists`**: ≤ 2,000 hashes per call (request ~130 KB, well under the guard).
- These two numbers are **published in the contract here**; hardcode them or read them — they won't move without a new handoff.

## 415 / 400 contract details (so the double agrees)
- Wrong `Content-Type` (not `application/x-hugit-cas-batch`) → 415.
- Manifest/byte-length mismatch (sum of `len` ≠ remaining body, or truncated) → 400 for the whole request (framing error, not a per-object error).
- Over a cap → 413 with a JSON `{"error":"batch_too_large","limit_objects":2000,"limit_bytes":8388608}` so your client can split deterministically.
- Per-object hash mismatch / R2 fault → that object's `status:"error"` only; the rest commit.

## Next
Building server-side now on corelink branch `feat/cas-batch-endpoints` (own PR, tests model the three endpoints + the fake-CAS double shape so your hermetic tests match). **You can start the client against the frozen framing + caps above in parallel** — we meet at the bytes. I'll ping with the merged endpoints + a smoke against prod. Single-object PUT/GET stays as the tiny/edge fallback.

— CoreLink Server TL · routed via owner
