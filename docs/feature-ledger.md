# Feature Ledger: CLI-Local Baseline `6f9bbfa`

This is a brownfield reconstruction from shipped source, dispatch, persistence,
read projections, and real-binary acceptance tests at `6f9bbfa`. Prior ledger,
README, plans, and benchmark prose were not evidence.

## Evidence And Status

| Status | Meaning |
|---|---|
| **SHIPPED+BINARY-OBSERVABLE** | Registry and `main.rs` reach implementation; source names effect; checked-in real-binary acceptance inspects feature-specific output or durable state. This is an executable observation specification, not a retained empirical run. Exit 0 alone is insufficient. |
| **IMPLEMENTED+TESTED** | Registry and dispatch reach implementation, with source-level tests, but this audit did not establish feature-specific real-binary observation for whole claim. |
| **HISTORICAL** | Code may remain, but belongs to former serve/remote product, not current CLI-local product. |
| **RESERVED** | Token appears only in `HUGIT_RESERVED_VERBS`; absent from `Command` and dispatch. Namespace protection, not feature or backlog. |
| **DISCONTINUED** | Permanently excluded from current product even if historical or reachable code remains. Do not benchmark or count as pending. |
| **ABSENT/UNKNOWN** | No source-plus-observation proof was established. Absence is asserted only after calibrated search; otherwise claim remains unknown. |

Dispatch evidence: `crates/hugit-cli/src/lib.rs::HUGIT_VERBS` and
`crates/hugit-cli/src/main.rs::{Command,main}`. `acceptance_rcli` checks registry
against binary command names, but equality proves reachability only.

Negative-search calibration: search over `main.rs` first found known
`Command::Capture` and `Command::Pr` arms, then found no `Command::Ws` or
`Command::Dispatch`. Search for cache backends found known `FileAc::new` calls
and `LeaseClient::from_runtime`, while finding no `HttpAcClient` in CLI source.
Thus top-level `ws`/`dispatch` are absent from dispatch, and `check`/`land queue`
select local file AC directly. This says nothing beyond searched CLI source.

## Hook-First Git Journey

Normal capture path is Git, not MCP:

1. `hugit setup --repo <repo>` or `hugit attach` installs/adopts hooks. Global
   `hugit setup` writes owned template hooks and `init.templateDir`.
2. Normal `git commit`, checkout, merge, rewrite, ref transaction, and push
   attempt invoke hook scripts from `crates/hugit-cli/src/init/mod.rs::hook_script`.
3. Hugit-owned hooks call `hugit capture`. `capture_ref_update` first writes scrubbed
   `ReceiptV1` under `<git-common-dir>/hugit/receipts/`, then starts detached
   drain. Hugit capture segment never waits for canonical append and exits 0.
   Adopted dispatchers run foreign hook synchronously and preserve its status;
   foreign latency or failure can still delay or fail Git (`attach::dispatcher`).
4. `capture::drain::drain_one` injects local-generated `receipt_id`, appends
   hash-chained `ref.update`, atomically writes
   `<git-common-dir>/hugit/event-log.json`, and marks completed receipt state.
5. Read commands integrity-check local hash chain. `why`, `watch`, and `fleet`
   also expose projection state; `health` exposes drain state. Capture use is
   proved by resulting event bytes, never hook exit status.

Normal post-commit hook supplies Git-derived `HEAD` OID. Commit capture payload
contains that `target`, `branch`, derived
`refs/heads/<branch>` ref, and changed `files` when Git can derive them from that
OID (`capture::capture_commit` and `changed_paths`). Checkout, rewrite, merge,
reference-transaction, and push-attempt use qualifiers on same `ref.update`
kind. Direct CLI/MCP `capture` accepts caller text and does not independently
prove target names a reachable commit. Push hook proves attempt plus typed
local/remote OIDs, not remote success.

Explicit MCP capture remains documented fallback for tools such as jj operations
that update refs without Git hooks. `crates/hugit-mcp/src/tools/capture.rs::run`
shells same CLI command but returns only `status:"dispatched"`; it has no
invocation-correlated landing proof. Verify canonical receipt/event separately.

Real-binary witnesses: `crates/hugit-cli/tests/acceptance_setup.rs`,
`acceptance_capture.rs`, `acceptance_capture_jj_checkout_merge.rs`, and
`acceptance_pr_commit.rs` inspect installed hooks, receipts/log records, payload
fields, and downstream commit binding.

## Capability Register

