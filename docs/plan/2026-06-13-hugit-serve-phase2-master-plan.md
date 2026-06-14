# hugit-serve Phase 2 — "close the product" master plan (TechLead)

**Baseline:** `072e21f` @ `main` · **Date:** 2026-06-13 · **Lead:** hugit techlead (orchestrator)
**Driver:** `../githugr/docs/handoff/2026-06-13-hugit-phase2-close-the-product.md` (the window is LIVE in
`fixture`; the whole gap to "real product" is engine-side).
**Frozen contract:** `../githugr/docs/spec/2026-06-11-backend-api-v1.md` + the `githugr-live` clients +
`crates/githugr-live/tests/parity.rs` (the acceptance ORACLE).

> **The axiom:** judgment stays with the lead; agents execute pre-decided, verifiable slices only.
> **The anti-corruption rule (banked incident):** file-mutating agents in the SAME tree corrupt
> state. THEREFORE: build agents RETURN CODE AS STRUCTURED TEXT; the orchestrator writes + compiles
> + tests + audits centrally. No agent runs `git`/`cargo`/`fmt` in the shared tree. (Worktree
> isolation only where an agent genuinely must build.)

---

## 0. The scope, honestly tiered (recon-grounded, not guessed)

Three blocks from the handoff, re-tiered by what the engine actually has today:

- **Contract** (root dependency): 26 new read VMs + 4 shared atoms + the write/error/session shapes
  → `hugit-http-contracts`. **Mechanical, low-risk, byte-perfect.**
- **Reads** (26): thin domain→VM mappers in `hugit-serve`. Real-where-real + honest-defaults.
  **Mechanical fan-out.**
- **Writes** (9 verbs): split by engine reality —
  - **Tier 1 (wrap existing):** `verdict`, `undo`, `land`(union).
  - **Tier 2 (extend existing):** `policy_set` (delta-apply), `dispatch` (compose), `land` serial/window.
  - **Tier 3 (build spine):** `comment`, `issue_transition`, `erasure` (2-phase), `edit_propose` —
    **new canonical event kinds + D14 endpoints. JUDGMENT-HEAVY. Lead-owned, sequential, adversarially reviewed. OWNER DESIGN CHECKPOINT before it lands.**
- **Seams:** `/v1/token` (RFC-8693 logic + fixtures; live JWKS/key = P2), SSE (frames/since/gap/heartbeat
  over a local log-tail; live fan-out = P2), R2 read source (a `LogSource` in `state.rs`).

---

## 1. GO/NO-GO

| Phase | Mode | Why |
|---|---|---|
| A — Contract freeze | **PARALLEL (text-return)** | disjoint module files; shared-file (`lib.rs`/`common.rs`) ELIMINATED — lead assembles them |
| B — Reads (26) | **PARALLEL (text-return)** | one disjoint `handlers/<x>.rs` + `tests/parity_<x>.rs` per VM; lead assembles routing |
| C — Writes HTTP scaffold + Tier-1 | **HYBRID** | uniform plumbing (lead) + 3 thin wrap handlers (agents) |
| D — Writes Tier-2/3 (spine) | **SEQUENTIAL, lead-driven** | new event kinds + D14 + hash-chain = decisions; owner checkpoint |
| E — Seams | **HYBRID** | 3 focused pieces; token/SSE logic buildable now, live infra P2 |
| F — Adversarial audit | **PARALLEL (read-only)** | distinct lenses; free to fan out; then RE-audit |

**Concurrency cap:** ≤10 building agents per wave (text-return removes tree-contention, but merge-triage
cost still bounds the batch). Read-only audit/recon fans wider.

---

## 2. CONTRACT FREEZE (the único shared dependency) — DECIDED

From the cartographer (full type-graph map, `docs/review/` notes):
- **4 NEW shared atoms → `common.rs`** (lead writes): `KpiVm`, `KpiSubKind` (Default+Copy),
  `GithubAppStripVm` (**f64 → PartialEq-only**), `DashboardRepoVm`.
