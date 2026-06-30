# Design — precomputed read-index (path→revisions + content→files), built on the push path

> **Status:** proposed (hugit TL, 2026-06-29). Unblocks the shared fidelity bottleneck
> behind blob-"Histórico" (#70a) and code-search (#63): both currently do an unbounded-but-
> wall-clock-capped walk over the lazy-CAS git graph on the single-threaded engine, so a
> deep/large repo returns partial-or-empty within the ~2 s budget. The fix is to precompute
> the answer and serve it O(1).

## The problem (one sentence)
On the single-threaded lazy-CAS engine, every read that must traverse history or the full
tree (blob-history's `git log --first-parent <path>`, code-search's `O(tree × R2)` scan) costs
N synchronous R2 fetches → it is wall-clock-bounded for safety (2 s), so it returns only what
it reached: **populated for recently-touched / shallow, empty for deep / large.** A count or
budget can't fix that — only precomputation can.

## Key insight — there is already a build point: the push (receive-pack) handler
The engine has **no background-job infra** (single CF Container instance), which is why this
"needs a design decision." But it does NOT need one: **the receive-pack handler already runs
on every push** and already rewrites `refs.json` + `oid-index.json` in R2 after finalizing
objects to the CAS (`finalize_cas_push`). That is the natural, already-transactional moment to
**incrementally update the read-index** for the refs that advanced. No cron, no separate
worker — the index is a write-path side-effect, consistent-by-construction with the objects it
indexes (built in the same fail-closed finalize step; if the index write fails the push still
`ok`s — the index is best-effort/rebuildable, never a push gate).

## Two indexes, same mechanism (R2 JSON manifests, repo-scoped under the tenant)
1. **Path-history index** (`<tenant>/<repo>/index/path-history.json` or sharded):
   `path → [{commit_oid, author_time_ms, author, summary}]`, newest-first, capped at K (e.g.
   200) per path. Built by: on push, for each new commit on the advanced ref, diff its tree vs
   first-parent (the same touch-detection `blob_history` already does — reuse `read::history`
   + `tree_diff`), and prepend the touching entry to each changed path's list. **Bootstrap**:
   one bounded backfill walk per repo at first index-enable (or lazily: a read miss triggers a
   one-shot bounded walk that populates + caches). Read path: `blob_history` checks the index
   first (O(1) R2 GET), falls back to the live bounded walk on a miss (so it degrades to today's
   behavior, never worse).
2. **Code-search index** (`<tenant>/<repo>/index/symbols.json` or a trigram/postings shard):
   `content-token → [file_path, …]` (or a filename + symbol index for the cheap win first).
   Built by: on push, for each added/modified blob in the new tree, tokenize (reuse
   `hugit_symbols::outline_blob` for symbol names; optionally trigrams for substring) and update
   the postings. Read path: `search` consults the index (O(matches)) instead of the O(tree×R2)
   scan. This retires the `CODE_SCAN_BUDGET` latency-DoS at its root (the walk no longer happens
   on the read path).

## Honesty + safety invariants (non-negotiable, carry the existing discipline)
- **Real-or-absent:** the index stores only what the live walk would have produced; a miss
  falls back to the live bounded walk (never a fabricated/stale row presented as fresh).
- **Redaction at READ, not in the index:** store raw; `scrub()` author/summary/results at the
  read boundary exactly as today (so a redaction-rule change applies retroactively without an
  index rebuild). (Note: the index does NOT fix the redact over-match on `Org/branch` summaries
  — that's the separate #70b precision item.)
- **Best-effort, rebuildable:** an index write failure never fails the push (`ok` still means
  objects durable); a corrupt/missing index → read falls back to the live walk. The index is a
  cache, not a source of truth — the CAS git objects remain authoritative.
- **Single-writer consistency:** the index PUT rides the same single-instance + single-threaded
  invariant as the existing `refs.json`/`oid-index.json` rewrite; it inherits the same pre-HA
  `If-Match` follow-up (no new HA debt).
- **Bounded backfill:** the one-time bootstrap walk is itself wall-clock + count bounded (it
  must not wedge the engine) — partial-then-resume, or lazy-on-read-miss.

## Phasing (smallest valuable first)
- **P0 — path-history index** (unblocks #70a, the one we just shipped + saw empty for deep
  files): smaller surface, reuses `read::history` touch-detection, immediate user-visible win.
- **P1 — code-search index** (unblocks #63, retires `CODE_SCAN_BUDGET` at root): larger
  (tokenizer + postings), higher payoff (kills the read-path scan entirely).
- Both share the push-side build hook + the read-side "index-first, live-walk-fallback" shape;
  build P0's hook first, generalize it for P1.

## Open decisions for the owner / review
- **Sharding:** one JSON per repo vs per-path-prefix shards (a hot repo's path-history.json
  could grow large). Start single-file with a size cap + a TODO to shard.
- **Backfill trigger:** eager (one bounded walk at index-enable) vs lazy (on first read-miss).
  Lazy is simpler + self-healing; recommend lazy P0.
- **Storage:** R2 JSON (consistent with refs.json) vs D1 rows (queryable). R2 matches the
  existing manifest pattern + the zero-egress CAS story; recommend R2 for P0.

## What this does NOT need
No background worker, no cron, no second instance, no new tenant. It is a write-path side-effect
+ a read-path cache, both inside the existing engine. That is why it's tractable now.

## CRITICAL FINDING (2026-06-30, after recon) — the single-threaded-invariant constraint
The recon surfaced a real architectural fork that reshapes the phasing:
- The engine HAS an R2 write seam (`cas.rs::put_object`, used for `refs.json`/`oid-index.json`),
  reachable on prod (the `cas:rw` seam is live with receive-pack). So R2 index writes ARE possible.
- BUT a **complete first-view/first-query** needs the index built BEFORE the read, which on this
  engine means either (a) push-side incremental (covers only new pushes, not existing history) or
  (b) a backfill of existing history. A backfill that doesn't wedge the single-threaded accept loop
  would want a **background thread** — and that **BREAKS the single-threaded invariant the write
  path's safety explicitly depends on** ("the unconditional refs.json PUT is safe ONLY by the
  single-instance + single-threaded invariant"). So a backfill thread is **gated on the pre-HA
  `If-Match` conditional-PUT work** (the tracked pre-HA seam) — not free.
- A **read-side lazy/progressive cache** (build incrementally across reads, in-memory, single-thread-
  safe, no new thread, no write-path change) is the safe option that ships now — but it only helps
  **REPEAT access** (the first read of a path/term is still bounded-partial). Its value therefore
  depends on the access pattern:
  - **blob-history (#70a): LOW** — users view a file's history once; a repeat-access cache rarely
    helps. The real fix (complete first-view) needs the pre-build, which is HA-gated. → **DEFER #70a
    to post-HA**; the #217 carry-forward (~2x deeper per segment) is the honest interim.
  - **code-search (#63): HIGH** — search is issued MANY times over one repo; a lazy progressive
    in-memory index builds across the first few queries, then serves all subsequent queries O(1),
    and retires the per-query `CODE_SCAN_BUDGET` tree scan at root. Single-thread-safe, self-contained.

## Revised phasing (this finding supersedes the P0/P1 above)
- **P0 (build now): code-search lazy progressive in-memory index** — the high-value, single-thread-
  safe target. Build incrementally across queries (each query advances a bounded build segment +
  serves from the postings built so far), in-memory `AppState` cache, scrub at read boundary,
  invalidate/rebuild on HEAD change. No R2 write, no background thread, no write-path change.
- **P1 (post-HA): R2-persistent + push-incremental + backfill-thread** for complete first-view of
  BOTH indexes — gated on the `If-Match` conditional-PUT pre-HA seam (so a background writer is safe).
  This is where blob-history (#70a) gets its complete-first-view fix too.

— hugit TL
