# FROZEN — Wave-2 engine seams the multi-tenant go-live rides on (W-METENANT · W-PROVISION)

> **Status:** 2026-06-30. FROZEN by hugit TL for fleet execution (Wave 2, sequenced) AND for the
> githugr TL to thread the window side in parallel. These two engine WPs are githugr's NEEDS #1
> (cross-principal read exposure) + #2 (self-service repo provision) — the gate that flips githugr
> from interim-honest-suppression to a real multi-user, multi-repo forge. Bar: real go-live, real
> consequences → the contract is fail-closed, no-oracle, owner-tenant-scoped.

These two are **central-file WPs** (server.rs + state.rs + writes/verbs) → they run in Wave 2,
sequenced, the lead owning the auth/write seam. This doc freezes the WIRE so githugr can build
against it now and so the executing agent has zero interface judgment to make.

---

## W-METENANT — `/v1/me/*` scoped to the CALLER (closes the cross-principal read exposure)

### The defect (grounded, server.rs:318-360 + handlers/{dashboard,attention}.rs)
`GET /v1/me/dashboard` + `GET /v1/me/attention` run `two_tier_auth` → derive the principal →
`authorize_read`, BUT then call `build_dashboard(log, ME_DEFAULT_REPO)` / `build_attention(log,
ME_DEFAULT_REPO)` — a HARDCODED default repo; the builders take **no principal**. So every caller
(any valid session) gets the DEFAULT repo's activity rendered as "Seus repositórios". Real
cross-principal exposure.

### The frozen contract
1. **A per-tenant repo index.** A new `AppState` projection: `repos_for(principal) -> Vec<&str>` —
   the repos a `clerk:{org}:{user}` principal owns/accesses. Source of truth: each repo's
   `repo.meta{visibility, owner_tenant}` (the same record the read-gate's `authorize_read`
   consults). v0 enumeration: iterate the loaded repos, include a repo iff
   `authorize_read(principal, &repo.meta)` is `Allow` (public → everyone; private → owner_tenant
   match). This REUSES the existing authz predicate — no new visibility logic, no second gate to
   drift. `Caller::Operator` (dev-token) sees all loaded repos; `Caller::Unknown`/anonymous → `[]`.
