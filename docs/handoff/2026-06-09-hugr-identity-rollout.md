# Handoff — ADR-0002 (HuGR identity) rollout

> 2026-06-09 · from the hugit-anchored session · owner-ratified decision.
> Canonical text: `docs/adr/0002-hugr-identity.md` (this repo). Companions are
> already landed in githugr and corelink-runners. This handoff carries the two
> pieces that **cannot land from here** (the session fence blocks writes into
> `corelink-server` and `corelink-workspaces` — by design): the corelink-server
> work items and the corelink-workspaces companion file.

## 1. For the corelink-server techlead — three work items

**The decision in one line:** *one HuGR account, CoreLink machinery* — the
family's identity stays on your production stack (Clerk + org=tenant + PAT),
branded HuGR, consumed through a frozen contract. No new auth service.

1. **Session→token mint endpoint (the one NEW piece).** Mints a short-lived,
   tenant-scoped token from a valid Clerk session, for githugr's server-side
   `Provider`. Invariants (need an oracle, fail-closed): token TTL short;
   scope = one tenant, derived from org membership, never from client input;
   **no cross-tenant mint**; a PAT is never issued to, or echoed at, a browser
   surface. (ADR §6.)
2. **Clerk configuration for githugr.** Same user pool serving the CoreLink
   dashboard and githugr (multi-domain / satellite-domain mechanism — validate
   the exact Clerk feature), and enable the **GitHub social connection** (this
   is githugr's "sign in with GitHub"; it is NOT the GitHub App — that stays a
   separate repo-access integration in hugit-app).
3. **Signup-worker provisioning fixes — now doubly load-bearing.** The two
   defects corelink-workspaces reported sit on the exact account-creation route
   this ADR standardizes, and block CoreLink pilots and future githugr signup
   equally:
   - fresh tenant → HTTP 500 on every AC op (envelope-signing key not
     provisioned): `../corelink-workspaces/docs/BUGREPORT-prod-ac-500.md`
   - hardcoded `enam` region / no-op `configureTenant`:
     `../corelink-workspaces/docs/HANDOFF-signup-region-hardcode.md`

## 2. For corelink-workspaces — the companion ADR (drop-in)

The fence blocks this session from writing into `corelink-workspaces`. Land the
file below from a workspaces-anchored session (or a plain terminal):
`mkdir -p docs/adr` then save as `docs/adr/0002-hugr-identity.md`, commit with
the usual DCO + Co-Authored-By trailers.

```markdown
# ADR-0002 — HuGR identity: one account, CoreLink machinery (companion)

- **Status:** Accepted (adopts the canonical ADR:
  `../hugit/docs/adr/0002-hugr-identity.md`)
- **Date:** 2026-06-09

Family decision: the user-facing identity is the **HuGR account** everywhere;
underneath it is CoreLink's production machinery (Clerk sessions, org = tenant,
PATs) behind a frozen contract; a standalone identity service is deferred
indefinitely and pre-authorized behind that contract.

What it means in this repo:

- **`clw`'s credential model is unchanged.** Config carries tenant + PAT; the
  PAT remains the machine credential, verified server-side. No code change.
- **User-facing copy** (README, errors, signup pointers) says **"HuGR
  account"** for the human identity — never "CoreLink login". The PAT remains
  named what it is: the CoreLink PAT, the machine credential under that
  account.
- **The signup bugs this repo reported** (`docs/BUGREPORT-prod-ac-500.md`,
  `docs/HANDOFF-signup-region-hardcode.md`) sit on the account-creation route
  this ADR standardizes — their fix (owned by corelink-server) unblocks
  CoreLink pilots and githugr signup equally.
```

## 3. For corelink-workspaces — the interop doc (drop-in)

Owner directive (2026-06-09): every family repo carries a microscopic interop
doc. Land the file below as `docs/interop.md` (same session/terminal as §2).

