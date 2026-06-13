# hugit — correctness + quality code review (sweep 2026-06-12)

Reviewer: senior code reviewer (cold context). Scope: correctness bugs,
DRY/reuse, over-complexity, maintainability traps, API misuse, resource
handling. Security audit is a sibling agent's job; secret-redaction *logic* is
reviewed here only for correctness, not as a threat model.

`main` HEAD `5457730`. Read-only review. Weighted to Wave L/M (the chokepoint
`checks::rehydrate_and_verify`, the scrub unification `secret_shape.rs`, the
intent two-phase + `reconcile_store_from_log`, the redaction hybrid). Build +
the `secret_shape`/scrub unit suites were probed green during the review.

## Summary

| id | sev | area | one-line |
|----|-----|------|----------|
| F1 | P2 | intent new.rs / list.rs | Docs promise `intent list` self-heals a log-ahead store; it never reconciles — only `intent new` does. |
| F2 | P2 | checks/mod.rs + acceptance_wave_m_readpath.rs | The "chokepoint is structurally unbypassable" claim is enforced only over a HARD-CODED 5-file list; a new read verb in a new file escapes the invariant. |
| F3 | P2 | intent new.rs (land path) | No same-file guard for `--store == --log`: the process self-deadlocks into a spurious `log_busy` instead of a clear diagnostic. |
| F4 | P3 | checks/run.rs `record_on_log` | Function-level doc + caller comment say "dedups on memo_key"; the code dedups on the `(memo_key, cache_hit)` PAIR (the inline block is correct, the headline docs are not). |
| F5 | P3 | secret_shape.rs `is_cas_payload_shaped` | Dead/over-broad base32 branch: a 40- or 64-char all-lowercase-hex-and-`2-7` payload is already accepted by the hex branch; the base32 branch's `40|64`-length overlap is unreachable for those, and the `[a-f]`∩`[a-z2-7]` overlap makes the two branches non-disjoint (cosmetic, not a leak). |
| F6 | P3 | intent new.rs `now_ms` | `u64::MAX` fallback on a >584-million-year duration is dead defensive code that would stamp a nonsense `recorded_at`; harmless but misleading. |

No P0/P1 correctness defects found. The integrity spine (chain verify on every
canonical-log read), the scrub-on-append seam, the hermetic check executor
(pgid kill, bounded drain threads, depth+visited cycle guard), and the two-phase
log-first ordering are correct as written. Findings are honesty/maintainability
gaps and one ergonomic edge, not data-loss or leak bugs.

---

## Findings

### F1 — `intent list` does not self-heal a log-ahead store (docs overclaim) · P2
`crates/hugit-cli/src/intent/new.rs:142-144`, `:306-309`, `:332-350`;
contradicted by `crates/hugit-cli/src/intent/list.rs:76-125`.

