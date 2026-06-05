# WP-B1 — App skeleton
squad B · M · sonnet · route sonnet · budget 70k · branch: wp/B1

## Charter
Build the GitHub App skeleton on a Cloudflare Worker (`hugit-app`): webhook
auth + ingest, PR-event persistence, Checks-API write-back, a least-privilege
manifest, and lifecycle handling (install/uninstall). This is the front door
for Phase B; everything in Squad B that touches GitHub rides this skeleton.

## Owned acceptance
B1 owns all 5 items of B1 (no split). VERBATIM from decomposition v2.0 §2:

① forged webhook→401+audit · ② PR event persisted, ack<1s · ③ check-run on
real PR · ④ least-privilege manifest snapshot · **⑤(+) uninstall revokes
access + halts processing (audited)**

## Contract deps
Consumes from `hugit-contracts` (frozen, never modified here): **AppWebhooks**
(signed-event envelope, ack-receipt, Checks-API write-back request/response),
**EventRecord** (for the audit trail of ① and ⑤). No contract type is
authored or changed in this WP.

## Claims
`crates/hugit-app/` — the Worker entrypoint, webhook router, auth/signature
verification, persistence adapter, and Checks-API client. Does NOT touch
`crates/hugit-app/sidecar/` (B6) or `crates/hugit-app/ui/` (B7) — those module
paths are reserved disjointly for their WPs.

## Dispatch packet
- This contract file (`docs/plan/wp-contracts/WP-B1.md`).
- `hugit-contracts` (AppWebhooks, EventRecord).
- whitepaper §5 (substrate — CF Worker / L4), §9 (fail-closed audit), §1.5
  (10-second webhook constraint — the ack<1s reason).
- warp-10-days §Squad B (B1 deliverable + claims).
- The failing acceptance suite at `tests/acceptance/wp-B1/`.
- Conventions: secrets are CoreLink write-only model; CF secrets are write-only
  (CLAUDE.md) — read names only, values from binding.
Estimated packet size: ~48k tokens (inside 70k).

## Implementation notes
Every fork pre-decided:
- **Runtime:** `hugit-app` is a Cloudflare Worker (whitepaper L4, warp §Day-0).
  HTTP router handles `/webhook` (ingest) and the Checks-API callbacks.
- **Webhook auth (①):** verify the `X-Hub-Signature-256` HMAC against the App
  webhook secret; mismatch → `401` AND append an `EventRecord` of kind
  `webhook.rejected` (fail-closed audit, whitepaper §9). No processing on a
  bad signature.
- **Ack budget (②):** persist the raw PR event to durable storage and return
  `2xx` in `<1s` (GitHub's 10s fire-and-forget limit, §1.5); heavy work is
  enqueued, never done inline. Persistence target = CoreLink CAS via the
  client (CAS `/v1/cas`), keyed by delivery id; `clw` is the reference client.
- **Check-run (③):** write a check-run back via the Checks API on a real PR
  using the AppWebhooks write-back type.
- **Manifest (④):** the App manifest requests least privilege (only the scopes
  Phase B needs: Checks read/write, PR read, contents read, webhook events);
  commit a snapshot fixture and assert the live manifest matches it.
- **Uninstall (⑤):** on the `installation.deleted` event, revoke the stored
  installation token, halt all queued processing for that install, and append
  an `EventRecord` of kind `installation.revoked` (audited).
- **Auth tokens:** GitHub App installation tokens minted per-install; stored
  write-only (never logged), consistent with CoreLink's write-only secret model.
- **CoreLink consumed as a CLIENT only** — zero corelink-server changes
  (governance law §8).

## DoD
Global bar: `cargo fmt` + `clippy -D warnings` + `cargo test` + `cargo audit`
green · owned items ①–⑤ red→green via the suite · cold verification by a
non-author agent · zero writes outside Claims.

## Completeness
All 5 owned items green; zero writes outside `crates/hugit-app/` (excluding the
reserved sidecar/ui module paths); evidence bundle attached to SEAL.

## Return shape
SEAL ≤20 lines: status, evidence refs (suite log, manifest snapshot diff,
audit-event samples for ①/⑤), deviations = none | waiver-ref.
