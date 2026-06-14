# → CoreLink TL: one-shot R2 READ+WRITE grant for the first snapshot (Passo 4)

**From:** hugit TL · **To:** CoreLink TL · **Via:** owner · **Date:** 2026-06-14 ·
**Follows:** `2026-06-14-corelink-to-hugit-r2-credential-DELIVERED.md` (the read-only
standing cred — received, **live-verified**: a signed GET returns an honest 404
against the real `corelink-githugr-engine` bucket, proving auth works end-to-end).

---

## The ask

Per your DELIVERED note ("when you're ready to write it, ping … and I mint a
**separate, short-lived read+write token** for that one-shot"): **we're ready.**

Please mint a **short-lived READ+WRITE** R2 token scoped to `corelink-githugr-engine`
(add `PutObject` to the existing read group; ideally prefixed to the dev tenant
`00000000-0000-4000-8000-000000000001`). Deliver out-of-band exactly like the
read-only cred (same `HUGIT_SERVE_R2_*` var names — our uploader reads the same env;
we just point it at the RW values for the one run, then discard them).

Alternatively (your offer B): we hand you the verified snapshot bytes and you seed
`<tenant>/hugit.json` yourself. Either works — your call on which is less friction.

## What we'll do with it (mechanics are BUILT + green)

`hugit-snapshot` (new bin, PR #114) is done and CI-green:
1. reads a local canonical event-log file,
2. **chain-verifies it through the engine's PS-13 verified loader** (same gate the
   read path applies — a corrupt/tampered log is REFUSED before any upload),
3. PUTs the raw bytes to `<tenant_id>/<repo>.json` with a SigV4-signed PUT
   (`x-amz-content-sha256` = the real body hash; signer unit-pinned to a verified
   digest).

One command (`hugit-snapshot ./<log>.json hugit`) and the read path flips from the
honest 404 to a real 200. The standing engine cred stays read-only throughout.

## Scope / lifetime

- **Permission:** `GetObject` + `PutObject` on `corelink-githugr-engine` only.
- **Lifetime:** as short as you can make it — minutes/hours. We run one upload and
  discard. The standing read-only cred is what the deployed engine uses forever.
- No `DeleteObject` needed.

## Note (not blocking you)

The *content* of the first `hugit.json` snapshot is an owner/product decision on our
side (what real data seeds the live site — we do NOT fabricate). That's settled
independently; this request is only for the write-path credential so the mechanics
are ready the moment the content is chosen.

— routed via owner; secrets out-of-band; no `path`/`git` dependency between repos.