All rows derive from 29 tokens in `HUGIT_VERBS` and matching `Command` arms.
Source citation names implementation symbol. Artifact column names bytes/state
needed to prove use.

| Command and atomic claim ids | Status | Implemented capability and source | Persisted/read artifact proving use |
|---|---|---|---|
| `hugit setup` (`C-SETUP-GLOBAL`, `C-SETUP-REPO`, `C-SETUP-STATUS`) | **SHIPPED+BINARY-OBSERVABLE** (global, repo); **IMPLEMENTED+TESTED** (`--status`) | Global template or existing-repo hook install/status; `setup::{run,do_setup,do_status}`. | Binary witnesses inspect hook bytes, ownership marker, Git config, and subsequent `ref.update`; status has implementation tests but no binary witness established. |
| `hugit attach` (`C-ATTACH-PREVIEW`, `C-ATTACH-ADOPT`, `C-ATTACH-DETACH`) | **SHIPPED+BINARY-OBSERVABLE** | Preview/hash-bound adoption/detach preserving foreign hook; `attach::{run,do_run}`. Adopted dispatcher preserves foreign status. | Dispatcher bytes, immutable foreign backup, `manifest.json`, restored bytes, and foreign exit witness; `acceptance_attach.rs`. |
| `hugit detach` (`C-DETACH`) | **IMPLEMENTED+TESTED** | Remove managed hooks through `init::detach_run`; dispatched by `Command::Detach`. | Before/after hook bytes and retained `<git-common-dir>/hugit/` evidence. |
| `hugit health` (`C-HEALTH-HOOKS`, `C-HEALTH-LOG`, `C-HEALTH-DRAIN`, `C-HEALTH-COVERAGE`) | **IMPLEMENTED+TESTED** | Read hook/log/receipt/capability state; `health::{run,health}`. | JSON `mode`, per-hook state, log state, pending/dead-letter counts, coverage; no mutation claim. |
| `hugit capture` (`C-CAPTURE-COMMIT`, `C-CAPTURE-CHECKOUT`, `C-CAPTURE-PUSH-ATTEMPT`, `C-CAPTURE-MERGE`, `C-CAPTURE-REWRITE`, `C-CAPTURE-REF-TXN`, `C-CAPTURE-RECEIPT-ID`, `C-CAPTURE-DRAIN-STATUS`) | **SHIPPED+BINARY-OBSERVABLE** (six event forms); **IMPLEMENTED+TESTED** (receipt-id/status persistence) | Internal hook receipt producer/drainer; `capture::run`, `capture::drain`. | Binary witness inspects event payloads and recovery-directory quiescence in `acceptance_capture.rs`. Source/tests establish durable `ref.update.receipt_id` plus `status.json`; receipt and completion-marker files are transient. |
| `hugit campaign` (`C-CAMPAIGN-OPEN`, `C-CAMPAIGN-CLOSE`, `C-CAMPAIGN-SHOW`, `C-CAMPAIGN-LIST`, `C-CAMPAIGN-ABANDON`) | **SHIPPED+BINARY-OBSERVABLE** | `open|close|show|list|abandon`; `campaign::{CampaignCommand,run}`. | `campaign.opened|closed|abandoned` records and show/list JSON folded from same log; `acceptance_pc1.rs`, `acceptance_wbcamp.rs`. |
| `hugit intent` (`C-INTENT-NEW`, `C-INTENT-SHOW`, `C-INTENT-LIST`) | **SHIPPED+BINARY-OBSERVABLE** (`new`, `list`); **IMPLEMENTED+TESTED** (`show`) | `new|show|list`; sidecar/store plus optional canonical-log append; `intent::{IntentCommand,run}`, `intent::new::run`. | Binary witness creates store/log and inspects resulting new/list projection in `acceptance_wbint.rs`. Principal-chain and show details have source/library tests. |
| `hugit issue` (`C-ISSUE-TRANSITION`) | **IMPLEMENTED+TESTED** | `transition` appends state value `backlog|open|closed|dispatch`; `issue::transition::{run,ISSUE_TRANSITION_KIND}`. | `issue.transition` payload `{issue_id,to,priority?}`. `dispatch` here is inert enum value, not runner execution. |
| `hugit pr` (`C-PR-OPEN`, `C-PR-QUEUE`, `C-PR-LAND`, `C-PR-SHOW`, `C-PR-LIST`, `C-PR-ABANDON`, `C-COMMIT-INTENT-BIND`) | **SHIPPED+BINARY-OBSERVABLE** | Local lifecycle plus explicit captured-target/intent binding; `pr::cli::{PrCommand,run}` and `pr::{open,land,settle,show,list,abandon}`. `pr land --dispatch` excluded below. | `pr.opened|queued|landed|abandoned`, `intent.envelope`, `pr.envelope`; read JSON; `acceptance_pc3.rs`, `acceptance_pr_commit.rs`, `acceptance_wprlanded.rs`. |
| `hugit meta` (`C-META-SET`) | **IMPLEMENTED+TESTED** | `set` records local visibility/owner metadata; `meta::{MetaCommand,run}`, `meta::set::run`. | `repo.meta` payload. This records bytes only; it supplies no hosting or tenancy. |
| `hugit queue` (`C-QUEUE-SHOW`) | **SHIPPED+BINARY-OBSERVABLE** | `show` folds active `pr.queued`, verdicts, and `queue.union_fail`; `queue::{QueueCommand,show}`. | Queue JSON with ordered entries/batches/failing pair from canonical records; `acceptance_land_queue.rs`. |
| `hugit check` (`C-CHECK-RUN`, `C-CHECK-SHOW`, `C-CHECK-KEY`) | **SHIPPED+BINARY-OBSERVABLE** | Local `run|show|key`; memoized subprocess on miss, file AC, optional `check.recorded`; `checks::{CheckCommand,run}`, `checks::run::run`. | `<log>.ac` entry, run JSON `cache_hit`/memo axes/result, optional `check.recorded`, show/key JSON; `acceptance_wcheck.rs`. |
| `hugit verdict` (`C-VERDICT-RECORD`, `C-VERDICT-APPROVE`, `C-VERDICT-REJECT`) | **SHIPPED+BINARY-OBSERVABLE** (`record`); **IMPLEMENTED+TESTED** (`approve`, `reject`) | Three caller-supplied verdict recorders; `verdict::{VerdictCommand,run,record}`. | Binary witness covers `record` in `acceptance_wverdict.rs`; approve/reject have implementation/unit tests. No model call. |
| `hugit undo` (`C-UNDO`) | **IMPLEMENTED+TESTED** | Locally Human-role-labelled compensating append; role is caller-asserted, not authenticated; `undo::{run,do_run}`. | Additional inverse event referencing selected sequence; prior records remain byte-present. |
| `hugit policy` (`C-POLICY-TEST`, `C-POLICY-EDIT`) | **IMPLEMENTED+TESTED** | Local house-gate test and locally Human-role-labelled edit; role is caller-asserted; `policy::{PolicyCommand,run}`. | Test-result JSON and `policy.change`; no actor authentication or remote enforcement claim. |
| `hugit note` (`C-NOTE`) | **SHIPPED+BINARY-OBSERVABLE** | Append session note; `note::run` -> `journal::note::run`. | `journal.note` read by `ctx resume`; `acceptance_ctx.rs`. |
| `hugit diag` (`C-DIAG`) | **IMPLEMENTED+TESTED** | Read-only diagnosis over recorded check history; `diag::{run,do_run}`. | Structured diagnosis JSON derived from `check.recorded`; no `diag.recorded` event. |
| `hugit ledger` (`C-LEDGER`) | **SHIPPED+BINARY-OBSERVABLE** | Asked/done/proven projection; `ledger::{run,project}`. | JSON folded from integrity-checked local log; `acceptance_ledger.rs`. |
| `hugit fleet` (`C-FLEET`) | **SHIPPED+BINARY-OBSERVABLE** | Versioned local workspace/agent projection; `fleet::{run,project}`. | Fleet JSON from canonical records; `acceptance_fleet.rs`. No workspace execution. |
| `hugit watch` (`C-WATCH`) | **SHIPPED+BINARY-OBSERVABLE** | Replay classified, redacted local event stream; `watch::{run,project}`. | Ordered watch JSON rows; `acceptance_watch.rs`. Not network tailing. |
| `hugit symbol` (`C-SYMBOL-FILE`, `C-SYMBOL-COMMITTED`, `C-SYMBOL-UNSUPPORTED`) | **SHIPPED+BINARY-OBSERVABLE** (`--file`, unsupported); **IMPLEMENTED+TESTED** (committed) | Local-file or committed-blob tree-sitter outline; `symbol::{run,project}`. | Binary witness covers local file and unsupported empty result in `acceptance_symbol.rs`; committed mode has source/library tests. |
| `hugit ctx` (`C-CTX-RESUME`, `C-CTX-USAGE`, `C-CTX-USAGE-PRICE`) | **SHIPPED+BINARY-OBSERVABLE** (`resume`); **IMPLEMENTED+TESTED** (`usage`, pricing join) | Resume notes; usage appends submitted counters; land may price matching usage. | Resume witness in `acceptance_ctx.rs`; usage and pricing have implementation tests. Submitted counters are not provider-authenticated. |
| `hugit review` (`C-REVIEW-ANSWER`, `C-REVIEW-REFUSE`) | **SHIPPED+BINARY-OBSERVABLE** | Grounded retrieval over logged checks/verdicts, otherwise refusal; `review::{run,project}`. | Answer cites selected evidence or refuses; `acceptance_review.rs`. No model call. |
| `hugit land` (`C-LAND-QUEUE`) | **SHIPPED+BINARY-OBSERVABLE**, limited simulator | Queue union/bisect mechanics and file memoization; `land::{LandCommand,run,batch_land}`. Local runner always exits 0; red pairs come only from `conflicts-with:` strings. | `<log>.ac`, `pr.landed`, `queue.union_fail`; `acceptance_land_queue.rs`. Not repository-tree integration. |
| `hugit dock` (`C-DOCK-COIN`, `C-DOCK-LS`, `C-DOCK-SHOW`, `C-DOCK-CLOSE`, `C-DOCK-RECONCILE`, `C-DOCK-INSIGHT`, `C-DOCK-LAND`) | **SHIPPED+BINARY-OBSERVABLE** | Seven local worktree binding/read/reconcile/land modes; `dock::{DockCommand,run,coin_dock}` plus submodules. | Gitdir marker, dock events, cost samples/spool, dock JSON; dock acceptance suites. |
| `hugit why` (`C-WHY-PATH`, `C-WHY-WALK`, `C-WHY-LINE`, `C-WHY-SYMBOL`) | **SHIPPED+BINARY-OBSERVABLE** (path, walk); **IMPLEMENTED+TESTED** (line, symbol) | Path chain; precise modes use Git blame plus captured target; `main.rs::run_why`, `why::resolver`. | Binary hook path/walk witness in `acceptance_capture.rs`; precise modes have implementation/library tests. |
| `hugit impact` (`C-IMPACT`) | **SHIPPED+BINARY-OBSERVABLE** | Blast radius over caller graph; `main.rs::run_impact`, `impact::compute_impact`. | Exact result from graph/path input; `acceptance_rcli.rs`. No graph discovery claim. |
| `hugit tournament` (`C-TOURNAMENT-IN-CAP`, `C-TOURNAMENT-OVER-CAP`) | **SHIPPED+BINARY-OBSERVABLE** | Policy-capped deterministic candidate descriptions; `main.rs::run_tournament`. | Candidate JSON and over-cap refusal; `acceptance_rcli.rs`. No candidate runs. |
| `hugit export` (`C-EXPORT-GIT`, `C-EXPORT-ENVELOPE`, `C-EXPORT-REDACTION`) | **SHIPPED+BINARY-OBSERVABLE** (Git, envelope); **IMPLEMENTED+TESTED** (redaction) | Integrity-checked local-log exit artifact; `main.rs::run_export`, `export::export`. | Binary witnesses inspect usable Git directory/envelope; redaction manifest has library tests. No source-topology claim. |

