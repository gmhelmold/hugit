# hugit — KungFu Pilot-Readiness Roadmap (grounded survey → roadmap)

> **Status:** 2026-06-30. Produced by an 8-subsystem **grounded** survey (code-verified, not narrative) + synthesis.
> Full survey output: `tasks/wxug32hzk.output` (the workflow result). This doc is the actionable roadmap →
> the source for the WP decomposition + the parallel execution phase.
>
> **The pilot (KungFu) can start ONLY when EVERY blocker closes with CERTAINTY** (the owner's bar). The survey's
> #1 finding: the prior narrative (CLAUDE.md + the KungFu reply) **over- and under-stated reality on ~12 points** —
> trusting it would have started the pilot on a broken foundation. Code wins; corrections are §5.

## 1. Honest readiness: **~40%** (weighted by pilot-criticality)
Real assets (some UNDER-claimed before): the single-tenant WRITE wire (create/update/incremental/delete via standard
git) is genuinely live + prod-verified; the multi-tenant authz LOGIC is solid + fail-closed + fully tested;
`hugit-symbols` (#6) + git-from-CAS read-by-sha (#7) are real; the cost A-path is wire-proven. **But four hard
blockers stand between today and a runnable pilot**, three of them mis-framed as "deferred features" when the code
shows unset flags / unwired provisioning / absent net-new surfaces.

## 2. The blockers (biggest-first) + what each is gated on
1. **Live engine is 2 commits behind main** — an undeployed read-path SECURITY fix (#222 ref-name scrub) + #221 are
   unshipped on the only prod surface. → **AUTONOMOUS** (deploy HEAD; verify via /readyz version flip + www decode,
   never wrangler exit-0). *[IN PROGRESS this session.]*
2. **Pilot repo CANNOT be cloned back over the wire.** Anonymous → 404 (no repo carries `repo.meta{visibility:public}`
   — the gate is BUILT + test-proven, an UNSET flag, NOT a missing feature). Authed → **impossible**: upload-pack
   passes an EMPTY principal (`git.rs:280`) — **zero Bearer-extraction on the clone route**, so a private repo is
   uncloneable by ANYONE incl. the owner. → **OWNER ratified AUTHED read** ⇒ **AUTONOMOUS: build Bearer-extraction on
   upload-pack** (mirror receive-pack's `two_tier_auth`).
3. **No single "provision a new repo" path** — `git-ingest` seeds only git-content (CAS objects + refs.json/oid-index);
   it never creates the `<tenant>/<repo>.json` EVENT-LOG the read-gate reads, and the `repo.meta` POST verb can't
   bootstrap an absent log (`with_write` loads head first → 404). An ingested repo is 404 REGARDLESS of visibility. →
   **AUTONOMOUS: one-shot provision path** (seed the event-log carrying `repo.meta` + git-ingest in ONE op).
4. **Outbound merge-event webhook does NOT exist** (KungFu #5, P0 self-maintenance) — net-new: no `touched_paths`
   computation in the write path, no emit seam, and the SSE stream silently DROPS `ref.update`/`ref.delete`. →
   **OWNER-APPROVED + contract FROZEN** (`KungFuMergeEvent`) ⇒ **AUTONOMOUS build** (task #72).
5. **Single-threaded + single-instance can't sustain heavy-parallel load** — every request serializes; a burst of
   2s-bounded reads stacks latency until /readyz wedges (3× verified prod outage); the If-Match conditional manifest
   PUT (HA pre-condition) is UNBUILT. → **AUTONOMOUS** (If-Match + concurrency-shed + observability); **OWNER/INFRA**
   for running >1 instance.
6. **Multi-tenant isolation UNPROVEN on live data** — no real Clerk JWT ever minted a tenant token (exchange wired,
   401-not-404, never exercised E2E); no live repo has `owner_tenant`; `Cache-Control:private` is **entirely absent**.
   → **AUTONOMOUS** (owner_tenant assign + Cache-Control) + **OWNER** (pilot org→tenant_id) + **EXTERNAL: githugr-TL**
   (real Clerk login mint).
7. **Killer-data render verified only to "route serves + 401 unauthed"** — NOT proven to render real bytes through the
   githugr www (a silent VM-decode drift broke this before). → **EXTERNAL: githugr-TL** (authed www smoke).

## 3. The critical path (ordered — the roadmap spine)
1. **Deploy HEAD** (close the drift incl. #222 security) — fastest, must precede pilot traffic.
2. **Resolve read-back** — authed ratified ⇒ build Bearer-extraction on upload-pack; + the one-shot provision path.
3. **Build the merge-event webhook** (#5) — `touched_paths` tree-diff in finalize + HMAC/SSRF-safe outbound emit, off
   the accept loop, post-durable.
4. **Harden for heavy-parallel** — If-Match conditional PUT, concurrency-shed (503 past N), /metrics observability.
5. **Exercise multi-tenant identity live** — owner_tenant + Cache-Control (hugit) + real Clerk mint (githugr-TL).
6. **Verify killer-data render** from the real www (githugr-TL).
7. **Provision + ingest the KungFu repo** into CAS, add to `HUGIT_SERVE_CAS_REPO`, redeploy, verify `git_repos`++.

## 4. The autonomous WP clusters (parallelizable — the execution phase)
- **A. DEPLOY + DRIFT-GUARD** (prereq, non-parallel): ship HEAD; a deploy runbook/script; a drift check (/readyz
  version vs HEAD); the restore decision-tree runbook. *Also: reconcile CLAUDE.md's over-claims (§5).*
- **B. READ-BACK + PROVISIONING** (parallel; dep A + the authed decision [ratified]): the one-shot provision path;
  Bearer-extraction on upload-pack + a real-git e2e test; a documented new-repo ingest procedure; push-incremental
  read-index update.
- **C. MERGE-EVENT WEBHOOK** (parallel; dep A + KungFu contract [frozen] + owner nod [given]): `touched_paths`
  tree-diff; the HMAC/SSRF-safe emit seam; off-loop bounded queue + idempotency; SSE `ref.update`/`ref.delete`
  (scrubbed); an adversarial security review of the new egress.
- **D. PARALLEL-LOAD HARDENING** (parallel; dep A): `put_object_conditional`/If-Match on refs.json+oid-index; a
  boot/deploy guard refusing >1 instance without it; a concurrency-shed; /metrics; a `BudgetedSource` invariant +
  audit the remaining CAS-touching handlers; the code-search lazy in-memory index (2026-06-29 design).
- **E. MULTI-TENANT (autonomous half)** (parallel; dep A + owner org→tenant): owner_tenant assign; `Cache-Control:
  private` on every authed/private response + a test; the dev-token god-principal posture doc; the TTL revocation
  posture.
- **F. CONTRACT DOCS + SCHEMA HONESTY** (fully parallel, no deps): the #8 doc stating the TRUTH (deny_unknown_fields →
  versioned bumps, NOT serde-default — corrects the KungFu reply); the #6 `outline_blob` fact contract; the #7
  read-by-sha contract; wire `diff_vm` into compare/pr_detail/commit_detail + delete the stale "DiffVm is a stub" comment.

## 5. Over-claim corrections (code vs narrative — RECONCILE CLAUDE.md)
1. **Anonymous-clone is BUILT + test-proven**, not a "deferred feature" — an unset flag + provisioning gap.
2. **Authed (private) clone-back is IMPOSSIBLE today** (no Bearer-extraction on upload-pack) — a hard block, not a flag.
3. **"Host the repo as soon as it exists" understates** the two-store provisioning gap (no event-log seed for an
   ingested repo).
4. **The Clerk exchange env-gate is CLOSED** (401-not-404, route deployed) — CLAUDE.md/honest-audit are STALE here;
   only a real-JWT mint remains.
5. **The multi-tenant read gate is UNDER-stated** ("single-digit %") — the 404-no-oracle matrix is fully live+tested;
   only the LIVE EXERCISE is missing.
6. **KungFu #18's "VM lacks visibility/owner_tenant" is half-wrong** — `visibility` IS carried; `owner_tenant` is a
   server-side gate input by design; the real gap is **Cache-Control:private (entirely absent)**.
7. **The merge-webhook does NOT exist** — the single largest unflagged write-narrative↔pilot-need gap.
8. **The cost proof is wire-only / hand-supplied** (4200000); the porcelain `pr land --dispatch` always submits
   honest-zero; the cost rides same-trust as tokens (CloseResponse ignores the attestation block — NOT
   cryptographically attested E2E yet).
9. **#8 schemas are `deny_unknown_fields`** (the OPPOSITE of serde-default forward-compat — the KungFu reply was WRONG;
   KungFu is exposed to versioned BREAKS). 1.1.0→1.2.0 was a clean versioned break (safe only because zero producers).
10. **Effective push pack cap is ~8 MiB** (not 16 — `read_body_capped` at MAX_BODY_BYTES=8MiB precedes proto's check);
    **v0 receive-pack REJECTS multi-ref pushes** (`git push --all` refused).
11. **"Single-instance race closed" is true only for the EVENT-LOG** — the receive-pack/delete manifest write is an
    UNCONDITIONAL refs.json PUT (race-free ONLY by max_instances:1 + single-thread); the If-Match seam is genuinely UNBUILT.
12. **CI runs on the contention-flaky self-hosted Mac** (~37% infra-flake) — the ci.yml header comments still say
    ubuntu + contradict the live `runs-on` (#219). Assert green only on concluded CLEAN + both checks SUCCESS.
13. **#221/#222 are narrower** than "write+cost path audited SOUND / read-path redaction audited" — #221 a thin-pack
    tip re-assert (defence-in-depth MED), #222 specifically ref/branch-NAME scrubbing.

## 6. Owner decisions
**RATIFIED (2026-06-30):** D-1 = hugit CONSUMES KungFu; public-flag = **AUTHED read** (no anon exposure); merge-emit
APPROVED + contract frozen.
**STILL OPEN:** pilot **org→tenant_id** mapping; **run >1 instance?** (only after If-Match); **dev-token god-principal**
acceptance for the single-tenant pilot; the **first-public-cost hold** (non-zero only); the **cross-repo crate
consumption mechanism** (hugit-symbols/hugit-proto/hugit-contracts are `publish=false` — git-dep vs path-dep vs vendor
vs publish + licensing for KungFu to consume them).

## 7. External dependencies (other TLs / infra)
- **githugr-TL:** render-verify the 3 killer reads from the public www; drive a real Clerk login → /v1/token → tenant
  read (hugit can't mint a Clerk JWT from here); KnowledgeVm/Provider co-design (if D-1=consume, KungFu is the engine).
- **KungFu-TL:** the merge-event payload schema (frozen); the KnowledgeVm read-path seam co-design.
- **corelink-runners-TL:** the off-box agent-loop that reads the provider `/usage` (the real cost SOURCE); the frozen
  ws/dispatch/workspace-exec contract (XL, not hugit-buildable); the fabricd `/v1/leases/{id}/exec` spawn-500 fix.
- **CoreLink:** confirm a revoked Clerk session fail-closes within the engine-token TTL (≤300s).
- **INFRA:** `hugit-prod-d1` shared token store (sessions vanish on reboot — needed before >1 instance); a pre-prod
  canary engine; CI runner pool recovery.

— hugit TL
