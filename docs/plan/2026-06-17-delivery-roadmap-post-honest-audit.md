# Delivery roadmap — post honest-audit (2026-06-17)

> TechLead wave plan to attack the gaps in
> `docs/review/2026-06-17-honest-delivery-audit-double-checked.md`, grounded in a
> 44-agent read-only scope+design+verify sweep. Doctrine: **parallelism comes
> from disjointness + pre-decided contracts, never from cutting quality**; agents
> return code-as-text, the lead integrates centrally + runs the full gate
> (parallel tree-mutation is the banked corruption incident); judgment stays with
> the lead. Aggressive fan-out on DRAFTING/VERIFYING; disciplined per-disjoint-PR
> integration, each fully gated.

## What is fleet-buildable vs gated (honest partition)

| Class | Items | Disposition |
|---|---|---|
| **Fleet-buildable NOW (ungated)** | Wave-A `/v1` reads (new_pr, login, compare, search-fix, knowledge-stub); `hugit issue transition` CLI; reserved CLI verbs that wire to existing D-phase logic (undo·policy·journal·diag·approve·reject) | drafted by fleet → lead integrates per-PR + gate |
| **TL-decided then build** | login auth placement (→ pre-match guard, mirrors `readyz`); issue_create authz (→ `Endpoint::Push` universal — anyone files an issue) | decided here, in the build briefs |
| **OWNER-decision-gated** | **CAS/file-content seam → blob/edit/symbol** — reverses **PS-18** (owner decided blob/edit stay honest-default fixture). Needs owner sign-off to reverse. | BLOCKED on owner |
| **Architecture-gated** | knowledge BYOK + any in-request LLM/CAS call — `tiny_http` is single-threaded; 30-60s calls HOL-block ALL requests. Needs a worker-thread / async redesign first. | design wave before build |
| **Infra/owner-gated** | CoreLink P2 tenant (hot CAS+AC), runner fabric, live git wire serving, GitHub App, multi-tenant Clerk + `hugit-prod-d1`, deploy current `main`, cold-store, Rekor, fabric attestation pubkey | not fleet-able; owner/infra |

## Wave sequence (DAG-ordered)

- **W1 — `/v1` read-surface completion** (one atomic PR; central): new_pr, login,
  compare, knowledge-honest-stub created + search.rs fixed in place. Fixes from
  the verify fleet baked in (append_for_test not manual EventRecord; COMMITS_CAP
  truncate-after-reverse; search pr_lifecycle uses all_pr_queued; login pre-match
  guard). DoD: `cargo fmt --check` + `clippy --workspace --all-targets --locked -D
  warnings` + `test --workspace --locked` green; each handler has empty-log +
  populated tests; route arms mirror a peer (auth/slug/load_verified/authorize_read).
- **W2 — `hugit issue transition` CLI** (separate PR; build-ready): inline
  reimpl in hugit-cli (no hugit-serve dep — circular), mirrors
  `write_issue_transition` VALID_STATES + scrub; X5 namespace-law check (verify
  `issue` ∉ `git help -a`); extend `build-engine-snapshot.sh`. Enables a real
  recorded snapshot.
- **W3 — reserved CLI verbs** (separate PRs, batched by disjoint file): undo·policy·
  journal·diag·approve·reject — each wires to existing D-phase backing logic. Per
  verb: spec (confirm backing reachable, else flag gated) → draft → adversarial
  verify. X5 law per verb. Lead integrates; the HUGIT_VERBS registry + main.rs
  dispatch are lead-scaffolded centrally (the shared-file hazard).
- **W4 — issue engine verbs (4B)**: issue_create (new `issue.created` event kind,
  `Endpoint::Push`) → issue_comment (depends on issue_create) → triage_trigger
  (gated on a policy GateFn redesign). Co-author spec §3 rows + githugr actions.rs.
- **W5 — CAS seam / blob+edit** (OWNER-GATED on PS-18 reversal): add `hugit-proto`
  dep + `Option<Arc<CasObjectSource>>` to AppState via eager startup load
  (`git cat-file --batch`, `HUGIT_SERVE_GIT_DIR` — avoids per-request tiny_http
  blocking); blob.rs + edit.rs; compare gains real diff data.
- **W6 — symbol index** (prereq: W5): `EventLog::append_inert_event` + `hugit-symbols`
  crate (tree-sitter at intent.landed) + additive `def_href` on OutlineItemVm.
- **W7 — knowledge BYOK** (gated: D1 + the tiny_http worker-thread redesign).
- **W8 — me/orgs identity tail** (gated: CoreLink profile API + githugr fallback confirm).

The infra/owner-gated class (P2 tenant, runners, live git serving, GitHub App,
Clerk, deploy) runs on the owner/infra track in parallel — see the audit doc's
critical path. The fleet attacks W1–W4 + W6 now; W5/W7/W8 unblock on decisions.
