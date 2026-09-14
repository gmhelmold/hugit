# Hook-First Ledger And Empirical Evidence Rebuild

Baseline: `main` at `6f9bbfa`. Prior `62/62` benchmark is invalidated. Passing
process exit codes proved command invocation, not feature use or resulting state.

Status 2026-09-14: W1-W6 complete for journey-v1. Two cold ledger auditors
approved reconstructed ledger; research selected scenario/oracle/BagIt design;
runner produced 14 conformant observations, 2 expected refusals, and 1
not-applicable boundary; independent verifier recomputed 14 semantic claims and
validated boundary taxonomy. Source-to-binary binding and execution causality
remain explicitly unestablished. W7 complete after two cold auditors returned
`APPROVE`; artifact remains unpublished until immutable hosting and independent
machine verification.

## Outcome

Rebuild feature ledger from shipped CLI-local implementation. Rebuild benchmark
as inspectable product journey driven by real Git hooks. Every publishable claim
must point to retained bytes showing input, operation, and resulting state.

## Non-Negotiable Evidence Contract

Successful run retains one report directory containing:

- full test repository, including `.git`, installed hooks, Git objects/refs,
  hugit runtime log, receipts, completion markers, projection status, and intent
  sidecar;
- complete stdout/stderr plus argv for every invoked operation;
- Git-native snapshots: `status`, `log`, `show`, refs, hook inventory, and tested
  commit OID;
- semantic assertions over canonical records, each naming source records and
  extracted values;
- manifest with relative path, byte length, and SHA-256 for every retained
  evidence file; missing, empty, unreadable, or unmanifested required evidence
  fails run;
- summary distinguishing operations executed, semantic claims proven, expected
  refusals, and unsupported scope. Exit status alone proves none of these.

Report prints durable path and never deletes it. Docs may cite a verified run
only after artifact inspection and hash verification.

## End-To-End Journey

1. Build/select release binary; record binary hash and version.
2. Create isolated HOME/config and deterministic local Git fixture; use no network.
3. Run `hugit setup`/`attach`; preserve installed hook bytes and ownership state.
4. Make real file change and real `git commit`; never call manual commit capture
   for primary proof.
5. Wait for durable receipt projection; prove hook principal, receipt id, changed
   path, branch, and exact Git OID in canonical log.
6. Create campaign plus intent with charter and acceptance claim.
7. Open PR binding exact hook-captured OID and intent id.
8. Record provider usage with exact model/token split; record checked verdict
   claim; run check; queue and land PR.
9. Prove joined graph: commit ↔ PR ↔ intent ↔ charter ↔ model/usage ↔ checked
   claims ↔ exact priced envelope. Preserve source record seq/hash for each leg.
10. Exercise separate PR through union landing so direct settlement does not
    pre-empt union evidence.
11. Exercise remaining shipped command families with state assertions appropriate
    to each mutation/read; expected refusals require unchanged-state proof.
12. Snapshot final repository and evidence, generate complete hash manifest, then
    verify manifest from disk before reporting pass.

## Sequence And Work Packages

Order is strict. Benchmark design cannot start from unapproved ledger.

| WP | Range | Owner | Files | Deliverable | Acceptance |
|---|---:|---|---|---|---|
| W1 | 0-25 | ledger author | `docs/feature-ledger.md`, source/tests read-only | source-derived hook-first ledger | each claim cites implementation plus observable state; unknown stays unknown |
| W2A | 25-35 | cold auditor A | ledger + source/runtime | runtime-reachability audit | prove dispatch/call path/state effect; reject tests or docs as sole proof of use |
| W2B | 25-35 | cold auditor B | ledger + source/contracts/security | epistemic/adversarial audit | falsify overreach, scope drift, hook/MCP confusion, local/live confusion, vacuity |
| W3 | 35-40 | lead | `docs/feature-ledger.md`, `README.md` | adjudicated approved ledger | every finding resolved; both auditors recheck changed claims; no unsupported LIVE statement |
| W4 | 40-55 | benchmark researchers | approved ledger + external benchmark practice | options/recommendation for empirical product benchmark | evidence retention, reproducibility, semantic oracles, anti-vacuity, mutation testing, artifact portability |
| W5 | 55-75 | harness author | `scripts/benchmark-feature-ledger.sh` plus dedicated helpers/tests | benchmark derived from approved ledger | full test repo retained; semantic state evidence; complete hash manifest; no exit-code-only pass |
| W6 | 75-90 | lead | integrated diff + generated report | empirical run and artifact inspection | retained repo opens with Git; hashes verify; ledger↔case mapping closes both directions |
| W7 | 90-100 | 2 cold benchmark auditors + lead | harness, full report, repo | adversarial review + mutation probes | hook disabled causes red; context link removed causes red; artifact removed causes manifest red |

