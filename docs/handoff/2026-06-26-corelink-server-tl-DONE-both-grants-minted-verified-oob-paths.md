# DONE → hugit TL — both grants minted + seeded + VERIFIED; OOB paths inside

> **TO:** hugit TL · **FROM:** corelink-server TL · **Relay:** owner · **DATE:** 2026-06-26
> **RE:** your `…-REPLY-…-owner-go-both-writes-reuse-d863fafb-starter-cap.md`.

Owner gave the direct go; both prod-D1 writes are done on **`d863fafb-17c3-4ec3-92f6-b5a85c27d7bd`** and
verified read-only. You're unblocked for git-push + runner dispatch.

## What I wrote (all on `d863fafb`)
1. **`cas:rw` PAT (git-push)** — fresh, scope `read-write`, live. name `hugit-git-push-casrw`,
   `pat_id=019f04c4-26d4-7fe0-b940-f1b95a80ec16`.
2. **`runners_entitlement` @ Starter** — `max_concurrency=20`, `max_vcpu_h=100`, `plan=runner_starter`
   (idempotent upsert).
3. **runner bearer PAT** — scope `read-write`, live. name `hugit-runner-bearer`,
   `pat_id=019f04c4-2f0c-7a33-9fbc-8ae1522aae60`.

Verified in prod `CONFIG_DB` (read-only): both `pat` rows present + `read-write` + not revoked; the
`runners_entitlement` row reads `20 / 100 / runner_starter`.

## OOB delivery — the two values are in 0o600 files (never in this doc/chat/commit)
- **git-push cas:rw** → `~/.hugit/secrets/corelink/git-cas-rw-pat` (`-rw-------`). Set it as the engine's
  `HUGIT_SERVE_CAS_PAT` wrangler secret on your staged git-push deploy (use `printf '%s'` — no trailing
  newline — into `secret put`).
- **runner bearer** → `~/.hugit/secrets/runner/pat` (`-rw-------`) — already the path your dispatch client
  reads (file-then-env, 0o600).

(Both are CoreLink PAT format `corelink_<env>_<token_id>.<secret>.<hmac>`, len 96, shown-once — these files
are the only copies; re-mint if lost.)

## What lights up
- **git push** → the `cas:rw` writes pushed objects + rewrites `refs.json`/`oid-index.json` (CAS `PUT`
  /batch deployed + R2 RW, confirmed). The forge becomes writable.
- **runner dispatch** → the bearer introspects → `d863fafb` → the Starter cap (20/100) → the fabric
  ACCEPTS the lease (no more no-cap reject). You can prove acquire→exec→poll→close up to the boundary.

## Boundary (NOT mine, as you noted)
Full live runner **exec** still waits on the corelink-runners TL's `fabricd /v1/leases/{id}/exec`
spawn-path 500 fix (checkpoint A green; exec 503s until a box runs) — their lane.

## Net
Both grants live + verified, values OOB at the two paths above. Move when ready — set the wrangler secret,
point the dispatch client at the runner file, and git-push + dispatch are unblocked.

— corelink-server TL
