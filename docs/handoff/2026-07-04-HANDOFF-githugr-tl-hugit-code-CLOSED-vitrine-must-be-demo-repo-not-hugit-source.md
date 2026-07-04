# HANDOFF → githugr TL — owner reversed the OSS decision: **hugit's code stays CLOSED** (free-to-use ≠ open-source). I've closed the two anon-exposure doors from hugit's side (GitHub mirror private + engine `hugit` repo private). Residual is YOURS: the public vitrine must NOT showcase hugit's proprietary source — switch it to a demo repo, and stop publicly rendering hugit's file contents.

> **From:** hugit TL · **To:** githugr TL · **cc** owner · **Relay:** owner · **Date:** 2026-07-04

## The decision (owner, 2026-07-04)
*"o hugit é free, mas não vou abrir o código dele."* hugit is **FREE TO USE** but the **CODE STAYS CLOSED** — this REVERSES the 2026-06-20 "full OSS Apache-2.0" decision. **Do not describe hugit as open-source anywhere on the site.**

## Done on hugit's side (both verified live, reversible)
1. **`github.com/HumanGuardrail/hugit-oss` → PRIVATE.** The public Apache-2.0 mirror is off the air (anon page → 404). Reversible (flip back to public if the owner ever reverts).
2. **The engine's `hugit` repo → PRIVATE.** Appended a chain-valid `repo.meta{visibility:private, owner_tenant:ee30f7ba…}` record to hugit's event log (the gate reads `project_repo_meta` **per-request**, so NO redeploy — effective immediately). Verified live: anon `git clone`/`git ls-remote` → **"repository not found"**, anon `GET /v1/repos/hugit/home` → **404**, anon `info/refs` → **404**. Engine `/readyz` stayed **200** (no outage); `www.githugr.com` stayed **200**. **Operator/authed reads are UNAFFECTED** — `authorize_read` gives the operator (`orchestrator:*`) a bypass regardless of visibility, so the engine dev-token githugr uses still reads hugit. So your site did not break; only the ANONYMOUS bulk-clone vector closed.

## The residual — YOURS to close (why hugit private isn't 100%)
Making the repo private blocks the **anonymous bulk clone** (the primary "copy the whole source" vector). But because **githugr reads hugit with the operator dev-token and then renders it publicly**, hugit's **individual files can still be viewed on `www.githugr.com`** (file-by-file, no bulk clone). To honour "code closed":

1. **Switch the public vitrine/showcase OFF hugit's proprietary source → a purpose-built DEMO repo.** The owner's call (#3): the public storefront should demo the product on a non-proprietary example repo, not on hugit's own implementation. Ingest a demo repo into the CAS (I can host it engine-side, like githugr) and point the vitrine at it.
2. **Stop publicly serving hugit's file/blob CONTENTS.** If the www renders `hugit` blobs/home/tree to anonymous visitors via the operator token, that's the residual leak. Either drop hugit from the public browse surface, or gate hugit's file views behind auth. (Metadata like PR counts / the cost `/insights` demo can stay if the owner wants — the concern is the SOURCE CODE, i.e. blob/tree/home contents.)
3. **Confirm the www still functions with hugit private** (it reads via operator, so it should) and that no view relies on hugit being anon-readable.

## Ask
Confirm you'll (a) move the vitrine to a demo repo and (b) close the public blob/tree rendering of hugit's source. Ping me for the demo-repo CAS ingest whenever the demo repo exists. Routing via owner.

— hugit TL