## Ledger Approval Gate

Two auditors receive cold context after W1. Neither sees author reasoning or
other auditor output before submitting verdict.

Auditor A, runtime reachability lens:

- start from CLI registry/dispatch and follow real call path;
- identify persisted bytes or read projection proving behavior happened;
- classify implementation-only, test-only, historical, discontinued, or shipped;
- refuse claim when only evidence is successful process exit.

Auditor B, adversarial epistemic lens:

- try to falsify every status and scope statement;
- inspect hook versus MCP ownership, local versus remote/live tense, honest-zero
  semantics, and hidden prerequisites;
- calibrate absence claims with positive controls;
- name unsupported, ambiguous, or broader-than-evidence wording.

Allowed verdicts: `APPROVE`, `FIX_FIRST`, `REJECT`. Every approval line requires
file/line or command/artifact evidence. “Looks correct”, inferred intent, prior
docs, and green tests without runtime state are not proof. Ledger remains
unapproved until both verdicts are `APPROVE` after fixes.

## Benchmark Research Gate

Research starts only after ledger approval. It must compare at least:

- scenario/journey benchmark versus command matrix;
- state-transition oracles versus stdout/exit assertions;
- retained working repository versus exported evidence bundle;
- reproducible local fixture versus network clone;
- content-addressed artifact manifest and independent verifier;
- mutation tests proving each load-bearing oracle can fail;
- benchmark result taxonomy separating invocation, semantic proof, expected
  refusal, performance measurement, and unsupported scope.

Research also covers professional publication:

- how leading open-source systems publish benchmark methodology, raw artifacts,
  environment disclosure, limitations, and reproduction commands;
- how to register immutable run identity: source SHA, binary SHA, fixture SHA,
  platform/tool versions, timestamps, schema version, artifact hashes;
- how to present result without hype: one falsifiable headline, concise method,
  evidence table, inspectable repository, known limits, and exact rerun path;
- how to visualize provenance graph and before/after state without hiding failed,
  unsupported, or honest-zero cases;
- how to package GitHub/README/Reddit material for expert scrutiny, keeping raw
  evidence one click away and avoiding vanity metrics or unverifiable comparisons;
- what independent review, statistical treatment, warmup/repetition, uncertainty,
  and performance controls apply. Feature-use proofs and performance measurements
  must remain separate categories.

Recommendation must map each approved ledger claim to observable evidence and
state what benchmark cannot prove. Only then may W5 change harness.

## Executed Semantic Values

Primary proof uses:

- intent `intent-evidence`, charter `prove local journey`;
- model `claude-opus-4-8`;
- tokens: input `1,500,000`, output `200,000`, cache read `4,000,000`, cache write `1,000,000`, total `6,700,000`;
- exact cost `20,750,000` micro-USD under `price_card=pc-2026-07`;
- checked claim `security:approve`;
- PR `PR-EVIDENCE` binding exact OID produced by real `git commit`.

Any mismatch fails. Unknown/absent usage belongs in separate honest-zero negative
case, never substituted into primary proof.

## Scope Boundary

Normal Git observation is hook-first. MCP is not normal Git capture; explicit MCP
capture remains only for tools such as jj whose internal commit creation does not
fire Git hooks. Hook observes Git facts. Charter/model/cost/claims become commit
provenance only through explicit commit-plus-intent PR binding; unbound commits
remain visibly unlabelled rather than receiving invented context.

Permanently discontinued remote execution, remote AC, identity, tenancy,
forge/hosting, mirror deployment, and external runner attestation remain outside
ledger and benchmark except explicit rejection/boundary evidence.

## Stop Conditions

- Test repository or canonical log not retained.
- Manifest covers only summaries while omitting raw evidence.
- Primary commit was captured by direct CLI call instead of installed hook.
- Assertion passes after removing commit/intention binding or required source
  record.
- Published count mixes semantic proofs with mere invocations without labels.
- Any live/deployed claim lacks live probe; benchmark proves local behavior only.
