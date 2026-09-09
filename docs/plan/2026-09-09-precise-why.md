# Precise Why: Git-Native Line And Symbol Provenance

**Status:** proposed implementation plan
**Date:** 2026-09-09
**Scope:** git-local CLI and hooks only. No runner, CI cache service, forge, workspace manager, CoreLink, or agent transcript capture.

## Goal

Make `hugit why` answer precise questions from Git facts without slowing or changing normal Git use:

```text
why --repo . --commit HEAD --path src/auth.ts --line 84
  -> selected committed-tree line 84
  -> Git blame commit that last introduced its bytes
  -> captured ref.update for that commit
  -> exact changed hunk, intent and attestation when present

why --repo . --commit HEAD --path src/auth.ts --symbol refreshToken
  -> current symbol range from its Git blob
  -> blame each line in that range
  -> all contributing captured commits, never an invented single author
```

The product is precise Git provenance. It does not claim to prove an agent's reasoning, intent, or runtime behavior unless separately captured evidence exists.

## Actual Baseline

- `post-commit` runs asynchronously and currently invokes `git diff-tree --name-only`; it passes paths to `hugit capture`. `crates/hugit-cli/src/init/mod.rs`.
- `capture commit` writes a hash-chained `ref.update` containing `target`, `branch`, and an optional string `files` list. `crates/hugit-cli/src/capture/mod.rs`.
- `why` accepts object entries with `path`, `ranges`, and `symbols`, but the installed hook produces only strings. `crates/hugit-cli/src/why/resolver.rs`.
- `hugit-symbols` already outlines a file/blob with Tree-sitter. `crates/hugit-symbols/src/lib.rs` and `crates/hugit-cli/src/symbol/mod.rs`.

The gap is producer-side precision, then current-tree-to-commit resolution. It is not missing CI or remote infrastructure.

## Invariants

1. Git hooks remain silent, detached, idempotent, and exit zero. A failed enrichment may lose only enrichment; it never blocks or changes `git commit`.
2. Hook payloads record observed Git facts only. No inferred author, intent, test result, cost, or semantic claim enters `ref.update`.
3. Existing logs with `files: ["path"]` remain readable and retain file-level `why` behavior.
4. Precise queries address a committed tree only. They never inspect or attribute uncommitted worktree/index bytes.
5. A line or symbol query that cannot establish its commit/range returns structured `unknown` or `unattributed`; it never widens to an unrelated file event.
6. Hunk precision is explicitly `complete`, `partial`, or `unavailable`; a caller can never mistake fallback file attribution for exact range attribution.
7. Blob parsing never occurs in a Git hook. Semantic data is cacheable by immutable blob id plus parser-schema version.
8. Query output distinguishes `observed` Git facts from optional `attestation` and `intent` records.

## Contract

Keep `ref.update` as the one captured event kind. Evolve its `files` field additively:

```json
{
  "target": "<commit oid>",
  "branch": "feat/auth",
  "files": [
    {
      "path": "src/auth.ts",
      "ranges": [{"start": 44, "end": 50}]
    },
    "docs/legacy.md"
  ],
  "hunk_capture": "complete"
}
```

- String entry: legacy file-level attribution only.
- Object entry: exact, non-empty target-side unified-zero diff ranges. This matches resolver's existing `{start,end}` shape.
- Added-file hunks are target-side ranges. Deleted-only and binary paths remain string entries because no line exists in target tree to attribute.
- `hunk_capture` is `complete`, `partial`, or `unavailable`. A partial/unavailable record may contain legacy strings but must never contain guessed ranges.
- Rename/copy lineage and merge-parent attribution are deferred. First slice answers paths that exist at selected commit; it never follows a renamed/deleted path implicitly.
- The event's `target` commit oid anchors blobs. Per-file blob ids are not duplicated in v1.

## TechLead Decision Record