```markdown
# corelink-workspaces interop — how this repo talks to the family (microscopic seam map)

> 2026-06-09. `clw` is a PURE CLIENT of the live CoreLink API — no server code
> here. Frozen client contract: `clw-types` (Digest, Manifest v1, RefRecord,
> `CasTransport`/`AcTransport` traits; contract hash fe251941).

## 1. clw → corelink-server (the only wire)

| Aspect | Exact detail |
|---|---|
| Endpoint / auth | `https://corelink-api.humangr.com` · `Authorization: Bearer <PAT>` from `~/.clw/config.toml` (flag → env → file resolution) |
| snapshot | walk (ignore-aware) → FastCDC chunks (256K/1M/4M, compile-time const-assert `CHUNK_MAX < CAS_BLOB_CAP`) → dedupe via `CasTransport::exists()` → upload → canonical-JSON **manifest v1** → CAS → `RefRecord` written to the AC under namespace `clw/ref/v1/<name>` (carries `parent` lineage) |
| hydrate | RefRecord → manifest → parallel chunk fetch, **cache-first** (`~/.clw/cache`, atomic, self-healing, LRU gc) → atomic file reconstruct (symlinks + mode bits) · **BLAKE3 verify on every fetch** |
| run (memoized) | AC key = hash(command ‖ args ‖ env ‖ workspace manifest) · hit replays stdout/stderr/exit **byte-exact** · outputs chunked into CAS · **success-only caching** (non-zero exit never memoized) |
| status / ls | local tree hash vs remote ref → exit 0 clean / 1 dirty · ref metadata (root digest, counts, parent) |
| Rate limits | client-side **global throttle with patient backoff through 429s** (PR #19) — server-side limits are tuned for low concurrency today |

## 2. Relationships upward

- **hugit:** independent implementations over the same CAS concepts today
  (hugit-fence materializes claim-fenced workspaces itself; no runtime call to
  clw yet). Convergence: Runners **M4** (sandboxes/dev boxes as Workspace SKUs).
- **githugr:** "open in workspace" = `clw hydrate` of an exact
  content-addressed state; intent pages show the workspace chip.
- **Runners:** the fabric boots cache-warm off the same CAS/AC this client
  feeds; Workspace SKUs ride the fabric at M4.
- **Identity (ADR-0002, `docs/adr/0002-hugr-identity.md`):** PAT flow
  unchanged; user-facing copy says **HuGR account**.

## 3. Open defects this repo filed (owned by corelink-server)

`docs/BUGREPORT-prod-ac-500.md` (fresh tenant → 500 on all AC ops: envelope
signing key not provisioned) · `docs/HANDOFF-signup-region-hardcode.md`
(`enam` hardcode, no-op `configureTenant`, colo↔macro-region mismatch). Both
sit on the account-creation route of ADR-0002 — they block CoreLink pilots and
githugr signup equally.
```

## 4. For corelink-server — seam summary (place/adapt as their techlead sees fit)

What the family consumes from corelink-server, in one table (companion to the
§1 work items; their repo's own conventions decide where this lives):

| Consumer | Seam | Notes |
|---|---|---|
| `clw` (workspaces) | CAS/AC HTTP + PAT verify | §3 defects above are on the signup path |
| `hugit-checks` | AC by memo key `H(tree ‖ def ‖ toolchain)` + CAS (envelopes, packs, exports) | smoke DoD: miss-404 · round-trip · cross-tenant-403 |
| Runners fabric (M1) | CAS/AC warm-boot reads + result stores + PAT + tenancy | fail-closed when cache degraded |
| githugr | `cas:` envelope fetches (via hugit's Provider) + per-repo DO event-log hosting (P2) + **session→token mint endpoint (ADR-0002, NEW)** + Clerk pool w/ GitHub social | PAT never to a browser |
| direct cache customers | REAPI v2 gRPC · Bazel HTTP · Turborepo v8 · sccache/cargo · OCI (phase B) | the cache product proper |
| everyone | PAT verification (Argon2id, constant-time tenant claim) · HMAC-prefix tenancy · org=tenant=Stripe customer | identity per ADR-0002 |

Tense discipline (family rule): dedup is intra-tenant at GA; cross-tenant is
staged (`CAP-DEDUP-CROSS-TENANT`) — production-state claims cite the GA notes.

## 5. Status ledger for this rollout

| Repo | Artifact | State |
|---|---|---|
| hugit | canonical ADR + index row + `docs/interop.md` + this handoff | ✅ landed |
| githugr | companion ADR + product.md §10.6 → DECIDED + whitepaper §7 update + `docs/interop.md` | ✅ landed |
| corelink-runners | companion ADR + `docs/interop.md` | ✅ landed |
| corelink-workspaces | companion ADR (§2) + interop doc (§3) | ⏳ awaiting a workspaces-anchored session |
| corelink-server | work items (§1) + seam summary (§4) | ⏳ awaiting the corelink techlead |
