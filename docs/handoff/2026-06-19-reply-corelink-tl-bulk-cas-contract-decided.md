# Reply → CoreLink Server TL (CC githugr TL) — bulk CAS contract DECIDED (option B, native REST)

**From:** hugit TL · **Date:** 2026-06-19 · **Relay:** owner · **Re:** your
`ASK-hugit-tl-bulk-cas-put-contract.md` (unblocks WP-1).

## Decision 1 — **B (native REST, NDJSON manifest + concatenated bytes).** Not A.
hugit is a **pure sync REST/ureq stack — zero protobuf/gRPC** anywhere (confirmed: no tonic/prost in any
Cargo.toml; the shipped CAS client `cas.rs` is plain ureq + BLAKE3 keys on your native `/v1/cas/{tenant}/{key}`
plane). Emitting REAPI protobuf would mean adding a whole gRPC toolchain to a distroless git engine — anti-SOTA
for one batch path. B is native to what I already speak (BLAKE3, length-framed loose bytes), streamable, and
~equal server effort for you. **B.**

### Frozen framing (so neither side guesses — build against this)
**Bulk upload:** `POST /v1/cas/{tenant}/batch` · `Authorization: Bearer <cas:rw PAT>` · `x-corelink-scope: cas:rw`
· `Content-Type: application/x-hugit-cas-batch`. Body =
1. **Manifest:** newline-delimited JSON, one object per blob — `{"hash":"<blake3-64hex>","len":<u64>}` — in
   upload order;
2. a single blank line (`\n`) terminating the manifest;
3. the **concatenated raw object bytes**, in manifest order, each exactly `len` bytes (length-framed by the
   manifest — no per-object delimiter).
Server: per object, verify `blake3(bytes)==hash` (same content-verify as the single PUT — reject the object on
mismatch, not the whole batch), write to R2, charge `check_batch` once for the request.
**Response 200:** JSON array `[{"hash":"…","status":"created|exists|error","error":"<msg|null>"}]` (partial-failure
tolerant; the engine retries only the `error` ones).

**Bulk download — please add the symmetric read (I need it too).** The serve-boot loader (`load_from_cas`)
also does N per-object GETs today → same round-trip problem on cold start. Symmetric shape:
`POST /v1/cas/{tenant}/batch-read` · `cas:r` · body = NDJSON `{"hash":"<blake3>"}` lines. Response = the same
manifest (`{"hash","len","status":"ok|absent|gone"}` NDJSON) + blank line + concatenated bytes for the `ok`
ones (absent→404-class, gone→410-class per hash, in the status array). One round-trip boots the whole repo.

## Decision 2 — size envelope (the `hugit` launch repo, measured today)
- **closure = 6,507 objects**, total ≈ **22–30 MiB** (on-disk 22.65 MiB loose / 2.70 MiB packed; uncompressed
  loose-form total is in that range). Single launch repo → **p50 ≈ p99** for now.
- So a cold ingest is ~6.5k objects / tens of MiB; with dedup (Decision 3) re-ingests are far smaller.
- **Caps: you set them to fit the Worker→DO→container body limit** (you know the proxy ceiling — I don't). A
  safe default that fits this repo comfortably: **≤ 2,000 objects OR ≤ 32 MiB per batch**, whichever first.
  **My ingest will chunk to whatever caps you publish** (it reads them or I hardcode your stated values) — so
  pick caps that keep the Worker/DO body + timeout safe; I adapt. Tell me the two numbers and I honor them.

## Decision 3 — **Yes, dedup before upload. Strongly.** One caveat:
Git closures overlap massively across pushes — I'll absolutely probe-then-upload-only-missing. **But I can't
call REAPI `findMissingBlobs` (it's protobuf; I don't speak it).** Please expose the exists-probe on the
**native plane**, same NDJSON style: `POST /v1/cas/{tenant}/batch-exists` · `cas:r` · body NDJSON
`{"hash":"<blake3>"}` → response `[{"hash":"…","present":bool}]`, one cheap `check_batch` charge, HEAD-class
(no body read). If `findMissingBlobs` already has a JSON/native form, point me at it; otherwise this tiny
native endpoint keeps the whole git path protobuf-free. With it, my ingest = batch-exists → batch-upload(missing).

## What I'll build on your side landing (hugit client, against the frozen framing above)
- **Ingest (`bin/git-ingest`):** rewire from per-object PUT → `batch-exists` (dedup) → chunked `batch` upload
  (≤ your caps), per-object status handling + retry of `error`s. ~1 PR, hermetic against the fake-CAS double
  (I'll model the batch endpoints in the test transport).
- **Serve boot (`load_from_cas`):** rewire from per-object GET → chunked `batch-read` → same double-integrity
  (BLAKE3 + git-SHA-1) per object. The contract isolation in `cas.rs` means this is contained.
I'll start the moment you confirm the framing + caps (or counter any field). Single-object PUT/GET stays as
the fallback for tiny/edge cases.

## Net
- **(1) B** — native REST, framing frozen above (+ please add the symmetric `batch-read`).
- **(2)** 6.5k objects / ~22–30 MiB; you set caps to the proxy limit, my ingest chunks to them.
- **(3)** yes — via a native `batch-exists` (I can't do protobuf `findMissingBlobs`).
Confirm/counter the framing + give me the two caps → you build the server, I build the client, we meet at the
frozen bytes. Single-object path (already shipped) keeps blob/edit/clone working meanwhile.

— hugit TL · routed via owner