## Provenance Chain

Chain is additive; no stage may infer missing context:

1. Hook capture: post-commit obtains OID from Git; `capture_commit` writes receipt
   facts; drain appends `ref.update` carrying receipt id, target, branch/ref, and files
   when derivable. Source: `capture/mod.rs::{capture_ref_update,capture_commit}`
   and `capture/drain.rs::drain_one`.
2. Intent: `intent new` persists non-authoritative sidecar and optional
   `intent.landed`; payload carries `intent_id` and charter, while acceptance and
   context ref remain in intent store sidecar. Source:
   `intent/new.rs::run`, `intent/canonical_log.rs::land_intent`.
3. Binding: `pr open --commit <oid> --intent <id>` accepts commit only when raw
   `ref.update.target` or complete push-attempt `updates[].local_oid` captured it;
   resulting `pr.opened` keeps `commit_ids` and `intent_ids` separate. Source:
   `pr::validate_commits`, `commit_ref_target_on_log`, `open`.
4. Usage: `ctx usage` appends as-submitted model and input/output/cache counters
   in `ctx.usage`; computes checked total only. It does not contact provider or
   prove counters came from provider. Source: `ctx/usage.rs::do_run`.
5. Verdict: `verdict record` appends caller-supplied per-lens results as
   `claims_checked`, aggregate verdict, tree hash, and empty evidence refs in
   `verdict.recorded`. Source: `verdict/mod.rs::record`.
