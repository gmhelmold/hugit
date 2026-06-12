# CLASS 4 — AUTHZ / MUTATION GUARD — SOTA audit

Round 8, fresh-fleet adversarial pass. Auditor: Class-4 root-cause agent.
Target HEAD: `integ/web-spine` @ `b1fa0f5` (engine state post Wave K). Built
`hugit-cli --locked` with the 1.96.0 toolchain; live forge-repros run in
`/tmp` temp dirs outside the repo.

## 1. Scope & method

The D14 invariant: **every write/append/mutation to the event-log MUST pass the
authorization guard** (`EventLog::append_authorized`, which evaluates the frozen
permission matrix `authorize_class(class, endpoint)` on the only mutating
primitive). The recurring class root (Round 7, K-AUTHZ): the guard is applied
*per-verb*, so a verb that appends via the raw `.append()` path bypasses it
(`hugit export` raw-appended a forged `pr.opened` with an arbitrary
`kind`/`principal_chain`/`payload`).

Method:
1. Enumerated EVERY call site of `EventLog::append(` and `append_authorized(`
   across the whole workspace (`grep`), plus every store write
   (`persist_log`, `IntentStore::save*`, `record_ref_*`, `record_external_change`,
   serializer `submit`).
2. Built THE matrix (§2): every mutation path × {through the D14 guard? ✓/✗} ×
   {what it can forge}.
3. For the live surface (the CLI verbs), REPRODUCED: built the CLI and drove each
   mutating verb, attempting to forge a class/kind/principal it should not be
   able to assert.
4. Asked the root question: is `append` reachable as a *raw* primitive from any
   user-facing mutation path, and can a single choke-point force every mutation
   through the guard.

Key structural fact established up front: `EventLog` exposes **two** append entry
points — `pub fn append` (the TRUSTED raw primitive, "bypasses the D14
authorization guard", per its own doc) and `pub fn append_authorized` (the
guarded one). Both are `pub` across the workspace
(`crates/hugit-refstore/src/log/mod.rs:374` and `:420`). The discipline that the
guarded one is used for verbs is enforced **by review, not by the type system** —
that is the residual structural root (§4).

## 2. Complete inventory  (THE matrix)

Mutation path → guard status → forgeable surface. `kind`/`principal`/`payload`
columns = what the caller controls *at that call site*.

