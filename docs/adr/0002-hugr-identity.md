# ADR-0002 — HuGR identity: one account, CoreLink machinery

- **Status:** Accepted — 2026-06-09
- **Date:** 2026-06-09
- **Applies to:** family-wide — **hugit** (canonical, this file) · **githugr**
  (first consumer; thin companion) · **corelink-runners** (companion) ·
  **corelink-workspaces** (companion) · **corelink-server**
  (the producer)
- **Resolves:** identity unification across the HuGR product family.

## 1. Context

Three credential domains existed with no unifying decision:

1. **Login** — githugr planned "GitHub OAuth" for its account wave.
2. **Machine API** — CoreLink PATs (Argon2id-verified, tenant-scoped): what
   `clw`, `hugit-runner`/`hugit-checks`, and githugr's `Provider` speak.
3. **Accounts** — CoreLink's production stack: Clerk (users/sessions),
   signup-worker (tenant provisioning), Stripe/Neon (billing/subscriptions).

The tempting shape was a standalone "HuGR auth service" with every product
orbiting it. But CoreLink's identity machinery is **production-deployed,
formally verified** (PAT revocation propagation is TLA+-verified —
`INV-PAT-REVOKE-PROPAGATION`, wave 25) **and inside the GA compliance scope**
(SOC 2 / GDPR / LGPD). Re-opening a sealed, audited subsystem pre-GA, with zero
customers, to rebuild what already works is the defocus death-vector the review
panel ranked #1. The owner's instinct (one identity domain, products orbiting)
is right about the **shape**; the efficient implementation extracts a **brand
and a contract**, not a service.

## 2. Decision — four laws

1. **One HuGR account (the brand law).** The user-facing identity is the
   **"HuGR account"** on every surface of every product. No screen ever says
   "log into CoreLink". Branding decouples now; infrastructure does not.
2. **CoreLink machinery underneath (the no-rebuild law).**
   - *Sessions/login:* the existing production **Clerk** instance — one user
     pool across surfaces (CoreLink dashboard, githugr; multi-domain/satellite
     configuration — exact mechanism validated by the CoreLink team), with
     the **GitHub social connection** enabled. That IS githugr's
     "sign in with GitHub" — zero new infrastructure.
   - *Accounts/tenancy:* **org = tenant_id, 1:1** — the same unit CoreLink
     already isolates (HMAC-derived prefixes) and bills (Stripe customer).
   - *Machine credentials:* **PATs, unchanged** (Argon2id verification,
     tenant scope).
3. **Products consume a frozen contract, never internals (the seam law).**
   The HuGR Identity contract is:
   `user (Clerk) → memberships → org · org = tenant_id · PAT
   issuance/verification · session→token exchange`.
   The exchange is the **one new piece**: a corelink-server endpoint that mints
   a **short-lived, tenant-scoped token** from a valid session.
   **A PAT never reaches a browser** — githugr's server exchanges the session
   for the short-lived token; the browser holds only the session cookie.
4. **Physical extraction: deferred indefinitely, pre-authorized (the exit
   law).** A standalone identity service happens only on a forcing function
   (enterprise SSO, compliance isolation, blast-radius). Because every consumer
   speaks the contract, extraction is a **deployment change behind a stable
   seam — never a migration**.

## 3. Non-confusions (explicit)

- **Login-with-GitHub ≠ the GitHub App.** Clerk's GitHub social connection
  authenticates a *human*. The GitHub App (`hugit-app`) grants *repository
  access* (mirror, import, webhooks, Checks API). Two integrations; never
  merged; this contract covers only the first.
- **This ADR creates no new product, repo, or campaign.** (The X10 focus-gate
  spirit applies to infrastructure too.)
- **Billing identity is already decided elsewhere:** Stripe customer ↔
  org = tenant. Unchanged here.

## 4. Consequences (per repo)

- **corelink-server (producer — via the rollout handoff):**
  ① the session→token mint endpoint (new, small; invariants in §6 — needs its
  own oracle); ② Clerk multi-domain config for githugr + the GitHub social
  connection; ③ the signup-worker provisioning fixes already reported by
  corelink-workspaces (AC envelope-signing key missing → AC-500 on fresh
  tenants; hardcoded `enam` region) sit **on this exact path** — they block
  CoreLink pilots and future githugr signup equally.
- **githugr (first consumer):** login brand "HuGR account"; GitHub social
  enabled; the `Provider`'s live implementation authenticates server-side with
  exchanged tokens; Wave 4 (account level) builds against the contract;
  product.md §10.6 flips to DECIDED.
- **hugit (this repo):** **no code change.** The PAT flow is already
  contract-shaped; the P2 tenant request stands as written; D14 authz is
  unaffected. The `operator` field in envelopes/attestations references the
  HuGR account principal.
- **corelink-runners:** the lease API stays Bearer-PAT (integration contract
  §1, unchanged); M2 direct self-serve onboards via the same HuGR account;
  per-tenant caps/fairness (X10/X6/C7) key off the same org = tenant.
- **corelink-workspaces:** `clw` config (tenant + PAT) unchanged; the signup
  bugs it surfaced are the account-creation route this ADR standardizes.

## 5. Alternatives considered

- **Standalone HuGR auth service now** — rejected. Re-opens a sealed, verified,
  compliance-scoped subsystem pre-GA; months of work serving zero customers;
  the highest-probability death vector (defocus) wearing an architecture hat.
- **Raw CoreLink auth with no contract and no brand** — rejected. Brands the
  cache product as the family's identity root; internals leak into consumers;
  a later extraction becomes a migration instead of a deployment change.

## 6. Security invariants (must hold; oracle-tested where they land)

1. **A PAT never reaches a browser.**
2. Exchanged tokens are **short-lived and tenant-scoped**; the mint endpoint
   cannot mint across tenants (membership-checked, fail-closed).
3. PAT revocation keeps its verified propagation invariant
   (`INV-PAT-REVOKE-PROPAGATION`).
4. The login surface adds **no parallel auth domain** — one user pool, one
   org→tenant mapping.
