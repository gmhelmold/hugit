# CLI porcelain wave — `intent` · `pr` · `campaign` (owner-directed 2026-06-10)

> Owner: *"isso aí tem que existir — fricção é pedir pro usuário fazer a mesma
> coisa de um jeito diferente sem ganho real. E quem vai digitar são os modelos
> de LLM que o usuário rodar."* The flow porcelain becomes real commands.
> **Status: SHIPPED** (2026-06-10 — PC0/PC1/PC2/PC3 complete; `intent new/show`,
> `pr open/land/show`, `campaign open/close/show` all live with stable JSON output,
> structured errors, idempotency, and D14 authz at the door).
> ~~Today's CLI has only why/impact/tournament/export; intents/PRs are born
> through engine seams with no porcelain; campaign has NO lifecycle at all.~~

## Design law (decided)

- **Git-proximate, LLM-first.** Names a git user guesses (`hugit pr open` ≈
  `gh pr create`); ergonomics for agents: **stable JSON on stdout always**
  (`--human` for pretty), structured errors carrying the suggested fix,
  **idempotent** (re-running `open` with the same key returns the existing
  record, exit 0, `"already_exists":true` — agents retry safely).
- **Hermetic-first like everything**: commands operate on the local
  event-log/refstore through the REAL append/projection paths
  (`canonical_json` → `Ledger::from_records`/`intents_from_log`); live DO/CAS
  binding stays the P2 disclosed seam.
- **CI trigger model (owner-confirmed):** union-test fires per-PR at landing
  (continuous); `campaign close` is NOT the CI trigger — it is the **seal**:
  final whole-bundle proof + Ledger "provado" + cost rollup (WP-F3's module)
  + campaign envelope sealing (full+compacted transcripts — hooks into
  WP-F2's capture; until F2b lands, sealing the envelope is a disclosed seam,
  never faked).
- **Frozen contracts stay frozen.** If a command needs a new event kind and
  the EventRecord shape can't take it additively → ESCALATE, don't unfreeze.

## WPs

| WP | Scope (owned files are DISJOINT after PC0) | Size · model |
|---|---|---|
| **PC0 — scaffold (lead-reviewed first)** | The ONE shared-file commit: `Command` enum + module stubs `cli/src/{campaign,intent,pr}/mod.rs` + lib.rs registry update (the no-drift test) + empty arg structs. Compiles green. | S · opus |
| **PC1 — `campaign open/close/show`** | `cli/src/campaign/**` + `tests/acceptance_pc1.rs`. open: charter + human owner (D14) + campaign.opened record. close: final bundle proof via queue/checks seams + F3 `campaign_rollup` printed + envelope-seal seam (disclosed until F2b). show: progress landed/in-flight/blocked. | M · opus |
| **PC2 — `intent new/show`** | `cli/src/intent/**` + `tests/acceptance_pc2.rs`. new: charter/acceptance/campaign → sidecar + claim through refstore seam, returns intent id. show: sidecar + envelope refs + verdicts. | S · opus |
| **PC3 — `pr open/land/show`** | `cli/src/pr/**` + `tests/acceptance_pc3.rs`. open: bundle intents → PROPOSED (authz: author is orchestrator/human, never subagent — D14 at the door). land: enters landing queue (LandableEntry), reports position. show: PR record incl. F3 `pr_record` rollup. | M · opus |

Conflict map: PC0 owns the shared files; PC1/PC2/PC3 own disjoint dirs —
parallel after PC0. Each: full gate in worktree, compact card, lead cold-verify.
DoD per WP: acceptance tests drive the REAL projections (no hand-faked state),
JSON output snapshot-tested, idempotency proven by a double-run test.

Sequence: gate-green main (post-F3) → PC0 → PC1 ∥ PC2 ∥ PC3 → integrate (DAG
order irrelevant post-PC0, disjoint) → CHANGELOG (lead).