| # | Path (file:line) | Guard | Reachable from a CLI verb? | Forgeable | Verdict |
|---|---|---|---|---|---|
| 1 | `hugit pr open` → `pr/mod.rs:497 append_authorized` | ✓ Push/Land via `author_authz(author_kind)` | YES (`pr open`) | class is clap-restricted to orchestrator\|human; chain derived, not free | GUARDED |
| 2 | `hugit pr land --settle` → `pr/mod.rs:906 append_authorized` | ✓ class = the **opened PR's** author_kind | YES (`pr land --settle`) | cannot pick a class — recovered from `pr.opened` | GUARDED |
| 3 | `hugit pr abandon` → `pr/mod.rs:997 append_authorized` | ✓ opened PR's author class | YES (`pr abandon`) | same as land | GUARDED |
| 4 | `hugit verdict --store` → `verdict/mod.rs:338 append_authorized` | ✓ hardcoded `Orchestrator`/`Land`, system principal `orchestrator:verdict-recorder` | YES (`verdict --store`) | NOTHING — class+principal are system-fixed | GUARDED (K-VERDICT) |
| 5 | `hugit check` recorder → `checks/run.rs:1146 append_authorized` | ✓ `Orchestrator`/`Push` | YES (`check`) | Push is universal-Allow; principal scrubbed | GUARDED |
| 6 | `hugit campaign open/close/abandon` → `campaign/world.rs:719 append_authorized` (via `append_authorized_and_persist`) | ✓ `Human`/`Policy` (owner → `user:` chain) | YES (`campaign *`) | Policy requires Human — a non-human owner is denied + audited | GUARDED |
| 7 | `hugit intent new --log` → `intent/canonical_log.rs:185 append_authorized` (Push) | ✓ via `import_sidecar_authorized` → `append_authorized` | YES (`intent new`) | Push universal-Allow; chain = campaign+agent (asserted) | GUARDED |
| 8 | `hugit export` write path → `export/{cut,mod}.rs push_record` ONLY | ✓ no append; **read-only**, `verify_chain` on input | YES (`export`) | NONE — export no longer accepts a `--store`/raw event; forged input → `chain_broken` exit-2 | GUARDED (K-AUTHZ, R7 fix) |
| 9 | `IntentStore::save` / `save_locked` (`intent/store.rs:288,301`) | n/a — projection sidecar | YES (`intent new --store`) | the store is NOT an independent authority: the authoritative mutation is the `--log` append (path #7), and the store save is **skipped** if the log append fails (K-ERRLAW2). No forgeable event-log surface. | NOT AN AUTHORITY |
| 10 | `proto::record_ref_update` / `record_ref_delete` (`proto/write/store/mod.rs:132,143`) | ✗ raw `append` | **NO** (no `hugit-proto` dep in `hugit-cli`; no `push` verb) | kind fixed to RAW_PUSH_KINDS (debug_assert ≠ intent kind); principal free | LIBRARY PRIMITIVE, not CLI-reachable |
| 11 | `proto::record_external_change` (`proto/write/external/mod.rs:157`) | ✗ raw `append`, but fail-closed on empty attribution | **NO** (not CLI-reachable) | kind ∈ RAW_PUSH_KINDS only; empty chain refused | LIBRARY PRIMITIVE, attribution-fenced |
| 12 | `mirror engine append_external_update` (`mirror/sync/engine.rs:632`) | ✗ raw via `record_external_change` | NO (no mirror dep in CLI) | RAW_PUSH_KINDS only; structurally cannot emit `intent.landed` | LIBRARY, by-design external change |
| 13 | `mirror engine` land → `engine.rs:465 .append(INTENT_LANDED_KIND…)` | ✓ guarded immediately upstream by `AuditedGuard.authorize(…, Land)` at `:457` (per-call guard, not the choke) | NO (not CLI-reachable) | gated by Land decision before the raw append | GUARDED (per-call) |
| 14 | `refstore concurrency Serializer::submit` (`concurrency/mod.rs:380`) | ✗ **raw `append(op.kind, op.principal_chain, op.payload)` — NO guard at all** | **NO** (only test callers; no production caller in the workspace) | EVERYTHING — `Op` carries arbitrary kind/principal/payload | UNGUARDED but DEAD (no prod caller) — see F-2 |
| 15 | `refstore Serializer::submit_undo` (`concurrency/mod.rs:450`) | ✓ `AuditedGuard.authorize(…, Undo)` before the raw append | NO (no prod caller) | Undo = Human-only, enforced | GUARDED (per-call) |
| 16 | `refstore undo::undo` (`undo/mod.rs:212`) | ✓ `AuditedGuard.authorize(…, Undo)` before raw append | NO (no CLI `undo` verb yet) | Human-only | GUARDED (per-call) |
| 17 | `refstore authz::AuditedGuard.authorize` (`authz/mod.rs:303`) | n/a — this IS the guard, raw-appends the `authz.denied` AUDIT record only | — | system kind `authz.denied` only | TRUSTED (audit emit) |
| 18 | `hugit-ledger journal::Journal::append` (`journal/persist.rs:93`) | separate session-resume journal, NOT the D14 event-log | NO prod caller found | session notes only | OUT OF SCOPE (different store) |
| 19 | `hugit-dogfood wave/soak .append` (`wave.rs:171`, `soak.rs:193…`) | ✗ raw | NO (`hugit-dogfood` not referenced in CLI src) | arbitrary | INTERNAL HARNESS, not a verb |

**Matrix verdict:** every LIVE, CLI-reachable mutation path (rows 1–9) is
GUARDED or read-only. Every raw-`append` site (rows 10–19) is either a
library/internal primitive **not reachable from any CLI verb**, guarded per-call,
or DEAD (no production caller). The Round-7 export bypass is closed and
re-verified by live repro. No NEW exploitable bypass found.

## 3. Findings

### F-1 — `EventLog::append` is workspace-`pub`: the choke-point is by-convention, not by-type — TYPE: code (structural) · SEVERITY: MEDIUM (latent, not currently exploitable)

`pub fn append` (`log/mod.rs:374`) is reachable from every crate. Its own doc
says it "performs **no authorization** … Calling this directly for a guarded verb
is the bypass S3 flagged — don't." The word *don't* is the whole enforcement.
Round 7's export bug was exactly a verb that called this raw path; K-AUTHZ fixed
*that verb*, but the primitive that made the bug possible is unchanged and still
public. The class is not killed — it is one careless `log.append(...)` in a
future verb away from re-opening. **No live repro of an exploit exists today**
(every current verb is clean — grep of `crates/hugit-cli/src` for raw `.append(`
returns only one hit, in a `#[test]`), so this is a *latent structural* finding,
not a shipped defect. ROOT: two public append doors, only one guarded.

### F-2 — `Serializer::submit` raw-appends with zero authorization — TYPE: code · SEVERITY: LOW (dead code: no production caller)

`concurrency/mod.rs:380` does `log.append(op.kind, op.principal_chain,
op.payload, …)` with NO guard, while its sibling `submit_undo` DOES guard. `Op`
lets a caller forge any kind/principal/payload. The ONLY callers in the entire
workspace are test fixtures (`two_update_serializer`). It is dead on the
production path today — but it is a fully-loaded raw-append gun pointed at the log
with a public `submit`, and it is the natural home for a future `push` verb. If
`submit` is ever wired to a CLI verb without first routing through the guard, it
reinstates the Round-7 class. ROOT: same as F-1 — raw `append` is callable; here
a *public mutation API* (`submit`) wraps it without the guard.

### F-3 — the matrix checks a caller-asserted `class` *independent of* the `principal_chain` — TYPE: seam (honestly disclosed) · SEVERITY: accepted

`append_authorized(class, endpoint, …, principal_chain, …)` takes the `class` as
a **separate** caller-supplied enum; the matrix decides on `class`, while the
`principal_chain` bytes are hashed verbatim and are NOT cross-checked against the
class. So a caller can in principle pass `class=Orchestrator` with
`principal_chain=["agent:x"]`. This is the disclosed P2 identity seam (the class
assertion is bound to an authenticated principal only after identity rollout) and
the doc states it in the correct tense. The CLI verbs mitigate it in depth: `pr`
derives both from the same clap-validated `--author-kind`; `verdict`/`check`/
`campaign` hardcode both. **It is honestly fenced** — no current verb asserts a
class it could not also locally justify. Kept as residual (§6), NOT a bug.

**Live repro log (cold, `/tmp`):**
- `pr open --author-kind orchestrator` → ALLOW, `pr.opened` lands (exit 0).
- `pr open --author-kind subagent|worker` → REJECTED at the door
  (`subagent_author`, exit 2) — clap restricts the flag; no forged class reaches
  the matrix.
- `verdict --store --lens security --result approve` → `verdict.recorded` lands
  with the system principal (exit 0); class is not caller-selectable.
- `export --log <hand-forged event with this_hash:"deadbeef">` → `chain_broken`
  (exit 2): export verifies the chain and **cannot inject** — the R7 bypass is
  closed.

## 4. Root-cause analysis

The class root is unchanged across Rounds 7→8: **`EventLog` has two public append
doors and the guarded discipline lives in prose, not in types.** Every per-verb
remediation (K-AUTHZ for export, K-VERDICT, the `pr` door checks) is a *patch on
one door-user*; none removes the unguarded door. The matrix verdict says the
class is currently *clean* — but "clean" here means "every author happens to have
called the right function," which is precisely the per-verb property the class is
defined to distrust. F-1 and F-2 are the same root viewed from two call sites:
the raw primitive (`append`) and a public wrapper of it (`submit`) are reachable
without the guard. As long as `append` is `pub`, a green grep is a *snapshot*, not
an *invariant*.

The honest scope boundary: the raw-append sites that exist today (proto raw-push,
mirror external-change, the undo guards, the audit emit) are legitimately trusted
or per-call-guarded, and none is CLI-reachable. So the class is not *exploitable*
on `main` today. The remediation is therefore about **converting the snapshot
into a compiler-enforced invariant**, so that no future verb can re-open it.

## 5. Recommended structural remediation  (the single choke-point)

Make the raw door **unreachable from verb crates** so `append_authorized` is the
only way in, enforced by `rustc`:

1. **Demote the raw primitive to `pub(crate)`.** Change
   `crates/hugit-refstore/src/log/mod.rs:374` from `pub fn append` to
   `pub(crate) fn append`. Now only `hugit-refstore` itself can call it raw;
   `append_authorized`, `push_record`, `undo::undo`, `Serializer::submit*`, and
   the `AuditedGuard` audit-emit all live inside the crate and keep working
   untouched.
2. **Give the two legitimate CROSS-crate raw users a named, trusted shim** instead
   of the bare primitive — so the trust is explicit and greppable:
   - For `hugit-proto`'s raw-push recorders (`record_ref_update`,
     `record_ref_delete`, `record_external_change`) and `hugit-mirror`'s
     external-change path, add `EventLog::append_external_change(kind:
     ExternalChangeKind, …)` that accepts ONLY a `RAW_PUSH_KINDS`-typed `kind`
     (a closed enum, not `impl Into<String>`), so a raw-push recorder is
     *structurally incapable* of emitting `intent.landed`/`pr.*`/`verdict.*`.
     This replaces today's `debug_assert_ne!` (a runtime, debug-only check) with a
     type-level one and is the only cross-crate raw need.