**What's wrong.** The M-3 two-phase recovery contract is documented (in three
places) as: a transient log-ahead state "is self-healing… the next `intent
new`/**`intent list`** over this pair reconciles the store from the log
automatically." `intent new::run` does call `reconcile_store_from_log` (new.rs:235-237).
But `intent list::run` (list.rs:76) loads the store with `IntentStore::load_existing`
and projects rows from `store.intent_for_all()` — it NEVER calls
`reconcile_store_from_log` (grep confirms no `reconcile` symbol in `list.rs`).
`landed` state is read from the `--log`, but the intent ROWS come from the
store's own embedded log, so an intent that is on the canonical `--log` yet
missing from the `--store` (the exact log-ahead divergence M-3 describes) does
NOT appear in `intent list` output, and `intent list` does not persist any heal.

**Why it matters.** A reader trusting the doc will believe `intent list`
converges the divergence; it does not. Until the next *mutating* `intent new`
over the same pair runs, `intent list` under-reports. The data is not lost (the
log is the source of truth and `intent new` will heal it), but the stated
invariant is false and an operator could be misled during incident triage.

**Fix.** Either (a) make `intent list` actually reconcile when `--log` is given
(lock the store, run `reconcile_store_from_log`, then project) — note this turns
a read verb into a writer, so weigh against the read-only contract; or (b) the
cheaper honest fix: strike `intent list` from the self-healing claim in all
three doc sites and say only `intent new` reconciles.

### F2 — chokepoint invariant is enforced over a hard-coded file list, not the tree · P2
`crates/hugit-cli/src/checks/mod.rs:402-408` (the claim);
`crates/hugit-cli/tests/acceptance_wave_m_readpath.rs:251-301` (the guard).

**What's wrong.** `rehydrate_and_verify`'s doc asserts that routing every read
verb through it makes "a NEW read verb cannot project an unverified log… the
recurring 'a new read verb forgot `verify_chain`' root becomes structurally
unreachable." The source invariant that is supposed to enforce this
(`canonical_log_loaders_route_through_the_chokepoint`) scans a HARD-CODED array
of five files (`converged` at lines 259-271). A brand-new read verb authored in
a *new* file that hand-rolls `EventLog::new()+push_record+verify_chain` is not in
that array, so the test passes and the regression the chokepoint exists to
prevent ships. The invariant catches a regression in the five known loaders, not
the open-ended "new verb" class the prose claims.

**Why it matters.** This is precisely the failure mode that recurred in R7/R8
(why/export/intent-list each "forgot verify_chain"). The structural guarantee is
weaker than advertised: it is convention-by-enumeration, not tree-enforced.

**Fix.** Walk `src/**.rs` (minus the chokepoint file `checks/mod.rs` and the
documented special-shape siblings, which carry the `readpath-verify-exempt`
marker) instead of a fixed list; flag any inline `verify_chain(`/`.push_record(`
that lacks the exemption marker. That makes "a new file with an inline verify"
fail the gate. Alternatively, soften the prose in `checks/mod.rs` to state the
guard's true (enumerated) scope.

### F3 — no `--store == --log` same-file guard → self-inflicted `log_busy` · P2
`crates/hugit-cli/src/intent/new.rs:224-237, :294-296`; lock at
`crates/hugit-cli/src/intent/canonical_log.rs:90`;
`crates/hugit-cli/src/pr/filelock.rs:120-157`.

**What's wrong.** `intent new::run` acquires the store lock (`<store>.lock`,
held across the whole run, new.rs:224) and then, inside `reconcile_store_from_log`
and `canonical_log::land_intent`, acquires the log lock (`<log>.lock`). The
FileLock is keyed on `<target>.lock` (filelock.rs:121, `lock_path_for`). If the
user passes the SAME path for `--store` and `--log`, the second `acquire`
inside the same process sees its OWN live lock file (`AlreadyExists`, not stale)
and returns `LockError::Busy` → the run fails with `log_busy` ("locked by another
hugit verb") even though no other process is involved. The "the `--log`
reconciliation locks its OWN, DISTINCT file (`--log` ≠ `--store`), so there is no
deadlock" comment (new.rs:222-223) silently ASSUMES the two paths differ and
nothing enforces it.

**Why it matters.** Lower severity because the two files have different on-disk
shapes (`IntentStore` is a JSON object; the canonical log is an `[EventRecord]`
array), so a shared path would also fail to parse one way or the other — a user
is unlikely to do this deliberately. But the surfaced error (`log_busy`,
"another hugit process holds the lock; retry") is actively misleading: it tells
the operator to retry a phantom contention that will never clear.

**Fix.** Early in `run`, if `--log` is `Some(l)` and `l` canonicalizes to the
same path as `--store`, return a structured `invalid_argument` ("--log and
--store must be distinct files") instead of letting it manifest as a spurious
`log_busy`.

### F4 — `record_on_log` dedup docs say "memo_key", code keys on the pair · P3
`crates/hugit-cli/src/checks/run.rs:1200-1214` (fn doc), `:1158-1159` (caller
comment) vs `:1238-1243` + `check_already_recorded` `:1293-1302`.

**What's wrong.** The function-level doc ("dedups on `memo_key`") and the caller
comment ("if this memo_key already has a `check.recorded` row… appends NOTHING")
describe a memo_key-only dedup. The actual predicate `check_already_recorded`
keys on the `(memo_key, cache_hit)` PAIR — deliberately, so the wedge keeps one
cold-MISS row and one warm-HIT row for the same action (correctly explained in
the inline block at :1225-1237 and in `check_already_recorded`'s own doc). The
two headline doc sites contradict the correct behaviour.

**Why it matters.** Pure doc drift — the code is right and tested. But a
maintainer reading the function header would conclude a second HIT for an
already-MISS'd key is deduped, which is the opposite of what happens (the first
HIT is kept; only the THIRD+ identical run dedups). Misleads future edits.

**Fix.** Align the fn doc + caller comment with the `(memo_key, cache_hit)`-pair
reality already documented two lines below.

### F5 — `is_cas_payload_shaped` base32 branch overlaps the hex branch · P3
`crates/hugit-ledger/src/secret_shape.rs:242-250`.

**What's wrong.** The function accepts a `cas:` payload if it is a 40/64-char hex
run (first branch) OR a 32..=64-char `[a-z2-7]` base32 run (second branch). The
two predicates are not disjoint: a 40- or 64-char all-lowercase string over
`[a-f]` digits satisfies BOTH the hex branch and the base32 charset (since
`[a-f]` ⊂ `[a-z]` and the length is in `32..=64`). The hex branch already returns
`true` first, so the base32 `40|64` overlap is unreachable for hex-shaped input —
harmless but a code smell, and the comment ("a base32 content-id… of a content-
address-plausible length") implies the branches partition the space, which they
do not.

**Why it matters.** No correctness/leak impact (acceptance only widens). It is a
clarity trap: a future tightening of one branch could leave a believed-covered
case silently handled by the other, or vice versa.

**Fix.** Make the branches disjoint (e.g. base32 only for lengths the hex branch
does not own, or gate base32 on at least one non-hex char), or add a comment
stating the deliberate superset relationship so the overlap is not read as a bug.

### F6 — `now_ms` `u64::MAX` fallback is dead/misleading defensive code · P3
`crates/hugit-cli/src/intent/new.rs:503-508`.

**What's wrong.** `now_ms` maps a `SystemTime` duration to ms with
`u64::try_from(d.as_millis()).unwrap_or(u64::MAX)`. `as_millis()` only exceeds
`u64::MAX` for durations beyond ~584 million years past the epoch — impossible
for a real clock. The `unwrap_or(u64::MAX)` arm therefore can only fire on an
absurd/garbage clock, in which case it stamps a nonsense maximal `recorded_at`
onto the event (an observability annotation, excluded from the hash pre-image, so
not a chain hazard) rather than `0` or an error.

**Why it matters.** Trivial. It reads as a considered overflow guard but would
silently emit `recorded_at: 18446744073709551615`. Not worth a code change on
its own; noted for honesty.

**Fix.** If touched, fall back to `0` (consistent with the `duration_since`-fails
arm two lines down) or drop the unreachable arm.

---

## No-issue areas reviewed (honest negative results)

- **`rehydrate_and_verify` chokepoint (checks/mod.rs:411-459)** — correct:
  `EventLog::new()` → `push_record` (maps to `Rehydrate` fault) → `verify_chain`
  (maps to `ChainBroken`), fail-closed; `load_event_log` maps `NotFound`
  explicitly (never a silent empty world), `parse`/`io` distinctly. The four
  canonical loaders (list/canonical_log/pr/campaign/new-reconcile) all route
  through it. The CLAIM about its unbypassability is over-stated (F2), the CODE
  is right.

- **Scrub-on-append seam (porcelain.rs:279-371)** — recursion over the JSON
  tree is correct: keys never scrubbed, per-`(key,value)` mode re-evaluation,
  arrays inherit the parent mode, digest fields value-gated (`is_digest_key` ∧
  `is_digest_shaped`), identifier fields routed to `structural_secret_scrub`,
  everything else free-text. The deny-by-default `is_safe_identifier_shape`
  inversion is sound and the M-2 single-sourcing genuinely removed the
  hand-mirrored porcelain copies (verified: `KNOWN_SECRET_PREFIXES`,
  `contains_sk_key`, etc. are gone; all primitives `pub use`/imported from
  `secret_shape`). No drift between the free-text engine, the door, and the
  payload boundary.

- **`secret_shape` primitives** — `has_sk_key` (token-char run incl. `-`/`_`,
  `proj-` marker, progress-guaranteed loop), `has_connection_string_password`
  (authority slice, empty-password exclusion), `has_keyword_context_secret`
  (word-boundary guard, whitespace-tolerant separator, value read from the
  ORIGINAL string at a byte offset that is identical to the lowercased one since
  `to_ascii_lowercase` preserves length — checked, no off-by-one), `shannon_entropy`,
  `is_ulid_shaped`, `is_safe_identifier_shape` (PS-14 hybrid {40,64}-hex pin):
  all correct, loops all make progress, no panics on empty/odd input.

- **Hermetic check executor (checks/run.rs)** — `process_group(0)` + negative-pid
  `kill -TERM/-KILL` + `wait()` reaps the whole subtree (no orphan grandchild,
  no zombie); the two drain threads cap at 4 MiB then keep consuming to EOF (no
  pipe-full deadlock, no OOM); `collect_files` is cycle-safe via a canonical
  visited-set AND a depth cap; stdin nulled; env cleared+reconstructed from the
  captured allowlist that is the SAME set folded into the memo key
  ("captured == present"); PATH value hashed into the axis. AC per-op locking
  (not across execute) trades a benign double-exec for no lock-poison, and the
  log-append dedup keeps that from inflating KPIs. `guard_axes_not_secret`
  fail-closed write boundary reuses the one shared detector. Solid.

- **`intent new` two-phase ordering** — log-append-BEFORE-store-save is correct;
  a log failure returns without an orphan store entry; each individual write is
  atomic (temp+rename under lock); `reconcile_store_from_log` only ADDS
  log-present intents to the store (direction-safe, no store-ahead phantom) and
  chain-verifies the log first. The K-ERRLAW2 regression test exercises the
  failure-then-retry-converges path. (The recovery is real; only the `intent
  list` half of the self-healing CLAIM is wrong — F1.)

- **`aggregate_kpis` (checks/mod.rs)** — honest-null law correct: unknown
  `cache_hit` contributes to neither count; `hit_rate_pct` null unless a known
  hit/miss exists; basis-point integer arithmetic avoids float drift; `saved_ms`
  sums only HIT-row durations. No division-by-zero (denom guarded).

- **`charter_excerpt` + list scrub** — scrub-then-excerpt is leak-safe because
  `redaction::scrub` redacts the WHOLE string to the sentinel on any hit, so an
  80-char excerpt of the sentinel cannot expose a secret prefix; char-boundary
  walk-back is correct.

- **`FileLock` / `atomic_write`** — `create_new` lock with pid/mtime stamp,
  age-based stale takeover with a create_new re-race resolver, Drop best-effort
  release, same-directory temp+fsync+rename. Correct (the only gap is the
  cross-seam same-path case, F3).