- **26 module files** (one per VM): full body transcribed from `provider.rs`, derives + serde attrs
  copied VERBATIM. House rules: all derive `Debug,Clone,PartialEq,Serialize,Deserialize`; add `Eq`
  ONLY where the source has it; **f64-bearing → PartialEq-only**; copy every `#[serde(default|rename|tag|rename_all)]`.
- **f64-bearing (PartialEq-only):** `GithubAppStripVm`, `DashboardVm`, `GithubAppVm`, `CampaignVm`.
- **Source-PartialEq-only-without-f64 (copy derive verbatim):** `InsightsVm`, `AttentionDecisionVm`, `AttentionVm`.
- **Internally-tagged enums** (`#[serde(tag="kind", rename_all="snake_case")]`): `AnswerSegmentVm`, `BlobReasonSegVm`.
- **`#[serde(rename="cost_usd_micros")]`**: `MetricsVm` (new); `CampaignPrVm` already done.
- **Write/error/session shapes → new `actions.rs`** (lead writes): `Accepted{seq,note,extra,queue_pos,pr_number,branch,state,charter_preview}` (Eq), `Denied` enum (externally-tagged), `SessionVm{user,org,fresh_auth}`.
- Reused atoms already present → import, never redefine (CampaignChipVm, DiffVm, VerdictVm, CostSplitVm, …).
- **Collisions: 0** (cartographer-verified).

**DoD (contract):** crate compiles `--locked`; every new VM has a round-trip parity test
(`serde_json` value-equality) against the canonical Appendix-A / `provider.rs` shape; `cargo deny` clean.

---

## 3. WP TABLE (phased)

### Phase A — contract (text-return agents; lead assembles `common.rs`+`actions.rs`+`lib.rs`)
| WP | Modules (disjoint) | model | dep |
|---|---|---|---|
| A0 | `common.rs` (+4 atoms), `actions.rs`, `lib.rs` wiring | **lead** | — |
| A1 | intent_detail, insights, security | sonnet | A0 |
| A2 | repo_settings, review, issues | sonnet | A0 |
| A3 | branches, releases, knowledge, search | sonnet | A0 |
| A4 | blob, edit, compare, commit_detail, new_pr | sonnet | A0 |
| A5 | campaign, dashboard, attention, repo_chrome, viewer_can | sonnet | A0 |
| A6 | account, import, login, github_app, org, profile | sonnet | A0 |

### Phase B — reads (text-return; lead assembles routing + audits redaction)
Order: **Group A (engine-backed) → B (partial) → C (stub)**. One `build_<x>(log,repo[,args]) -> Vm`
per WP + parity test. Free-text fields scrubbed via `crate::fmt::scrub`; Group-B/C gaps = honest
defaults (`""`/`0`/`[]`/`null`), NEVER faked.
- **B-A:** repo_chrome, viewer_can, intent_detail, attention, commit_detail, branches, insights, dashboard, security
- **B-B:** campaign, new_pr, compare, releases, repo_settings, review, search, issues, blob, edit, knowledge, github_app
- **B-C:** org, profile, account, import, login

### Phase C — writes plumbing + Tier-1
| WP | Owner-files | model | dep |
|---|---|---|---|
| C0 idempotency ledger | `hugit-serve/src/idem.rs` (atomic-rename on `DirColdStore` pattern; key=SHA256(LP(principal)·LP(verb)·LP(idemkey)); 24h TTL; byte-identical replay; mismatch→409) | **lead/opus** | — |
| C1 error+auth ext | `error.rs` (+5 codes), `auth.rs` (→ returns `Principal`), `server.rs` (POST gate, Idempotency-Key, step-up) | **lead** | C0 |
| C2 Tier-1 handlers | `handlers/w_verdict.rs`, `w_undo.rs`, `w_land.rs` (union) — wrap `verdict::record`(expose pub), `refstore::undo::undo`, `pr::land` | sonnet | C0,C1 |