6. Land envelope: non-dispatch `pr land` may price all matching `ctx.usage`
   records against frozen exact model card, complete-or-nothing, then append
   honest-zero per-intent envelopes and priced PR envelope. Manual nonzero
   metrics override auto-pricing. Source: `pr/capture.rs::{priced_usage,capture_on_land}`.

Boundary: normal hook target is a Git-derived fact. Direct CLI/MCP target is
caller-supplied text; current capture and PR validation do not authenticate
object existence. Hook alone knows no authoring context. Unbound captured commit has no intent,
charter, model, tokens, verdict, or price. `pr open` performs explicit join; no
fake intent is created. `acceptance_pr_commit.rs` checks separate arrays and
unchanged `intent.landed` count.

## Local And Unsupported Boundaries

- Runtime state is local hash-chained JSON under Git common dir. Verification
  detects partial edits, drops, insertion, or reorder. Chain is unkeyed: writer
  with full file access can rewrite records and recompute valid chain. It is
  integrity evidence, not authenticated truth. Explicit file logs and legacy
  migration exist; no server/account is required.
- `C-SCOPE-REMOTE-AC`: `check run` and `land queue` instantiate file-backed AC. CoreLink AC env is
  inert for these paths (`checks/run.rs::select_ac`,
  `land/mod.rs::run_queue_land`). Remote AC is **DISCONTINUED**, not backlog.