```text
TechLead WAVE PLAN — precise-why @ 90cb84b | 2026-09-09
=========================================================
GO/NO-GO     : SEQUENTIAL
CONTRACT     : ref.update.files accepts legacy strings or typed target-hunk objects;
               hunk_capture states completeness explicitly.
DISJOINTNESS : no. WP1 owns producer+decoder; WP2-WP4 depend on that exact decoder
               and change why's answer contract.
CONFLICT-MAP : WP1 -> WP2 -> WP3 -> WP4. No implementation fan-out.
DOD          : real-Git end-to-end capture; explicit unknown states; no hook blocking;
               legacy logs readable; hash-chain verification precedes every projection.
MERGE ORDER  : one focused change per WP. Cold review after each behavior-changing WP.
```

The first implementation starts only after the typed `files` payload and its error law are frozen in tests.
No agent gets discretion to invent a new event kind, background daemon, transcript store, or remote service.

## Delivery Plan

### WP1: Capture Exact Hunks

**Owner files**

- `crates/hugit-cli/src/init/mod.rs`
- `crates/hugit-cli/src/capture/mod.rs`
- `crates/hugit-cli/src/why/resolver.rs`
- focused capture and resolver tests

**Change**

1. Remove shell word splitting of changed paths from `post-commit`. Hook passes only immutable commit facts (`HEAD`, root, branch, timestamp) to detached capture child.
2. In `capture commit`, obtain changed paths from Git NUL-delimited output. Never recover paths by parsing `diff --git` headers or shell-splitting whitespace.
3. For each discovered target path, request its own zero-context Git patch with the path passed as an argument after `--`. Parse only `@@ -a[,b] +c[,d] @@` headers and emit non-empty target-side `{start,end}` ranges. Omitted count means one; zero target count produces no range.
4. Bound per-commit hunk extraction by a named, tested file-count limit. On limit/unsupported binary/parser failure, preserve discovered paths as legacy strings and stamp `hunk_capture: partial` or `unavailable`; no result is silently treated as complete.
5. Extend resolver decoding for typed hunk entries. Keep present legacy string handling byte-compatible.

**Acceptance**

- Added, deleted, changed, empty-file, root-commit, binary, and paths containing whitespace/newlines are covered.
- A normal commit still returns before capture completes; a missing `hugit` binary or parse failure leaves Git success unchanged.
- Every captured range matches a real `git diff --unified=0` hunk in an end-to-end temporary repository.
- A deliberately malformed hunk fixture and an over-limit commit emit non-complete capture state rather than shifted ranges.

### WP2: Current-Line Resolver

**Owner files**

- `crates/hugit-cli/src/main.rs`
- `crates/hugit-cli/src/why/*`
- focused CLI and resolver tests

**Change**

1. Add explicit query target options: `--repo <path>` and `--commit <oid>`. `--commit` defaults to `HEAD`, resolved inside `--repo`; selected commit must exist in that repository.
2. For `why --line`, use `git blame --porcelain -L N,N <commit> -- <path>` to resolve the commit owning current bytes.
3. Verify `--path` exists at selected commit before blame. The query reads that committed blob, never filesystem worktree bytes.
4. Match captured `ref.update.target` to blamed oid before applying hunk attribution. Query never uses log recency as proxy for current line ownership.
5. Return typed `unattributed` when Git knows blame oid but Hugit did not capture it. Return typed `range_unavailable` when matching event has non-complete hunk capture. A complete capture whose hunks do not contain blamed line is `range_mismatch`, not a fallback to file history.
6. Retain old log-only path query unchanged. It remains history projection, not current-tree line query.

**Acceptance**

- Inserting lines above an attributed line does not change its answer.
- Changing a line twice attributes current bytes to second commit, not first commit or latest file event.
- A blame oid absent from hash-verified log returns `unattributed`, not another event from same path.
- Query refuses a line outside selected blob, missing commit, and path outside selected tree with structured errors. A caller may intentionally query an exported log against repository holding same commit objects.

