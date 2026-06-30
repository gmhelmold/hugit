# hugit LLM-native interface — skill + MCP design (study → design → build-ready)

> **Status:** 2026-06-30. Produced by a grounded study (6 parallel researchers over hugit's real
> surface + the techlead skill conventions) → design synthesis. Full study output:
> `tasks/wj7i4c3lz.output`. This is the actionable design + build plan + the owner's open questions.
>
> **Goal:** teach LLMs to use hugit intelligently with MINIMAL FRICTION — a `/hugit` orchestrator
> skill + a `/hugit-worker` agent skill + a 4-tool MCP arsenal, all in the techlead house style,
> anchored on hugit's #1 principle (don't deviate from git) + the structural honesty law.

## The two skills

### `hugit` — the ORCHESTRATOR skill (techlead-for-the-forge)
The campaign seat: open campaign → declare+slice **DISJOINT claims** (the go/no-go) → dispatch
one-intent-per-agent (author-kind=orchestrator; a subagent can never be a PR author, D14) →
convene diversity-enforced verdicts → queue → **UNION-LAND the batch (`land queue`)** so main is
always green — under the inviolable honesty law (cost attributed-or-null; "built"≠"live"; verify
with a probe). It DECIDES (charter, claim slice, merge/DAG order, verdict aggregate, land); it
DELEGATES only execution.
**Sections (techlead house style):** Frontmatter · H1+axiom ("hugit is git you already know with
provenance + claim-isolation + always-green main; if it feels un-git you're doing it wrong; the
wedge is the LANDING problem") · §0 when-to-invoke · §0.5 the forge-surface charter (verb-status
table: every verb | git analogue | cli-real-over-log / serve-live / reserved) · §0.6 the honesty
law (INVIOLABLE) · §1 the claim go/no-go · §2 the lifecycle + canonical `--log` recipe · §3 the
WAVE-PLAN decision record · §4 the **anti-pattern catalog AP-1..AP-8** (hand-merge instead of
`land queue`; `pr land` to bypass a red union; dispatch onto intersecting claims; hand-stamp
`--cost-usd-micros` [the #113 revert]; over-claim live from a green hermetic test/cache-hit;
casually probe heavy `/v1` reads against the single-thread engine [the prod-wedge DoS]; invent a
git-shadowing/reserved verb [`hugit merge`/`repo`/`approve`/`ws`]; read a 404 as "missing" not
"denied") · §5 integration matrix · §6 changelog.

### `hugit-worker` — the AGENT skill (the worker seat)
Spine: **SEAL (push) is the ONLY verb you perform** — clone the claim-scoped workspace (real git
wire), work STRICTLY in-claim, `hugit check run` to self-verify (byte-identical local≡forge),
SEAL. Does NOT land/queue/merge/union-test/undo (the system), does NOT argue a verdict
(FIX-FIRST/REJECT ⇒ re-enter EXECUTING, no self-defense), does NOT fabricate a cost/result/done,
NEVER pastes a secret (scrubbed to `[REDACTED]` permanently in the hash chain). Fail-closed by
default.
**Sections:** Frontmatter · axiom ("the failure this kills: a worker that lands its own PR /
fabricates a cost / pastes a secret / argues a verdict") · git-proximate translation table · §0
when-to-invoke · §1 the worker loop (charter+claim+acceptance → in-claim work → `check key`/`check
run` → SEAL) · §2 honesty+fail-closed law (parse stdout JSON, exit 0/2/1, act on `error.fix`) · §3
reading a verdict without arguing · §4 the context-envelope bequest (the agent dies per intent —
context.json is the only WHY/COST/TRUST record) · §5 anti-pattern catalog · §6 changelog.

## The 4 MCP tools (ONLY where they beat a CLI call)
| tool | why an MCP tool | input → output | backs |
|---|---|---|---|
| `mcp__hugit__claim-disjointness` | NO CLI verb exists (private `hugit-queue::AffectedSet::is_disjoint`); the model would skip it or hand-roll a flaky path compare | `{intents:[{id,claims}], baseline_sha}` → `{conflict_map, lanes, dag_edges, reslice_needed, parallel_safe}` | orchestrator §1 go/no-go |
| `mcp__hugit__land-status` | composes `land queue`+`queue show` into one typed land decision; structurally routes through the union test (refuses a bare `pr land` as union-land) | `{log_path, campaign?}` → `{verdict, landed, excluded, failing_pair, rollup{cost_usd_micros\|null}}` | orchestrator §2 + AP-1/2 |
| `mcp__hugit__cost-attest` | mechanizes the honesty law: integer micro-USD bound to a SPECIFIC tree_hash from the runner fabric; STRUCTURALLY OMITS the hand-stamp flags → the #113 over-claim is impossible at the tool boundary | `{repo, scope, id}` → `{cost_usd_micros\|null, attributed_to, source, spend_proof?, attested}` | orchestrator §0.6 + AP-4; agent §4 |
| `mcp__hugit__liveness-probe` | answers "is it live?" safely: `/readyz` + a real-token 200-vs-405 (git UA), disambiguates 404-denied/404-missing/401-gate/403-bot-block, REFUSES heavy reads (search/diff) against the single-thread engine | `{endpoint, method, auth, token?}` → `{http_status, interpretation, readyz, real_data_render, safe_to_probe}` | orchestrator §0.6 + AP-5/6/8; agent §2 |

Everything else stays documented CLI verbs (the 23 live `HUGIT_VERBS`) — don't over-tool.

## Build plan (ordered, build-ready)
1. `~/.claude/skills/hugit/SKILL.md` — the orchestrator skill (~13KB, table-dense; verb-status
   table seeded from `crates/hugit-cli/src/lib.rs::HUGIT_VERBS`).
2. `~/.claude/skills/hugit-worker/SKILL.md` — the agent skill (~7-9KB; cross-linked from the
   orchestrator §0/§5).
3. `~/.claude/skills/hugit/docs/MCP-CATALOG.md` — the 4 deterministic tool contracts (mirrors how
   techlead cites its MCP-CATALOG): input/output JSON, the source primitive each wraps, the
   honesty/safety invariant each enforces.
4. `crates/hugit-mcp/` (new in-repo crate) — the MCP server exposing the 4 tools
   (claim-disjointness→`hugit-queue::AffectedSet`; land-status→`land/mod.rs` union+bisect;
   cost-attest→`/v1 insights` + runner fabric, hand-stamp path omitted; liveness-probe→`/readyz` +
   a bounded Bearer probe with a git UA). Wire into the workspace + the gate.
5. (owner-greenlit) expose `claims(I)` + pairwise disjointness as a stable `hugit` read verb / lib
   entrypoint so claim-disjointness wraps a stable surface (today it's a private type, P2
   path-approximation).
6. Smoke + register: confirm the verb-status table == `HUGIT_VERBS` (no drift — the X5 oracle is
   the source of truth); liveness-probe refuses a `/search` heavy read; cost-attest can't emit a
   hand-stamp.

## Open questions for the owner (decisions before/within the build)
1. **MCP host:** a new in-repo `crates/hugit-mcp` Rust crate (cleanest — re-exports hugit-queue/cli,
   but adds gate surface), or a thin tools/ server? (Recommend the in-repo crate.)
2. **claim-disjointness surface:** ship the MCP tool against the private `hugit-queue` library with
   an honest "approximate until the engine queue lands the real tree-hash disjointness" caveat, OR
   first add a real `hugit impact`-composed claim-intersection verb/lib export? (Recommend ship-now
   against the library + caveat; the real verb is a follow-up.)
3. **cost-attest honest-null:** the real per-PR figure needs the runner fabric (P2-deferred). Ship
   the tool honest-null-only now (correct per the honesty law), or defer the tool until the fabric
   is live? (Recommend ship honest-null-only.)
4. **liveness-probe safe-allowlist:** hard-refuse ALL heavy reads (search/diff/many-object blob) by
   default, only probe `/readyz` + one bounded authed endpoint? (Recommend yes — the prod-wedge
   history demands it.)
5. **skill home:** `~/.claude/skills/` (user-global, like techlead — any session invokes `/hugit`),
   or vendor into the hugit repo (`skills/hugit/`, ships with the forge, version-locked)? +
   should the gate add a check/generator that fails if the skill's verb table diverges from
   `HUGIT_VERBS`? (Recommend user-global now + a gate drift-check as a follow-up.)
6. **verdict vocabulary drift:** CLI is `approve|fix_first|reject` but the `/v1` wire is
   `approve|request-changes` — reconcile the wire to the CLI (a contract change), or document the
   drift in the skill? (Recommend document now, reconcile later.)

— hugit TL