2. **The builders take the principal + the caller's repo set:**
   - `build_dashboard(log_resolver, principal, repos: &[&str]) -> MeDashboardVm`
   - `build_attention(log_resolver, principal, repos: &[&str]) -> MeAttentionVm`
   The VM wire shape is UNCHANGED (githugr's `MeDashboardVm`/`MeAttentionVm` deserialize as today);
   only the DATA is now the caller's own repos' activity, aggregated across `repos`, never the
   default repo's.
3. **Empty is honest, not the default repo.** A caller who owns no loaded repo → an EMPTY
   dashboard/attention (no rows), NEVER `ME_DEFAULT_REPO`'s data. `ME_DEFAULT_REPO` is DELETED from
   the me/* path.
4. **No oracle.** The me/* response never reveals a repo the caller can't `authorize_read` — the
   index already filters by the authz predicate, so a private repo of another tenant is simply
   absent (the caller cannot tell it exists).

### Acceptance
- me/* resolves to the caller: owner-tenant principal → only their repos' activity; a foreign
  tenant → only public repos (or empty); operator → all.
- `ME_DEFAULT_REPO` gone from the me/* handlers (grep-proven).
- Tests: per-principal scoping (two principals, disjoint private repos, each sees only its own);
  anonymous/unknown → empty; operator → all; the VM wire shape byte-unchanged (a githugr-parity
  fixture still deserializes).

### githugr side (parallel, no engine change beyond this)
Thread the existing per-session `EngineTokenCache` into the me/* read path (a per-request
`LiveProvider` carrying the caller's `clerk:{org}:{user}` token, mirroring the write path). Once
this WP lands, the me/* reads resolve to the caller — drop the interim honest suppression.

---

## W-PROVISION — `POST /v1/repos` one-shot create/import (self-service repos)

### The defect (grounded — my roadmap blocker #3)
There is **no on-demand repo-create path**. `git-ingest` (a bin) seeds ONLY the git-content
manifests (CAS objects + refs.json + oid-index.json); it does NOT create the `<tenant>/<repo>.json`
EVENT-LOG the read-gate reads, and the `repo.meta` POST verb can't bootstrap an absent log
(`with_write` loads the head first → 404). So an ingested repo is 404 over the wire regardless of
visibility. No single provision path exists.

### The frozen contract
A real write verb githugr calls **as-the-user** (per-session token → `clerk:{org}:{user}`):

```
POST /v1/repos
Authorization: Bearer <per-session engine token>
{
  "name": "<repo-slug>",            // [a-z0-9._-], 1..=64, no path sep, no leading dot
  "visibility": "private"|"public", // default "private"
  "import_git_url": "<url>"|null    // null/absent → an EMPTY repo (no commits yet)
}
→ 201 { "repo": "<owner_tenant>/<name>", "visibility": "...", "ready": true }
   409 if the repo already exists for this owner_tenant (no clobber)
   400 on an invalid name / unsupported import source
   401 if the token does not resolve to a real tenant principal (NO operator/anon create)
```

Semantics (ONE op, fail-closed):
1. Resolve `owner_tenant` from the caller's principal (NOT a request field — the caller cannot
   create a repo under another tenant). Anonymous/operator → 401 (a real user creates their own;
   no god-create over the public door).
2. Seed the `<owner_tenant>/<repo>.json` EVENT-LOG with a genesis `repo.meta{visibility,
   owner_tenant}` record (chain-anchored) — this is what the read-gate + provision both read.
3. If `import_git_url`: ingest the git closure → CAS (objects + refs.json + oid-index.json) under
   the tenant, same path `git-ingest` uses; else an empty repo (no refs).
4. Register the repo in the runtime repo set (githugr moves `LIVE_REPOS` const → runtime config so
   no redeploy is needed) — the engine's `HUGIT_SERVE_CAS_REPO` becomes runtime-appendable.
5. **Atomicity:** the event-log genesis + the manifest seed either BOTH land or NEITHER (a partial
   provision must not leave a 404-but-half-ingested repo). On any leg failure → roll back / fail
   the whole op, return 5xx, leave no half-state.

Result: a freshly-created repo is **immediately push + clone live in one op** (push via the
existing receive-pack; clone via the authed-clone path #223 for a private repo by its owner).

### Acceptance
- A `POST /v1/repos` (empty) → the repo is immediately readable (`/v1/repos/{repo}/home` 200 for
  the owner, 404 for a foreign tenant) AND pushable (receive-pack accepts the first push) AND
  clone-able by the owner.
- A `POST /v1/repos` (import_git_url) → the imported history is live-readable + cloneable.
- 409 on duplicate; 401 on anon/operator; 400 on bad name; partial-failure leaves no half-state
  (atomicity test).
- e2e: create → push → clone round-trips as-the-user with a per-session token.

### Owner / infra gated (tracked, not fleet-buildable)
- `import_git_url` allow-list / SSRF posture (egress from the engine to fetch a remote git URL is a
  new outbound surface — MUST be SSRF-audited; v0 may restrict to a github.com allow-list or
  push-only-no-import until audited).
- The runtime repo-set persistence (where the appendable `HUGIT_SERVE_CAS_REPO` list lives so a
  restart keeps provisioned repos) — pairs with githugr's const→runtime move.

---

## Sequencing
W-METENANT (2.3) and W-PROVISION (2.5) both touch server.rs → sequenced in Wave 2 after the
write-path WPs (W-IFMATCH 2.1, W-WEBHOOK 2.2). githugr threads the token (me/*) + const→runtime
(LIVE_REPOS) in parallel against this frozen wire. The `import_git_url` SSRF posture is the one
owner/infra gate inside W-PROVISION — v0 ships push-to-empty-repo (no remote import) if the audit
isn't cleared, which still delivers self-service create.

— hugit TL
