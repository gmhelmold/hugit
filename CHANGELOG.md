# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

- feat(serve): **`git clone` works — git wire serving (upload-pack) goes live in the codebase.**
  The launch-blocking gap from the honest audit: `git clone` against the engine used to 404. Now
  `hugit-serve` speaks the git smart-HTTP wire over the clone/fetch logic `hugit-proto` already had.
  - **`GET /<repo>/info/refs?service=git-upload-pack`** → the v1 advertisement (`# service=…` pkt +
    flush, `HEAD` first with `object-format=sha1`/`symref=HEAD:<branch>` so the default branch checks
    out, then name-sorted refs); **`POST /<repo>/git-upload-pack`** → `0008NAK\n` + the real V2
    packfile from `serve_fetch`. pkt-line framing is hugit-proto's; the smart-HTTP envelope +
    Content-Types (`application/x-git-upload-pack-{advertisement,result}`) are the serve layer's. A
    binary-response bypass in `serve_on` (mirroring the SSE short-circuit) streams the packfile —
    the `(u16, String)` route path can't carry binary.
  - **`AppState`** gains `git_refs` (ref→tip oid, loaded via `git for-each-ref`); `load_git_dir`
    switched to `git rev-list --objects --all` so every branch's closure is in the CAS.
  - **Auth**: gated on the SAME `authz::authorize_read` predicate as `/v1` reads, with an
    **unauthenticated** principal (git sends no Bearer) — only a publicly-readable repo is cloneable;
    private/absent/not-live → uniform **404, no existence oracle**. No git dir wired → 404.
  - **Proven by a REAL `git clone`** (not a mock): the e2e spawns `serve_on`, shells
    `git clone http://127.0.0.1:<port>/<repo>`, and asserts success — all 7 objects, both branch
    tips, `HEAD`=main, working-tree content. 8 wire tests (advertisement shape, NAK+PACK framing,
    404 gating, push-out-of-scope). Two real interop bugs fixed in-crate (capability list on the
    first `want` line; HEAD+symref for checkout) — `hugit-proto` untouched.
  - **Scope**: clone/fetch only. `git push` (receive-pack) is **404 by design** (a later wave).
    Live-serving is deploy-gated on `HUGIT_SERVE_GIT_DIR` + a public repo (hermetic + CI-proven now).
    The server speaks smart-HTTP v1; v2 clients negotiate down (the test forces v1 for determinism).

- feat(cli): **`hugit diag` + `hugit policy edit` go REAL — the last two reserved verbs (no stubs).**
  Closes the design-gated pair from the honest audit; both are genuinely wired, not `not_implemented`.
  - **`hugit diag --log --def-digest <hex> [--toolchain <hex>]`**: drives the REAL `hugit-diag`
    bisect engine (`on_red_signal`) over a **log-backed `CheckOracle`** — projects an ordered
    `History` from the `check.recorded` events on the canonical log (grouped by the
    `(def_digest, toolchain_digest)` axes, chain-seq order) and answers each probe from the recorded
    `exit` codes via `hugit_refstore::compute_memo_key` (no live ActionCache, no re-execution). Emits
    the `DiagnosisObject` (culprit ref, diff-vs-green, suspect targets, bisect path) read-only — no
    log append (no `diag.recorded` kind; mirrors `checks show`). A green/empty tip → honest
    `{"diagnosis": null}` (exit 0); unknown def → `no_history`; >1 toolchain w/o `--toolchain` →
    `ambiguous_toolchain`. Adds the `hugit-diag` dep + graduates `diag` RESERVED→`HUGIT_VERBS`.
  - **`hugit policy edit --log --gate <id> (--enable|--disable) [--principal]`**: reconstructs the
    current gate set by folding `policy.change` events over the new `hugit_policy::house_gates()`
    baseline (latest-wins), toggles the named gate's `enabled`, and records the `{old, new}`
    `policy.change` through the D14 `(Human, Endpoint::Policy)` guard (Human-only — a non-human
    principal is denied fail-closed). Closed house set (`dco`/`changelog`/`secrets`); unknown gate →
    `unknown_gate`/exit-2; a no-op edit records nothing (`changed:false`). `policy edit` joins the
    `policy test` shipped in #145.
  - `house_gates()` extracted in `hugit-policy` (no behavior change — `Engine::house` builds from it).
    `diag` graduated in lockstep with `Command::Diag` (no-drift oracle ⑥ holds). clippy `-D warnings`
    clean. This empties the design-gated backlog: every CLI verb the audit flagged is now real.

- feat(serve): **`GET /v1/repos/{repo}/blob/{*path}` + `/edit/{*path}` serve REAL file content (roadmap W5 — reverses PS-18).**
  The owner-decided reversal of PS-18: blob/edit stop being honest-default fixtures and serve the
  actual file bytes from the git tree, via the new `hugit_proto::resolve_blob_at_path`.
  - **`build_blob`/`build_edit`** resolve `(repo-relative path)` → blob bytes through the path→oid
    tree-walk; `BlobVm.lines`/`EditVm.lines` are the REAL decoded content (UTF-8-lossy, CR-stripped,
    1-based), `size` (human bytes), `lang` (extension→name). `blame`/`outline`/`tree`/`via_*` stay
    honest-default (no attribution/symbol seam this wave — outline is W6). **Every line's text + the
    URL path are scrubbed** at the read boundary (`crate::fmt::scrub` → `hugit_ledger::redact::apply`)
    — file content may carry secrets, redacted on read.
  - **No content oracle**: an unresolvable path, or the content seam not wired, returns `None` → 404
    (never a fake blank file). `AppState` gains `git_source: Option<Arc<CasObjectSource>>` +
    `git_root_tree`, populated at boot from `HUGIT_SERVE_GIT_DIR` (a `git rev-list`/`cat-file` eager
    load that **fails closed** if the dir isn't a resolvable repo); absent → blob/edit 404 honestly.
    This is the live-infra seam — the handlers are hermetically tested against a seeded
    `CasObjectSource` (no live tenant); the boot loader is only exercised with a real git dir.
  - `dispatch_repo` gains `blob`/`edit` GET arms (distinct from the POST `edit/.../propose` write).
    Adds `hugit-proto` + `gix-hash` deps (cargo-deny `bans ok`, no new duplicate version). 10 new
    hermetic tests (real content, nested paths, missing-path→None, absent-seam→None, secret
    redaction); `cargo test -p hugit-serve` green; clippy `-D warnings` clean. `symbol` → W6.

- feat(proto): **`resolve_blob_at_path` — git tree-walk path→blob over the CAS (roadmap W5 foundation).**
  The missing bridge between a repo-relative path and its blob content. `CasObjectSource` /
  `ObjectSource` only exposed `get(oid)` / `contains(oid)`; nothing resolved `(tree, "src/foo.rs")`
  → blob oid. New `hugit_proto::resolve_blob_at_path(src, root_tree, path)` walks the tree
  subtree-by-subtree via the canonical `gix_object` decoder (git formats are NOT reimplemented),
  returning `Ok(Some((oid, bytes)))` or `Ok(None)` (absent / intermediate-not-a-tree /
  final-not-a-blob). **Security**: empty / `.` / `..` segments are rejected (`Ok(None)`) — the
  walk can never escape the root; symlinks are returned as blob content, never *followed*;
  gitlinks → `None`. Hermetic — `CasObjectSource::new()` is the test impl, no live tenant. 9 unit
  tests incl. nested paths, exe/symlink modes, and the full traversal-rejection set. This is the
  foundation the blob/edit serve reads (W5, reversing PS-18) build on; not yet wired to a handler.

- feat(cli): **`hugit approve` / `hugit reject` / `hugit journal note` go REAL (roadmap W3 — no stubs).**
  Three more stakeholder/session verbs graduated RESERVED→`HUGIT_VERBS` with genuine wiring.
  - **`hugit approve` / `hugit reject` --intent --log [--tree-hash] [--recorded-at]**: thin
    single-lens wrappers over the EXISTING `verdict::record` path — each records a
    `verdict.recorded` event with lens `human-approval` and result `approve`/`reject`,
    aggregating to that verdict. This mirrors the live serve verb `POST /prs/{n}/verdict`
    (which maps `approve`/`request-changes` into the same `verdict.recorded` kind through
    `(Orchestrator, Land)`) — ADR-0006 parity, **no new D14 endpoint, no matrix change**. They
    always store (an unrecorded approval is meaningless). Reuse keeps one producer + one wire
    kind; the idempotency/seal/existence guards of the verdict path apply unchanged.
  - **`hugit journal note --log --note [--principal] [--workspace] [--intent]`**: appends a
    `journal.note` record onto the CANONICAL event log (decided: unify with the log, not a
    separate journal file), mirroring `issue.transition` — `(Orchestrator, Land)`, free-text
    note + identifiers scrubbed at the boundary, canonical-JSON scrub before the hash chain,
    atomic `persist_log`. Carries the D11 `JournalEntry`/`JournalKey` semantic fields
    (note/principal/workspace_id/intent_id); the in-memory D11 `Journal` becomes a projection.
    Empty note → `empty_note`/exit-2; missing `--log` → `log_not_found`. Freezes the
    `journal.note` kind; a `write_journal_note` serve verb can mirror it later (ADR-0006).
  - `approve`/`reject`/`journal` graduated in lockstep with `Command::Approve`/`Reject`/`Journal`
    (the no-drift oracle ⑥ holds). clippy `-D warnings` clean.

- feat(cli): **`hugit undo` + `hugit policy test` go REAL (roadmap W3 — no stubs).**
  Two stakeholder verbs graduated RESERVED→`HUGIT_VERBS` with genuine backing wiring
  (the drafted not-implemented stubs were rejected — a hollow `not_implemented` surface is
  exactly the over-claim the honest audit denounced).
  - **`hugit undo --log --seq [--actor]`**: thin porcelain over the event-sourced
    `hugit_refstore::undo` — replays the prefix `[0, seq)`, computes the compensating event
    that restores the ref, and appends it (history preserved, never a rewrite/deletion).
    Human-only via the D14 `Endpoint::Undo` guard: a non-human/unrecognized principal is
    denied fail-closed (`authz_denied`/exit-2, audit record persisted). `--actor` defaults to
    `user:cli` and is structurally scrubbed before reaching the chain. Missing `--log` →
    `log_not_found`; out-of-range `--seq` → `out_of_range`; tampered chain → `chain_broken`.
  - **`hugit policy test --context <path>`**: runs `hugit_policy::Engine::house().eval` — the
    SAME evaluator the forge landing path uses, so the local verdict is byte-identical
    (`local ≡ forge`, WP-D6 ①). Reads a JSON `EvalContext` (CLI input seam, like `impact`'s
    `GraphInput`), emits per-gate `{id, outcome, reason?}` + `all_pass`; a failing gate is a
    real exit-0 result. Missing/malformed context → `context_not_found`/`parse_context`/exit-2.
    Gate reasons scrubbed at the read boundary. `policy` ships `test` only — `policy edit`
    stays deferred (no gate-set mutation/persistence model yet; an honest partial surface,
    not a stub). `undo`/`policy` graduated in lockstep with `Command::Undo`/`Command::Policy`
    (the no-drift oracle ⑥ holds count-equality). clippy `-D warnings` clean.

- feat(cli): **`hugit issue transition` — CLI parity for `issue.transition` (roadmap W2).**
  Restores ADR-0006 CLI parity (no web-only verb): the serve verb
  `write_issue_transition` now has its `hugit issue transition --log --n --to
  [--priority]` equivalent. Inline reimplementation (no `hugit-serve` dep — circular):
  same 4-value `VALID_STATES`, priority scrubbed via the redaction seam, appended through
  the D14 `(Orchestrator, Land)` guard, canonical-JSON scrub before the hash chain, atomic
  `persist_log`. A missing `--log` is `log_not_found`/exit-2 (never a ghost record).
  `issue` graduated RESERVED→`HUGIT_VERBS` + `Command::Issue` (the no-drift oracle holds).
  Drafted by a spec→draft→verify fleet, lead-integrated; verify fixes applied (scrubbed
  priority in the success JSON, `_lock` held without the no-op `map_err`, the public
  `CampaignError` re-export, the missing-FILE test setup). clippy `-D warnings` clean.

- feat(serve): **Wave A — 4 `/v1` read handlers go real + a knowledge honest-stub.** Closes
  the fixture/data-gated gap the honest audit flagged for the easy reads (the data-model
  reads — blob/edit/symbol — remain the owner-gated CAS seam, NOT in this wave).
  - **`GET /v1/repos/{repo}/new-pr` → NewPrVm**: campaigns (`campaign.opened`), commits
    (`project_machine`, COMMITS_CAP newest after reverse), checks-ok (`check.recorded`),
    base/head (`replay` RefState), policy_note (`policy.set`); presentation fields are
    honest-default. Free-text scrubbed at the read boundary.
  - **`GET /v1/me/login` → LoginVm**: PUBLIC (no Bearer — pre-match guard mirroring `/readyz`,
    the auth entry-point); static presentational card.
  - **`GET /v1/repos/{repo}/compare/{base}/{head}` → CompareVm**: branches/generated_branches
    REAL from `replay` RefState; diff/commits/can_merge HONEST-STUB (TL decision — every
    DiffVm in the codebase is a stub; no diffstat seam).
  - **`GET /v1/repos/{repo}/search?q=` corrected**: pr_lifecycle uses `all_pr_queued`,
    seq-indexed age, `seen` marked only on emit (post-cap).
  - **`GET /v1/repos/{repo}/knowledge` → KnowledgeVm honest-stub**: P2 knowledge-index seam
    disclosed via `engine_note`; nothing fabricated.
  Built by a draft→adversarial-verify fleet, lead-integrated centrally (no parallel
  tree-mutation), all P0/P1 fixes applied; clippy `-D warnings` clean, 226 lib tests green.

- feat(serve): **`POST /v1/token` via CoreLink session-exchange (Option B).** The endpoint
  now DELEGATES Clerk-JWT verification to CoreLink's `/v1/session/exchange` instead of
  validating the JWT locally (the Server-TL decision — "consume CoreLink, never fork"; no
  duplicated azp/issuer/JWKS pipeline to drift). The client-facing contract is UNCHANGED
  (`{subject_token, audience}` → `{engine_token, expires_in, accepted}`); only the internal
  validation path changed.
  - hugit forwards the user's Clerk session JWT (`Authorization: Bearer`, **no internal-auth
    key** — the route is Clerk-JWT-gated) and mints its opaque engine token from the verified
    `{principal, tenant, expires_ms}`. `token_plaintext` (the upstream `cas:rw` PAT) is
    IGNORED — never stored or logged.
  - Error mapping: exchange `401`|`403` → `401 TOKEN_INVALID` (the `403` is COLLAPSED — no
    tenant-existence oracle on the mint path, per the frozen client §Q2 + the read-path
    no-existence-leak doctrine); `429` → `429 RATE_LIMITED`; `405`/`5xx`/network/malformed-200
    → `503 ENGINE_UNAVAILABLE`.
  - The engine-token TTL is bounded by `min(ceiling, upstream remaining)` (`mint_with_ttl`),
    so it never outlives the upstream session; an already-expired upstream session is refused
    fail-closed. `fresh_auth = false` for exchange-minted principals (the contract carries no
    `auth_time`; step-up must come from a fresh session, never `/v1/token` alone).
  - New `HUGIT_SESSION_EXCHANGE_URL` (absent ⇒ the route 404s, dev-token-only). Removed the
    now-dead local `ClerkValidator`/`JwksCache`/`TokenConfig` + the `jsonwebtoken` + `base64`
    deps from hugit-serve (both remain in the lock via other crates). Mock-tested end-to-end
    against the documented exchange contract (27 token tests). Live wiring needs the deployed
    Worker pointed at the dev Clerk instance + the endpoint host (the disclosed P2 seams).

- feat(checks): **`result_binding_sig_v2` verifier — closes the verdict-forgery window**
  (contract §7.1 amendment v1.4.0; the corelink-runners P0 security item, Path 1 =
  transcribe-in-hugit). The v1 binding covered `memo_key‖stdout_ref‖stderr_ref` but NOT
  `CheckResult.exit` (the pass/fail verdict) or `artifacts` — a malicious runner/MITM could
  flip `exit: 1 → 0` (and rewrite artifacts) with the v1-covered fields intact and a v1-only
  verifier still accepted it. Now:
  - `hugit_refstore::result_binding_preimage_v2` builds the v2 pre-image
    (`LP(memo_key)‖LP(stdout_ref)‖LP(stderr_ref)‖i32_be(exit)‖u32_be(artifacts.len)‖
    Σ(LP(path)‖LP(digest))`), beside the other single-sourced LP pre-image builders (never
    re-transcribed).
  - `hugit_checks::attest_v2::verify_result_binding_v2` decodes the fabric key/sig and runs
    `verify_strict` over that pre-image; fail-closed on any malformed input.
  - Proven byte-exact against the shared `conformance/result_binding_v2.json`: pre-image hex
    matches, the genuine fabric signature verifies, a flipped `exit` is REJECTED (the forgery
    v2 closes), a tampered artifact digest is REJECTED, malformed inputs fail closed.
  Live wiring into the attestation-verify path stays the P2 AC seam; the verifier is complete
  + conformance-green now, so P2 is plumbing. No new crypto dep (reuses `ed25519-dalek`).

- fix(serve): **write-path audit hardening** (post-CAS adversarial sweep — a 4-agent
  read-only fan-out over the write door + verbs + authz/redaction). Real findings closed
  at root:
  - **Idempotency fail-OPEN → fail-CLOSED (H12).** `idem_lookup` used `.ok()?` on the
    stored outcome, so a ledger entry that MATCHED the key tuple but whose `outcome`
    would not deserialize folded to "no prior found" — silently RE-EXECUTING the verb
    (a double effect). A matched-but-corrupt entry now returns 503, never re-executes
    (test `corrupt_idem_entry_with_matching_key_fails_closed_not_re_executes`). A true
    no-match still runs the verb.
  - **Unscrubbed enum echo in 400 errors (F-9).** `land`/`verdict`/`issue_transition`
    echoed the raw invalid `mode`/`verdict`/`to` value into the error reason; a
    secret-shaped value round-tripped to the caller verbatim. Now `scrub`bed at the
    boundary like every other free-text field.
  - **Unsigned `If-Match` threat-model note (H6).** Documented that the unsigned
    conditional header is strip-able by an on-path intermediary (bounded: the engine
    talks to R2 over direct TLS, so not exploitable in the deployed topology); narrowed
    the safety claim and noted signing as the future close.
  - Verified-and-sound (no change): the serve loop is single-threaded (the Local CAS
    comment is accurate; no in-process race), `undo`'s ownership gate holds (cross-tenant
    undo is 404), and the core CAS race/exhaustion logic. Tracked (by-design / latent):
    `undo`'s caller-asserted `Human` class (P2 identity seam), `repo` not in the
    idempotency tuple (latent until logs ever consolidate).
  fmt + clippy `--workspace -D warnings` clean; serve suite green.

