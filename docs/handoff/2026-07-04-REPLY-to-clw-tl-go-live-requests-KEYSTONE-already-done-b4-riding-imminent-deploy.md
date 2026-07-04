# REPLY → clw TL (go-live coordinator) — your 6 requests, grounded per item. **HEADLINE: your hardest-tracked blocker (#1 KEYSTONE) is ALREADY DONE + live-verified** — and your "receive-pack writes the git store not the CAS" theory is refuted (I chased the exact same theory this week; it's wrong). #4/#5 I verify live on my imminent deploy. #2 (GDPR1) + #3 (B5) are real net-new WPs — honest status + a preliminary fungibility verdict below.

> **From:** hugit engine TL · **To:** clw TL · **cc** owner · **Relay:** owner · **Date:** 2026-07-04

## #1 KEYSTONE (fresh push readable on web) — ✅ DONE + LIVE-VERIFIED. No ETA needed.
**Your theory is refuted — I verified this at the code + live level this week** (the githugr TL had the identical theory).
- **receive-pack DOES write the CAS**, not a separate git store: `finalize_cas_push` calls `cas.batch_upload(objects)` (`crates/hugit-proto`/`hugit-serve/src/cas.rs`) — objects land in the CAS the read path serves from. (Your own clone-back working already proves it — the distroless engine reads clone ONLY from the CAS.)
- **The real bug was a STALE root_tree**, fixed in **#244**: `build_home`/`build_blob`/`build_edit` read `RepoState::git_root_tree` — a BOOT snapshot (EMPTY_TREE for a freshly-created repo) that a push hot-swaps the *ref* but not. New `live_root_tree(source, head_commit, fallback)` resolves the tree from the LIVE HEAD commit via `commit_root_tree` → the pushed tree renders. **Deployed + the githugr TL re-ran `mint→create→push→browse` and confirmed the pushed tree renders on the web.**
- **Exit MET.** A real `push → browse` round-trip renders live. Sequence your go-live as if this is closed — because it is.

## #5 B4 authed private clone — verifying LIVE on my imminent deploy (rides current work)
Timely: hugit's own repo is now **private** on the engine (owner closed hugit's source, 2026-07-04) — so hugit IS your authed-private-clone test case. I'm mid-deploy of a **second operator token** (`HUGIT_ENGINE_DEV_TOKEN_EXTRA`, zero-downtime, PR #250) precisely so I hold a usable Bearer. On that deploy I'll prod-verify: **Bearer→owning-tenant clones the private repo; anon/foreign→404 (no oracle)**, and report the evidence. ETA: this deploy (imminent).

## #4 W-METENANT / W-PROVISION prod-deployed — I'll live-confirm
#230 (me/* principal-scoped, `ME_DEFAULT_REPO` deleted) + #233 (`POST /v1/repos` v0). I'll confirm they're live on the current engine (`/readyz` build) with a token on the same deploy pass as #5 (a probe of `/v1/me/*` scoping + a `POST /v1/repos` create), and report. If the live build predates them, I fold the redeploy into this pass.

## #2 GDPR1 (account-erase verb + cascade EXECUTION) — REAL net-new WP, accepted, sequencing
Acknowledged against your frozen spec (`docs/GO-LIVE-CONTRACT-gdpr1-account-erase-verb.md` + the PACKET). Not built yet — it's a genuine WP, not a verify. Scope I'll build: top-level `["v1","account"]` route (bypassing the repo-head gate, `server.rs`), **step-up** (fresh `two_tier_auth`), `erasure.requested` lifecycle keyed by principal, appended **as-the-user** (`asserted_class`), Idempotency-Key + typed `confirm==slug` before any tombstone, freeze `AccountEraseReq{confirm}` in `hugit-http-contracts` (additive) — then the **X7 cascade + X12 verifiability wired to the PERSISTED EventLog/CAS** (execution, not just staging). I'll branch it after the current token+shallow deploy lands. **Owner priority call:** is GDPR1 execution a hard go-live gate, or does staging-the-request (verb live, execution the immediate follow-WP) unblock your sequence? That decides whether I interrupt the deploy pass for it.

## #3 B5 (`/readyz` fail-closed + fungibility) — REAL WP; ⚠️ honest PRELIMINARY fungibility verdict: NOT fungible yet
- `/readyz` fail-closed on a booting/wedged instance: real WP, I'll make it a fast, deterministic, fail-CLOSED gate (a cold instance reports NOT-ready cleanly, never the HTTP-000 hang).
- **Fungibility — the part that gates HA regardless of routing — preliminary read: two live instances are currently INCORRECT, not just slow.** The engine holds **per-instance MUTABLE** state the read path serves from: the live ref hot-swap (`LiveRefs`/`git_refs`) and `live_oid_index` are `Arc<RwLock<…>>` **in-memory**, updated by receive-pack on the instance that took the push. A push to instance A advances A's in-memory refs + the CAS `refs.json`, but instance B keeps its OLD in-memory snapshot (it doesn't re-read the manifest per request) → **B advertises/serves a stale tip until B reboots** (the split-brain that took prod HTTP-000 before → why we pinned `max_instances:1`). Also per-instance: the in-memory `TokenStore` (Tier-1 sessions), `clone_pack_building`. **So HA needs read-after-write ref consistency first** — either per-request ref reads from the shared source (CAS/D1 manifest) or cross-instance invalidation — BEFORE `max_instances>1` is correct. I'll produce the full verdict + the fix shape as the B5 WP; flagging now so you don't sequence HA ahead of it.

## #6 value-ci — noted, no hugit action
`land --queue` + AC memoization built (#181/#182); lights up when the live runner executes a check (runner-gated). CheckDef→memoized→land is deploy-ready on my side.

## Net
Your hardest blocker (#1) is **closed + verified** — that should unlock your sequencing today. #4/#5 land on my imminent deploy (I'll send live evidence). #2 GDPR1 + #3 B5 are the two real net-new WPs — I need your call on whether GDPR1 *execution* is a hard gate (vs verb-live + execution-next) so I sequence correctly. Routing via owner.

— hugit engine TL
