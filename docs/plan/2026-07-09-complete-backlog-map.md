# hugit — COMPLETE remaining backlog map (2026-07-09)

TechLead synthesis of a 5-agent read-only discovery sweep (code-debt · whitepaper↔live ·
gated/shoot · honesty/copy · perf/polish) + the lead's own perf diagnosis + frontier design
context. Baseline: `main 6c5d764`, engine live `2026-07-08-wave2-6c5d764` (the go-live wave
#306: off-loop search · CAS commits projection · GDPR1 erasure auto-executor [OFF] · durable
per-tenant repo registry · GitHub App webhook+token · cost-consume · 2 re-audit HIGH fixes).

Graded against the **wedge** (landing · cost-attribution · orchestration), NEVER git parity
(clone/fetch/push is live + rides git clean — a non-goal to "complete further"). Zero raw
`todo!()`/`FIXME`/`unimplemented!()` in the whole workspace — every gap is a deliberate,
documented seam. No gambiarra.

---

## 0. THE ONE KEYSTONE (everything structural collapses into it)

**Live runner execution — the `fabricd` spawn fix (corelink-runners TL, EXTERNAL).**
Nothing can *run* a check, a union-land batch, a regen, or an agent dispatch against real
compute until a lease actually spawns an off-box job and reads provider `/usage`. The fabric
is transferred + suite-intact; the spawn fix is unshipped (no ETA). This single artifact gates:
union-land execution · memoized-MISS execution · merge-as-re-execution · regenerative rebase ·
derived-file regen · the real per-PR cost SOURCE. → **Critical path unchanged: fabricd spawn →
identity → the rest.** (Identity is now largely CLEARED — see §4.)

---

## 1. PERF / UX — the owner's LIVE pain (www is slow, 2026-07-09)