- fix(serve): **CAS hardening — fail-closed on a missing R2 version token** (adversarial
  audit of the PR #137 CAS guard). If an R2 GET returned no ETag for an existing object,
  the captured `CasToken` was `Unsupported`, which on the write path would have made
  `persist` an UNCONDITIONAL PUT — silently reintroducing last-writer-wins. R2 always
  returns an ETag (so this is defense-in-depth against an anomalous response), but a
  SILENT CAS bypass is exactly the class to close: the R2 write path now refuses an
  `Unsupported` token with a 503 (no network PUT) rather than degrade. The snapshot
  uploader's intentional unconditional create (`put`) is documented as seeding-only.
  Test: `r2_persist_with_unsupported_token_fails_closed_not_unconditional`.

- fix(deps): bump `git2` `0.20 → 0.21` (hugit-proto test-only dev-dep) to clear
  **RUSTSEC-2026-0183 / RUSTSEC-2026-0184** (git2 0.20 `Remote::list()` /
  `BlameHunk` unsoundness). Test-only and never shipped in the product binary, but
  the advisory gate (`cargo deny`) flags any version; drop-in (hugit-proto suite green).

- feat(serve): **compare-and-swap write durability (closes the P2→P1 CAS obligation).**
  The write-door's `load → mutate → persist` cycle held no lock across the gap, so a
  concurrent writer (the snapshot uploader today; horizontal scale tomorrow) could
  silently clobber another request's records AND its idempotency-ledger entry
  (last-writer-wins). Now persist is a **compare-and-swap** against the head the
  matching load returned:
  - `LogSink::load` returns `(EventLog, CasToken)`; `persist` takes the expected
    `CasToken` (`Absent` | `Version(etag/hash)` | `Unsupported`). The chain verify
    stays the single PS-13 chokepoint (`load_verified_with_token` IS the body;
    `load_verified` drops the token — readers untouched).
  - **R2**: a conditional signed PUT — `If-Match: <etag>` (overwrite-if-unchanged) /
    `If-None-Match: *` (create-if-absent); the ETag is captured from the GET. R2's
    **412 Precondition Failed** maps to a typed `EngineErr::cas_conflict`. The
    conditional header is a standard (non-`x-amz`) header → sent UNSIGNED per the
    SigV4 spec (the proven signer is untouched), and proven against LIVE R2.
  - **Local**: a content-hash compare before the atomic temp+rename (dev/test source;
    residual TOCTOU documented — the production multi-writer source is R2).
  - `with_write` wraps the cycle in a bounded retry loop (`MAX_CAS_ATTEMPTS=5`): on a
    `cas_conflict` it reloads + re-runs, so two concurrent requests with the SAME key
    collapse to one execution + one replay (no double effect), and with DIFFERENT keys
    both survive (no silent drop). Exhaustion → honest transient 503 (nothing
    persisted on a losing attempt).
  - Tests: three CAS unit tests (different-writer reload-and-both-survive, same-key
    winner-collapses-to-replay, exhaustion-503-with-no-partial-write) + a live R2
    `If-Match` round-trip proof (`tests/r2_cas_live.rs`, `#[ignore]`: stale→412/
    cas_conflict, correct→200, ETag stable/non-destructive). fmt + clippy
    `--workspace -D warnings` clean; full serve suite green.

- fix(serve): **pre-go-live audit remediation — 3 findings** (the audit's non-P1
  remainder; the P1 JWKS DoS shipped separately):
  - **Existence/integrity oracle (info-leak).** The read path verified the log
    (→ 503 on tamper/transport fault) BEFORE the per-tenant gate, so a non-owner
    could distinguish a tampered/existing PRIVATE repo (503) from a non-existent
    one (404) — violating the "deny→404, no existence leak" law. New
    `authz::is_operator`; a load failure now maps to the honest 503 ONLY for the
    operator, and to a uniform 404 for every other caller (applied at all four load
    sites: repo read, SSE, `/v1/me/dashboard`, `/v1/me/attention`).
  - **`viewer-can` reported operator capabilities to EVERY viewer.** The handler was
    called with a hardcoded `dev_principal()` stub AND projected the per-class D14
    matrix — which is not the per-caller write gate (verbs append as a fixed class;
    the real gate is `authorize_write`/ownership). Now built from the REAL caller +
    the repo's meta as `authorize_write(principal, meta)` across all six affordances
    — faithful to the write-door: owner/operator → all-true; a non-owner (even on a
    PUBLIC repo — public opens READS only) → all-false. (Render hint only; the engine
    re-decides regardless.)
  - **`write_policy` echoed an unscrubbed secret-shaped `rule_id` (P3 self-leak).**
    The `[A-Za-z0-9-_.:]` shape check allows `_`, so a `ghp_…`-shaped rule_id passed
    and was persisted/echoed verbatim. `rule_id` is now scrubbed at the boundary
    (a legit rule key is not secret-shaped → no-op).
  - Tests: oracle (`is_operator` matrix + the non-operator 404 mapping), viewer-can
    (operator/owner full, non-owner all-false on public+private, unclassifiable
    all-false), policy rule_id scrub. fmt + clippy `-D warnings` clean; full serve
    suite green.

- fix(serve): **close a P1 unauthenticated JWKS-refetch DoS** (pre-go-live audit, the
  one confirmed-critical finding). `/v1/token` is the only no-Bearer route; it read
  the attacker-supplied `kid` from the JWT header BEFORE signature work, and a cache
  miss unconditionally triggered a **blocking 10s upstream JWKS GET**. A flood of
  random `kid`s thus forced one synchronous fetch per request — and the engine is
  single-threaded, so each fetch stalled ALL traffic (reads/writes/SSE). ROOT FIX: a
  refetch throttle (`JwksCache::refetch_allowed`) — a miss refetches at most once per
  `JWKS_MIN_REFETCH_SECS` (60s) and otherwise fails closed WITHOUT a fetch; the
  ATTEMPT time is recorded (so a slow/unreachable Clerk JWKS also stalls ≤1 request
  per window, not every request). Key rotation still works (bounded ≤60s staleness).
  Test: cold → allowed, immediate repeats → throttled, post-interval → allowed.

- fix(serve): **`R2Config` accepts the S3-standard cred var names** alongside the
  engine-native ones, so a CoreLink/AWS credential file (`HUGIT_SERVE_R2_ENDPOINT` /
  `_ACCESS_KEY_ID` / `_SECRET_ACCESS_KEY`) can be `source`d verbatim — only
  `HUGIT_SERVE_R2_TENANT_ID` is supplied separately (it is not part of a generic
  cred). Removes the manual name-mapping the snapshot upload needed. The pure core
  is factored to `R2Config::from_vars(get)` (testable without mutating global env);
  R2-source selection now triggers on `_ACCOUNT_ID` **or** `_ENDPOINT`. Tests: native
  names, S3-standard names, and a missing-host error.

- fix(serve): **reject a `:` in the resolved Clerk org (tenant) at the mint boundary**
  — pre-go-live adversarial audit of the live-write path. The engine principal is
  `clerk:{org}:{user}` and `authz::caller` splits it on `:` to recover the org for
  ownership comparison; an org containing a `:` would MIS-PARSE (a structural
  "exemption-is-a-hole"-class confusion). Real Clerk tenant_ids/org_ids (UUIDs,
  `org_…`) never contain `:`, so this was LATENT not exploitable — but it's closed
  fail-closed at `Claims::org()` (a colon-bearing `tenant_id`/`org_id` → `None` →
  `/v1/token` 401, no token minted) so a colon can never reach the authz delimiter.
  Audit also CONFIRMED CLEAN: no HTTP path writes `repo.meta` (ownership is
  unforgeable via the API — only the local `repo meta set` verb); the principal org
  comes from the RS256-validated JWT (not user-injectable); engine tokens are 32
  CSPRNG bytes, SHA-256-keyed, constant-time looked up; a `clerk:` token whose org
  is literally `orchestrator` does NOT escalate to operator (the `clerk:` prefix
  guards). Test: colon `tenant_id`/`org_id` → rejected.

- feat(cli): **`hugit repo meta set`** — the `repo.meta` PRODUCER (closes the
  `owner_tenant` assignment seam's write half). The engine
  (`hugit-serve::authz::project_repo_meta`) already CONSUMED the latest `repo.meta`
  record to decide reads (visibility) and writes (ownership), but nothing WROTE it
  — so every repo defaulted fail-safe PRIVATE / no-owner (operator-only), and an
  owner's per-session token could never read/write its own repo. `repo meta set
  --visibility <public|private> [--owner-tenant <org>] [--by <human>]` records a
  chain-valid `repo.meta` through the SAME D14-guarded / scrub-on-append / atomic
  -persist chokepoint the campaign verbs use — never hand-assembled JSON.
  Visibility is a STRICT closed enum (a typo errors, never silently → private);
  `owner_tenant` is identifier-validated; empty ⇒ unassigned (operator-only).
  Registered in `HUGIT_VERBS` (no-drift oracle); `repo` shadows no `git` verb (X5).
  - **Snapshot wired:** `build-engine-snapshot.sh` now seeds hugit's `repo.meta`
    as **PRIVATE** (owner-decided 2026-06-16: the product is free, but the forge
    view is not a public showcase), with `owner_tenant` from `$HUGIT_OWNER_TENANT`
    — unset ⇒ empty ⇒ the snapshot NEVER fabricates a tenant id. `engine-snapshots/
    hugit.json` regenerated (35 records, 1 `repo.meta`, chain-valid by construction).

- fix(serve): **`public` repos were WRITABLE by any authenticated tenant** — the
  write gate reused `authorize_read`, which opens `public` to everyone. A latent
  hole (today every repo is fail-safe private, so it's not yet reachable) that the
  close-the-product step would ACTIVATE: the moment the launch repo `hugit` is
  marked `public` for reads-by-all, ANY signed-up tenant could `land`/`verdict`/
  `policy`/`erasure`/`dispatch`/`undo`/`edit` into it — the exact risk the githugr
  TL flagged (why they refuse the operator token with signup open). Found by
  cold-verifying the `/v1/token` deploy thread.
  - **New `authz::authorize_write`** — write permission is OWNERSHIP, not
    read-visibility: operator (`orchestrator:*`) OR the owning tenant
    (`clerk:{org}:…` whose org == a SET `owner_tenant`). Visibility is NOT
    consulted — `public` opens reads only; writes to a public (or owner-less) repo
    stay operator-only. The write door (`writes::with_write`) now gates on
    `authorize_write`, not `authorize_read`.
  - Tests: a non-owner tenant WRITE to a PUBLIC repo → 404 + no log trace (owner →
    200, read still open); unit matrix (public/private/no-owner/unknown × owner/
    operator/tenant). fmt + clippy clean; full serve suite green.

- feat(serve): **`repo_chrome` serves the REAL machine visibility** + **lock the
  2 corrected write routes** (closes the githugr TL's 2026-06-15 authz-confirms +
  write-route-correction loop; the engine half of the cross-tenant authz wave).
  - **`visibility` field now REAL.** `build_repo_chrome` projects `visibility`
    from the SAME `crate::authz::project_repo_meta` source the read gate decides
    on (one law) — was honest-default empty. The wire carries the **machine
    value** `"public" | "private"` via a new `Visibility::as_machine_str()`
    (single source of truth); the **window** maps it to a localized display label
    (TL decision: i18n in the presentation layer). `RepoChromeVm.visibility`
    doc-locked to the machine value; a pre-`repo.meta` repo fail-safe defaults to
    `"private"` (matches the gate).
  - **Write routes confirmed-by-test.** The githugr TL cold-checked `LiveActions`
    and corrected 2 routes (comment → plural `/prs/{pr}/comments`; edit_propose →
    path-scoped `/edit/{*path}/propose`). The engine ALREADY served exactly these
    — no code change; locked with routing tests so the Fixture→Live flip is
    proven: plural `/comments` → 200 / singular `/comment` → 404; multi-segment
    `/edit/src/foo/bar.rs/propose` → 200 / bare `/edit` + no-`/propose` → 404. All
    9 spec-§3 write routes match the verified client list.
  - Tests: 4 new (chrome machine-value public/private + pre-meta default; the 2
    route-shape proofs). fmt + clippy clean; full serve + contracts suites green.

- fix(serve): **close two cross-tenant holes the post-#126 adversarial audit found**
  — the read gate alone wasn't enough.
  - **P0 — writes were UNgated.** `POST /v1/repos/{repo}/*` (land/verdict/comment/
    policy/erasure/dispatch/undo/edit) had NO per-tenant check — any authenticated
    tenant could WRITE into another tenant's repo. Now the same fail-closed gate
    runs at the `with_write` chokepoint (after load, before idempotency/effect):
    `authorize_read(principal, project_repo_meta(log))` → deny is 404, no effect.
    The engine re-decides on EVERY verb (ADR-0007 §3), not just reads.
  - **P1 — `/v1/me/*` leaked the launch repo.** `me/dashboard` + `me/attention`
    discarded the principal and served `hugit`'s operational data to any
    authenticated tenant. Now gated against the bound repo (operator bypass;
    non-owner tenant → 404).
  - Audit also confirmed CLEAN: `caller()` parsing (no operator-forge via a
    `clerk:` org), `repo.meta` not HTTP-writable, `/v1/admin/tokens` org-scoping,
    pagination/panic-safety. The load-then-gate 503-on-tampered-private edge is a
    documented LOW residual (same family as PS-18's 503-vs-404).
  - Tests: a cross-tenant WRITE → 404 + leaves no trace (owner → 200); me/* tenant
    → 404, operator → 200. fmt + clippy clean; full serve suite green.

- feat(serve): **engine-side per-tenant READ authorization** (closes the
  cross-tenant read critical from the githugr 360° audit; the engine half of the
  githugr TL's 2026-06-15 request). In live/hybrid the engine served any repo's
  substance to any authenticated session — now the **engine re-decides fail-closed
  on every `/v1/repos/{repo}/*` read** (the window's `viewer_can` is a cosmetic
  hint only; ADR-0007 §3).
  - **`crate::authz`** (new): `project_repo_meta(log)` projects `visibility` +
    `owner_tenant` from the latest `repo.meta` record (**fail-safe defaults:
    PRIVATE, no owner** — never default-public); `authorize_read(principal, meta)`
    is the gate — `orchestrator:*` operator → bypass (keeps single-tenant dev +
    the launch repo working); `public` → open; `private` → ONLY a `clerk:{org}:…`
    principal whose org equals a SET `owner_tenant`; else deny.
  - **The gate is wired at the read chokepoint** (`route()` repo arm + the SSE
    `events` path): load+verify ONCE → gate → dispatch, so EVERY repo read +
    admin read + event stream inherits it with no double-verify. A denied private
    repo is a **404**, identical to a non-existent one (no existence oracle, never
    403). The tenant is the ENGINE-resolved principal org (from `two_tier_auth`) —
    never a window claim; the current engine-token flow needs NO per-read CoreLink
    introspect.
  - 13 tests (10 `authz` unit incl. the cross-tenant matrix + 3 HTTP integration:
    owner→200, cross-tenant→404, operator-bypass, public-cross-tenant, private-no-
    owner→404, gate-covers-admin-reads). `dispatch_repo` refactored to take the
    pre-loaded `&EventLog` (single verify per request). fmt + clippy clean; 197
    serve tests green. Build order per the handoff reply: gate now; the
    `visibility` VM field + `owner_tenant` assignment-at-creation land with the
    githugr-vm mirror (awaiting their confirm).

- fix(serve): **harden the admin control-plane from a 3-lens adversarial audit.**
  A fresh-context adversarial sweep (redaction · auth/access · DoS/correctness) of
  the new `/v1` admin reads found — and this closes — the real holes; the base held
  (auth gate, slug-guard, non-reversible token handle, pagination, panic-safety all
  confirmed clean).
  - **Cross-tenant session leak (P1):** `GET /v1/admin/tokens` discarded the
    principal and listed EVERY org's sessions. Now scoped by the caller's tenant —
    a Clerk principal (`clerk:{org}:{user}`) sees ONLY its own org; the platform
    operator (`orchestrator:`) sees all; an unrecognized principal fails closed to
    empty. New `TokenStore::list_for_org(scope)` + test
    `list_for_org_scopes_to_tenant_no_cross_tenant_leak`.
  - **Read-boundary scrub completeness:** `ErasureRowVm.state` (P1) and
    `AuditEntryVm.kind` were the two VM fields surfaced RAW — a tampered-log value
    could ride them past the module's "every surfaced field scrubbed" invariant.
    Both now `scrub`-ed (no-op on the legit fixed vocab; closes the leak vector).
  - **Honesty fixes:** corrected the `TokenStore::lookup` comment that overstated
    its constant-time guarantee (it `break`s on match — per-key compare is
    constant-time, loop position is not, but it's not an exploitable oracle);
    documented the `principal_of` length-1-chain assumption and the
    `build_admin_overview` O(open_prs × log) bound (accepted at current scale,
    tracked optimization — not silently shipped as "fine").

- feat(serve): **admin control-plane — active token sessions** (`GET /v1/admin/tokens`
  → `AdminTokensVm`). Lists the ACTIVE engine-token sessions from the in-process
  store (handle = hex of the stored `SHA-256(token)` — non-secret, non-reversible;
  the raw token is never stored), with user/org (scrubbed), `fresh_auth`, and TTL
  remaining; soonest-to-expire first. `TokenStore::list()` sweeps expired entries
  first, so the list is exactly the live sessions. Account-level, Bearer-gated.
  **Honest scope:** single-host (the in-process store) — a fleet-wide session list
  + explicit revoke are the P2 shared-store seam (and revoke earns little on a
  300s-TTL single-host store that auto-sweeps). 3 tests (2 projection unit + the
  HTTP route/wiring). Completes the buildable-now admin reads (audit · erasure ·
  overview · tokens); runner status / fleet KPIs / multi-repo identity stay P2.

- feat(serve): **admin control-plane reads (operator area), engine half.** Three
  new `/v1` reads back the hugit-authored / githugr-hosted operator admin area —
  all pure projections over the already-chain-verified log (no P2 infra):
  - `GET /v1/repos/{repo}/audit?since=&limit=&kind=&principal=` → `AuditVm`: the
    paginated, all-kinds event timeline (who/what/when + integrity `hash_short`),
    forward cursor (`next_since`), exact-`kind` + `principal`-substring filters.
    The raw payload is NEVER echoed — `summary` is a kind-aware, scrubbed
    one-liner over the safe id field only (proven: a planted `ghp_…` in a payload
    does not appear in the projection).
  - `GET /v1/repos/{repo}/erasure` → `ErasureHistoryVm`: every erasure decision
    (approved + denied, latest-per-id, newest first), execution always `pending`
    (X12 execution is the P2 CAS-scrub seam).
  - `GET /v1/repos/{repo}/admin/overview` → `AdminOverviewVm`: the one-call
    operational snapshot (queue depth, active campaigns, attention — reusing
    `build_dashboard` so it agrees by construction — total PRs, enabled policy
    rules, erasure decisions, log depth, last-activity age).
  Auth: the admin reads ride the same Bearer two-tier gate as every `/v1` read.
  6 tests (5 projection unit + 1 HTTP route/wiring). The githugr UI (the admin
  AREA screens consuming these) is the next slice, authored in `../githugr`.

- docs(seams): **close the remaining in-control pending-seam halves + reconcile the
  register to reality** (the infra-gated ones are owner/P2, untouched).
  - **PS-4** (hugit-side DONE): `docs/interop.md` §8 now frames `HUGIT_RUNNER_HOST`
    as the INTENTIONAL cross-product seam name (contract-frozen, not a naming error,
    do-not-fix). Only the sibling-repo doc-title rename (in `../corelink-runners`)
    remains — owner-coordinated.
  - **PS-7** (docs-acceptance DONE): interop.md §8 documents that fleet-dispatch /
    multi-env orchestration MUST pass `--toolchain <digest>` explicitly (so the
    toolchain axis is never the shared `toolchain-unprobed` constant → no cross-env
    false cache hit).
  - **PS-12b** (registry-corruption half CLOSED): recorded that the `ci.yml`
    CARGO_HOME isolation (`$HOME/.cargo-hugit-ci`) kills the cross-repo registry
    corruption race by construction; only benign CPU/IO contention remains (the
    dedicated-runner P2 seam).
  - **PS-18** (reconciled): noted that Waves 1–5b shipped most originally-listed
    gaps (20 reads + SSE + 9 writes + token + R2 source); narrowed the residual to
    the genuinely owner/infra-gated set (live Clerk config, CoreLink CAS, GitHub-App
    mirror fields, fleet KPIs, live-tail SSE) + the ~12 git-layer reads kept
    honest-default by owner product decision.
  - **token.rs hygiene**: removed a stale comment block that called the embedded
    test RSA keypair a "PLACEHOLDER" and warned tests would `todo!()`-panic "until
    filled" — the keypair is a real (owner-waived, test-only) 2048-bit key and the
    21 token tests sign against it and pass. No code/behavior change.

- fix(seams): **close three in-control pending seams — PS-6, PS-10, PS-15 F-2 —
  at root (no infra gate).**
  - **PS-6 (queue verdict projection):** `hugit queue show` now carries a REAL
    per-entry + per-batch `verdict` + `implicated_pr`, projected from the SAME
    `verdict.recorded` events `campaign show` reads (the shared
    `hugit_ledger::Ledger` reject-sticky fold) — so the two views AGREE by
    construction. A union batch `"reject"`s if any member intent has an
    outstanding reject (`implicated_pr` names the first such PR in queue order),
    `"approve"`s once every member intent is proven, and stays `null` (disclosed
    `verdict_note`) until a verdict covers the batch — honest unknown, never a
    faked pass/fail. No P2 dependency (pure projection on the existing log).
    Tests: `acceptance_wb2::queue_show_projects_real_union_verdict_and_blame` +
    `…_verdict_is_null_until_a_verdict_covers_the_batch`.
  - **PS-10 (AC axis guard hoisted to the shared trait):** the structural-secret
    write-boundary guard now lives on `hugit_checks::client::ac::guard_axes_not_secret`
    and is enforced by EVERY `ActionCache` backend (`FileAc`/`InMemoryAc`/
    `HttpAcClient`), not only the CLI `FileAc` — close-by-construction completion
    of WK-AC. The predicate (`!is_safe_identifier_shape`) is byte-equivalent to
    the CLI's prior scrub, so behavior is unchanged; `FileAc` delegates and dropped
    its local copy. `hugit-checks` gained a `hugit-ledger` path-dep (no cycle; **0**
    new lock versions, single-line lock diff).
  - **PS-15 F-2 (error JSON `kind`-first):** every agent-facing CLI error envelope
    now emits `kind`→`message`→`fix`→context (was alphabetical `fix`/`kind`/`message`
    from `serde_json`'s BTreeMap object) — agents stream-match on `kind`, so it must
    lead. Both porcelain-error twins render through one shared
    `porcelain::ordered_error_object` builder; **zero new dep** (no workspace-wide
    `preserve_order`). PS-15 PERF F5 was found ALREADY verify-once-per-invocation
    (honest register correction); the residual alloc micro-shave is lead-deferred
    (marginal, integrity-critical surface). All affected-crate gates green.

- chore(conformance): **mirror the `result_binding_v2` vector (§7.1, contract
  v1.4.0)** byte-identical from corelink-runners (sha `600c99b5…`), under the X4
  drift tripwire (`manifest.sha256` + `acceptance_x4_wire`). Pins the full-outcome
  attestation binding formula (`…‖i32_be(exit)‖u32_be(artifacts.len)‖Σ(LP(path)‖
  LP(digest))`) so it cannot drift between the repos ahead of hugit's P2
  attestation-verify path. Ratifies the §7.1 v2 amendment hugit-side.

- feat(serve): **Wave-5b — `POST /v1/token` (RFC-8693 Clerk exchange) + real
  request identity.** The engine now exchanges a Clerk session JWT for a
  short-lived opaque engine token and authenticates every request through a
  two-tier gate (a Clerk-minted engine token, else the dev-token fallback).
  Security-critical, so built conservatively + lead crypto-audited: the `alg` is
  pinned to RS256 on the UNTRUSTED header BEFORE any key material (kills `alg=none`
  + HS256-confusion), `kid`→JWKS (refetch-once, fail-closed), `exp`/`nbf`/`iss`
  mandatory, `azp` checked from the verified payload, tenant from
  `publicMetadata.tenant_id`→`org_id`→reject, `auth_time`→`fresh_auth` (300 s,
  absent/future ⇒ false). Engine tokens are 32 `/dev/urandom` bytes stored as
  `SHA-256(token)` with a 300 s TTL + sweep, looked up constant-time; the raw token
  and the `subject_token` are NEVER logged. Cross-tenant mint is blocked
  (`audience == claims.org`). Uses `jsonwebtoken 9.3.1` — already the single locked
  version (zero new package versions; the lock gains only dependency edges). Step-up
  verbs require a fresh Clerk session (`fresh_auth`) or the dev `X-Step-Up` header.
  **P2 seams (honest, disclosed):** the live Clerk JWKS URL + mandatory `azp` are
  owner/CoreLink-gated; `auth_time` isn't a Clerk claim yet so `fresh_auth` is
  `false` in prod until frontend re-verification; the in-process token store is
  single-host (same seam as the idem ledger). Drafted by a research agent, then
  lead-integrated + crypto-audited; **167** hugit-serve tests green (21 new token
  tests incl. the alg-confusion/tampered-sig/expired/cross-tenant attack matrix,
  signed against a real ephemeral RSA keypair under an owner-authorized test waiver).
- feat(serve): **Wave-5a — the SSE event stream** (`GET /v1/repos/{repo}/events?since=<seq>`,
  spec §2). The live-update channel the githugr window subscribes to, as
  **replay-then-close**: every record with `seq > since` is emitted as an SSE frame
  (`id: <seq>` ≡ `data.seq`, `data` = `{seq, kind, summary}`), a `gap` sentinel is
  prepended when `since` is below the last-10k retention floor, and a trailing
  `: hb` heartbeat closes the body. The `summary` is read-boundary-scrubbed (payload
  free text); `kind`/`seq` are structural. Auth + safe-slug + the PS-13 verified
  load gate the stream identically to every other read. **True live-tail is the
  documented P2 seam** — the sync `tiny_http` loop processes responses serially and
  cannot hold a stream open; the client reconnects from its advanced `since` cursor
  (the githugr `EventsClient` already handles clean closure). Designed by a research
  agent against the frozen client frame format, built + lead-integrated.

- feat(serve): **Wave-4 reads — 6 more deferred surfaces go REAL** (`settings` ·
  `releases` · `search` · `viewer-can` · `dashboard` · `attention`). Built by a
  Spec→Build→Audit agent pipeline whose **Spec phase gated out the hollow ones**:
  `actions` was correctly DROPPED (its contract type is the write-path `Accepted`
  shape, not an activity-feed VM — no fake handler shipped). The six real ones
  project live log data: `GET …/settings` (house rules + operator overrides from
  `policy.set`), `GET …/releases` (the `pr.landed` history, newest-first, with the
  `latest` pill), `GET …/search?q=` (over prs/intents/issues/campaigns records),
  `GET …/viewer-can` (the REAL D14 authz capability matrix per principal class),
  `GET /v1/me/dashboard` + `GET /v1/me/attention` (the dev-principal's real forge
  work — open/queued PRs, pending verdicts, blocked PRs — scoped to the launch repo;
  the per-principal multi-repo aggregation is the documented P2 identity seam).
  The per-handler **adversarial audit phase caught a real systemic class** — the
  "exemption-is-a-hole" pattern (Round 7/Wave K): `pr_id`/`rule_id` are
  user-supplied payload STRINGS (not validated numbers), so a secret-shaped id
  echoed into a composite text field leaked verbatim; fixed at the read boundary in
  `settings`/`releases`/`dashboard` (whole-field scrub + regression tests), plus a
  real projection bug in `releases` (the cap took the OLDEST page and dropped the
  `latest` release — now newest-first, cap at source). Every echoed free-text field
  is read-boundary-scrubbed; honest defaults for the still-P2 fields. clippy
  `-D warnings` clean; full hugit-serve suite green.

- feat(serve): **Wave-3 reads — the 3 write-backed surfaces go REAL** (`review` ·
  `issues` · `security`). Now that the Wave-2 writes emit `verdict.recorded`,
  `pr.comment`, `issue.transition`, `policy.set`, and `erasure.decided`, three more
  `/v1` reads serve REAL projected data (never hollow): `GET …/prs/{n}/review`
  (verdicts + comment timeline from a PR's records; `None`→404), `GET …/issues`
  (latest-`issue.transition`-per-id folded into the open/backlog/in-flight/closed
  tabs, latest-wins state + sticky priority), `GET …/security` (the 3 house policy
  rules' current state from latest-`policy.set`-per-rule + the latest approved
  `erasure.decided`; the erasure CAS-scrub step is always `pending`, never
  `executed` — execution is the P2 seam). Read-boundary redaction on every echoed
  free-text field; honest defaults for the still-P2 fields (diff/attestation/
  supply-chain/git-layer). Built by a 3-agent fleet against the frozen contract VMs;
  a lead fix corrected a latest-wins bug in the security policy-rule fold. Per-handler
  parity + secret-MATRIX tests; clippy `-D warnings` clean; full hugit-serve suite
  green. The git-layer/identity reads (blob/compare/dashboard/account/…) stay
  honest-default fixture — a product decision, not hollow shells.

- fix(serve): **Wave-3 read-audit hardening.** A fresh-context adversarial audit of
  the 3 new read handlers (cold-verified by the lead) closed two defensive defects in
  `issues`: an `issue_id as u32` overflow that silently wrapped ids > `u32::MAX` into a
  low-32-bit collision (now rejected, not truncated) and an unbounded transition fold
  that allocated one map entry per distinct id before the cap (now bounded at the
  source). `VerdictVm.adversarial = true` was documented as a forge invariant rather
  than left as a silent hardcode. `review`/`security` audited CLEAN.

- feat(serve): **Wave-2 write path — the 9 `/v1` POST verbs + the write-door**
  (`land`·`verdict`·`comments`·`dispatch`·`issues/{n}/transition`·`policy`·
  `erasure/{id}/decide`·`edit/{path}/propose`·`undo`). Each verb is a PURE function
  the shared write-door (`writes::with_write`) wraps: an idempotency ledger
  (`Idempotency-Key` mandatory → `400`; byte-identical 24h replay; `409` on
  key+body mismatch; the **land one-position invariant** — a lost-response retry
  never enqueues twice); read-boundary redaction on every free-text field; the D14
  authorized-append door; a STEP-UP gate (`policy`/`erasure` → `403` without fresh
  auth); a request-body size cap; and the `Denied`→`EngineErr` §3 error map. The
  `LogSink` persistence trait abstracts the write target (atomic temp+rename Local /
  signed-PUT R2; the write-credential + R2 compare-and-swap are the disclosed P2
  seams — the read-only standing cred 503s honestly). Built by a 9-agent fleet
  against a frozen interface, then adversarially audited (FIX-FIRST → all P0/P1
  fixed at root: `dispatch.campaign` scrub, `policy.rule_id` structural validation,
  comment fail-closed routing, the step-up gate, the body cap, the LogSink CAS
  contract, edit-content stored as a CAS hash never field-scrubbed). Lead-owned T3
  design: `dispatch` never auto-spawns (spawn is the P2 runner seam), `erasure`
  records the decision but NEVER executes (X12 execution is P2), `undo` is
  append-only (a compensating `op.undone`, never a chain rewrite). 7 end-to-end
  POST integration tests + per-verb secret-MATRIX guards; clippy `-D warnings` clean.

- fix(serve): **don't leak R2 storage topology in a public 503 (pre-launch hardening)**
  — the R2 GET error arms echoed `ureq`'s error (which embeds the request URL: R2 host
  + bucket + tenant + key) into the `{code, reason}` body sent to the client, so a
  transport/5xx fault on the PUBLIC read path would disclose the storage layout. The
  specifics now go to the SERVER log only; the client gets a generic
  `engine storage temporarily unavailable` (still fail-honest 503, no
  existence/topology oracle). Found by a pre-launch security sweep of the about-to-be-
  public read path (the snapshot itself scanned clean — no secret, only structural
  hashes). The operator-only PUT (snapshot uploader) keeps its detailed errors.

- feat(serve): **`hugit-snapshot` — the one-shot engine-storage snapshot uploader
  (Passo 4)**. A dedicated bin that reads a local canonical event-log file,
  **chain-verifies it through the SAME PS-13 verified loader the read path uses**
  (`load_event_log_from_bytes` → `rehydrate_and_verify` → `verify_chain`), and only
  THEN PUTs the raw bytes to `<tenant_id>/<repo>.json` in the `corelink-githugr-engine`
  bucket — a corrupt/tampered log is REFUSED before any upload, so the snapshot the
  read path later serves is proven trustworthy at WRITE time, not just read time.
  Adds `sigv4::sign_s3_put` (same proven `authorization` core as the GET signer;
  the one PUT-specific bit — `x-amz-content-sha256` = SHA-256 of the ACTUAL body —
  is unit-pinned to a `shasum`/Python-verified digest, never recalled) and
  `R2Config::{from_env,put}`. The standing engine credential is read-only by design
  (a PUT 403s with a clear message); this tool runs with the one-shot READ+WRITE
  grant. Live-verified: the read path returns an honest 404 against the real bucket
  (auth OK, object absent) until the first snapshot lands.

- feat(serve): **R2 read source — the engine reads its event logs from the
  dedicated `corelink-githugr-engine` bucket directly** (engine-storage Option A,
  per the CoreLink handoff). `state.rs` gains a `LogSource { Local | R2 }`: R2 fetches
  `<tenant_id>/<repo>.json` over the S3 API, **SigV4-signed by a hand-rolled signer**
  (`sigv4.rs`) over the existing `hmac`+`sha2`+`hex` pins — **zero new crypto dep** —
  proven against the canonical AWS test vectors (get-vanilla signature · published
  signing-key derivation · empty-payload hash · RFC-4231 HMAC). Both sources route
  through the SAME PS-13 verified loader (new `hugit_cli::checks::load_event_log_from_bytes`
  next to the path loader): a tampered chain fails CLOSED (503) regardless of source;
  absent object → 404 (no existence leak); transport fault → 503 (fail-honest, never a
  fake-empty VM). Source is env-selected (`HUGIT_SERVE_R2_*` → R2; else
  `HUGIT_SERVE_LOG_DIR` → Local). The tenant prefix is the configured dev tenant until
  the P2 Clerk identity seam (disclosed, not faked). `cargo deny` clean (no new
  duplicate/advisory from `ureq`+TLS). This is the seam that takes
  `engine.githugr.com` from `fixture` to REAL on the read path — live on the scoped
  R2 credential.

- fix(ci): **make the gate steps deterministic against the self-hosted runner's
  PATH/proxy flake (PS-12b)** — the gate failed non-deterministically in DIFFERENT
  steps across runs, ALWAYS a command-not-found, NEVER a real lint/test/advisory
  failure: `cargo: command not found` in `deny`/`test` (the rustup `cargo` *proxy*
  at `~/.cargo/bin` vanishes on this box, while `fmt`/`clippy` found it in the SAME
  run) and `no such command: audit` (the proxy's external-subcommand search does not
  reliably consult `~/.cargo/bin`). Root-cause fix: every step prepends the DIRECT
  toolchain bin (`~/.rustup/toolchains/1.96.0-*/bin` — real `cargo`/`rustc`/`rustfmt`/
  `cargo-clippy` binaries that do not vanish) ahead of `~/.cargo/bin` + `/usr/local/bin`,
  and the advisory tools are invoked as their OWN binaries (`cargo-deny`/`cargo-audit`,
  not via the flaky proxy dispatch) with a self-healing reinstall guard. Code gates
  (fmt/clippy/test) have been green every run; this stops the infra flake from masking
  that.

- fix(test): **honest-up the scrubber proptest `safe_address` generator (PS-14
  boundary)** — the randomized `safe_addresses_survive_the_identifier_gate`
  property found a real counterexample (`bcjdf_3gh_i1k6l-e4m5.027`): a 24-char
  near-all-distinct mixed-alnum slug whose Shannon entropy (4.5016) clears the
  identifier gate's `IDENT_ENTROPY_THRESHOLD` (4.5), so the gate correctly redacts
  it. This is the owner-decided PS-14 policy ("prefer redaction" — a dense long run
  is indistinguishable from a credential blob), NOT a scrubber bug. The generator
  was overclaiming: its slug regex can emit high-entropy runs while its own comment
  promised "low entropy". Fix the GENERATOR, never the scrubber: a `prop_filter`
  excludes long-AND-high-entropy slugs (the disclosed over-scrub residual), so the
  property asserts only what the policy guarantees — short slugs always survive,
  long slugs survive iff low-entropy. Security spine untouched; 15×512 cases clean.

- feat(contracts): **hugit-serve Phase 2 wire freeze — 26 remaining read VMs +
  the `Accepted` write shape**. Transcribes the remaining `githugr-vm` view-models
  byte-for-field into `hugit-http-contracts` (intent_detail · insights · security ·
  review · issues · blob · campaign · github_app · repo_settings · branches ·
  releases · knowledge · search · new_pr · edit · compare · commit_detail ·
  dashboard · attention · account · import · login · org · profile · repo_chrome ·
  viewer_can), plus 4 shared atoms (`KpiVm`/`KpiSubKind`/`GithubAppStripVm`/
  `DashboardRepoVm`) and the §3 write success body `Accepted`. Derives copied
  verbatim from the canonical source — f64-bearing types `PartialEq`-only;
  internally-tagged enums and `#[serde(rename="cost_usd_micros")]` preserved
  exactly; one inline round-trip parity test per module. Additive-only (no existing
  type/field/signature changed). Phase A of the hugit-serve Phase-2 master plan.

- feat(serve): **hugit-serve Phase 2 reads — 6 real-backbone handlers**
  (`repo_chrome` · `branches` · `commit_detail` · `intent_detail` · `insights` ·
  `campaign`), wired into the `/v1` router. Each maps the verified event-log → its
  frozen VM with read-boundary redaction + honest defaults (never faked); by-id
  handlers return `None`→404 with no existence leak. Lead scope: only endpoints the
  engine can back with REAL data are served; ≥80%-honest-default surfaces
  (`viewer_can`/`attention`/`dashboard`/`security` + git-layer/identity reads) stay on
  the window fixture, disclosed as P2 seams (serving hollow shells would downgrade the
  live site). A 4-agent adversarial audit (redaction · honesty · robustness ·
  completeness) found + fixed: a P0 `env_manifest` redaction leak, a P0 `recorded_at`
  overflow, a FAKE `tokens_by_campaign`, unbounded per-request scans (now `PR_CARDS_CAP`),
  plus sound make-it-real reads (`campaign` chip, `when`, cost-xray `waste`/`first_pass`,
  `cost_xray_totals`). Each handler carries a parity test + a secret-MATRIX guard
  (a `ghp_…` PAT in a charter must serialize as `[REDACTED]`); whole hugit-serve suite
  green + clippy `-D warnings` clean.

- fix(test): **de-flake the serve_integration fixture race** — `scratch_dir()`
  named its temp dir from a `nanos + pid` suffix only, so two parallel test threads
  that landed in the same nanosecond bucket shared one `<dir>/hugit.json` and
  clobbered each other's fixture (a valid `[]` log vs. a tampered one). The flake
  surfaced non-deterministically (`present_repo_home…` saw the tampered log → 503;
  `tampered_log…` saw the valid `[]` → 200) — once on CI, reproduced locally at
  ~1-in-3 runs. Fix: a process-global `AtomicU64` per-call suffix guarantees a
  distinct dir regardless of clock resolution; 11/11 stress runs clean (was flaky
  at 3 runs). Test-only; no production code touched.

- fix(test): **de-flake the N-2 reconcile perf test** — drop its unsound ABSOLUTE
  wall-clock ceiling (`t5k < 12 s`), which false-failed at ~16 s on the contended
  shared runner (PS-12b infra contention, NOT an algorithmic regression). The
  contended O(n) @5k time (~16 s) OVERLAPS the old O(n²) @5k (18.2 s), so no absolute
  bound can separate "contended-but-linear" from "quadratic". The contention-INVARIANT
  SCALING RATIO assertion (2k→5k grows < 4×, vs O(n²)'s 6.25×) is kept as the sole,
  sound proof of the O(n²)→O(n) fix.

- feat(intent): **PS-9 — truthful per-source-log `intent list`/`show` + `--log` scope
  filter** (owner-decided SOTA). `intent new --log L` now records `L` as the intent's
  OWNING log in the store (`source_logs`, a backward-compatible
  `skip_serializing_if`-empty field — zero on-disk shape change for stores that never
  used `--log`). `intent list`/`show` resolve each intent's `landed` state against ITS
  OWN owning log, so a fleet orchestrator's default global one-call `intent list`
  reports the TRUE landed state per intent (and a new `log` field showing which log
  owns each) — no more misleading `landed:null` for every cross-log intent. A
  recorded-but-missing owning log → `landed:null` (honest unknown, never a false); a
  **tampered** owning log fails the call closed (`chain_broken`/exit-2 — the shared
  read-path invariant). `--log` on `intent list` is now a **scope filter** (restrict to
  intents authored against that log) rather than a resolve-against override; the
  reconcile path records source-logs for healed intents too. Closes PS-9.

- feat(conformance): **IntentMetrics conformance vector (§13.4) landed on `main`** —
  the hugit twin that corelink-runners PR #5 ("WAITS on hugit twin") explicitly
  blocks on. `conformance/IntentMetrics.json` + its `manifest.sha256` line are
  **byte-identical** to the sibling twin branch (`feat/intent-metrics-vector`),
  SHA-256 `2d8d22…d402`, honoring the iron rule that conformance vectors are
  committed byte-identical in both repos. Pinned by the x4-wire oracle — item ①
  (manifest lists exactly the now-three frozen vectors) and a new item ②
  (`IntentMetrics.json` round-trips byte-exactly through the frozen
  `hugit_contracts::IntentMetrics` type). Completes the contract v1.2.0 §13.4
  amendment on the hugit side; unblocks the sibling PR.
- feat(checks): **PS-11 — `hugit check --env-axis <VAR>` declares a custom env
  dependency** (owner-approved opt-in, repeatable). The hermetic spawn clears every
  ambient var not on the built-in result-affecting allowlist, so an ad-hoc `--cmd`
  check that reads a CUSTOM var (e.g. `MY_GATE_MODE`) would see it unset and a change
  to it could not bust the memo key. Declaring it with `--env-axis MY_GATE_MODE` adds
  it to the single captured-env source, so it is BOTH folded into the memo key (value
  change → MISS) AND passed through to the spawn — "declared == keyed == present", a
  sound dependency, never a stale green. An UNDECLARED custom var stays cleared
  (hit-rate preserved); the value is hashed into `def_digest` before persistence (no
  raw leak even for a secret-valued var). Verified end-to-end against the binary
  (`acceptance_ps11_env_axis`) + pure unit tests. Closes PS-11.

- fix(checks): **PS-17 — bound peak memory on the memo-key snapshot read.** The
  tree-axis + ancestor-config snapshot previously read each matched file whole into
  memory (a pathological 200 MB config → ~400 MB peak). All four read sites now route
  through `read_snapshot_content`: a file `≤ 64 MiB` folds as its raw bytes
  (byte-identical to before — memo key + hit-rate unchanged for every realistic
  input), and a file `> 64 MiB` folds as a bounded `OVERSIZE:<len>:<streamed-sha256>`
  sentinel (1 MiB streaming buffer). Soundness holds — a change to an oversized file
  still busts the key (no stale green) — while peak memory stays bounded. Defensive
  P3, closed by construction.

- fix(security,cli,ledger): **Round-8 SEVERE class sweep + Wave L — six root-cause
  classes closed by construction** (branch `integ/wave-l`; pending Round 9 re-audit
  + merge to `main`; push HELD). After the owner judged the prior fix waves to be
  band-aiding instances rather than killing the class, Round 8 shifted method from
  point-finding to **exhaustive root-cause CLASS audits** (6 SOTA reports,
  `docs/review/round8/`). All six classes shared ONE root — *open-by-default,
  enforced by convention/per-verb* — and Wave L closes each *by construction*:
  **L-A (C1 redaction)** inverts the identifier scrub to **deny-by-default** — a
  value survives verbatim only if it proves a bounded safe-address shape
  (`is_safe_identifier_shape`), else the door rejects (`secret_in_identifier` exit-2)
  or the boundary redacts; a prefix-less AWS/SendGrid/Stripe/base64 credential in any
  identifier field no longer leaks, while ULID/sha-hex/`cas:`/slug addresses survive.
  **L-B (C2 read-path)** routes `intent list` through `verify_chain` (a tampered log
  → `chain_broken` exit-2); the single-chokepoint loader refactor is tracked PS-13.
  **L-C (C3 memo-key + C6 error-law)** makes check execution **hermetic**
  (`env_clear` + captured allowlist, `cwd` pinned to `--root`, PATH pinned + hashed
  into the env axis) so cwd/env/PATH changes can no longer serve a stale green
  (FS/network/clock remain the disclosed P2 runner-sandbox seam); and makes
  retryability a **type** (`AcError::Busy` matched exhaustively in `map_exec_error`,
  the `starts_with("ac_busy:")` string-sniff deleted) plus a structured
  `invalid_arguments` envelope for clap arg errors (`try_parse`). **L-D (C5
  state-machine + C4 authz)** makes the per-lens verdict fold **reject-sticky within
  a record** (and the recorder refuses a conflicting duplicate-lens →
  `duplicate_lens` exit-2), enforces a **single seal-guard chokepoint** at the
  `hugit-refstore` append boundary so a SEALED campaign is terminal for ALL verbs
  (`campaign_sealed` exit-2), and demotes the raw `EventLog::append` door to
  `pub(crate)` with a typed closed-enum `append_external_change(ExternalChangeKind)`
  shim (test-only raw access behind a `#[cfg(feature="test-support")]` `append_for_test`).
  Each fix was cold-verified by live attack reproduction by the orchestrator; the
  full workspace gate is green (fmt + clippy `--workspace --all-targets --locked
  -D warnings` = 0/0 + test `--workspace --locked` = **1233 / 140 suites, 0 failed**).

- fix(security,cli): **Adversarial Round-5 fixes (Wave I) — identifier-redaction
  hardened structurally, forge state-machine coherence, log-auth honestly scoped**.
  Round 5 (fresh 7-agent fleet + a convergence synthesizer) held the spine an 8th
  time; findings were the identifier-redaction coupling, multi-step state-machine
  coherence, and one honesty gap. WI-SCRUB: identifier fields {campaign,intent_id,
  pr_id,run_id} are no longer blanket-exempt from scrub — their value routes
  through the engine's STRUCTURAL secret detectors (prefix/connection-string/JWT/
  PEM) at the single boundary, exempt ONLY bare-hex/entropy, so a `xoxb-`/`clp_`/
  `Bearer`/`ghp_` smuggled into ANY identifier field of ANY verb redacts while a
  40/64-hex address survives (no per-verb-validator dependence; the exemption-is-a-
  hole class is closed structurally). WI-PR: `pr land` is idempotent on a terminal
  landed PR (no post-terminal `pr.queued` corruption); `pr open` validates intent
  existence (no phantom-intent PR); `intent new` bootstraps `.hugit/`. WI-PROVEN2:
  verdict revision resolves latest-wins (`proven` XOR `rejected` per intent — an
  approve-then-reject no longer leaves `proven` stuck); `campaign close` soft-gates
  rejected work (`campaign_has_rejected` exit-2 by default, `--allow-rejected`
  seals with `sealed_with_rejected:true`). WI-HONESTY: PS-8 tracks event-log
  cryptographic authentication as a P2 server-side seam (the local unkeyed chain
  is tamper-EVIDENT, not tamper-PROOF against a competent rewriter — disclosed with
  the SAME honesty as the AC HMAC seam); accepted-risk register entries (double-exec,
  kill-portability, rate-limit, orphan); status truth. WI-TESTS + follow-up: killed
  the concurrency/symlink/hit_rate test theater (tests now fail if the fix regresses).
  Also hotfixed a fmt-RED + clippy-RED HEAD (a piped gate-check had masked both).

- fix(security,cli): **Adversarial Round-4 fixes (Wave H) — wedge EXECUTE path
  hardened**. Round 4 (fresh 7-agent fleet) held the spine a 7th time; findings
  concentrated in the fast-built EXECUTE path + the scrub exemption + the proven
  projection, several being consequences of Wave G's own fixes. WH-SCRUB:
  the digest-key scrub exemption is now VALUE-GATED — a `*_digest`/`tree_hash`/
  `hash`/`commit` field is exempt only if its value is actually digest-shaped
  (64/40-hex or `sha256:`/`cas:` prefix), so a secret smuggled via
  `check --toolchain`/`verdict --tree-hash` now redacts; identifier fields
  {campaign,intent_id,pr_id,run_id} are exempt-from-scrub (addresses, not free
  text). WH-IDENT: identifiers are validated at INPUT (reject empty +
  known-secret-prefix shapes) so the exemption can't leak or collapse — two
  distinct 40-hex campaign keys no longer collapse to one `[REDACTED]`. WH-CHECK:
  the `.ac` lock is held ONLY for cache ops, never across execute (a hung/slow
  command no longer poisons the log); the child runs in its own process group
  and is killed as a group on timeout (no surviving orphans); `check --store` is
  idempotent (dedup on memo_key+cache_hit → no KPI inflation); `collect_files`
  guards symlink cycles (visited-set + depth cap → no SIGSEGV); ad-hoc `--cmd`
  now memoizes (state files excluded from the tree axis). WH-PROVEN: `proven`
  advances ONLY on an APPROVE verdict; a REJECT surfaces `ledger.rejected` in
  campaign show/close (rejected work is no longer sealed as proven). WH-DOCS:
  queue-verdict CHANGELOG claim corrected, PS-6 (queue batch-verdict) + PS-7
  (toolchain-unprobed) tracked, status truthful, + an e2e wedge-chain test.

- fix(security,cli): **Adversarial Round-3 fixes (Wave G) — wedge hardened to
  engine standards**. Round 3 (fresh 7-agent fleet) confirmed the spine held
  through all three rounds; the findings were the fast wedge wave's rough edges,
  now closed. WG-SCRUB: redaction is now STRUCTURAL — `porcelain::scrub_to_canonical`
  (which calls `scrub_payload` internally) at the single append boundary
  scrubs every user string before the hash chain, so no porcelain verb can
  leak a secret (the per-verb gaps in check/verdict/pr are closed at the
  root; digests survive). WG-CACHE: the `<log>.ac` local cache
  is tamper-evident (sha256 self-hash verified on read → a forged green is never
  served), real toolchain digest (no cross-toolchain false-hit), 300s exec
  timeout with child-kill (no lock starvation on a hung command),
  lookup-before-decision lock (no double-exec at the time — **superseded by
  Wave H / WH-CHECK**: the lock is held only for cache ops, not across
  execute; a bounded double-exec window is accepted + tracked as AR-1 in
  `docs/plan/2026-06-11-pending-seams.md`), `hugit check` honours the
  log-not-found/exit-2 law, `cmd_ignored` honesty. WG-PR: reachable `.expect()`
  panics → structured internal errors (one-error-law), a settled `pr.landed` PR
  leaves the queue projection. WG-COHERENCE: `hugit verdict` rejects a
  nonexistent intent (existence guard), `intent.landed` carries `campaign` so
  the ledger files it correctly, and a landed+approved intent advances
  `proven` — `campaign show`'s two halves now agree. WG-DOCS: PS-1 moved to the
  Closed table, Wave F CHANGELOG entry added, CLAUDE/README status truthful,
  interop v1.2.0, CI rate corrected, honesty tests (a memoized RED stays red).

- feat(cli): **the memoized-CI wedge is now operationally REAL (PS-1 closed,
  P2-independent)**. Owner-directed (execute-for-real, full wedge):
  `hugit check --def <fmt|clippy|test|--cmd> --log <l> [--store]` executes via
  `hugit_checks::executor::run_memoized` over a file-backed local Action Cache
  (`<log>.ac`, cross-process; live `HttpAcClient` swaps in at P2 behind the
  `ActionCache` seam) — a cold run executes (cache_hit:false, 1 exec), a warm
  re-run with byte-identical inputs is a HIT (cache_hit:true, duration_ms:0,
  same 64-char memo key), editing a globbed input busts the key. `hugit checks
  show` now reports a REAL hit-rate (50% over a seeded log; non-null
  hits/executed/saved_ms) — the all-null negative-control note is gone.
  `hugit verdict --intent <id> --lens <l> --result approve|reject … [--store]`
  records a canonical `VerdictObject` (`verdict.recorded`) via the guarded
  append seam; this flows to `proven` in campaign show. `queue show`'s
  per-entry verdict remains null-disclosed (the union-batch verdict seam is
  P2-tracked as PS-6). `pr land --settle` emits `pr.landed` so a campaign
  settles to `closed:true` via landed (not only abandon). All appends route
  through the D14 `append_authorized` guard.
  Built W0 (scaffold/contract freeze) → W-CHECK/W-VERDICT/W-PRLANDED (builders,
  stale-base) → W-INT (lead integration onto the frozen contract). The wedge is
  visible locally TODAY; P2 makes the cache fleet-shared.

- fix(security,cli): **Adversarial Round-2 fixes (Wave F)** — 7/7 refutation
  fleet (after Wave E) found narrower but real pendencies; all four closed.
  WF-REDACT: bare-hex-secret leak closed (a raw `[0-9a-f]{40,}` blob was not
  caught by the keyword-context engine; fix extends the redaction engine to
  detect bare-hex digests long enough to be secrets) + `export` unified to the
  ledger engine so the two code paths cannot drift. WF-CLI: abandon-projection
  deadlock closed (the `campaign abandon` handler held the log-lock while
  calling `checks show`, which re-acquired it; now a single atomic
  read-project path); `verify_chain` wired onto `hugit checks show` and
  `hugit queue show` (chain-invalid log → structured error, never silently-
  projected corrupt world); error uniformity sweep (`--log` missing, tournament,
  list consistency — all verbs surface stable JSON errors under the WB0
  one-error/one-exit law). WF-AUTHZ: `import_sidecar` cannot advance a
  protected ref under the all-class Push cell — the D14 guard was missing from
  the sidecar-import mutation path; fix routes it through `append_authorized`
  with the same matrix as every other guarded verb; caller audit confirms no
  remaining bypass. WF-CLI2: `campaign close`/`abandon` ghost-record guard (a
  `close` on a campaign with no `campaign.opened` event fabricated a record;
  guard now requires a live entry before any mutation) + load→lock TOCTOU on
  campaign/intent-store closed with the WC1 atomic-lock pattern. Gate green
  after each of the 4 merge commits (WF-REDACT · WF-CLI · WF-AUTHZ · WF-CLI2).
  Round 3 (fresh fleet) re-audits next.

- fix(security,cli): **Adversarial Round-1 fixes (Wave E)** — 7/7 refutation
  fleet found real pendencies behind the premature "SOTA" claim; all closed.
  E-REDACT: gitleaks-class redaction engine (connection strings, keyword-
  context, sk- length-gate). E-CLI: redaction parity across read+write
  surfaces — a real `ghp_` PAT in a charter no longer reaches `.hugit/
  intents.json`, the hash-chained log, or any show/list (live-reproduced) —
  plus pr verbs under the one error/exit law + verify_chain, campaign
  show/list reject a missing --log (no silent empty world), why hint
  corrected. E-GUARD + E-GUARD2: the D14 guard now wraps EVERY guarded-verb
  mutation path (porcelain + mirror-land + undo + import_sidecar +
  Serializer-undo + policy-emit) — full caller audit confirms no remaining
  bypass; push stays raw-by-design. E-PINS: hand-authored byte-pins for 5
  security-critical frozen contracts (de-launders the self-oracle). E-TESTS:
  e2e four-altitude chain, mid-write cold-store fault, rollup-at-scale (1000+
  intents). E-DOCS: CHANGELOG truth (18→17 packages, audit-closed→honest,
  wedge claim downgraded to match reality), `docs/plan/2026-06-11-pending-
  seams.md` register (recorder verbs · authn binding · coldtier erasure),
  contract-1.2.0 propagated to feature branches in corelink-runners
  (`integ/seed-runner`) + githugr (branch) with §12 amendment logs — sibling
  mainline merges are pending, not yet on their respective `main`s. Gate green:
  118 suites. Round 2 (fresh fleet) re-audits next.

- fix(robustness): **SOTA-audit Wave C — Tier-3 hardening + Wave D records sweep**
  (audit Tier-1/2 fixed; Tier-4 records + transplant-naming tracked). WC1: porcelain file seam is atomic+locked (`.lock` create_new
  + tmp/fsync/rename) — the TOCTOU read-modify-write race is gone (proof: 2
  concurrent `intent new` × 8 rounds → exactly one wins with `log_busy` or both
  serialize, chain always valid); corrupt/truncated/wrong-shape logs surface
  structured errors, never a panic or silent clobber (9 reject tests). WC2:
  rollup tiebreak deterministic (born_at DESC, run_id, seq — input-order
  independent), span≤sum law honest both directions, capture serialize path
  fail-closed (`EnvelopeError::Serialize`, no `.expect()` panic), arithmetic
  saturating. WC3: env-gated lanes print SKIP+reason (the green count now
  carries live-lane signal), x4 wire round-trip byte-exact (no trim_end mask),
  scratch dirs collision-proof (atomic counter). WD: docs swept true post
  Waves A/B — ADR JSONC synced to schema 1.2.0 + four altitudes, plan status
  lines flipped to COMPLETE, handoff marked APPLIED/CLOSED. (Note: the Wave D
  entry originally claimed "package count corrected to 18" — that was false;
  `hugit-runner` had already transferred out, leaving 17 packages. The 18-count
  claim was a doc error corrected in Wave E / WF-DOCS.)

- feat(cli): **SOTA-audit Wave B — the wedge made visible + one porcelain law**
  (audit Tier-2, P1-P8 closed). WB0: ONE canonical error shape
  (`{"error":{kind,message,fix,…}}`) + ONE exit law (0/2/1) across ALL verbs —
  the legacy four (why/impact/tournament/export) now speak stable JSON;
  missing/corrupt logs are explicit structured errors, never a silent empty
  world. WB2: `hugit checks show` (reads+aggregates check.recorded events;
  the recorder verb (`hugit check`) is a tracked pending seam — KPIs are null
  on logs without it; honest nulls), `hugit checks key` (engine-parity memo
  key — agents predict the cache), `hugit queue show` (union batches by
  campaign; verdict null-disclosed until the recorder seam lands). New verbs: `pr list/abandon`
  · `campaign list/abandon` · `intent list` (id recovery). Referential
  symmetry (orchestrator⇒--run-id, human⇒--principal, ghost campaigns
  refused), stable key-sets across idempotent re-runs, every fix-hint names
  only verbs that exist, `suggested_fix` is dead, and ALL porcelain appends
  route through the WA2 D14 guard (WA2b closed: subagent intent authorship
  allowed-with-audit per the matrix; pr/campaign classes enforced).

- feat(security)!: **SOTA-audit Wave A — the four security structurals closed**
  (audit: `docs/review/2026-06-11-sota-audit.md`). WA1 real redaction engine
  (known-prefix + Shannon-entropy detectors, EVERY envelope field scrubbed on
  the write path, `why`/ledger parity; content-address refs exempt with
  proof). WA2 D14 guard wired onto the mutation path (`append_authorized`:
  deny ⇒ no mutation + `authz.denied` audit record; authn binding honestly
  disclosed as the identity/P2 seam) + feature-proof explicit `canonical_json`
  (pinned `this_hash` byte-compatible). WA3 tombstone erasure on the REAL
  cold-store trait (`GetOutcome::{Present,Erased,Absent}`, resurrection
  refused, dedup-by-content disclosed, X12 re-pointed to production impls;
  refstore provenance tier correctly excluded — the proof must survive).
  WA4 money as integer micro-USD (**contract 1.2.0**, owner-ratified): 9
  fields renamed, exact-integer cost identity, checked_add fail-closed,
  hand-pinned goldens (generator no longer its own oracle). WA5 local
  escape-law regression (7 red-team vector classes + cross-repo sentinel).

- feat(workspace)!: **`hugit-runner` transferred OUT to ../corelink-runners**
  (WP-R4, runner-transfer campaign — owner-directed 2026-06-10: "hugit é
  basicamente um git, não faz sentido o runner ser feature do hugit"). The
  execution core (lease → container → forensic teardown, warm boot,
  concurrency/expiry/recovery, Actions-YAML shim + suites C2a/C2b/C3/C9/E4)
  now lives in corelink-runners (campaign #1 seed, transplant lead-verified
  @ b8fcde6); the seam is the **wire contract** — `conformance/{RunnerLease,
  FenceManifest}.json` + `manifest.sha256` committed byte-identical in both
  repos, pinned on this side by the new X4 wire oracle; no git dependency in
  either direction. Re-cut, with every proof kept: the WP-F2/F2b envelope
  producer relocated `hugit-runner::envelope` → `hugit-ledger::envelope`
  (capture is forge domain; F2 suites pass unchanged), `hugit-fence` keeps
  the C5b broker over a minimal disclosed seam (`BoxExec`/`CmdOutput`/
  `RunningContainer`; live impl = the runner product across the wire) while
  `materialize`/`enforce` + the escape red-team harness moved WITH the
  classifier they drive (relocated, never weakened), and X4's spawn-surface
  oracle moved with the spawn surface (rigor preserved by relocation; R0
  freeze). Workspace: 18 → 17 packages.
- docs(plan): **WP-R5 — runner-transfer campaign records (R0–R6)** (2026-06-10).
  Supersession appendix appended to `docs/plan/decomposition.md` (E6 precedent;
  register body frozen): C2a/C2b/C3/E4 acceptance suites + C9 harness +
  C5a materialize/enforce + X4 spawn-surface oracle TRANSFERRED to
  `corelink-runners/crates/corelink-runner` @ b6319a3, on top of contracts @
  78702d6 (triplet + MaterializedEntry closure) and integration contract v1.1 @
  9796aa8 (envelope emission obligations, WP-R6). Wire-contract seam: JSON
  conformance vectors byte-identical in both repos (manifest sha256 `159fe8c5…`;
  `RunnerLease` `ab1744c9…`; `FenceManifest` `07940b9a…`). hugit retains:
  `hugit-ledger::envelope` (F2/F2b capture, A7 green), `hugit-fence::{broker,seam}`
  (C5b + wire seam, no assertion weakened), `hugit-invariants` (INV-* green,
  wire-level X4 conformance assertion), `hugit-contracts` (frozen, untouched),
  17 packages. WP-R3 absorbed into R1b + R4② — no standalone commit, A3
  satisfied. `docs/strategy/absorption-map.md` Actions YAML row corrected.
  corelink-runners CLAUDE.md advanced to CODE status; handoff note authored.
  (runner-transfer-campaign)

- feat(cli): **the flow porcelain — `hugit campaign` · `hugit intent` · `hugit pr`**
  (owner-directed 2026-06-10: "quem vai digitar são os modelos de LLM").
  LLM-first design law: stable JSON always on stdout, structured errors
  carrying the suggested fix, idempotent re-runs (`already_exists`/
  `already_queued`, exit 0, no duplicate records). `campaign open` registers
  charter + human owner (D14) on the real EventLog; `close` is the SEAL —
  refuses with PRs in flight, prints the WP-F3 `campaign_rollup`; `intent new`
  lands through the real refstore path (tamper fails closed); `pr open`
  rejects subagent authorship at the door, `land` enters the real
  `hugit-queue` batch and reports position, `show` carries the `pr_record`
  cost block when envelopes are captured — honest nulls otherwise. 4 binary
  end-to-end acceptance tests; registry no-drift oracle extended (PC0–PC3b).

- chore(workspace)!: **`hugit-web` migrated OUT to ../githugr** (owner law
  2026-06-10: "hugit não tem tela — as telas todas são do githugr", the
  headless-engine doctrine applied to the workspace itself). The wave-1 crate
  tree was delivered verbatim to githugr (`incoming/hugit-web` @ githugr
  `8d08d6a` + DELIVERY manifest: seam = contracts·refstore·ledger·checks·
  queue·dogfood, no pub(crate)/feature walls); wave-1 history stays here
  (`debbd9e`…`33a6be6`). Workspace: 18 → 17 packages; web-only workspace deps
  (axum·tokio·maud·tower·http-body-util) removed. The engine carries no UI.

- feat(contracts): **WP-F1 — `ContextEnvelope` frozen at three altitudes** (ADR-0001,
  owner-ratified 2026-06-10). New frozen contract in `hugit-contracts`:
  `ContextEnvelope` + `IntentMetrics` with the `altitude: intent|pr|campaign`
  discriminator (each PR carries its orchestrator-session envelope, each campaign its
  own — owner-directed extension), `deny_unknown_fields`, `schema_version 1.0.0`,
  JSON Schema + 4 golden byte-exact round-trips (intent/pr/campaign/null-refs). No TTL
  field by ratified design (retention forever; erasure-by-tombstone only). Derived
  NOT-frozen read shapes `PrRecord`/`CampaignRollup` with decomposed cost
  (work/orchestration/verification/ci/waste), span-vs-sum time, efficiency, and
  `envelope_ref`; `PrAuthorKind` has no subagent variant (D14 authz at type level).
  Naming reconciliation documented additively — zero frozen v1 types/schemas touched
  (15 regen byte-identical). Trajectory refs are tier-agnostic (cold-store decision in
  the ADR). Cold-verified: full gate green, 120 test suites ok.

- feat(web): **githugr spine — wave 1 of campaign #4 (mockup → product)**. New workspace crate `crates/hugit-web` (`[[bin]] hugit-web`, axum 0.8 + maud + the Linear Noir kit lifted verbatim from the design corpus): the forge web surface as a **read-only MVP over a frozen `Provider` seam** (trait + 13 view-model families; screens render VMs, providers never emit HTML). Six spine routes (`/` → redirect, `/static/kit.css`, `/r/{repo}`, `/r/{repo}/landing`, `/r/{repo}/intent/{id}`, `/r/{repo}/checks`, `/r/{repo}/insights`) with real tab navigation (fixes the design corpus' #1 systemic gap), an honest `fixture` badge whenever the world is seeded, and 404 on unknown repo/intent. **The fixture world is derived from the real engine, not hand-typed** (parity law): `hugit_dogfood::run_wave_with_ac` runs wave A twice on a shared AC (cold: 5 executions / measured ms · warm: 0 executions — the memoization wedge, MEASURED) + wave B with a failing pair (→ Bloqueado); the world `EventLog` is built through the real `append`/`canonical_json` path (`intent.landed` + `verdict.recorded` events) and projected through the real `Ledger::from_records` + `intents_from_log`; the parity oracle (`tests/provider_fixture.rs`, 5 tests) re-derives those projections over the same records and holds every VM number to them — plus `verify_chain` over the world log. Five screens transcribed render-faithfully from their approved mockups (`../githugr/design/`, DDD): **Landing** (kanban + campaign bundles + per-card PR drawer with intents/charter/context/diff/verdicts; Land disabled-honest), **Repo home** (GitHub-faithful: file rows with `.ix` intent deep-links, README, About, synergy panel), **Intent detail** (charter + acceptance, 4 trajectory accordions with the load-bearing honest "não capturado" state on every `None` — never a silent blank; context.json block with disabled-honest ⤓/▸; diff + why-blame; rail: autoria/métricas/snapshot/verdicts), **Checks** (measured hit-rate displayed AS-IS with the honest FULL/PARTIAL/NONE/NO-DATA shape vocabulary, cache-hit vs executed affordances, expandable logs, bisect walkthrough, memo note), **Insights + Ledger view** (KPIs, server-rendered landed bars, token-by-campaign, Cost X-ray, pedido→feito→provado per campaign with intent deep-links). 46 new tests (5 smoke + 5 parity + 36 screen oracles), all VM-hand-built and decoupled from the fixture. Built by a 6-agent wave (worktree-isolated, disjoint 2-file claims, zero merge conflicts) over the lead's frozen W0 scaffold; cost/token/metric figures remain fixture-illustrative (the crate was migrated out to ../githugr before WP-F2 landed — WP-F2 + §7 are now both complete in hugit; githugr's Provider wires them in wave 2) and are surfaced as such via `Provider::is_fixture`. Workspace deps hoisted exact-pinned (axum/tokio/maud/tower/http-body-util). (wave githugr-spine-w1)

<!-- audit-remediation (post-0.1.0) entries -->
- fix(runner): FP-2 — production-crash hardening. (1) Poison-recovering every `entries`-mutex `.lock()`/condvar `.wait()` in `ws/mod.rs` `DedupSpawner` (`.unwrap_or_else(|p| p.into_inner())`, matching `hugit-proto write/order:140`): one job panicking mid-guard no longer turns the spawner into a permanent DoS for every later spawn (oracle `poisoned_entries_mutex_still_serves` is RED before, GREEN after). (2) `lease.rs` SSH now uses `StrictHostKeyChecking=accept-new` + a pinned `UserKnownHostsFile` (`HUGIT_RUNNER_KNOWN_HOSTS` or `$HOME/.hugit/known_hosts`) — trust-on-first-use, pin thereafter, instead of blind-accept-any-key. (3) `acceptance_c9.rs` box-gated skips now print a reason. (4) `pin.rs` adds `permanent_pull_failure_is_case_insensitive`. Behavior-preserving on the happy path; all gates green.

- fix(proto): hugit-proto attribution fail-closed + JSON-escape consolidation (FP-3). `SerializedWriter::push` no longer `.expect()`s on the external-change recorder (panicked on an empty `principal_chain`); it now validates attribution first and returns the new typed `PushOutcome::Rejected(PushReject::MissingAttribution)` (`PushOutcome` is now `#[must_use]`). `receive_pack` rejects an empty `principal_chain` with the new typed `ReceiveError::MissingAttribution` BEFORE recording, instead of silently appending an unattributed event (symmetry with the order path). The two divergent `json_str` encoders are consolidated into one full-escaping `pub(crate) fn write::json_str` (the store-side copy was lossy — it dropped control chars); both call sites use it. Oracle-tested (RED→GREEN, each confirmed RED with its fix reverted). All gates green.

- fix(fence): FP-1 — harden the session fence to a fail-closed allow-list. The old substring denylist was bypassable (`find <sib> -delete`, `git -C <sib> gc/update-ref/reflog expire`, path-assembly, and it enumerated siblings so new ones were unprotected). Now: DEFAULT-DENY the `~/Documents/HuGR/` parent for Edit/Write (only `.../hugit/` writable); Bash referencing a sibling allowed only as a single composition-free allow-listed read-only call (read-only progs / allow-listed read-only git subcommands); fail-closed on parse failure; `~`/`$HOME` normalized. Embedded `--selftest` encodes the audit's bypass vectors (16 deny + 10 allow, green).
- fix(checks): FP-4 — AC client trust-boundary hardening. `validate_memo_key` (`^[0-9a-f]{64}$`) + `AcError::InvalidKey` reject before any request (path-traversal/SSRF/cross-tenant via the key segment); `is_valid_tenant_slug` at config; `InMemoryAc` is `#[doc(hidden)]` + banner; `read_pat` rejects a world/group-readable PAT file (mode & 0o077) on unix. Loader fixtures now create the PAT at 0600 (matches the production discipline). Oracle-tested.
- fix(invariants): FP-5 — invariant proof rigor. `assert_scope_separation` takes `&ScopeSeparation` so the X10③ anti-vacuity test feeds a broken value to the REAL oracle (was an inline-predicate copy); swept every `*_not_vacuous`/`*_caught_red` x-test (also fixed X6 `item_2f`, confirmed x7/x11/x13 sound); X1③ leak-check is now an allow-list of platform sentinels (catches encoded leaks the substring scan missed) + an encoded-leak regression test. Mutation-verified RED→GREEN.
- fix(queue,policy,refstore,diag): FP-6 — `land_in_order` O(n²)→HashMap + `#[must_use]`; removed a hardcoded dev secrets path (→ `HUGIT_SECRETS_DIR` + skip); tightened the c7 fairness bound (fixture-derived, was vacuously loose); tested the policy fail-closed unregistered-gate branch; secrets-gate boundary tests; d1c p99 budget 500→2000ms (anti-flake); `denial_payload` JSON-shape pins; bounded the bisect `exact_serialized_size` loop.
- docs: FP-DOCS — README build-complete status (was "no code yet"); WP-E6.md → SUPERSEDED+BUILT with the correct `src/sync/` path; removed the no-op `HUGIT_QUEUE_AUTOTRIGGER` from the P2 runbook; CLAUDE.md crate count + fence-vs-profile wording; whitepaper status note; CHANGELOG `[0.1.0]` cut; X3③ reconciliation. docs(d2b): libgit2 honesty note (since upgraded to a REAL clone — see below).
- test(proto): close D2b item ③ libgit2 FOR REAL — new `clone_object_set_via_libgit2` performs a genuine libgit2 clone (the `git2` crate / vendored libgit2 C lib, a TEST-ONLY dev-dependency, never shipped in the product binary) of the served pack; the reconstructed object closure is asserted byte-identical to git's. Supersedes the earlier construction-equivalence deferral. Note: building the `hugit-proto` tests now requires `cmake` (present on CI).
- fix(contracts,ledger,cli): FP-7 — single-source the `"[REDACTED]"` redaction sentinel. New additive `hugit_contracts::REDACTED_MARKER` is the one source of truth; `hugit-ledger::redact::REDACTED` and `hugit-cli::export::redaction::REDACTED_TOKEN` now alias it so they cannot drift. No frozen type / serialization touched (golden serde tests green).
- build: FP-CONFIG — proprietary closed-source `LICENSE` + `[workspace.package]` (`license-file`, `publish = false`, `rust-version = "1.96"`); `[workspace.dependencies]` hoist with crypto exact-pins; `thiserror` unified 1→2; `deny.toml` (license allow-list, crates.io-only sources, version bans) + `cargo deny check` in CI; `.cargo/audit.toml` + `cargo audit --deny warnings`; CI actions SHA-pinned; `permissions: contents: read`; toolchain+cache steps.

## [0.1.0] — 2026-06-08
- fix(hygiene): hugit-cli — remove vacuous `let _ = account.is_terminating()` (no-op call, result discarded, zero behavioral effect; replace with explanatory comment; rename param to `_account` to signal intentional non-use); mark `fixture_event` with `#[doc(hidden)]` and a test-infrastructure note (it is `pub` only because integration tests require it through the public API, never called from production code). Behavior-preserving; all 30 tests green.
- fix(hygiene): hugit-app — fix typo `InsufficiendOrOutOfWindow` → `InsufficientOrOutOfWindow` in `exit/src/cohort.rs` doc comment; remove two needless closures in `src/webhook.rs` (`map(|s| s.to_string())` → `map(str::to_string)`, `and_then(|v| v.clone())` → `and_then(Clone::clone)`); rename `TokenRevoked { installation_id }` → `TokenRevoked { repo }` in `src/checks.rs` (field was populated with `request.repo`, not an installation ID — error message was misleading). Behavior-preserving; all gates green.

- fix(hygiene): hugit-runner — promote magic literals `"64m"` (tmpfs size), `"3600"` (idle-sleep ceiling), and `40` (census poll iterations) to named consts with explanatory doc comments (`TMPFS_SIZE`, `IDLE_SLEEP_SECS`, `CENSUS_POLL_ITERATIONS`); remove tautological `spawn_lt_1s_budget_duration` unit test (asserted `1000ms >= 1000ms` — trivially true, exercised nothing). Behavior-preserving; all gates green.
- fix(hygiene): hugit-diag — add missing `bisect/` entry to `lib.rs` module-layout doc comment (stale omission); remove vacuous `refusal_error` helper in `experiment/gate.rs` (identity wrapper that discarded the owned `EventRecord` parameter without using it; inlined `Err(…)` directly at each call site and changed `refuse` to return `()` since its event is already appended to the log). Behavior-preserving; all 25 tests green.
- fix(hygiene): hugit-checks — replace `BTreeMap<&str, ()>` set idiom with `BTreeSet` in `byte_identity::compare_byte_identity`; remove dead `verdict: &VerdictObject` param from private `Gate::land` (verified upstream, suppressed via `let _ = verdict`); lift duplicated `which_tool` helper from `cargo_lock` into `driver::mod` as `pub(super)` and call it from both `CargoLockDriver` and `PnpmLockDriver`; clarify `store_body` error-mapping comment (serialisation error mapped to `AcError::Decode`). Behavior-preserving; all 64 tests green.
- fix(hygiene): hugit-invariants — complete WP layout doc comment in lib.rs (was missing x3, x7, x9, x11, x14); add blank-line separators between all WP sections in lib.rs for consistent visual scanning; remove dead `label: String` field + `#[allow(dead_code)]` from `ProbeResult` in x6 and x10 (field was set but never read anywhere). Behavior-preserving; all gates green.

- fix(hygiene): hugit-refstore — replace raw `format!` JSON string-building in `denial_payload` with `serde_json::json!` (consistent with every other payload site in the crate; eliminates divergence hazard for future value changes)

- fix(hygiene): hugit-queue — promote magic JWT window constants (`IAT_BACKDATE_SECS`, `TOKEN_LIFETIME_SECS`, `GITHUB_MAX_JWT_WINDOW_SECS`) in `app_auth`; add `P95_PERCENTILE` const for the 0.95 fraction in `budget::p95_wait_ticks`; fix doc/attribute ordering in `negative_scope::paths` (second `///` block was after `#[allow(dead_code)]`); remove redundant "ordered," prefix in `batch` module doc. Behavior-preserving; all gates green.
- fix(hygiene): hugit-proto — misplaced doc comment on `line_data` (described `decode_lines`, not the function it sat on); `std::mem::forget(dst)` in `push_core::clone_and_head` leaked scratch temp dir on every call (should be `drop`); `git_env` in `push_core` returned `String` but all callers discarded the value (return `()`, matching sister module); `REF_UPDATE_KIND`/`REF_DELETE_KIND` duplicated in `write::store` independently of `write::external` (same values but drift-hazard; `store` now re-imports the canonical constants from `external`, and `RAW_PUSH_KINDS` usage scoped to the test that needs it). Behavior-preserving; all gates green.
- fix(hygiene): hugit-mirror — remove vacuous `_now_ms` param from `BadgeState::unknown_api_down` (infinite-staleness path never needs a clock; callers updated); remove vacuous `_trigger: FallbackTrigger` param from `detect_via_poll` (single-variant enum, detection timing is cadence-only; `FallbackTrigger` removed entirely); add `SAFETY` comment on `unsafe set_var` in outbound test. Behavior-preserving; all gates green.

- fix(hygiene): hugit-contracts — add missing module doc descriptions to runner_lease/shadow_policy/verdict_object; drop spurious `pub` from test-only pin constants and fixture helper in integration_tests.rs
- fix(hygiene): hugit-ledger — eliminate vacuous `Option` return from `EventClass::classify` (return `Self` directly; call-site `.unwrap_or` was dead); de-duplicate `REDACTED` constant by re-exporting `redact::REDACTED` from `ledger` instead of re-declaring it (drift hazard, single source of truth). Behavior-preserving; all gates green.
- fix(hygiene): hugit-policy — stale doc param names (`old_gate_json`/`new_gate_json` → `old_gates_json`/`new_gates_json` in `emit_policy_change` doc-comment); needless `.clone()` on `this_hash` in `emit_policy_change` (original unused after struct construction); needless `.clone()` on CHANGELOG content in `changelog::eval` (`&String` passed directly as `&str`). All behavior-preserving; gates green.
- fix(hygiene): hugit-fence — deduplicate `shell_quote`/`base64_encode` into `crate::util`; rename `normalize_segments_pub` → `normalize_path` (removes `_pub` code smell); document `BLOCK` magic constant in HMAC (SHA-256 block size). Behavior-preserving; 53 tests green.

- fix(checks): strengthen ac-loader test to prove HTTP is attempted (reject Ok stub) (re-review MED). `all_present_yields_configured_client_that_attempts_http` now points the configured client at `http://127.0.0.1:0` (guaranteed unreachable) and asserts `Err(Transport(_))`; any `Ok(_)` arm (including `Ok(None)` from a stub) now panics, proving the transport was never called. A stub `lookup()` returning `Ok(None)` previously passed; it now fails the test. No production code changed.

- fix(dogfood,invariants): real measured B8 baseline + single-source the focus-gate allowlist (re-review HIGH). (1) WP-B8 item ② baseline was a GAMED oracle: `baseline_exec_ms` was a STATIC product (`landed.len() × entries.len() × 10ms`) that ignored the wave's real executions, and the oracle only asserted `> 0` (a constant would pass). The baseline is now a REAL memoization-OFF run — the wave accumulates the SUM of measured `CheckResult.duration_ms` over checks that actually executed (AC hits add 0) into `WaveReport::measured_exec_ms`, and `baseline_exec_ms` is read from it. The oracle is strengthened: it asserts the memo-OFF pass executed ALL checks (`baseline_local_executions == a cold-AC wave's executions`), `baseline_exec_ms` equals the MEASURED cold-wave time, the memoized pass executes STRICTLY FEWER (0) — the wedge — and `memoized_exec_ms == 0`. Mutation-verified: the old static-formula baseline (250) ≠ measured (50) turns the oracle RED. Versioned report + formulas preserved. (2) WP-X10 carried its OWN divergent `DOGFOOD_TARGET_ALLOWLIST` (synthetic-fleet-a/b) + `assert_dogfood_enrollment_allowed`, diverging from the PRODUCTION gate `hugit_dogfood::focus_gate` (synthetic-fleet-alpha/beta) and tested its own copy — so item ② was verified against the wrong list and the two could silently desync. X10 now CONSUMES the production gate single-source: `hugit-dogfood` is a dependency of `hugit-invariants`, X10 re-exports `DOGFOOD_TARGET_ALLOWLIST`/`CORELINK_SERVER_EXCLUDED`/`assert_excluded` from `focus_gate`, and the divergent copy is DELETED. No dependency cycle (hugit-dogfood depends only on contracts/queue/checks/refstore). Mutation-verified: weakening the PRODUCTION gate to accept corelink-server turns the X10② oracle RED (proving it tests the real gate). (re-review HIGH)
- fix(mirror): make bidir item ⑤ no-symmetric-authority property non-vacuous + real RED-guard (re-review HIGH). The rule-5 oracle was a tautology: `AuthorityModel::authority_for` hard-coded `Forge` for the protected branch, so `is_symmetric_for_main` (which read main's authority through it) could NEVER be true — the property test `item_5_no_symmetric_authority_property` always passed trivially and the RED-guard only re-checked the hardcoded logic, so neither could catch a GitHub-authoritative-for-`main` setter. Fix: authority for `main` is now REAL stored state derived from the engine's transitions (seeded `Forge` by `new`, only ever rewritten by `arbitrate`, which always writes `Forge`; reroute/ingest never name `main`) — `authority_for` reports the stored value verbatim (no `Forge` short-circuit), and `is_symmetric_for_main` ⇔ main's stored authority == `GitHub`, a genuine invariant over actual state. The forbidden state is now REPRESENTABLE via a test-only `set_github_authoritative_for_main` (exposed to the `acceptance_bidir` integration test through a `test-internals` feature on a self dev-dependency; never in a production build). RED-guard (`item_5_mutation_injected_symmetry_is_caught_red`) CONSTRUCTS main→GitHub and proves `is_symmetric_for_main` returns true / the oracle goes RED; the property sweep now asserts main never becomes GitHub-authoritative over real interleavings. Mutation-verified: reinstating the `Forge` hardcode in `authority_for` turns the RED-guard RED (`left: Forge, right: GitHub`), GREEN on the real engine. Items ①②③④ unchanged and green. (fix/bidir-item5-vacuous)

- feat(checks): CoreLink AC config loader + gated live smoke test (P2 plug-and-play). A loader (`corelink_ac_from_env`) reads `HUGIT_CORELINK_AC_URL` + `HUGIT_CORELINK_TENANT` + the PAT from `~/.hugit/secrets/corelink/pat` (override `HUGIT_CORELINK_PAT_FILE`, env fallback `HUGIT_CORELINK_PAT`) and builds the live `HttpAcClient`, fail-closed with a `NotConfigured` naming the missing piece (PAT never rendered). The gated smoke test (`tests/corelink_ac_smoke.rs`) runs the handoff §6 three probes (miss-404, round-trip hit, cross-tenant 403) against the live endpoint only when all three are present — clean skip with a printed reason otherwise, so it cannot rot to green before the CoreLink tenant exists. Hermetic loader tests cover all-present→configured, each-missing→NotConfigured, file-over-env precedence, and PAT-never-rendered. See `docs/handoff/2026-06-08-corelink-p2-tenant-request.md`. (P2-plug)

- feat(mirror): seamless forge-arbitrated bidirectional sync (supersedes E6). New `crates/hugit-mirror/src/sync/` — a model that feels like instant two-way GitHub ↔ hugit sync to the user yet keeps the FORGE as the single source of truth (never "naive symmetric sync"). `engine` is `BidirSync`, an append-only `hugit_refstore::EventLog` whose refs are the replay-derived view; every forge mutation goes through exactly one of two appends, and which one is reachable is what makes `main` single-writer BY CONSTRUCTION: the GitHub-ingest path uses ONLY `hugit_proto::record_external_change` (D3⑤: a raw push is an opaque, attributed `ref.update` external-change event, NEVER a fabricated intent) and CANNOT name the protected branch (a direct-`main` push is rerouted onto `refs/hugit/proposed/…`), while `land_via_queue` (the B4 landing queue, the sole writer to `main`) is the ONLY path that emits `intent.landed` to advance `main`. ① branches round-trip both ways byte-identically (GitHub branch → ingested as that forge ref via a change-event, `main` untouched; forge branch → mirrored out); ② convergence is content-hash idempotent (reuse E1's "mirror_write from observed mutation, not tip-inequality") so an echo re-emits nothing; ③ a direct GitHub push to `main` is rerouted (proposed branch), never lands symmetrically, and `main` advances only via the queue; ④ same-branch concurrent divergence is arbitrated forge-authoritative (reusing `crate::divergence::resolve`) with the divergent GitHub tip PRESERVED as a recoverable incident side-ref (`refs/hugit/incidents/…`) on the chain-verifiable log — never silently dropped; ⑤ no reachable state has both sides authoritative for `main` (structural `AuthorityModel` property test). Oracle-first (`tests/acceptance_bidir.rs`, 10 tests): items ①–⑤ proven RED→GREEN against the REAL forge surfaces (canonical `EventLog`/`verify_chain`/`replay` + `record_external_change` + the E1 divergence machinery) with a REAL local-git "GitHub side" fixture (real objects/oids, e2a-style), and each oracle MUTATION-VERIFIED load-bearing (corrupt-tip → ① RED, drop echo guard → ② RED, symmetric main write → ③ RED, silent incident drop → ④ RED, GitHub-authoritative arbiter → ⑤ RED). P2 seam: the LIVE GitHub change-DETECT (webhook/poll) is `sync::detect` — gated behind `HUGIT_GH_TEST_REPO`, run-not-skip when set, asserted `NotWired` in the bare gate so it can't rot to a fake green; ALL arbitration/convergence/single-writer LOGIC is proven hermetically now. Consumes hugit-proto/hugit-refstore + the E1 divergence/refops surfaces read-only; modifies no other crate. (WP-bidir-sync)
- feat(diag): B5 — auto-bisect over memoized checks + the bounded `DiagnosisObject` (`crates/hugit-diag/src/bisect/`; consumes B2's memoized-check `ActionCache` and the frozen `hugit_contracts::DiagnosisObject` read-only). Memoization makes bisect ≈ free (whitepaper §5.1), so culprit-finding is a default that fires on every red, not a manual chore. (1) `oracle` — the `CheckOracle` abstraction the search runs over and `MemoizedCheckOracle`, its production binding onto the B2 Action Cache: every probe is ONE AC lookup, a HIT yields the verdict with ZERO real check execution (the wedge), a MISS counts as a real execution and is treated red fail-closed so a partially-memoized history never reports a fake free hit; probes and real executions are counted honestly. (2) `engine` — the `Bisector`: leftmost-red binary search treating index 0 as the known-green baseline (like `git bisect`'s good ref, never probed) and locating the culprit (first red tree) in `≤ ⌈log₂ n⌉` probes (①, proven across a size sweep 2..1000; a linear scan would break the bound). `diagnose`/`try_diagnose_with_suspects` assemble the bounded `DiagnosisObject` — `culprit_ref`, a `diff_vs_green_ref` spanning last-green→culprit, deduplicated `suspect_targets` reachable from the culprit's change, and the `bisect_path` probe trail (②) — carrying CAS refs to logs (the culprit `CheckResult`'s `stdout_ref`/`stderr_ref`), NEVER inlined raw logs; `size_bytes` is the EXACT serialized size (computed via a self-referential fixpoint) and is held `≤ DIAGNOSIS_SIZE_BOUND` (8 KiB), with the assembler refusing an over-bound (raw-log-dump) payload fail-closed as `DiagnosisTooLarge` (④). (3) `trigger` — the auto-trigger: `on_red_signal(oracle, &RedSignal)` is the SOLE entry point; a `RedSignal` is only constructible from a queue signal (`from_red_tip`/`from_queue`), a green tip or empty history yields `Ok(None)` (no fabricated diagnosis), and there is no manual `bisect()` reachable outside the red-signal path (⑤). Oracle-first (`tests/acceptance_b5.rs`, 9 tests): owned items ①–⑤ RED→GREEN on the real surface against the in-process B2 `InMemoryAc`, plus adversarial guards (a linear scan would exceed the bound; an inlined-log diagnosis would exceed the size bound) so the oracle is NOT gamed; the <2min fixture (③) runs end-to-end on a 1024-deep memoized history in ≤10 probes / 0 real executions. P2 DEFERRED + documented: the live `hugit_contracts::QueueApi` UNION-FAIL event subscription that resolves a batch into the in-process `History` (gated behind `HUGIT_QUEUE_AUTOTRIGGER`; the same prod-AC-tenant deferral as B2's `HttpAcClient`) — only the live event wiring is deferred; the auto-INVOCATION semantics are proven hermetically NOW. No new external dependencies (hugit-checks was already in the workspace lock). (WP-B5)
- feat(dogfood): B8 — dogfood harness. New `crates/hugit-dogfood` drives a real in-process 5-PR wave end-to-end through the actual union-landing queue + memoized-checks + event-log surfaces (item ①), runs the SAME wave with memoization OFF as a baseline and emits a VERSIONED report with the formulas (item ②, measured not fabricated), and proves the soak invariant harness (0 wrong-merge / 0 lost-PR, event-audited) over a compressed deterministic run (item ③). Dogfood targets exclude corelink-server (non-interference, cf X10②). P2 seam: the real 48h wall-clock soak + live GitHub install gated behind env, run-not-skip when set. Oracle-first; RED on a wrong-merge, lost PR, or un-versioned report. (WP-B8)
- fix(hygiene): polish pass — corrected misleading comments (app/exit `current_epoch_ms` was labelled a no-std "stub" but is a complete std impl; refstore `NoMirror` doc now notes E1/hugit-mirror is built), promoted a dead-code `_assert_all_subtypes_importable` helper in contracts golden tests to a real `#[test]` so it actually runs, changed the always-`true` `roundtrip` helper return to `()`, exported fence `check_access`/`is_admitted` alongside the `FenceViolation` they construct (no more orphaned result type), and removed vacuous field-presence checks in `FleetState::validate` (non-Optional fields are always serialized; the only real invariant — non-empty `schema_version` — is kept). Behavior-preserving; full --workspace --locked green. (hygiene-pass-1)
- feat(invariants): X11 — degradation composition. New `crates/hugit-invariants/x11/`: a `DegradableSmartLayer` with a `FaultPoint::MidOperation` injection proves the three composed invariants across one partial-degradation window — ① the secrets broker fails CLOSED mid-operation (delivers nothing; no credential-on-runner fallback; raw credential byte-absent from the workspace, with a healthy positive control that DOES deliver a public result), ② objects written during degradation are marked provenance-ABSENT with no synthetic intent/attestation fabricated, ③ the X10 CoreLink non-interference invariant still holds while degraded. Oracle-first + mutation-verified: red-guard tests (`item_1_a_leaked_credential_would_be_caught_red`, `item_2_a_fabricated_provenance_would_be_caught_red`) confirm the oracle is load-bearing. P2 seam: live mid-op fault injection on the runner box (`HUGIT_RUNNER_HOST`), run-not-skip when set. (WP-X11)
- feat(checks): wire B2a `HttpAcClient` to the CoreLink Action-Cache REST contract (P2 transport ready). The live AC client now speaks CoreLink's surface — `{GET,PUT} {base}/v1/ac/{tenant}/{action_digest}`, `Authorization: Bearer <PAT>` — behind a mockable transport trait so the only deferred piece is the actual network call. Hermetically tested (`tests/ac_http.rs`, 12 tests with a recording mock transport): request construction (route, Bearer header present WITHOUT leaking the secret value, octet-stream PUT body), response mapping (200→hit / 404→miss / 401/403/5xx→explicit error), the content-address guard (a 200 whose record keys to a DIFFERENT action_digest is rejected — never a blind hit), and fail-CLOSED when unconfigured (`new()` with no PAT/base → `NotWired`, can't rot to green). `ureq` added to hugit-checks (already in the workspace lock via hugit-queue — no new transitive surface). The live network call remains the P2 seam (needs the CoreLink prod AC tenant + PAT). (B2a-ac-seam)

- feat(checks): B2b — runner-side execution + byte-identity + non-determinism + honest hit-rate (the execution half of the same pure check function B2a's client memoizes; whitepaper §6.2). New `crates/hugit-checks/src/runner/` (consumes B2a's FROZEN `client/` memo-key surface, never modifies it): (1) `lease_exec` — the `RunnerExecutor` trait (runner analogue of B2a's `CheckRunner`) executes a `CheckDef` under a HELD `RunnerLease` (fail-closed: an expired/released/crashed lease refuses execution) and produces a real `CheckResult` whose output artifacts carry canonical lowercase-hex SHA-256 content digests; `InProcessRunnerExecutor` is the in-process REFERENCE executor (artifacts are a pure function of the three memo axes → byte-identical across runners; an injectable `EntropySource` models a non-hermetic check), and `LiveBoxRunnerExecutor` is the explicitly-marked P2 seam over `HUGIT_RUNNER_HOST` (returns `BoxNotWired{host}` until the runner box is provisioned by Phase-C/C2). (2) `byte_identity` — `compare_byte_identity` compares LOCAL vs RUNNER results by ARTIFACT CONTENT DIGEST (and the three axes + memo key + exit), NOT result-equal: a tampered artifact digest with a matching exit code is caught as a `DigestMismatch` (result-equal would pass). (3) `nondeterminism` — `NonDeterminismTracker` records the per-`memo_key` produced-artifact fingerprint across runs and flags `DeterminismState::NonDeterministic` after exactly 3 divergent runs (`NON_DETERMINISM_THRESHOLD=3`), surfaced honestly (`Deterministic`→`Diverging`→`NonDeterministic`); a flagged key is never a clean memoized hit, and a deterministic check run 3× is never falsely flagged. (4) `hit_rate` — `HitRateMeter`/`HitRateReport` measure the ACTUAL AC hit-rate over a lookup sequence and display it AS-IS with a measured shape label (`FULL`/`PARTIAL`/`NONE`/`NO DATA`); never a fabricated full-memo claim. Oracle-first (`tests/acceptance_b2b.rs`, 5 tests): owned items ③(local≡runner byte-identical via artifact/digest compare, with a result-equal-is-too-weak cross-check) ④(non-determinism flagged after 3 divergent runs + deterministic negative control) ⑤(npm fixture: measured PARTIAL 37.5% hit-rate displayed as-is, no full-memo claim) proven GREEN on the real surface; lease fail-closed proven; the live-box P2 seam asserted absent in the bare gate (cannot rot) and run-not-skip when `HUGIT_RUNNER_HOST` is set. Mutation-checked load-bearing (threshold 3→2 turns item ④ RED). P2 DEFERRED: only the live runner-box transport (needs the Phase-C runner box). (WP-B2b)
- feat(invariants): X9 — cross-phase object identity invariant (new `crates/hugit-invariants/x9/`, no production crate paths; consumes B2 memoization, D7 verdict panels, B6 sidecar, D4 native intents read-only). ① A `CheckResult` memoized by phase-B is BIT-IDENTICAL to the one served as evidence in a phase-D verdict panel for the same `(tree,def,toolchain)`: both phases serve through one canonical serializer (`canonical_bytes`, routed through the production `hugit_refstore::canonical_json`) so the bytes — and the `content_id` derived from those exact bytes via `hugit_refstore::compute_this_hash` — agree byte-for-byte; identity is a BYTE compare, not value-equality. ② A mismatch fails CLOSED + alerts: `serve_evidence` re-derives the canonical bytes phase-D would serve and refuses (`IdentityError::EvidenceMismatch`, carrying a stable-coded `Alert`) on any divergence, including a subtle non-key-field divergence (a tampered artifact digest), and refuses to invent evidence with no phase-B memo behind it (`NoMemo`). ③ intent_id identity (the R10 addition): the id minted by a phase-B `IntentSidecar` is IDENTICAL and NON-COLLIDING with the native phase-D intent (`VerdictObject.intent`) for the same logical intent — one lifecycle, one id; `reconcile_intent` fails CLOSED on divergence (a second id minted in phase-D) and `IntentRegistry::bind` fails CLOSED on collision (two distinct logical intents folding onto one id), while an idempotent re-bind of the SAME lifecycle is allowed (identity ≠ resolution — X14 covers deep-link resolution). Oracle-first: 10 acceptance tests over the frozen `hugit_contracts::{CheckResult,IntentSidecar,VerdictObject}` + the canonical refstore surface (`canonical_json`/`compute_memo_key`/`compute_this_hash`), with gamed-oracle guards (canonical bytes stable across authoring, subtle divergence still fails closed, idempotent re-bind not a collision) — RED on a re-serialization divergence or an admitted collision, never a gamed oracle. No new dependencies; modifies no production crate (WP-X9)
- feat(invariants): X10 — focus gate: adjacent-product boundary + shared API-tenancy. ② corelink-server enrollment FAILS THE BUILD (compile-time `const` assertion; `CORELINK_SERVER_EXCLUDED` evaluated at const-eval time; any addition to `DOGFOOD_TARGET_ALLOWLIST` causes `const _: () = assert!(CORELINK_SERVER_EXCLUDED, …)` to fire before link). ③ X6 rescoped to intra-hugit isolation; X10 owns the adjacent-product boundary — scope-separation asserted structurally (oracle goes RED if scopes merge). ⑤ Preventive bound: `HUGIT_CORELINK_TENANT_CAP` (100 RPS / $10/day / 200-burst, enforced=true, precedes_load_test=true) is the PRECONDITION for ④ — asserted before every live lane execution. ①④ pre-committed `X10_TOLERANCE` (≤2% p50 latency / ≤0.05 pp availability / 1% abort threshold stated before any load) and `FLEET_SCALE_WORKLOAD` spec (4 ramp steps, CAS+AC+R2 workload within cap) — live measurement gated behind `HUGIT_CORELINK_PROD_URL` (item ①) and `HUGIT_X10_LIVE` (item ④ fleet-scale write-storm); FAIL-not-skip when env set but endpoint unreachable (PARTIAL-over-fake is law). P2-deferred seam documented: real CAS/AC/R2 tenant credentials absent until P2 provisioned; both SEALs re-measure ①④ after P2. 13 acceptance tests all GREEN: 4 item ② (build guard + enrollment gate + positive + unknown-rejected), 2 item ③ (scope separation + vacuity guard), 3 item ④ (workload-within-cap structural + live + workload-overflow detected), 4 item ⑤ (cap valid/enforced + unenforced detected + late detected + workload-exceeds detected). Zero writes outside `crates/hugit-invariants/x10/`. (WP-X10)
- feat(invariants): X14 — deep-link referential integrity (lifecycle): new `crates/hugit-invariants/x14/`, no production crate paths. Proves deep-link referential integrity across the FULL object lifecycle. ① Property test: drives ledger/intent deep links through compaction/cold-tier to R2 (D1), mirror round-trip (E1), and tombstoning (X7) — asserts every link resolves to its TARGET or a tamper-evident TOMBSTONE at EVERY stage, never to a dangling void. Compaction is proven to operate on the event log (not the object store), so CAS link targets survive log prefix offload; mirror round-trip preserves content-addressed hashes by the CAS guarantee; erasure installs a tombstone keyed by the SAME content hash so the link keeps resolving. The REAL `hugit_refstore::{compact, recover_from_cold, verify_chain}` and `hugit_ledger::resolve` surfaces are exercised, never stand-ins. ② Standing fixture: `check_integrity` checks ALL tracked links in one pass and returns a report whose `zero_dangling()` must be true continuously; zero dangling is the standing invariant — a single dangling link turns it false. Oracle-first: 15 acceptance tests with adversarial guards — `adversarial_dangling_link_is_detected`, `adversarial_standing_fixture_detects_dangling_link`, and `adversarial_lifecycle_property_fails_on_dangling_link` all go RED on a broken store, GREEN on a correct one; the oracle is NOT gamed. Consumes D1/E1/D5/X7 read-only; modifies none (WP-X14)
- feat(invariants): X1 — tenant-isolation RED-TEAM invariant (new `crates/hugit-invariants/x1/`, no production crate paths). Proves the CoreLink-inherited tenant boundary (whitepaper §9, HMAC-derived prefixes) holds for hugit, fully hermetic and p2-independent. The boundary under test is modelled as a tenant-scoped memo namespace: a private entry's physical storage key is `lower_hex(HMAC-SHA256(tenant_secret, base_memo_key))`, where the base key comes from the single-source `hugit_refstore::compute_memo_key` (never re-transcribed) and each tenant holds a distinct private HMAC secret. ① a cross-tenant private lookup MISSES/denies, never served — tenant B derives its own namespaced key from its own secret and physically cannot name tenant A's slot, so A's private bytes are never returned to B (positive control: A still resolves its own entry). ② forged + collision memo-key writes are denied fail-closed with an ALERT audit event and ZERO poisoning — a claimed namespaced key that does not match the tenant's HMAC derivation is a forgery, and a collision write of DIFFERENT bytes to an occupied slot is a poison attempt; both refuse, the existing bytes are read back byte-for-byte unchanged, the store grew by zero entries, and an idempotent re-admit of identical bytes still succeeds (the guard is not vacuous). ③ a public-deterministic artifact IS shared cross-tenant from a single global slot any tenant resolves identically, with PROOF no private bytes rode along: the served attestation is the platform-anonymized form of the frozen `hugit_contracts::AttestationChain` — public-deterministic links (tree/def/model) preserved, tenant-identifying links (runner/principal) replaced by platform sentinels, signature cleared — and `carries_none_of` proves none of A's private runner/principal/sig bytes survive. ④ private artifacts emit NO cross-tenant side-channel: a private miss and a genuine absent return the identical `Miss` shape and identical audit-event shape (no "exists-but-forbidden" verdict), the lookup does the same work in both cases (one HMAC + one map probe — asserted within a latency class), and the existence signal inherent to PUBLIC-deterministic sharing is documented as BY-DESIGN disclosure (only the shared registry discloses; the private store never does). Oracle-first: 4 acceptance tests (one VERBATIM per owned item) over a real in-process CAS/AC surface — confirmed RED when isolation is bypassed (a tenant-independent namespace serves A's bytes to B and collapses the side-channel; items ①②④ go RED), GREEN on the real surface; never a gamed oracle. Consumes the canonical memo-key + frozen AttestationChain read-only; modifies neither. hmac/sha2/hex are already in the workspace lockfile (no new supply-chain surface) (WP-X1)

- feat(invariants): X7 — right-to-erasure CASCADE (new `crates/hugit-invariants/x7/`, no production crate paths; consumes the five stores' surfaces read-only). Proves a data subject's personal data is provably erased across EVERY store with no orphaned provenance refs, attestation chains that re-seal OR fail CLOSED, and the erasure×seal precedence resolved. ① FIVE-STORE cascade: one right-to-erasure request sweeps CAS (live object → tamper-evident `Tombstone` keyed by the SAME content hash), provenance/ledger (the append-only `EventLog` is never rewritten — the canonical `hugit_refstore::verify_chain` still passes byte-for-byte), the context store (bytes genuinely PURGED, not tombstoned-with-bytes), the GitHub mirror (a `MirrorObligation` discharged-or-residual-risk-disclosed — the documented P2 seam, identical to X12②), and the experiment corpus (the sealed datapoint removed); each leg exposes an ABSENCE scan (re-read the store, never a flag) folded into `scan_cascade_absence` → `CascadeAbsenceScan::fully_erased`. ② NO ORPHANS: `scan_orphans` walks every provenance link and an erased object must resolve to a live target or a tombstone, never a void; a void-delete surfaces a detected `OrphanRef`, and a silent re-link is caught fail-closed via the canonical hasher (`ThisHashMismatch`, and re-sealing to hide it propagates `PrevHashMismatch` downstream). ③ ATTESTATION re-seal OR fail CLOSED: after erasure the frozen `hugit_contracts::AttestationChain` is re-pointed at the post-erasure (tombstone-bearing) manifest AND RE-SIGNED with REAL ed25519 over the single-source `hugit_refstore::attestation_sig_preimage` (`reseal_attestation` → `verify_attestation` = Ok); a stale seal (link changed, sig not refreshed) is rejected fail-closed (`SignatureMismatch`), an unsigned chain rejected (`Unsigned`), and an attacker-signed re-seal does not verify against the producer key — there is NO silently-broken-but-accepted outcome. ④ ERASURE × SEAL precedence (the R10 X7/D8⑤ contradiction resolution): `SealedCorpus::erase_datapoint` (a) PERMITS lawful erasure of a SEALED datapoint (the seal does not block it; erasure wins), (b) INVALIDATES the gate verdict fail-closed — the `hugit_contracts::RegenGate` is re-pinned `repass=false`/`indep_verdict=""` (claims/regen re-pin advisory/OFF/blocked until re-evaluation), and (c) RECORDS the invalidation as a serialisable, audited `GateInvalidation` naming the erased datapoint + request — never blocked by the seal, never silent. Oracle-first (`tests/acceptance_x7.rs`, 14 acceptance tests over real fixtures + the canonical refstore hash/verify/preimage surface + real ed25519): owned items ①–④ RED→GREEN with adversarial guards that go RED on an incomplete cascade, an empty mirror disclosure, a void/orphan, a silent re-link, a stale/forged seal, or a silently-kept passing verdict — PARTIAL-over-fake, never a gamed oracle. CLOSES the X3③ context-store-purge PARTIAL (X7 supplies the cross-store cascade that proof was awaiting). P2-deferred seam: live GitHub-mirror erasure + live experiment-corpus erasure are modelled as the documented obligation (as in X12), not driven against live infra (WP-X7)
- feat(invariants): X6 — resource non-interference: infra isolation config-asserted (separate Hetzner project, SSH key, quota cap), structural resource-accounting model proves hugit load bounded within its own quota (cannot draw on CoreLink), live latency/availability measurement gated behind HUGIT_RUNNER_HOST+HUGIT_CORELINK_PROBE_URL (both SEALs). P2-deferred seam documented: full CAS/AC/R2 write-storm measurement awaits CoreLink tenant provisioning (WP-X6)
- feat(invariants): X8 — self-release attestation: the supply-chain invariant (X4) turned on hugit itself (new `crates/hugit-invariants/x8/`, no production crate paths; consumes the frozen `AttestationChain` form + the contract-frozen canonical signature preimage `hugit_refstore::attestation_sig_preimage` read-only, modifies neither). ① every hugit App/CLI/runner-image release is SIGNED with a real ed25519 key over the frozen canonical preimage (never re-transcribed) AND PUBLISHED to an append-only `TransparencyLog` whose entries are independently verifiable FROM THE LOG ALONE (holding only the entry + the release public key); the log is tamper-evident — a post-publication digest tamper breaks verification, proven RED. ② the running App verifies its OWN provenance AT BOOT via `boot_self_verify`: it locates the published entry for exactly the build that is running (matched by content digest) and verifies the signature against the trusted release public key BEFORE serving — the check GATES serving (`BootOutcome::Serving` reachable only on success) and is not skippable. ③ an UNSIGNED (unpublished/empty-sig), TAMPERED (digest-mismatched), or FOREIGN-KEY-signed self-build makes boot FAIL CLOSED — the App does not serve, with an audit event (`admitted=false` + machine-readable reason); the self-turned form of X4③. Oracle-first: 7 hermetic acceptance tests (no network, no env) over real ed25519 signatures; the fail-closed items are proven LOAD-BEARING — gaming the boot gate to always serve turns all three RED (mutation-verified, never a gamed oracle). P2 seam: the LIVE public transparency log (e.g. Rekor) is the documented next impl behind the SAME `TransparencyLog` trait — a Rekor-backed impl makes publication globally verifiable without changing the boot path or the oracle (WP-X8)
- feat(invariants): X13 — legibility × degradation/erasure composition invariant (new `crates/hugit-invariants/x13/`, no production crate paths). Proves the human can ALWAYS follow, in every substrate state, composing the human-facing down-zoom — the raw-commit view (D2, plain git), deep links (D5, `hugit_ledger::deeplink::resolve`) and `why` (D10, `hugit_cli::why::resolver::resolve_why`) — with the two adverse substrate states. ① With the intelligence layer DEGRADED the down-zoom still resolves via plain git OR fails HONESTLY with an explicit `DownZoom::LayerUnavailable` — never a silent 404/blank: the raw-commit leg is served by the GIT substrate and resolves identically whether the smart layer is healthy or degraded (whitepaper §9 lock 5, "a valid git repo keeps serving"), while the deep-link and `why` legs return an explicit "layer unavailable" the human is TOLD about; the `DownZoom` enum has no "blank" variant and `is_honest()` rejects any success carrying an empty object/summary, so a silent blank cannot pass. ② After an erasure cascade following ANY chain (a deep-link chain or an attestation/provenance chain) reaches a live target OR an honest tamper-evident `Tombstone` carrying the original object hash — never a dangling `Follow::Broken` link; erasure operates on the object store only (idempotent, never a void), so the canonical `hugit_refstore::verify_chain` still verifies byte-for-byte and `human_can_always_follow()` holds at every hop. Oracle-first: 11 acceptance tests over real fixtures + the REAL canonical deep-link/`why`/`verify_chain`/`EventLog` surfaces, with adversarial guards that go RED on a silent blank (`item_1_a_silent_blank_would_be_caught_red`) or a broken/dangling link (`item_2_a_broken_link_would_be_caught_red`) — never a gamed oracle. Consumes D2/D5/D10/X7 + the canonical hash-chain read-only; modifies none (WP-X13)
- feat(invariants): X12 — erasure × provenance × mirror composition invariant (new `crates/hugit-invariants/x12/`, no production crate paths). Proves the three-way intersection of the erasure cascade (X7), the attestation/provenance chain (X2/refstore), and the GitHub mirror (E1) tied into the export/exit proof (E5). ① After an erasure request the attestation chain stays INDEPENDENTLY verifiable with the erased object as a tamper-evident TOMBSTONE, never silently re-linked: erasure operates on the object store only, so the surviving provenance link keeps referencing the SAME content hash (now resolving to a `Tombstone` carrying that exact hash) and the canonical `hugit_refstore::verify_chain` still passes byte-for-byte. A silent re-link is caught fail-closed — the payload swap trips `ThisHashMismatch`, and re-sealing the record to hide it propagates a `PrevHashMismatch` downstream (proven: there is NO re-link that survives `verify_chain`); an erased object never resolves to a void/orphan. ② The mirror-side erasure obligation (data already replicated to GitHub) is modelled as a `MirrorObligation` that resolves to either `Discharged` (mirror-side erasure executed + verified) OR `ResidualRisk` (an honest, non-empty disclosure), and the `ExitProof` (a frozen `hugit_contracts::ExportSchema` envelope) validates only when the disclosure is a STATED element of the export/exit proof — an obligation whose `mirror_erasure_obligation` class is absent from `ExportSchema.object_classes`, or whose residual disclosure is empty, is rejected fail-closed. Oracle-first: 9 acceptance tests over real fixtures + the canonical refstore hash/verify surface (`compute_this_hash`/`verify_chain`/`EventLog`), with adversarial guards that go RED on a silent re-link or an omitted/empty disclosure — never a gamed oracle. Consumes X7/E1/E5 + the canonical attestation surface read-only; modifies none (WP-X12)
- feat(checks): B2a — checks-as-code CLIENT + local memoized executor + three-axis memo key (the memoized-CI wedge: "your green checks never re-run"). New `crates/hugit-checks/src/client/`: (1) `memo_key` derives the three axes `H(tree_root ‖ def_digest ‖ toolchain_digest)` — the `tree_root` is a length-prefixed SHA-256 Merkle hash over ONLY the check's input subtree (files matching `CheckDef::glob_set`, sorted), the `def_digest` is a canonical LP/VEC-framed digest over every load-bearing definition field (command, inputs, toolchain_ref, env_manifest, glob_set), and the FINAL key is single-sourced via `hugit_refstore::compute_memo_key` (never re-transcribed); (2) `glob` is a dependency-free, fully-tested glob matcher (`*`/`**`/`?`, `**/` matches zero dirs) that scopes the tree axis; (3) `ac` defines the `ActionCache` trait with an in-process reference cache (`InMemoryAc`, content-keyed store→hit) and a thin live `HttpAcClient` seam over CoreLink's `GET/PUT /v1/ac/{memo_key}` REST surface (P2: transport bodies return `NotWired{endpoint}`, the only deferred piece); (4) `executor` is `hugit check --local` — derive key → AC lookup → HIT returns the memoized `CheckResult` with ZERO local executions (structural: the `CheckRunner` is never touched on a hit) | MISS executes once + stores. Oracle-first (`tests/acceptance_b2a.rs`, 9 tests): owned items ①(repeat→AC hit, 0 exec, <500ms) ②(glob in→rerun/out→hit) ⑥(toolchain MISS, never false hit) ⑦(changed def MISS + re-execute) proven GREEN on the real surface against the in-process AC + a counting runner; a cross-axis no-false-hit property proves each of the three axes is necessary AND sufficient; the parser rejects forged `def_digest`/empty fields/unknown fields fail-closed; the HTTP seam's deferral is asserted (cannot silently rot to green). P2 DEFERRED: only the live HTTP transport (needs the CoreLink prod AC tenant + PAT). Runner-side byte-identity (B2b items ③④⑤) is out of scope. (WP-B2a)
- feat(diag): WP-C6 — flake-stats collector + quarantine policy + false-positive guard (`crates/hugit-diag/src/flake/`). Every `CheckResult` from the B2 executor feeds per-test running statistics (`FlakeCollector`/`TestStats`); a planted 20%-flake test is classified `Flaky` within <30 runs (MIN_RUNS_FLAKY=5, detection deterministic via the 1-in-5 fail pattern); the quarantine list is a pure policy artifact — annotation-only, `gates=false`, no reorder/skip/block surface; `AutoActMechanism` is an uninhabited (zero-variant) enum proving the mechanism is structurally absent; a deterministically-failing (100% fail rate) test is classified `Real` and never quarantined. Acceptance oracle `tests/acceptance_c6.rs` items ①–④ RED→GREEN, hermetic (no live infra). (WP-C6)
- feat(runner): C3 — cache-warm boot: hydrate-on-lease via `clw hydrate`, toolchain layers shared from CAS across jobs (content-addressed, one physical copy per digest), CAS/AC-down fail-closed (§9 lock 5: SubstrateDown error, zero poisoned writes, no hang, no false green). Hermetic structural proof always runs in bare gate (warm=0 fetches, cold=N fetches, relative ordering invariant); real-box timing assertion (≤10s warm / ≥60s cold) gated behind HUGIT_RUNNER_HOST (WP-C3)

- fix(refstore,ci): final-closure hardening caught by the last full cold-verify. (1) `acceptance_d1c::admit_lock_split_never_exceeds_capacity_deterministic` could deadlock at join (observed wedged ~4h; root-caused by stack sample): the test released the pinned-writer admission gate the instant `CAPACITY` admit-hook signals arrived, without waiting for the `SURPLUS` submitters to be refused — a straggler surplus thread reaching the gate after the admitted ops drained their permits was wrongly admitted into an admission hook with no release left and blocked forever on `recv()`. Production is correct (`Serializer::submit` uses non-blocking `try_acquire`; the bound is atomic) — the defect was purely the test's rendezvous choreography. Fixed with a surplus-rejection rendezvous: main now awaits `CAPACITY` admits AND `SURPLUS` refusals before sampling/releasing, so no straggler can still be en route to the gate (proven with 45 watchdog'd runs). (2) The committed `Cargo.lock` was stale — missing `clap` + transitive deps required by the cli `[[bin]] hugit` (dropped during a lock-conflict integration); CI ran `cargo test --workspace` without `--locked`, silently regenerating the lock at build time and hiding the divergence. Lock regenerated; CI `clippy`/`test` now pass `--locked` so any manifest/lock divergence fails the gate (WP-closure-final-hardening)

- fix(app,exit,mirror): R1 — route every EventRecord `this_hash` through the canonical single-source `hugit_refstore::compute_this_hash` (LP-framed fields, VEC-framed principal_chain with the load-bearing u32 element-count prefix) and chain canonical-JSON payloads via `hugit_refstore::canonical_json`; delete the three divergent hand-rolled `compute_event_hash` copies (hugit-app `webhook.rs` installation.revoked/webhook.rejected — omitted the VEC count prefix; hugit-app-exit `gate.rs` enable-billing — hashed the principal as one scalar LP field with no count; hugit-mirror `import/history` git.commit). Each emitted record with a non-empty principal_chain now verifies against `verify_chain`. Oracle-first: acceptance_wp_b1, acceptance_wp-b9 item⑥, and acceptance_e2a item① now RECOMPUTE the expected `this_hash` via the canonical fn and assert byte-equality (plus `verify_chain`), confirmed RED on the divergent hashers and GREEN after routing to canonical (WP-r1-canonical-hash)

- fix(cli): cli-verbs-live-surface — `HUGIT_VERBS` now contains only the 4 LIVE dispatched verbs (`why`, `impact`, `tournament`, `export`); the 18 planned-but-unwired verbs move to `HUGIT_RESERVED_VERBS` (clearly not live). The cli oracle (`acceptance_rcli` item ⑥) is upgraded from a subset check to an EQUALITY assertion in both directions (registry→binary and binary→registry), so a phantom verb in `HUGIT_VERBS` or a dispatched verb missing from it turns the test RED immediately. WP-X5 `item_1_no_hugit_verb_shadows_git_verb` now tests the live 4-verb surface only (was testing 22 phantom verbs against `git help -a`); X5 remains structurally meaningful: a real git-shadowing verb added to the dispatched surface still turns it RED.

- fix(diag): thread real `recorded_at` (now_ms: u64) from callers into `emit_event` at all 3 call sites (gate.rs authorized/refused x2, datapoint.rs ingest x1) — every audit event timestamp was permanently 0 despite the parameter existing; `attempt_promotion` and `ingest` now accept a caller-supplied epoch-ms so production passes real wall-clock and tests pass deterministic fixed values; strengthen rdiag_c oracle (RED→GREEN: assert recorded_at == NOW and != 0 for authorized, refused, and ingestion-rejected events); harden rdiag_a to reconstruct principal_chain from known test inputs rather than round-tripping through the record (WP-D8-recorded-at)

- fix(fence): broker result-path traversal guard + hermetic fence-escape oracle (closure remediation, oracle-first). (1) `Broker::execute_into_container` delivered the broker result to a caller-supplied `result_path` with NO traversal/absolute guard — `shell_quote` blocked injection but not traversal, so a `..`/absolute path could write OUTSIDE the workspace root (the `place_file` re-guard existed but the broker delivery path did not). The API now takes `workspace_root` + a FENCE-RELATIVE `result_rel`, rejected fail-closed via the *same* `normalize_segments_pub` rule `place_file` uses (no leading `/`, no `..`) BEFORE the secret is resolved or any remote write is constructed (new `BrokerError::ResultPathEscapes`); the result is joined under the root by the broker, never supplied pre-joined. Oracle: `result_path_traversal_and_absolute_are_rejected_before_any_box_write` proves a `..`/absolute path is rejected with NO box command issued (RED before via a SpyBox that records `run`, GREEN after), plus a box-lane negative in C5b item⑥ confirming no file leaks to `/tmp`. (2) The real fence-escape redteam vector (materialize a real `FenceManifest`, then probe an out-of-fence path in the same container — the one that escapes under a no-op classifier) only ran when `HUGIT_RUNNER_HOST` was set (gate-blind in CI). Added a HERMETIC version (`fence_materialized_escape_is_contained_hermetically_no_box`) that drives the REAL `materialize_sparse` + `probe_outside_enoent` against an in-memory `FakeFsBox` in the BARE `cargo test` gate (fail-not-skip): the in-fence file is materialized, the out-of-fence file is ENOENT — proven LOAD-BEARING (goes RED if `classify` is replaced by constant-`Inside`). The box-backed vector is retained (WP-rfence-closure)

- fix(checks): R-checks — remediation of the brutal-review hugit-checks defects, oracle-first (each oracle strengthened to RED on baseline, then fixed to GREEN): (1) the pnpm regen driver runs the deterministic `pnpm install --lockfile-only --dir` (was a bare `pnpm install --dir` despite the docstring → nondeterministic regen); the exact command is pinned via `PNPM_REGEN_ARGS` and asserted without spawning pnpm; (2) `DriverRegistry::regenerate` is now the fail-closed boundary — a driver-internal `ToolNotFound`/`Io` error is funnelled into `FailClosed` so a caller matching only `FailClosed` can never treat it as non-fatal and fall back to a text-merge; (3) the D12 gate audit events compute a REAL `this_hash` via the canonical `hugit_refstore::compute_this_hash` over a canonical-JSON payload (was `String::new()`), chaining onto a caller-supplied `prev_hash` — the oracle independently recomputes and asserts chain continuity across two events; (4) the per-regen `AttestationChain` is SIGNED with ed25519 (`=2.1.1`) over the frozen `hugit_refstore::attestation_sig_preimage` (was empty `sig`); the oracle verifies with the PUBLIC key alone and rejects tampered/forged signatures; (5) anti-smuggling blocks an EMPTY `derived_claims` set (was a vacuously-satisfied loop that let a regen with nothing provably derived land) and resolves the independent verdict by `tree_hash` ONLY (dropped the unauthenticated `verdict.intent` resolution bypass); (6) the C4 snapshot oracle passes the REAL workspace root (the MockDriver now writes the requested nested path, not a path re-derived from a bent root), and item⑥ gains a real structural method-proof — register NO driver and assert `regenerate` fails closed (no text-merge fallback) (WP-rchecks)

- fix(policy): DCO parent-count + changelog fix-detection + secrets fail-closed (remediation): merge exemption now driven by `commit_parent_counts` (≥2) not message prefix — a non-merge commit whose subject starts "Merge " is no longer exempt; `is_feat_or_fix` checks char at index 3 for "fix" (was index 4, causing all "fix:" commits to silently bypass the changelog gate); a changed file with no content entry in the eval context now returns `Blocked` instead of silently passing (fail-closed scanning). Oracle-first: each defect was confirmed RED before the fix, GREEN after. (WP-rpolicy-gates)

- fix(mirror): E2a CI-harden — make the real-git import/recovery oracles hermetic and deterministic so they pass on any git version under CI's empty global config. The E2a① 1k-commit source repo is now built through a single atomic `git fast-import` stream (was ~2000 `git add`/`git commit` process pairs), eliminating the per-commit `git gc --auto`/maintenance background races that could leave a parent object briefly unwritten and surface in CI as `fatal: Failed to traverse parents of commit <oid>`; the build still produces real git objects with an explicit parent chain and is gated by `git fsck --strict` before any traversal. All real-git helpers in hugit-mirror (e2a, e1c, e1a) now run via a shared `git_command` that pins identity/dates, neutralises ambient global/system config (`GIT_CONFIG_GLOBAL`/`GIT_CONFIG_SYSTEM=/dev/null`), and disables `gc.auto`/`maintenance.auto`/`core.commitGraph`. Verified green under `env -i … GIT_CONFIG_GLOBAL=/dev/null`; item_1 runtime ~130s → ~75s (WP-e2a-ci-harden)

- fix(invariants): R-invariants — remediation of the brutal-review hugit-invariants findings, oracle-first (each oracle strengthened to RED on baseline main, then fixed to GREEN). X3: `export_redacted` did a degenerate WHOLE-blob redaction (the entire serialized journal collapsed to one `[REDACTED]` token the instant any field carried a secret, destroying all data); now redacts PER-FIELD on the structured journal before serialize so non-secret fields (binding key, surviving notes/principals) survive while only the secret-bearing field's value is scrubbed. The X3② oracle now plants the secret in one field and asserts (a) the secret VALUE `hugit-api-key-9f3a2b` is absent (not merely the `SECRET:` marker), (b) every non-secret field SURVIVES, and (c) the redacted payload is still valid Journal JSON; the ③ purge stand-in is documented as PARTIAL (no production context-store erasure API exists yet to drive). X4: items ①③ were a silent no-op when `HUGIT_RUNNER_HOST` was unset (the load-bearing fail-closed-before-spawn ORDERING proof never ran in CI), and item_3's unpinned case derived its spec via `from_lease`, which the runner remediation now correctly REJECTS at the supply-chain floor; now a hermetic `FakeBox`/`FakeEngine` oracle (`item_3_fail_closed_before_spawn_hermetic`) proves the ordering offline in the bare `cargo test --workspace` gate (fail-not-skip: a tampered/unpinned image must reject BEFORE `Engine::spawn`, asserted via a recorded spawn flag — proven to go RED if verification is defeated), and the unpinned spec is built via STRUCT LITERAL (bypassing the now-rejecting `from_lease`). X5: the no-shadow check tested a stale HARDCODED verb list (already missing `export`); now re-exports `hugit_cli::HUGIT_VERBS` (the single canonical CLI registry) and the oracle asserts the X5 surface IS the real registry, so a git-shadowing verb added to hugit-cli turns X5① red automatically. X2 untouched and still GREEN; X4 box lane GREEN against the live runner (WP-rinvariants)

- fix(refstore): R-refstore — undo/recovery/compaction/serializer remediation (oracle-first; each oracle confirmed RED on baseline, then GREEN). (1) undo folds `intent.landed` ref mutations: the ref-state projection (`replay_unchecked`, the one fold undo/recovery/compaction-equivalence all use) now treats `intent.landed` as a ref-advancing kind alongside `ref.update`, so a ref last touched by a landed intent restores the correct prior oid on undo instead of degenerating to `ref.delete`; a landed intent is itself directly undoable (compensator is a raw ref event, never a fabricated intent). (2) recovery splices a surviving HOT TAIL: new `recover_with_sources(cold, mirror, hot_tail)` extends `max_seq` across all sources so a partial hot-DO loss includes the unsealed suffix before `verify_chain` — no more silently-stale rebuild from the cold prefix alone (`recover_with_mirror` delegates with an empty tail). (3) compaction no-op is distinguishable: a within-bound pass reports an empty interval anchored at the log tail (`sealed_start == sealed_end == len`) plus `CompactionReport::is_noop()`, never a false `[0,0)` that collides with a real head-seal. (4) single-writer is real for every append path: `Serializer::undo`/`Serializer::import_sidecar` route compute+append atomically through the one admission-gate+writer mutex (concurrent undo/import are serialized into one gap-free hash-linked total order, exactly-once). (WP-R-refstore)

- fix(policy): route audit-event hash to canonical `hugit_refstore::compute_this_hash` (VEC-framed principal_chain, LP-framed fields); delete bespoke hasher that diverged on principal as scalar with no u32 count prefix; strengthen acceptance_d6 item③ oracle to pin `this_hash` against the refstore formula (WP-rpolicy-hash)

- fix(cli): R-cli — remediation of the brutal-review hugit-cli findings, oracle-first (each oracle strengthened to RED on baseline, then fixed to GREEN): (1) the REAL `hugit` binary now exists — a clap dispatch shell wiring `why`/`impact`/`tournament`/`export` end-to-end to the library with correct exit codes (0 success / non-zero error), and the canonical verb registry is exposed as `hugit_cli::HUGIT_VERBS`/`hugit_verbs()` for WP-X5 to consume the real surface; (2) export streams the JSON envelope field-by-field / element-by-element (no whole-envelope `to_vec`), with a `peak_serialize_scratch` OOM-bound proof; (3) `hugit why` resolves line ranges + symbols (two different lines on one file → different events; unattributed line/symbol rejected, never mis-attributed); (4) export git-object OIDs are path-traversal-sanitized before any write (fail-closed); (5) cut `ref_state()` returns `Result` — a malformed ref payload is a hard export failure, never silent-empty; (6) the persuasion negative test is now a real structural barrier proof (a persuadable reviewer that is NOT flipped) and `RedactionManifest::content_ref()` propagates serialize errors instead of a fixed hash (WP-R-cli)

- fix(fence): R-fence — real enforcement seam + genuine fence redteam vectors (remediation): the named enforcement gate (`check_access`/`is_admitted`) is now the single live predicate the materialize seam routes every candidate through (no dead enforcement API; a path the gate denies can never be placed); a sixth red-team vector (`fence_materialized_escape`) materializes a real `FenceManifest` then reads an out-of-fence path in the *same* container — the one vector where the fence (`classify` + sparse materialize), not the Docker namespace, is the control, so it escapes under a no-op classifier; ENOENT probe is locale-independent (`test ! -e` exit-code, dir-at-path handled, fail-closed on probe-shell error); fork-bomb/disk-fill now sample the peak over a window and assert the cap actually BIT (saturation / ENOSPC observed), not a single race-y sample; classifier oracle hardened (symlink-into-fence, absolute-path injection, prefix-collision, `..`-escaping-root); allow-all (`./`/`.`/``) path_set entries rejected fail-closed before any box command; `place_file` re-guards no-`..`; credential scan fails closed on an unparseable/truncated report (no default-clean); broker enforces lease→op authz (non-`Held` lease refused) with the lease↔container trust boundary documented (WP-rfence)

- fix(ledger): R-ledger — complete view-redaction coverage + surface malformed records (remediation): routes intent_id/campaign/deep_link_target through redact::apply in the ledger projection (previously surfaced raw); adds redaction to fleet workspace_id/agent_id and deeplink target; adds FleetState.malformed counter (malformed payloads are counted, never coalesced to "unknown"); adds EventClass::Other so unknown event kinds no longer mislabel as Landing. Oracle-first: six new acceptance assertions go RED on old code, GREEN after fix. D5/D11 preserved green.

- fix(queue): R-queue — THE WEDGE remediation (oracle-first). Pair-exclusion now lands end-to-end: a `UnionFail` predecessor is transparent, so innocent successors behind an excluded pair actually land (was structurally blocked by the ordering gate). Recovery replay on the same batch is idempotent (no `AlreadyTerminal` hard-error). Bisection is minimal+honest: an individually-red item is a `SingleItem` failure (never a false pair), and an unlocalisable red union is an explicit `Unlocalised` (never a silent empty drop). Duplicate `order_index` is rejected at batch construction. Stale-head is enforced at the engine, not delegated — a force-pushed stale union is refused even when the `MergeApi` ignores `expected_head` (WP-R-queue)

- fix(proto): R-proto — remediation of the brutal-review write/read-path defects. Wire the flag-gate and compare-and-append total-order into the REAL receive-pack ingest (flag off ⇒ push refused before any CAS write/event; concurrent real pushes get a contiguous total order, stale tip ⇒ rejected, no lost update — via the single-writer `SerializedReceiver`); require the ref target be REACHABLE from the pushed pack (not merely present in the scratch odb); cap INFLATED bytes + object count to defuse decompression/object-count bombs (compressed bound alone no longer admits a tiny pack that inflates huge). Read path: degradation kill-test now exercises an observable smart-layer that Disabled genuinely bypasses (no longer `let _ = state`); CPU-budget fallback routes on the MEASURED cost of the actually-assembled pack (not an injected estimate); client-matrix conformance round-trips the real serve pack through a real `git` clone and diffs against the source (no self-comparison; FAIL-not-skip when git absent). Plus: `GitObject::try_oid` propagates a hashing error instead of `expect`, and the single-writer mutex recovers a poisoned lock instead of panicking. Every defect proven oracle-RED-then-GREEN. (R-proto)

- fix(runner): R-runner — enforce X4 pin on real spawn surface + sanitize tmp_root (remediation). Per the brutal review (R2/R4 §hugit-runner): the supply-chain pin/verify was a wrapper the live path bypassed, and `tmp_root` flowed unsanitized into `sh -c` (root RCE). Now `ContainerSpec::from_lease` rejects any non-`@sha256:`-pinned image AND validates `tmp_root` against `^/[A-Za-z0-9._/-]+$`; `DockerEngine::spawn` integrity-verifies the pin against the box BEFORE `docker run` (fail-closed, no container on failure). Added a hermetic `FakeBox`/`FakeEngine` oracle proving verify-before-run + tmp_root-reject offline in the bare `cargo test --workspace` gate (no box, fail-not-skip). Also: the dedup spawner no longer holds the global lock across spawn (claim-then-spawn with a per-id condvar) and liveness-probes cached handles (no dead-container reuse); state-restore streams its payload over stdin instead of shell-constructing it; `shell_join` always-quotes (no allowlist passthrough). Concurrency fix (cold-verify caught): under ≥8 jobs sharing one pinned digest, `verify_on_box` raced the Docker daemon — 8 simultaneous pulls of the SAME digest returned a *transient* error that was misread as an integrity failure and failed CLOSED, killing a genuinely-pinned job (C2b item_4 spurious fail). Now verify (a) serializes same-digest pulls through a per-digest lock (the box pulls a given digest once at a time; distinct digests never block each other) and (b) classifies pull errors — a permanent signal (manifest unknown / not found / digest mismatch / denied) fails CLOSED on the first attempt with NO retry, while a transient network/daemon/race hiccup is retried a bounded number of times with backoff. Tamper rejection is unchanged: an unpinned/tampered digest still fails closed before spawn. Added hermetic regression tests (transient-retried, permanent-fail-closed-no-retry, budget-exhaustion-fail-closed, 8-thread same-digest all-succeed, classifier). Box suites c2a/c2b/c9 green with resolved pinned digests (WP-R-runner)

- fix(mirror): R-mirror — structural one-way proof + real import/LFS oracles + token redact (remediation): the SACRED one-way invariant is now proven by construction (the only mirror-mutation primitive `MirrorMutation` can carry only the forge tip; a runtime architecture oracle scans the shipped one-way surface and goes RED if any reverse-sync sink is introduced) instead of a `const CODEPATH_PRESENT=false` fiat; the E2a① "1k-commit byte-identical import" oracle now runs the real `import_commits`/`read_git_object` path against a real on-disk git repo and compares to `git rev-list`/`git cat-file` (no hand-built strings); `read_git_object` validates the cat-file `--batch` header (oid/type/size) and slices exactly `size` bytes; the private-repo `InstallationToken` no longer derives `Debug` over a cleartext token (private field + redacting `Debug` + `expose()`, oracle asserts `{:?}` omits the secret); real Git LFS batch-API fetch implemented (`materialize_lfs_via_batch` speaks the real batch protocol over an `LfsTransport`, driven in-gate by a real on-disk LFS server fixture, fail-closed SHA-256/size verify); PR/issue batch import dedups by `intent_id` (two identical fixtures → one intent); divergence `mirror_write` derived from an observed `MutationOrigin::MirrorSide` event, not tip-inequality (no false positive on lag); E1a① landing + E1a③ soak now driven through at least one REAL git readback (`git rev-parse`), not echo-by-FixtureMirror. All defect oracles confirmed RED on the pre-fix code, GREEN after (WP-RMIRROR)

- fix(diag): remediation — canonical audit hash (route to hugit_refstore::compute_this_hash, eliminating bespoke hasher that omitted VEC count prefix); real recorded_at in emit_event (caller-supplied, no longer hardcoded 0); CorpusTampered returned for corpus-seal mismatch (was Forged) (wp/rdiag)

- fix(contracts,refstore): R0 — single-source + byte-exact hash-chain/memo-key/attestation formula: canonical `compute_this_hash`/`compute_memo_key`/`canonical_json`/`attestation_sig_preimage` in hugit-refstore (the one source of truth), byte-exact frozen doc-specs in contracts (LP/VEC framing, recorded_at excluded, canonical-JSON payload, 64-ASCII-'0' genesis), and an independent cross-crate hash-pin so re-transcription drift (brutal-review R1) goes RED (WP-R0)

- feat(runner): C9 — workspace lifecycle: attach/resume/spawn-dedup, local≡remote (WP-C9)

- feat(fence): C5b — secrets broker v0 (credentials never enter the runner; principal-chain audit; fail-closed) + active escape red-team harness (traversal/symlink/out-of-fence/fork-bomb/disk-fill contained, box residue 0) (WP-C5b)

- feat(diag): D8 — experiment harness + gate binds (claims/regen blocked until PASS) (WP-D8)

- feat(queue): C10 — pricing no-shock: per-surface cap/degrade + pre-exhaustion warning + zero-overage billing fixture (WP-C10)

- feat(proto): D2a — git wire read path: protocol-v2 negotiate/ls-refs/want-have, pack assembly from CAS, byte-identical clone + delta-only fetch (WP-D2a)

- feat(ledger): D11 — session journals + ctx resume: tenant-private journal objects bound to ws/intent, within-horizon reconstruction, beyond-horizon documented refusal (WP-D11)

- feat(runner): E4 — Actions-YAML shim v0: published supported-subset contract (15 features, proven-to-execute), explicit out-of-contract actionable reports, secrets fail-CLOSED via broker (red-team asserted), execution equivalence harness with determinism-precondition gate (PARTIAL: shim lane green, live GH lane wired) (WP-E4)

- feat: hugit-invariants — X4 supply-chain: content-pinned runner images, verified deps, fail-closed before spawn (WP-X4)

- feat(invariants): X5 — namespace laws: no hugit CLI verb shadows a git verb (mechanized `git help -a` check), managed refs/hugit/… never collide with user branches/tags (property test corpus) (WP-X5)

- feat(mirror): E2b — PR/issue import: proposed/non-authoritative intents with per-element provenance, fidelity contract (body/comments/state/labels/cross-refs), explicit NON_IMPORTED enumeration, idempotent re-sync (WP-E2b)

- feat(mirror): E1c — verified-mirror bootstrap + disaster recovery: resumable cold-seed of full history to a fresh repo (byte-identity hash-verified), GitHub-side loss DR (App-revocation + repo deletion/rename detected → fail-closed incident with pinned recovery-source and resume-from-recovered-state), and substrate-loss DR — the mirror is a byte-complete working git repo, recovery proven end-to-end, recovered content imports as change-events never fabricated intents (WP-E1c)

- feat(mirror): E1a — one-way mirror (hugit→GitHub): outbound sync writer + GitHub App installation-token auth, per-push content-hash verify (byte-identity, fail-CLOSED → divergence), durable ordered/capacity-bounded outage queue (stated bound, overflow → backpressure + incident, never drop/reorder), <60s SLA + soak metric; live GH lane PARTIAL when installation uncovered, never faked (WP-E1a)

- feat(cli): E5 — export + exit proof: one-command git+JSON dump validated against the versioned ExportSchema (machine check), object-for-object restore round-trip, redaction-at-export with manifest, bounded-memory streaming, point-in-time-consistent cut over D1, terminating-account read-only path, and THE EXIT PROOF — exported artifact clone/log/branch/push with ZERO hugit tooling on PATH; redaction red-team asserts seeded secrets nowhere (WP-E5)

- feat: D9 — attention queue: documented composite ranking (policy × blast-radius × confidence), policy-mandatory floor, fast-approve (90s) gating (blocked for high-risk/mandatory, permitted for low-risk), honest "ranking degraded" up-zoom under missing inputs (WP-D9)

- feat(proto): D2b — read-path edges: client matrix (git 2.40+/jj/libgit2), jj first-class stacked changes with change-ids stable across forge ops + identical stack reconstruction, per-request CPU budget (70% of platform limit) + chunked fallback, degradation kill-test (smart layers off, steady-state + mid-op → vanilla git still serves), per-dimension scale ceilings with documented bounded refusal (PARTIAL: jj binary absent locally → item ⑦ wire round-trip skipped, in-process model proven) (WP-D2b)

- feat(proto): D3b — push concurrency: concurrent pushes serialized through the D1 single-writer point get a strict total order with compare-and-append stale rejection (no lost update); raw push = opaque external-change event with attribution (who/when/ref), never a fabricated intent; write path off unless self-hosted-alpha; intent-log-clean negative + write-path red-team (WP-D3b)

- feat(cli): D13 — tournament: N-candidate fan-out, judge-panel selection per documented criteria, losers addressable as evidence, policy-capped + C7 budget-bounded (zero overage) (WP-D13)

- feat(checks): D12 — regen gate: opt-in scope only, lands only on acceptance re-pass + fresh independent verdict (fail-closed), per-regen attestation records the authorizing gate-verdict ref, anti-smuggling blocks+audits false "derived" declarations (WP-D12)

- feat(mirror): E1b — failure modes: outage durable queue with bounded backoff (no drop/reorder, drain-to-verified + gap incident), force-push/branch-delete/tag replication with orphan-aware no-false-divergence, partial divergence repaired scoped to the broken ref, webhook-loss poll fallback within SLA, and ONE-WAY enforced — reverse mirror writes treated as divergence → forge-authoritative repair, zero reverse-sync codepath (WP-E1b)

- feat(mirror): E2a — git history import: byte-identity (OID round-trip via SHA-1), LFS object materialization (SHA-256 verified, not pointer), resumable across timeouts (cursor-based, no restart-from-zero), idempotency (unchanged→no-op, changed→incremental, no dupes), import boundary (bare commit→opaque EventRecord, no intent synthesized) (WP-E2a)

- feat(invariants): X3 — context privacy: tenant-scoped journals (cross-tenant fetch denied + audited), redaction at capture AND export (ExportSchema-validated), retention/deletion purge verified-absent, training/eval exclusion documented control + audit trail (WP-X3)

- feat(invariants): X2 — attestation e2e with ed25519 public verification: full provenance chain (tree+def+runner+model+principal) resolves cryptographically, tampered/unsigned/forged rejected fail-closed at promotion (with audit), PUBLIC verification procedure provable with the public key alone (signing key never needed/present — replaces the prior HMAC keyed-MAC), cross-tenant shared-hit honesty via anonymized PLATFORM attestation (no producer-tenant leak, no mis-attribution) (WP-X2)

- feat(proto): D3a — git push path v0: receive-pack→CAS+log, raw push=change-event (WP-D3a)

- feat(app): B6 — intent sidecar: parse/validate/render + corpus to CAS ref (WP-B6)

- feat: hugit-checks/affected — affected-target engine v0; cargo/pnpm/turbo graph adapters, root-edit full-set, fail-open policy (WP-B3)

- feat: hugit-refstore — event-log core: append-only hash chain, deterministic replay, tamper detection (WP-D1a)

- feat(queue): hugit-queue union-queue core — batching, union-tree fold, minimal-failing-pair bisection, ordered idempotent landing, structural ordering state machine (WP-B4a)

- feat(queue): hugit-queue GitHub integration — ordered atomic merge API with honored merge method, force-push union recompute, branch-protection holds (never force-merged), crash-idempotent kill-test recovery; live App-JWT installations lane (WP-B4b)

- feat(checks): C4 — regen drivers v0: lockfile/codegen/snapshot regenerate-never-merge (WP-C4)

- feat: hugit-policy — declarative gate engine v0: 3 ported gates (DCO/changelog/secrets), fail-closed enforcement, audited policy changes via EventRecord (WP-D6)

- feat: hugit-app — GitHub App skeleton: X-Hub-Signature-256 webhook auth + ingest, PR-event persistence, Checks-API write-back, least-privilege manifest, uninstall revoke+halt (WP-B1)

- feat: hugit-contracts — 15 frozen contract types + JSON Schemas + golden serde suite (WP-00)
- feat: Rust workspace scaffold — 12 crates, CI gates (fmt/clippy/test/audit), DCO + changelog discipline (WP-01)
- feat(runner): ephemeral runner v0 — container-per-job lease lifecycle + tmp/net isolation, teardown with forensic re-scan (WP-C2a)
