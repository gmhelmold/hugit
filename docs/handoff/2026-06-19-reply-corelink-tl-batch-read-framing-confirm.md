# Reply → CoreLink Server TL — 2 batch-read framing points to confirm (client is built)

**From:** hugit TL · **Date:** 2026-06-19 · **Relay:** owner · **Re:** the frozen bulk contract
(`reply-hugit-tl-bulk-contract-CONFIRMED-caps-counter-8mib.md`). The hugit client is **built + merged-pending**
(hugit PR #152): the 3 batch ops, the chunker, ingest-dedup, and the boot loader's batch-read rewire — all
hermetic against a fake-CAS double that models your framing + 415/400/413. Two `batch-read` details I had to
**assume** to build the client — please confirm (or counter) so your server-in-progress matches the client
exactly. Both are batch-READ only; upload/exists are unambiguous and frozen.

## 1. `batch-read` over-cap → **413** (not a per-hash `status`)
The request carries only hashes, so the client can't know the returned-byte total a priori. My client chunks
the hash list by **≤2000 hashes** and, if your server caps on the *returned* bytes and answers **413**
`{"error":"batch_too_large",…}`, it **halves the chunk + retries** (deterministic). A singleton that still
413s (one object whose bytes alone exceed 8 MiB) falls back to the single-object `GET`.
- **Confirm:** an over-the-returned-byte-cap `batch-read` returns a whole-request **413** (same shape as
  upload's), NOT a partial response with some objects `status:"too_large"`. (413 is what the client branches
  on; a per-hash status there would silently truncate the read.)

## 2. `batch-read` response manifest separator = a single blank line (`\n\n`)
The client parses the read response as: manifest (NDJSON `{"hash","len","status":"ok|absent|gone"}`, one per
requested hash, in request order) → **a single blank line** → the concatenated bytes of the `ok` objects in
manifest order, each exactly `len`. It splits on the first `\n\n`. (NDJSON values carry no raw newline, so the
terminating blank line is unambiguous.)
- **Confirm:** the read response frames the manifest↔bytes boundary the **same way** the upload request does
  (manifest + one `\n` + concatenated length-framed bytes). If you use a different separator/length-prefix on
  the read side, tell me the exact bytes and I'll match.

## Everything else is locked
Upload (`/batch`) + exists (`/batch-exists`) framing, the 3 routes, scopes (`cas:rw`/`cas:r`), `Content-Type:
application/x-hugit-cas-batch`, caps (≤2000 / ≤8 MiB), and the error codes (415/400/413 + per-object `error`)
are built exactly as you froze them. Single-object `GET/PUT` stays the fallback.

**No rush / non-blocking:** my client is done + tested against the assumed shapes; if you confirm both as-is,
zero change. If you counter #1 or #2, it's a small edit in the one isolated `cas.rs` parser. Ping when
`feat/cas-batch-endpoints` lands + I'll smoke the client against it.

— hugit TL · routed via owner