- `land queue` validates queue-engine and memo mechanics against synthetic PR
  content. `LocalRunner::run` always returns success. It does not merge trees,
  run repository checks, or establish production-safe union landing.
- `verdict` and `review` perform no model API calls. Verdict records supplied
  decisions; review retrieves existing evidence.
- `tournament` emits deterministic candidate descriptions; it launches no agent.
- Hook `pre-push` records attempted updates only. No remote acceptance proof.
- `C-SCOPE-IDENTITY`, `C-SCOPE-TENANCY`, `C-SCOPE-FORGE-HOSTING`: `repo.meta`, `owner_tenant`, and `ctx --tenant` are stored/projected local
  fields only. Tenancy, Clerk identity, and forge/hosting are **DISCONTINUED**.
- `C-SCOPE-RUNNER`, `C-SCOPE-EXTERNAL-ATTESTATION`: `pr land --dispatch` reaches legacy external runner code in
  `pr/cli.rs::run_land` and `pr/dispatch.rs`, but external runner execution and
  attestation are **DISCONTINUED** current-product scope. Reachability does not
  make this shipped capability; do not exercise or benchmark it.
- `C-SCOPE-MIRROR`: `hugit-serve`, forge APIs, mirror deployment, and remote deployment claims are
  **HISTORICAL**. Mirror deployment is also **DISCONTINUED**.
- `C-SCOPE-WS`, `C-SCOPE-DISPATCH`: `ws` and top-level `dispatch` remain **RESERVED** in
  `HUGIT_RESERVED_VERBS` and **DISCONTINUED** as execution products.
- Anything not named with source plus observable bytes here is
  **ABSENT/UNKNOWN**, not implied by module presence or green tests.

## Evidence Report Mapping

Journey-v1 selects 17 atomic claims from this register. Fourteen have independent
semantic oracles, `C-SCOPE-WS` and `C-SCOPE-DISPATCH` are expected refusals, and
`C-SCOPE-RUNNER` is retained as `not_applicable`. Remaining claims are unselected,
not silently passed or counted.

Runner retains selected executable, full repository, command streams, assertions,
and export in BagIt package. Separate verifier recomputes 14 known journey
semantics, validates two refusals and one not-applicable declaration, and fails
unknown selected claims. It does not prove source-to-binary build provenance or
cryptographic execution causality. Method, exact mapping, reference run, and limits:
`docs/benchmark-feature-ledger.md`.

Discontinued remote AC, identity, tenancy, forge/hosting, mirror deployment,
runner execution, and external runner attestation remain neither successes nor
pending work. Evidence report does not change their scope.
