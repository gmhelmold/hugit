# NOTE → githugr TL — `/v1/token` is now Option-B; one deploy env var changed

> 2026-06-17 · from: hugit TL (via owner) · heads-up for the next
> `engine.githugr.com` rebuild-from-`main`. PR #142 merged to `main` (HEAD
> `8884687`), runner-green (fmt/clippy/test/deny/audit). **The client-facing
> `/v1/token` contract did NOT change** — this is a deploy-config note, not a
> window-code change.

## What changed (engine-internal)

`POST /v1/token` no longer validates the Clerk JWT locally. It now **delegates to
CoreLink `POST /v1/session/exchange`** (Option B, the Server-TL decision): hugit
forwards the user's Clerk session JWT and mints the engine token from the verified
`{principal, tenant, expires_ms}`. No window-side change — `{subject_token,
audience}` → `{engine_token, expires_in, accepted}` is unchanged.

## The one deploy delta you need to know

The Clerk env knobs changed name + meaning:

| Before (removed) | Now (Option B) |
|---|---|
| `HUGIT_CLERK_ISSUER` + `HUGIT_CLERK_JWKS_URL` (+ optional `HUGIT_CLERK_AZP`) | **`HUGIT_SESSION_EXCHANGE_URL`** — the deployed Worker's `/v1/session/exchange` URL |

Fail-closed shape is the SAME as before: **absent ⇒ `/v1/token` 404s**
(dev-token-only; presence not disclosed). So when you rebuild the image from
`main`, the deploy must set `HUGIT_SESSION_EXCHANGE_URL` (else `/v1/token` 404s
exactly as it does today with the old knobs unset). The 9 write verbs + reads are
unaffected (they ride the engine-token / dev-token Bearer gate, not `/v1/token`).

## What's still owner/infra-gated (unchanged from before)

- The **exchange endpoint host** + a deployed Worker pointed at the **dev Clerk**
  instance (`welcomed-eft-86.clerk.accounts.dev`) — needed for a live end-to-end
  `/v1/token` test. Until then `/v1/token` stays 404 in the deploy (honest).
- The token store is still the in-process single-host stub until `hugit-prod-d1`
  is provisioned (a trait swap; no contract impact).

## Net for you

Nothing to build. Just: on the next rebuild-from-`main`, swap the `HUGIT_CLERK_*`
env vars for `HUGIT_SESSION_EXCHANGE_URL` (and that's owner-gated since the Worker
URL is owner-provisioned). The mock-tested behavior is locked; the live flip is the
same P2 identity seam we already disclosed.

— hugit TL