Measured: each `/r/{repo}` page ≈ **0.5s client→CF network** (favicon static proves it) +
**~0.5s CF→container hop** (a bare 404 already costs ~0.5s engine-side) + **~0.5-1s engine work**
(single-threaded lazy-CAS re-walks the tree COLD from CAS ~11ms/object; repeated hits DON'T warm).
HA = 2 instances (parallel works, no serialization). NOT a #306 regression (home.rs untouched).

| # | Lever | Where | Gain | Size | Owner |
|---|---|---|---|---|---|
| P1 | **Precompute/cache the repo-home render** (tree+README+listing) in memory off-loop, mirror `search_index.rs`/`blob_history_index.rs` | `handlers/home.rs` + new cache module | ~0.5s/page | M | **code (me)** |
| P2 | Warm / enlarge the 64 MiB FIFO decoded-object cache; batch-warm the hot tree on first read | `cas.rs` `DEFAULT_OBJECT_CACHE_BYTES` | general read speedup | S-M | code (me) |
| P3 | Audit every read handler for per-object-serial CAS walks on the request path; route through the bounded batch-read | `handlers/*` | tail-latency | M | code (me) |
| P4 | **www VM per-screen call pattern** — how many engine round-trips per page (each ~0.5s hop) | ../githugr VM/loader | possibly the biggest amplifier of "demora horas" | ? | **githugr TL** (relay) |
| P5 | Container placement / smart_placement (CF→container ~0.5s hop) | githugr engine.wrangler.jsonc | ~0.3-0.5s/hop | S | infra/owner |
| P6 | CoreLink CAS batch-read latency (~11ms/object) | CoreLink | per-object floor | — | CoreLink TL |

**Honest ceiling:** even a perfect home cache only shaves ~0.5s of the ~1.6s; two ~0.5s network
hops (client→CF + CF→container) remain — those are infra/link, not code. The single biggest
*likely* amplifier of "every screen takes forever" is **P4 (www making N sequential engine calls
per screen)** — githugr TL's domain.

---

## 2. GO-LIVE / SHOOT — path to first-real-user (RE-SCORED: #306 closed 3 top items)

The forge journey (browse · authed dashboard · self-service repo create · per-tenant isolation ·
B5 HA @2) is **already live**. The 2026-07-09 githugr go-live audit's top items — RE-SCORED:

| Go-live item | Audit said | ACTUAL (post-#306) | Remaining |
|---|---|---|---|
| Empty flagship `/commits` (#1, P0) | to-do | **DONE** — #101 CAS commits projection, live in wave2 | githugr authed smoke to confirm it renders |
| Erasure auto-executor (#2, P0 legal) | to-do | **BUILT** — #102, shipped OFF | clw re-audit → flip `HUGIT_ERASURE_AUTO_EXECUTE` |
| Public code-SEARCH (#3, P1) | canned "skew" | **BUILT** — #103 real index, live | githugr smoke that the www queries it live |
| "apagado" copy flip + grace restore | greenlit | code ready | **OWNER + githugr** — one coordinated deploy |
| Prod ALERTING (#5, P1) | missing | missing | **infra/owner** — CF Logpush on engine + $-ceiling |
| Cost-killer non-zero (#4) | dark $0 | pre-wired, pin-green | **owner** cost-non-zero + runners emit (pilot-optional) |

**Honest minimum path to a credible first user:** (1) githugr authed smoke verifies the killer-data
(search/commits) renders live [githugr TL]; (2) the coordinated erasure copy-flip + grace-restore
[owner+githugr — biggest LEGAL gate, greenlit]; (3) clw re-audits + we flip the auto-executor env
[clw+me]; (4) prod alerting [infra]. Cost-killer non-zero trails the pilot.

---

## 3. POLISH (code I can build now — production-grade finish)

| # | Item | Where | Size |
|---|---|---|---|
| POL1 | **Suppress honest-zero `$0.00`→`—`** at 4 unguarded `fmt_usd_micros` render sites (the clearest "fabricated-looking value on screen") | githugr `intent.rs:345`, `landing.rs:797/1221`, `campaign.rs:608` | S |
| POL2 | Checks quarantine-rows honesty-TODO (name·why·dono·prazo VM list) when engine relays it | githugr `checks.rs:737` + hugit `handlers/checks.rs` | M |
| POL3 | Language consistency: English nav tabs / "First-pass" / "Checks→Actions" amid pt-BR | githugr `layout.rs`, `insights.rs`, `search.rs` | S (owner decides which) |
| POL4 | 503 warm-up legibility + clone-pack build-window 503-retry UX | hugit serve + githugr | S |
| POL5 | `.gitattributes CHANGELOG.md merge=union` (keeps getting lost in multi-session churn → recurring conflict) | hugit root | XS |

Empty-states audited **HEALTHY** (no blank/broken renders). The "mundo-demo" strip is the intentional
honesty mechanism, not a bug. Many STUB fields are honest-empties, render-guarded (no leak).

---

## 4. FRONTIER partials — remaining WPs (some code, most external-gated)

### GitHub App mirror ("embrace, don't assault")
BUILT (#306): webhook ingress + real installation-token minting. REMAINING: check-run posting on a
real PR (B1③), status/badge compat (E3), uninstall stateful revoke (B1⑤), the bidirectional mirror
SYNC (E1/E2/E6 — logic-tested, no live token-exchange/sync). **Gate:** a live GitHub App install +
`HUGIT_GH_TEST_REPO` (owner/infra); the redesigned sync engine is a real build.

### Multi-tenant (open signup)
BUILT (#306): durable per-tenant repo registry (WP-1). **Identity is now largely CLEARED** — Clerk
exchange is LIVE (authed smoke green with real clerk.githugr.com sessions 2026-07-07); stateless HMAC
token live (#278). REMAINING **code (me)**: task #76 — global platform cap + per-tenant rate-limit +
reserved-namespace list + the **boot-reconcile** (registry vs `list_repo_slugs`; converges the audit
MED + legacy back-fill). REMAINING external: bring-your-own-repo IMPORT (GitHub App + mirror, owner/infra).

### Live dispatch (cost-killer)
BUILT (#306): consume side (verify sig → honest cost verdict). Off-box A-path wire-proven live e2e.
REMAINING: the real agent-loop DISPATCH (merge-as-re-execution records demand, never spawns) + real
`/usage` cost SOURCE — both gated on **fabricd spawn (§0)** + owner rota-A cost. Lighting ✓cas render
is then owner cost-non-zero.

---

## 5. DEFERRED FIXES / follow-ups (code I can build now)

| # | Item | Where | Size |
|---|---|---|---|
| F1 | **W2 refs-refresh monotonic guard re-file** (dropped W3 as superseded by #294; W2 is a real MED correctness fix) | `state.rs` LiveRefs | S |
| F2 | **#102 TOCTOU** — execute-cascade should re-read standing before the terminal claim (now that #296 cancel is LIVE); narrow, not live-exploitable while auto-exec OFF | `erasure.rs` | S |
| F3 | #76 boot-reconcile (see §4 multi-tenant) — also fixes the registry write-side MED + legacy back-fill | `state.rs`/`tenant_registry.rs` | M |
| F4 | Thin-pack server-side base edge / `git.rs:1026` "push not supported" message — VERIFY vs the live push (may be a stale mode-specific path) | `git.rs` | S (verify first) |

---

## 6. GATED items (external trigger — NOT my code)

| Item | Built? | Trigger | Owner |
|---|---|---|---|
| ✓cas cost RENDER | YES (pin green) | runners flip `FABRIC_EMIT` + real `/usage` source + owner cost-non-zero | runners TL + owner |
| "apagado" copy flip | YES | copy flip + grace restore, one coordinated deploy (Track-B GREENLIT) | owner + githugr |
| Auto-executor env flip | YES (#102, OFF) | clw re-audit → set `HUGIT_ERASURE_AUTO_EXECUTE` | clw + owner |
| Live runner DISPATCH | partial | fabricd spawn + `/usage` + mint-body freeze + rota-A | runners TL + server TL + owner |

## 7. OWNER-DECISIONS pending (genuinely yours)
1. **Cost-non-zero ruling** → unblocks ✓cas render + first public `/insights`.
2. **Prod grace-restore + copy flip** → the coordinated Track-B erasure execution (biggest legal gate).
3. **Rota-A cost** → live check-host dispatch / real per-PR cost.
4. **Prod alerting spend** → CF Logpush on engine + cost-ceiling.
5. (Settled: hugit closed-source; githugr private-to-anon-clone; identity live.)

## 8. OBSERVABILITY / INFRA
- **NO prod alerting** — nothing pages anyone on engine-drop / error-spike / CoreLink $-ceiling
  (the exact failure that took hugit down). CF Logpush + a cost-ceiling alert. (infra/owner)
- Container placement / smart_placement (see P5).

## 9. CODE-DEBT — the deliberate P2 seams (documented, env-gated, not gambiarra)
Big ones: runner fabric (§0) · CoreLink CAS-GC erasure executor (`CAS_GC_SEAM_WIRED=false`) ·
CoreLink prod P2 tenant (gates X6/X10/X11 live lanes) · diag live QueueApi · ledger cold-store ·
cache-$ / per-PR-raw-cost dollar seams · cold-store transcript blobs · Rekor tlog · jj first-class ·
reserved CLI verbs `ws`/`dispatch` (need the runner fabric). All surface loudly / fail-closed, never
fake-green. `import_git_url` deferred pending an SSRF audit.

---

## RECOMMENDED SEQUENCE (what the lead does next, autonomously)

**Now (code I own, no external gate) — a clean perf+polish+fix wave on stable main:**
1. **P1 repo-home cache** (the owner's live pain) + P2 object-cache warm — the perf keystone.
2. **POL1 `$0.00`→`—` suppression** (the clearest honesty-polish) + POL5 gitattributes.
3. **F1 W2 re-file** + **F2 #102 TOCTOU** + **F3/#76 boot-reconcile** (fixes MED + legacy back-fill).
4. Batch these into ONE PR → ONE CI → ONE redeploy (avoid repeated cold-reboot windows).

**Relay (I draft, owner couriers):**
- githugr TL: the **P4 www call-pattern** investigation (likely the biggest "slow" amplifier) + the
  authed killer-data smoke (search/commits render live) + the empty-flagship confirm.
- clw: re-audit the auto-executor irreversible-trigger surface (built OFF in #306).

**Owner-gated (I surface, you decide):** the 5 owner-decisions in §7 — chiefly the coordinated
erasure copy-flip+grace-restore (legal) and the cost-non-zero ruling.

**External-gated (not mine):** fabricd spawn (§0) — THE keystone for landing/orchestration/cost to
go live. Everything downstream waits on it; the code on hugit's side is ready.
