# Adversarial convergence — Round 2 verdict (fresh 7-agent fleet)

> 2026-06-11, after Wave E. Fresh-context refutation fleet (1 fable + 2 opus +
> 4 sonnet), no memory of Round 1, hugit @ main `72a5f95`. **Result: 7/7
> DO-NOT-SHIP again — but narrower.** The structural spine HELD (Round-1 D14
> fixes confirmed closed, hash/canonical math sound, money clean, no cross-repo
> compile break — verified twice). What Round 2 found is one CRITICAL the
> Round-1 FIX created, plus consistency-completeness gaps (missed siblings,
> diverged engines, projection wiring) and doc/CI debt. Convergence is closer;
> not reached. Wave F + a P2-ceiling statement below.

## Confirmed by the lead (HIGH findings re-verified personally)
- **WF-1 CRITICAL — 40/64-hex secret leak (fable F1).** `redact.rs`
  `is_content_address_ref` exempts any 40/64-hex token from the entropy scan
  (to protect cas:/sha digests) — so a real HMAC/SECRET_KEY/API key of exactly
  that shape in a charter leaks VERBATIM to `.hugit/intents.json` + the
  hash-chained log. **My own Wave-A/E exemption created it; my own tests bless
  it** (`sha256_digest_survives` etc.). Reproduced live by the adversary.
- **WF-2 — abandon-projection deadlock (product F2).** `pr abandon` appends
  `pr.abandoned` but `pr show`/`queue show`/`campaign show` ignore it →
  abandoned PR stays in-flight forever → `campaign close` refuses forever →
  no verb sequence reaches `closed:true`. The verb's own help lies. Real bug.
- **WF-3 — verify_chain skipped on checks/queue loaders (authz F1).** E-CLI
  retrofitted pr but missed the two WB2 siblings (`checks/mod.rs` load,
  `queue/mod.rs`) — a tampered log projects as truth. 2 sites.
- **WF-4 — redaction-engine divergence (redaction F1/F8).** `export`'s static
  16-pattern list misses 9 classes the ledger engine catches (JWT, gho_, clp_,
  PEM, conn-strings, keyword-context, entropy). The export surface (ships data
  OUT) uses the WEAKER engine. Also: export's `intents` vector + the envelope
  `authorship.operator`/`campaign` fields bypass the scrub; `ghs_` missing from
  the ledger prefixes; `key = val` (space) + base64-FP edges.

## Also confirmed (MED/LOW, real)
- error-shape NOT uniform: campaign nests `path` under `detail`, others
  top-level (product F4). `--log` is 3 incompatible formats across
  why/export/flow (product F3). `tournament` fabricates output for a
  nonexistent intent, exit 0 (product F5). `intent list` missing-store →
  empty/exit-0 vs siblings' log_not_found/exit-2 (product F6).
- `intent.landed` gated on Push (all-class) via import vs Land (orch) via
  mirror; import accepts arbitrary ref_name → library caller could advance
  main under Push (authz F2, latent — CLI safe today via synthetic ref).
- Docs/process: PS-4/PS-5 (transplant-naming, runner-sec) claimed tracked but
  absent from the register; CLAUDE.md/README stale (pre-Wave-E, omit Round-1
  7/7); gate desc omits `cargo deny`; "remote CI green on HEAD" stale (HEAD
  in_progress, ~63% infra-fail rate); CHANGELOG "propagated" is branch-true
  not mainline-true + a false "corrected to 18"; CI fork-guard undocumented;
  files_read[].hash comment overclaims totality.

## False positives killed by the lead (evidence)
- docs-F1 "1.2.0 not propagated / no githugr": commits EXIST —
  corelink-runners `6b57690` (spec has cost_usd_micros/1.2.0), githugr
  `7d8eef7` (manifest superseded). Downgraded HIGH→MED (branch-true, sibling
  mainline-merge pending — a precision fix, not a falsified claim).
- cross-repo hard break: cleared AGAIN (fable) — githugr only cosmetic VM
  doc drift, no compile break; runner conformance vectors byte-identical.

## P2-CEILING — not Wave F, owner-gated (the honest frontier)
These 3 are HIGH and adversaries count them as ship-blockers, but they are
**honestly tracked** (PS-1/2/3) and blocked on OWNER infra, not code:
- **PS-1 wedge operationally null**: `checks show`/`queue show` null on every
  real log — no `check.recorded`/`verdict.recorded`/`pr.landed` producer. The
  RECORDER VERB is buildable now (P2-independent); the LIVE AC needs P2.
  → owner decision: build the recorder verb in a follow wave? (makes the wedge
  observable on local/fixture logs without P2).
- **PS-2 authn binding**: `--author-kind` caller-asserted/forgeable until
  ADR-0002 identity/P2.
- **PS-3 prod erasure**: tombstone proven on InMemory only; CoreLink R2/D1
  adapter has no erase until P2.
**"Ship as a production forge delivering the wedge" is P2-gated, not code-gated.**

## Wave F dispatch (partition by owned files, disjoint)
- **WF-REDACT** (opus): WF-1 critical (hex-exempt only in content-address
  CONTEXT, redact bare hex in free text; fix the blessing tests) + WF-4 unify
  export→ledger engine + export intents/envelope authorship+campaign scrub +
  ghs_/keyword-space/base64 edges. Owns hugit-ledger/{redact,envelope} +
  hugit-cli/src/export/**.
- **WF-CLI** (opus): WF-2 abandon-projection deadlock + WF-3 verify_chain on
  checks/queue + error-shape true uniformity + --log format + tournament/list
  consistency. Owns hugit-cli/src/{campaign,pr,intent,checks,queue,tournament,
  porcelain}/** (NOT export).
- **WF-AUTHZ** (sonnet): intent.landed ref-guard (import can't advance a
  protected ref under Push). Owns hugit-refstore/src/{intent/import,replay}.
- **WF-DOCS** (sonnet): PS-4/PS-5, CLAUDE.md/README refresh (honest:
  Round-1 found 7/7, Wave E remediated, Round 2 pending), gate desc +deny,
  CI-green wording, CHANGELOG precision, fork-guard note, hash comment.
Then Round 3: fresh fleet.