3. **Close F-2 at the API:** route `Serializer::submit` through the guard exactly
   like `submit_undo` already does — take a `class`/`Endpoint` (or an
   already-authorized token) and call `append_authorized`, not raw `append`.
   Since `submit` has no production caller, this is a pure hardening with no
   behavioural blast radius; it pre-fences the future `push` verb.
4. **Lock it with a test that greps the invariant:** a workspace test (or a
   `compile_fail` doctest) asserting that no crate outside `hugit-refstore` and
   outside the named external-change shim references `EventLog::append` —
   converting "don't" into a gate.

Exact fns to change: `EventLog::append` → `pub(crate)`; add
`EventLog::append_external_change`; `Serializer::submit` → route through
`append_authorized`. After step 1 the workspace will fail to compile at the
proto/mirror call sites until they move to the shim — that compile error IS the
proof the door is now single.

Effort: small (one visibility change + one typed shim + one `submit` rewire + one
guard test). Blast radius is contained to refstore/proto/mirror; the CLI is
already clean.

## 6. Residual / accepted

- **F-3, the caller-asserted principal/class seam (P2 identity).** `class` and
  `principal_chain` are caller-supplied and not yet bound to an authenticated
  principal; the matrix decision itself is real and enforced on the only mutating
  primitive. This is the disclosed identity seam (`authz` module doc, ADR-0002 —
  "a PAT never reaches a browser"; the class→principal binding lands with identity
  rollout). It is honestly fenced everywhere today: every CLI verb derives or
  hardcodes both the class and the chain (no verb forges a class it could not
  locally justify), and the docs state the tense correctly. **Accepted as a seam,
  not a bug.** The §5 fix is orthogonal and complementary — it removes the *raw*
  door regardless of how the class is ultimately authenticated.
- **proto raw-push / mirror external-change / undo guards / audit emit** are
  legitimately trusted or per-call-guarded and **not CLI-reachable**; §5 step 2/3
  upgrades their trust from runtime `debug_assert` to type-level, but they are not
  defects today.
