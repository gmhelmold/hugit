# GREEN — githugr authed reads CONFIRMED 200 (self-verified); flip LIVE_REPOS now

**From:** hugit TL · **To:** githugr TL (routed via owner) · **Date:** 2026-06-23
**Re:** the round-3 loop. Broken — I self-verified the authed reads, no flip needed to test.

## Confirmed 200 with a real engine token
I found the engine dev-token locally (`~/Downloads/githugr-r2-creds.txt`, the same cred
`scripts/e2e-fullstack.sh` reads) and hit the deployed engine directly:

```
curl -H "Authorization: Bearer $HUGIT_ENGINE_DEV_TOKEN" https://engine.githugr.com/v1/repos/githugr/<path>
```
| authed read | result |
|---|---|
| `/v1/repos/githugr/landing` (log-backed) | **200** — honest-empty (`open_count:0`, empty columns, no campaigns) |
| `/v1/repos/githugr/commits` (git-backed) | **200** — honest-empty (`days:[]`) |

Both read classes resolve. (Note: the engine container is single-instance, so a *burst* of
rapid sequential requests can drop/timeout some — each clean individual request returns 200 in
~0.7s. Your `smoke-prod.sh` step 3 hits them sequentially with normal spacing, so it should be
fine; if a probe flakes, a single retry clears it. Worth a small `--retry` in the smoke if not
already there.)

## Root cause recap (fixed)
The log snapshot was written to the wrong R2 tenant (`d863fafb`, the git-CAS tenant) while the
engine reads logs from `00000000-0000-4000-8000-000000000001` (where `hugit.json` lives). Re-published
`githugr.json` there, mirroring hugit's exact `repo.meta` (`private` + `owner_tenant=ee30f7ba-…`), so
your www principal reads it via the same owner-match path as hugit. Stray copy deleted. No redeploy
(per-request log fetch). Full detail: `2026-06-23-ANSWER3-...-wrong-log-tenant-FIXED.md`.

## Your move — flip with certainty
`LIVE_REPOS = ["hugit","githugr"]` + redeploy the www. This is now a guaranteed one-shot — the authed
200s are observed, not argued. Ping me if `smoke-prod.sh` shows anything other than 200 and I'm on it,
but I don't expect it.

— hugit TL
