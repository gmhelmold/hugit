# Day-0 provisioning checklist (owner-gated)

> All three items are `askBefore` classes in the TechLead profile
> (GitHub-App registration · paid infra · external services). Nothing here
> executes without explicit owner approval; this file is the exact ask.
> Source: warp-10-days §Day-0 provisioning row. Status owned by the
> orchestrator; checked off only on verified completion.

## P1 — GitHub App (dev) — gates B1 (D1 dispatch)

- [ ] Register a **dev** GitHub App under the `humangr-labs` org:
  - name: `hugit-dev` · homepage: repo URL · webhook URL: placeholder until
    the B1 Worker exists (update post-deploy); webhook secret: generated,
    stored in the secrets broker path (NEVER in repo/env files — profile law).
  - Permissions (least-privilege, snapshot for B1④): Checks RW · Contents RW
    (merge API for B4b) · Pull requests RW · Issues RW (sidecar comments) ·
    Metadata R. Events: pull_request, push, check_suite, check_run,
    installation.
- [ ] Install on: `hugit` + 2 synthetic fleet repos (B8 targets;
  `corelink-workspaces` added later per B8 contract). **NOT corelink-server**
  (X10② — enrollment during launch window fails the build).
- [ ] App ID + private key delivered to the broker path only.

## P2 — CoreLink prod tenancy (R2/AC namespaces) — gates B2a/C3

- [ ] Create a **new tenant** on the CoreLink **prod API, as a paying
  customer** — zero corelink-server changes (governance law §8; the
  shared API-tenancy channel is X10④⑤'s test surface later).
- [ ] Namespaces: CAS + AC for hugit CI memoization; R2 bucket per CoreLink's
  standard tenant provisioning.
- [ ] **Policy-cap the tenant from day 1** (X10⑤ preventive bound): rate +
  budget caps so a hugit-side storm is structurally bounded.
- [ ] PAT for the tenant delivered to the broker path only.

## P3 — Runner box (Hetzner-class) — gates C2a (D1 dispatch)

- [ ] 1 dedicated box (AX-class or equiv: ≥8 cores / 64 GB / NVMe), fresh OS,
  **zero shared infrastructure with CoreLink runners** (X6②/X10: separate
  box, separate account ok — resource isolation asserted by config test).
- [ ] SSH key minted for this project only; container runtime installed
  (container-per-job v0; Firecracker upgrade path is C2 docs, not Day 0).
- [ ] Box inventory recorded here (host, specs, OS, billing owner) once live.

## Sequencing

P1 and P3 gate **D1 dispatch** (B1, C2a are wave-1 WPs). P2 gates B2a/C3
(wave 2). None gate WP-00/WP-01 (today's code lane). Approve-by replying to
the orchestrator's ask; execution steps that need owner credentials stay
owner-executed (`! <command>` in-session works).
