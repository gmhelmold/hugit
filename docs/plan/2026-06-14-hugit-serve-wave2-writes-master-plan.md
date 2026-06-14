# hugit-serve Wave 2 — the WRITE verbs (master plan, techlead pre-work)

**Author:** hugit TL · **Date:** 2026-06-14 · **Status:** PLAN (no dispatch yet —
Tier-3 verbs gate on owner design checkpoints, §6). Grounds: the close-the-product
handoff §3 (`../githugr/docs/handoff/2026-06-13-hugit-phase2-close-the-product.md`)
+ the frozen `Accepted` body (#112). The reads are live (11/26 real on R2); writes are
"what makes the user ACT" and the highest-impact remaining gap.

## 0. The 9 write verbs (spec §3, verbatim surface)

| # | Verb | Body | Success | Engine backing today | Tier |
|---|------|------|---------|----------------------|------|
| 1 | `POST …/prs/{n}/land` | `{mode: union\|serial\|window}` | `{seq, queue_pos}` | **`hugit pr land`** (pr.queued/landed) | **T1** |
| 2 | `POST …/prs/{n}/verdict` | `{verdict, note?}` | `{seq}` | **`hugit verdict`** (verdict.recorded) | **T1** |
| 3 | `POST …/prs/{n}/comments` | `{body, anchor?}` | `{seq}` | new record kind (`pr.comment`) | **T1** |
| 4 | `POST …/dispatch` | `{ask, campaign?, draft}` | `{seq, pr_number, charter_preview}` | intent+pr open, but TRIGGERS fleet work | **T3** |
| 5 | `POST …/issues/{n}/transition` | `{to, priority?}` | `{seq}` | new issue state machine | **T2** |
| 6 | `POST …/policy` | `{rule_id, enabled?, param?}` `[STEP_UP]` | `{seq}` | touches `hugit-policy` | **T3** |
| 7 | `POST …/erasure/{id}/decide` | `{approve}` `[STEP_UP]` | `{state}` | touches X12 erasure/tombstone spine | **T3** |
| 8 | `POST …/edit/{path}/propose` | `{content, title, description?}` | `{pr_number, branch}` | pr open + a ref | **T2** |
| 9 | `POST …/undo` | `{op_seq}` | `{seq}` | event-log undo (R-refstore) | **T3** |

## 1. The forcing insight (conflict elimination, AP-1)

**Every POST shares ONE component: the idempotency ledger + the write-door.** If 9
agents each build their verb with its own idempotency handling, they (a) collide on the
shared ledger/middleware and (b) each re-implement the 24h-byte-identical-replay law →
drift → the exact "band-aid per instance" failure. **So the shared spine is built ONCE,
sequentially, and FROZEN as a contract before any verb fans out.** This is the whole
plan's spine (techlead Section 1.2: eliminate the shared file before dispatch).

## 2. Phase 0 — the write foundation (SEQUENTIAL, lead-built or single agent; NO parallel)

The shared, security-critical, build-once layer every verb rides:

1. **Idempotency ledger** — `(principal, verb, key) → stored outcome`. `Idempotency-Key`
   header MANDATORY; absent → `400 IDEMPOTENCY_REQUIRED`. Replay within 24h returns the
   **byte-identical** prior outcome (INCLUDING a prior rejection — a denied call replays
   denied, never re-attempted). Same key + different body → `409 IDEM_MISMATCH`, **never
   re-executes**. Persisted in the event log as its own record so it survives restart and
   is itself chain-verified.
   - **The land invariant (highest stakes):** a lost response + client retry with the
     same key MUST NOT yield two queue positions. One land = one position. This is the
     marquee idempotency test.
2. **The write-door middleware** (one chokepoint every POST passes through, like the
   PS-13 read loader is for reads):
   - **D14 author-kind guard** — assert `orchestrator|human`, reject `subagent` at the
     door (reuse the `pr open` guard; a write author is never a subagent).
   - **Redaction at the write boundary** — every free-text field (`comment.body`,
     `verdict.note`, `dispatch.ask`/charter, `edit.content`, `policy.param`) is scrubbed
     (`hugit_ledger::redact::apply`) BEFORE persist — a PAT pasted into a comment never
     lands in the log. (Symmetry with the read-boundary scrub.)
   - **Verified append** — all records go through the `pub(crate)` append door + chain
     (Wave L), never a raw push.
   - **STEP_UP gate** — `policy` + `erasure` require fresh-auth; absent → `403
     STEP_UP_REQUIRED` (the real fresh-auth is the P2 Clerk seam; Wave-2 ships the gate +
     a dev-stub assertion, disclosed).
   - **Error envelope** — the spec §3 map: `400 IDEMPOTENCY_REQUIRED`, `409 IDEM_MISMATCH`,
     `403 STEP_UP_REQUIRED`, `403 POLICY_DENIED`, plus the existing `401/404/503`.
3. **Contract freeze (Phase A-style):** byte-transcribe the 9 request payloads + their
   specific success bodies from `../githugr/crates/githugr-vm/src/actions.rs` into
   `hugit-http-contracts` (additive; `Accepted` already frozen). One round-trip test each.

**Phase 0 DoD:** the ledger + door compile and are unit-proven (idempotency replay,
409 mismatch, 400-absent, D14 reject, redaction-on-write, STEP_UP) with NO verb wired
yet. This is the frozen interface the verbs depend on.

## 3. Phase 1 — Tier-1 verbs (PARALLEL after Phase 0; highest impact)

`land` · `verdict` · `comments` — the user starts to ACT (handoff priority order).
Disjoint: each owns `handlers/write_<verb>.rs` + one route arm; all ride the frozen
Phase-0 door. `land`/`verdict` map to existing engine verbs (`pr land`/`verdict`);
`comments` adds the `pr.comment` record kind. The shared files (server.rs routes,
handlers/mod.rs) are wired by the LEAD from agent-returned code (text-return pattern —
no parallel tree mutation, the banked incident).

## 4. Phase 2 — Tier-2 verbs (PARALLEL)

`issues/{n}/transition` (new issue state machine + record) · `edit/{path}/propose`
(content → branch + `pr.opened`). New record kinds, but mechanical — no spine decision.

## 5. Phase 3 — Tier-3 verbs (each gated on an OWNER DESIGN CHECKPOINT, §6)

`dispatch` · `policy` · `erasure/decide` · `undo`. Spine-touching; NOT dispatched until
the owner rules the design (below).

## 6. ⚠️ Owner design checkpoints (decisions the lead will NOT make alone)

- **`dispatch`** — it TRIGGERS fleet work (an `ask` → a real spawned campaign/PR). Does
  the engine actually spawn (P2 runner seam) or only record the intent + a `draft`
  charter for human confirm? (Recommend: Wave-2 records intent + `charter_preview`,
  spawn is the P2 seam — never auto-spawn from a web POST without a gate.)
- **`policy`** `[STEP_UP]` — toggling a `hugit-policy` rule from the web changes gate
  behavior. STEP_UP fresh-auth model + which rules are web-mutable at all.
- **`erasure/decide`** `[STEP_UP]` — approving an erasure touches the X12 tombstone /
  provenance spine (mirror obligation, verifiability). The most sensitive verb; needs an
  explicit design (what "approve" does to the log + mirror, and the irreversibility).
- **`undo`** — reverting an `op_seq` must keep the event-log chain sound + verifiable
  (interacts with R-refstore undo/recovery). Soundness model before any code.

## 7. Cross-cutting quality bars (every verb)

- **Idempotency replay test** (byte-identical, incl. rejection) + the `land` one-position
  invariant.
- **Secret-MATRIX guard** — a `ghp_…` PAT in any free-text body persists as `[REDACTED]`.
- **Fail-honest** — a write that can't be durably recorded errors (5xx), never a silent
  partial / fake `accepted:true`.
- **Parity test** against the frozen request/response contract types.
- Gates green (fmt/clippy/test/deny/audit) before merge; runner-verified.

## 8. Dispatch shape (when Phase 1 is greenlit)

Text-return agents (return code, lead writes+compiles centrally — never N parallel tree
mutations). ≤6 concurrent (non-trivial prompts). Per-WP card: `{files, record-kind,
idempotency-test, redaction-fields, route-arm}`. Merge order: Phase 0 → 1 → 2 → 3, each
gate-green before the next.

---

**Next action:** none auto-fires. Phase 0 (the foundation) can start on owner "go" — it's
the unblocker for everything and touches no Tier-3 decision. Tier-3 waits on §6 rulings.