### Phase D — writes Tier-2/3 (**lead-driven, sequential, OWNER CHECKPOINT**)
New canonical event kinds + D14 endpoints + queue modes. Each verb = its own mini-design + adversarial
review. NOT mass-fanned. Gated on an owner design sign-off (which event kinds become canonical).

### Phase E — seams
| WP | Owner-files | notes |
|---|---|---|
| E1 `/v1/token` | `auth.rs`/new `token.rs` | RFC-8693 validate (jsonwebtoken=`=9.3.1`, already in lock via hugit-queue; JWKS via `ureq`). Logic+fixtures now; **live JWKS URL + signing key = P2** |
| E2 SSE | new `sse.rs` + `server.rs` | `id==data.seq`, `: hb` ≤25s, gap@retention-edge, since-cursor; over local log-tail. **Live fan-out = P2** |
| E3 R2 source | `state.rs` `LogSource{Local,Http}` | sync `ureq` fetch of `<repo>.json`; recommend Worker-reads-R2 (zero engine dep) as the deploy default |

### Phase F — adversarial audit (read-only fan-out) → fix at root → RE-audit
Lenses: **contract-fidelity · honesty/no-fake · security/redaction · oracle-parity · SOTA-quality ·
completeness-critic**. Bake findings into permanent guard tests (secret-MATRIX per new surface).

---

## 4. RETURN-SHAPE (every build agent)
```
WP: <id>
FILES: <path> — full file body in a single ```rust block (compile-ready, house style)
TESTS: <path> — full parity/guard test body
IMPORTS: <exact `use` lines from crate::common / crate::actions>
DEFAULTS-USED: <every honest-default field + the one-line justification (no-fake proof)>
SCRUB: <every free-text field routed through crate::fmt::scrub>  [reads/writes only]
NOTES: <≤3 lines: assumptions, anything the lead must verify>
```
No prose narration of the code. No `git`/`cargo`/`fmt`. Return text only.

## 5. DoD / completeness / quality bars (held on every WP)
- **Compiles** `cargo build --workspace --locked` + **clippy** `--workspace --all-targets --locked -D warnings`.
- **Tests** `--workspace --locked` green; new surface has a parity test + (reads/writes) a secret-MATRIX guard.
- **Contract fidelity:** byte-for-field vs `provider.rs`; serde attrs verbatim; f64→PartialEq-only.
- **Honesty:** every gap is a documented honest-default, never fabricated; disclosed in PS-18/PS-19.
- **Redaction:** every echoed free-text field passes `crate::fmt::scrub` at the boundary (reads) /
  write-path scrub before append (writes).
- **Oracle-parity (writes):** passes the exact `githugr-live` parity asserts (status→Denied map,
  Idempotency-Key, queue_pos, step-up) without the window changing a line.
- **`cargo deny` clean.** No `#[allow]`, no `--no-verify`, no deferral without an OWNER WAIVER line.

## 6. MERGE ORDER (DAG)
`A0 → (A1..A6) ⇒ contract PR` → `(B-A) ⇒ reads-A PR` → `(B-B,B-C) ⇒ reads-BC PR` →
`C0→C1→C2 ⇒ writes-tier1 PR` → `E1/E2/E3 ⇒ seams PR` → `D (owner checkpoint) ⇒ writes-spine PR(s)` →
`F audit gates each PR`. Each PR: branch → CI → merge green (local gate + runner). Audit BETWEEN phases.

## 7. P2 / disclosed seams (no-fake honesty — tracked, not shipped silently)
- Idempotency store is **local-durable** (single instance). Multi-instance shared store (R2/D1/DO) = P2.
- `/v1/token` live JWKS URL + engine signing key = P2 (logic + fixtures land now).
- SSE live fan-out (CoreLink pub/sub) = P2 (frame/gap/heartbeat logic lands now over local tail).
- Group-B/C reads with no engine data = honest-defaults until the data source (R2/P2) + git-layer read.
- Caller-asserted principal until identity rollout (ADR-0002) binds it (existing P2 seam).
→ append all to `docs/plan/2026-06-11-pending-seams.md` (PS-19 block).
