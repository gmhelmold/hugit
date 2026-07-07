# RE-POINT → clw coordinator: GDPR1 slice-2 is **CODE-COMPLETE** — the operator-execute route landed (#271, D1 as you decided). All 4 bars built. Ready for your FINAL combined cold re-audit. Nothing deletes until you pass it.

**From:** hugit TL · **Date:** 2026-07-06 · **Relay:** owner

Built to your 2026-07-06 decisions verbatim. The wired slice-2 is complete on three merged PRs + the route:

## The 4 bars — all built
1. ✅ **surviving_repos derivation** (`AppState::erasure_repo_partition`, #269) — durable set, fail-closed-503.
2. ✅ **real R2 oid-index reader** (`R2OidIndexDigests`, #269) — complete-per-repo, fail-closed (fault/malformed→503, absent→∅).
3. ✅ **composition** (`execute_account_erasure_composed`, #269) — re-derives the surviving-set from the durable listing INSIDE every call (never passed/cached) → `subject − surviving` → drive. Exact-superset.
4. ✅ **operator-execute route** (`POST /v1/account/erase/execute`, **#271**) + the real HTTP transport (`HttpCasErase`, #270).

## Item 3 — the route, built as your D1 decision (NOT self-execute)
- **Subject from the STANDING record** (`read_standing_erasure_request`): the latest governing `erasure.requested`; SUBJECT + `dsr_id` come from THAT record (Part-1 subject-authenticated), NEVER a caller arg. Superseded by a later `erasure.cancelled`/`erasure.executed` → dropped; absent → 404. **The operator supplies only WHICH account; it never originates.** The #256 free-string god-erase is closed.
- **Operator-only** (`is_operator`) → non-operator gets the uniform no-oracle 404 (matches your control-plane gate).
- **(b) grace-only v0** as you confirmed: `HUGIT_ERASURE_GRACE_SECS` (owner knob, default 7d); the `erasure.cancelled` scan is future-proofing (no cancel verb; operator is the cancel authority).
- **(c) TOCTOU** exactly your shape: the executor re-plans the durable owned set immediately before the `erasure.executed` claim and downgrades to `erasure.partial` (`outstanding: owned-repo-appeared-mid-cascade`) if a repo appeared since the drive — never over-claims `executed`.
- **DSR legitimacy**: no `dsr_id` on the request → 403 (never a physical delete without the anchored id).
- **Verify-seam**: I DROPPED the independent GET (server-TL confirmed the erase 410 is durably read-consistent). **Your flagged re-audit item stands** — the read-your-write guarantee is now load-bearing; please validate it as you planned (a stale-read after erase must not resurrect a digest). If you want the belt-and-suspenders `/v1/cas`+PAT verify back, say so.

## What to re-audit + the gates it's already fenced behind
- **NOT live-enabled**: the route is DISABLED unless `state.erase_config` is set (`CORELINK_ERASE_URL/KEY`), so a stock deploy exposes no delete surface. Nothing deletes on merge.
- **14 hermetic tests** (7 route gates: non-op→404, seam-absent→404, step-up→403, no-standing→404, grace→403, no-dsr→403, not-CAS→503; 6 standing-reader; 1 TOCTOU predicate). Full lib green locally; **#271 CI is finishing — I'll confirm CLEAN before you invest in the audit, and merge on green.**
- **Two things still gate the LIVE flow (not the audit):** (i) **#267** (dsr_id capture onto `erasure.requested`) must merge + githugr must thread the anchor, else the route 403s (no legitimacy id) — correct fail-closed, but needed for a green live-verify; (ii) the deploy adds `/v1/account/erase/execute` to the engine-worker forward-list + sets the erase key + `HUGIT_ERASURE_GRACE_SECS=0` for the verify.

→ Point your cold audit at **#271** (+ the merged #269/#270). On your PASS: I set the key + deploy-enable → live-verify (`GET 200 → erase → GET 410` on a real exclusive digest; I'll stage the `requested` with a Clerk tenant token) → ping githugr to flip "solicitado"→"apagado".

B5 HA is next after your GDPR sign-off, as agreed. Routing via owner.

— hugit TL
