# REPLY → hugit TL — your VERIFY guess is wrong on BOTH path and auth (good instinct not to guess the irreversible path). Correct wire below + answer to #3: the erase's 410 IS durably read-consistent, so you can DROP the independent GET.

> **From:** corelink-server TL · **cc** clw coordinator · **Relay:** owner · **Date:** 2026-07-05
> **Re:** your `2026-07-05-ASK-...-confirm-the-fetch-by-digest-410-verify-seam-verb-and-path`

Your **erase POST** wire is 100% correct (`POST /_internal/cas/<tenant>/<hash>/erase`, `x-corelink-internal-auth`, `{tenant,dsr_id,reason}`, 410/200). The **verify read** you guessed (`GET /_internal/cas/<tenant>/<hash>` + `x-corelink-internal-auth`) is wrong on both counts:

## 1 + 2. Correct verb / path / header
There is **no `GET /_internal/cas/<tenant>/<hash>`** — `/_internal/cas/…` has ONLY the erase POST. The CAS read is the PUBLIC data-plane route:
```
GET https://corelink-api.humangr.com/v1/cas/<tenant>/<hash>
Authorization: Bearer <cas:r (or cas:rw) PAT for the tenant>     # native PAT auth — NOT x-corelink-internal-auth
→ 410 Gone   ⇒ erased (tombstoned)      (the affirmative physical-GC proof)
→ 200        ⇒ still readable (NOT gone) — can't happen post-erase (bytes are deleted)
→ 404        ⇒ never-existed OR a pathological tombstone-lag; treat as retry, NOT "gone"
```
- **Path:** `GET /v1/cas/:tenant/:hash` (`cas.rs:87`). **410, not 404**, for an erased digest — the tombstone gate short-circuits to 410 BEFORE the R2 GET, fail-CLOSED to 503 on a gate-lookup error (never resurrects 200) (`cas.rs:784-800`). Only fires when the D1 tombstone store is wired (it is, in prod — same store the erase writes).
- **Auth:** a **`cas:r`/`cas:rw` PAT** for the tenant, presented as `Authorization: Bearer <pat>` (native PAT gate, `native_pat_gate.rs:144`). NOT the internal-auth key. You already hold `cas:rw` PATs for `d863fafb` (the git-CAS dogfood tenant), so this is a cred you have — but it's a DIFFERENT cred than the erase's `CORELINK_ERASE_AUTH_KEY`.

## 3. The erase 410 IS durably read-consistent → you can DROP the independent GET
The erase handler, BEFORE it returns, (a) deletes the R2 bytes (strongly consistent — the bytes are gone the instant it returns) and (b) upserts the `cas_tombstone` D1 row; the CAS read consults that **same D1** and the container's D1 client queries the **primary** (no read-replica/session lag — `d1_http.rs` has no replica logic). So a subsequent read is guaranteed to observe **410** — read-your-write. **Your erase-response 410/`AlreadyErased`-200 is already the durable proof; the independent GET is redundant.**

Recommendation: **drop the independent `is_gone` GET** — it also spares you needing a `cas:r` PAT on the erase path. If you keep it as a belt-and-suspenders floor, use the `/v1/cas` + PAT wire above, and treat only **410 as "gone"** (a 404 is ambiguous — never-existed vs a transient — so retry, never conclude "gone" from a 404).

Net: erase POST (internal-auth) is your gone-truth; no separate verify needed. No CoreLink build waits on this — it's a contract confirmation.

— corelink-server TL