### WP3: Cached Symbol Attribution

**Owner files**

- `crates/hugit-symbols/*` only if public output lacks source ranges
- `crates/hugit-cli/src/symbol/*`
- `crates/hugit-cli/src/why/*`
- cache implementation and tests

**Change**

1. Add an internal symbol-range projection from Git blob: `{name, kind, start_line, end_line}`. Current `SymbolItem` exposes only start line, so range is a deliberate new internal contract. Public outline stays unchanged.
2. Cache this projection under a Hugit-managed local cache keyed by `<blob oid>/<symbol schema version>`. Cache contents are derived and disposable; never append them to canonical log.
3. Resolve `why --symbol` against selected commit blob, then blame the complete symbol range. Group output by blamed commit and return all contributors in deterministic order.
4. Map each contributor through WP2's captured commit lookup. Uncaptured contributors remain explicit, not dropped.
5. Reject unsupported language, missing symbol, ambiguous overload, generated/binary blob, and oversized parse according to explicit structured result kinds.

**Acceptance**

- Same blob parses once across repeated symbol queries; test observes cache hit without relying on wall-clock timing.
- Symbol spanning commits returns multiple contributors, not one fabricated origin.
- A symbol with bytes from several commits returns contributors deterministically; no test claims rename/move lineage before that contract exists.
- Unsupported file returns `unsupported_language`, never file-level provenance.

### WP4: Causal Presentation, Not Causal Invention

**Owner files**

- `crates/hugit-cli/src/why/*`
- CLI integration tests and command documentation

**Change**

1. Keep existing log-only answer JSON unchanged. Precise line/symbol mode returns a separate, versioned additive answer grouped by epistemic class:

```json
{
  "observed": {"commit": "...", "path": "...", "range": [84, 84], "event_hash": "..."},
  "declared": {"intent": "...", "charter": "..."},
  "attested": {"model": "...", "cost": "..."},
  "status": "attributed"
}
```

2. Omit unavailable optional groups or emit `null`; never substitute empty strings as facts. `status` is one of `attributed`, `unattributed`, `range_unavailable`, `range_mismatch`, `unsupported_language`, `ambiguous_symbol`.
3. Document exact meanings: `attributed` means Git blame oid had a matching captured event. It does not mean behavior is correct or charter was satisfied.

**Acceptance**

- JSON consumers of current answer shape retain fields they use.
- Every status has fixture: attributed, unattributed, range_unavailable, range_mismatch, unsupported_language, ambiguous_symbol, and log-integrity failure.
- Tampered/reordered log fails before provenance projection.

## Explicit Deferrals

- Agent prompt/tool/transcript causality. Orchestra can later export separately signed session evidence keyed by commit/tree; this plan does not invent that bridge.
- Runtime traces, test proof, cost, and verdict generation.
- Full rename/copy lineage and merge-parent attribution. Add only after a raw NUL-delimited Git diff contract is specified.
- Background daemon. Capture is already detached. Symbol enrichment is query-time cached until a reliable queue is justified.
- New top-level command. Existing `capture` and `why` surfaces are sufficient.

## Verification

1. Unit tests: hunk parser, legacy decoder, blame parser, symbol contributor grouping, cache invalidation by schema version.
2. Deterministic real-Git integration tests call capture directly after real commits. One separate hook test uses bounded polling only to prove detached hook delivery; it is not oracle for parser behavior.
3. Negative controls: mutate recorded target oid, corrupt hunk range, omit captured commit, and tamper event chain. Each must fail or return explicit non-attribution.
4. Run repository scoped gate first, then full `verify.sh` before merge. Do not claim hook latency without a measured benchmark on real repositories.

## Order

WP1 -> WP2 -> WP3 -> WP4. Each depends on previous captured identity and query semantics. No parallel implementation across these steps until contracts are frozen.
