# CLAUDE.md

## Permanent Scope Closure

The following are permanently discontinued, not deferred roadmap items:
`ws`/`dispatch` workspace or runner execution, remote AC, Clerk identity,
tenancy, forge/hosting, mirror deployment, and external runner attestation.
Their historical docs may mention them, but current hugit work must not reopen
or count them as pending. `ws`/`dispatch` tokens remain in the reserved registry
only for namespace protection.

Context for AI agents working in this repo. Keep it lean + high-signal.

## What hugit is

The **git-local, LLM-native CLI** — CoreLink expansion campaign #3.
Intent capture, provenance, memoized local checks, review, and union landing
inside an existing Git repository. No server, account, or CoreLink dependency
in default path. Founded 2026-06-05.

> **Current release mode (2026-09-14):** CLI-local v0.1.5 ships from `main`.
> `hugit-serve`/remote forge, multi-tenant hosting, and runner execution are
> separate or external products, not current CLI blockers. The older delivery
> audit below preserves historical forge/serve context; do not treat it as the
> current CLI backlog.

## Status — honest delivery reality (double-checked 2026-06-17)

> This **supersedes the prior maximalist "67/67 built · complete · converged"
> narrative**, which conflated *test-green-hermetically* and *PR-merged* with
> *delivered live*. Full grounded, double-checked audit (44-agent read-only
> sweep vs the whitepaper + 67-WP decomposition):
> `docs/review/2026-06-17-honest-delivery-audit-double-checked.md`. Read it
> before making any "done"/"complete"/"live" claim.

**What this repo IS:** a **19-package** Rust workspace (hugit-app + {ui,exit,sidecar}
sub-crates + feature crates + `hugit-http-contracts` + `hugit-serve`; `hugit-web`
migrated to `../githugr` 2026-06-10; `hugit-runner` transferred to
`../corelink-runners` 2026-06-10 — hugit is git+forge, compute is campaign #1; the
seam is the byte-identical `conformance/` wire contract, no git dep either way).
It implements the **logic** of all 67 decomposition WPs + the `/v1` HTTP backend
the githugr window reads. The engine logic + the **Squad-X platform invariants**
are real, hermetically tested (real Ed25519/SHA-256 crypto), and have held every
adversarial round (1–13) + a SOTA sweep. The integrity spine is genuinely solid.

**What is NOT delivered ("built" ≠ "live" — the honest gap):**
- **Git wire serving: clone/fetch BUILT + the engine boots git-from-CAS, push NOT.** `hugit-serve`
  speaks the git smart-HTTP upload-pack wire over `hugit-proto`'s clone/fetch logic (CI-proven e2e).
  The DEPLOYED engine boots git-from-CAS and loads both repos (`/readyz git_serving:true,git_repos:2`,
  2026-06-22). The wire itself is git-native + done; whether ANONYMOUS `git clone` is *exposed* is the
  forge/owner **visibility decision** (`repo.meta{visibility}`) — NOT a wire or code gap, and never a
  hugit "build" item (see Principles: visibility is the forge's, not hugit's). Verify the deployed
  read-exposure posture live before asserting anon-open OR authed-only — it's owner-set. `git push`
  (receive-pack) is **LIVE** (2026-06-26, #198 —
  git-free gix-pack unpack on the distroless engine; first real push returned `unpack ok` +
  `ok refs/heads/_pushsmoke`). The **live ref hot-swap is DONE** (#201, deployed 2026-06-26 —
  verified: a ref pushed post-deploy appears in the receive-pack advertise immediately, same engine
  lifetime, no reboot), so the stale-advertise / false-non-fast-forward gap is CLOSED. **Caveat (a) is
  now also CLOSED:** an *incremental* push that builds on server-side history LANDS (#206 thin-pack /
  CAS-base reachability — the resolver + reachability walk consult the CAS for an ancestor/REF_DELTA base
  the pack omits; deployed + prod-verified 2026-06-26: a real fast-forward `0886fda..abb77f0` on a
  server-side ancestor landed, reads stayed 200). So CREATE, UPDATE, and INCREMENTAL push all work live.
  Remaining item: **(b)** clone-back *exposure* is the forge/owner **visibility decision**
  (`repo.meta{visibility}`), not hugit code — the engine enforces whatever flag it's handed.
- **Substrate — AC LIVE (2026-06-22), runner still transferred.** The CoreLink AC
  (memoization) is now LIVE: `check run` + `land queue` prefer `HttpAcClient::from_runtime`
  (#182), smoke-proven MISS→remote-HIT against tenant `3560e213`; the hot-CAS git tenant
  `d863fafb` serves the engine's repos. STILL deferred: the runner fabric → `corelink-runners`;
  cold-store (`UnwiredColdStore`) persists no transcript blobs; **merge-as-re-execution records
  the demand but never dispatches an agent** (intentional P2 deferral).
  **Cost-killer (A-path) WIRE PROVEN LIVE 2026-06-28** (after CoreLink's #224 share-agent token-store fix
  — my diagnosis): a manual smoke against `corelink-fabricd` with the minted `HUGIT_RUNNER_PAT` ran the
  WHOLE off-box A-path — `acquire 200` → `§13 ingest 200` (scoped cred) → `close 200` with the fabric
  **deriving tokens from the submitted events + signing the attestation** (`result_binding_sig_v2`). En
  route this surfaced + fixed the 3rd lease-client wire-drift (acquire REQUEST shape — `principal_chain`
  → the fabric's `{image_digest,net_policy,tmp_root,expiry_ms}`, #214; after the acquire-resp #204 + close
  #205). **Cost PATH NOW LIVE + verified end-to-end (2026-06-30):** the Runners TL added
  `CloseRequest.cost_usd_micros` (#226, recorded verbatim) + hugit landed the submit-side (#64,
  `Option<u64>` skip-when-none) — and a live close submitting `cost_usd_micros: 4200000` against the
  DEPLOYED fabricd recorded `metrics.cost_usd_micros == 4200000` exactly (submit→record→attest proven).
  The four lease DTOs are now frozen byte-identical in both repos (#220 conformance vectors + tripwire —
  no drift; the 3-wire-drift history ends). **The ONLY remaining gap to a non-zero RENDERED killer is a
  real provider-`/usage` cost SOURCE** — an off-box agent-loop (merge-as-re-execution P2) that reads the
  LLM provider's billed figure; hugit's dispatch passes `None` (honest-zero) until then, NEVER the derived
  `IntentMetrics` COGS (that would misattribute). Honest caveat: hugit's `CloseResponse` DTO currently
  ignores the attestation block (`result_binding_sig_v2`/`fabric_key_id`) — fine for the v1 cost (rides
  same-trust as the tokens), but must be captured to light the `✓ cas:` marker (a tracked additive change).
  Owner-decided 2026-06-28: hold the first public `/insights` land until cost is non-zero.
- **Deployed network surface:** `hugit-serve` (`/v1` + `/readyz` + git-from-CAS upload-pack),
  MULTI-REPO serving `hugit` + `githugr` (2026-06-22). NO runner endpoint; receive-pack (push)
  **now live** (flag-on + `cas:rw` PAT on prod, 2026-06-26 — caveats a+b above).
- **Identity = dev-token stub** live; the Clerk→engine-token exchange is code-complete
  (the CoreLink exchange endpoint is live), gated on the deploy env (`HUGIT_SESSION_EXCHANGE_URL`)
  + a stale deployed image + `hugit-prod-d1`.
- **Symbol outline IS wired** (W6, #161 + follow-up): `hugit_symbols::outline_blob`
  is called from `handlers/blob.rs:181` (`compute_outline`) and the `hugit symbol --file` CLI
  verb is real. The `hugit-symbols` tree-sitter crate supports Rust/TS/TSX/JS/Python/Go/Java/C/C++/Ruby.
- **Permanently discontinued CLI verbs:** `ws`/`dispatch` only — tokens remain reserved
  for namespace protection, never implementation. `ctx resume` +
  `review` (grounded Q&A) graduated to REAL (PR-D) and `land queue` is REAL (batch land
  via the union engine, #181).
  (`fleet`/`ledger`/`watch` graduated to REAL — #157/#158; `diag` graduated —
  log-backed bisect; `policy edit` graduated — guarded `policy.change` over the
  house baseline, the append-only log IS the gate-set store.)

**Update 2026-06-18 (W3 + W5 landed — built, gate-green on `main`; live-serving still
deploy-gated):** five reserved CLI verbs graduated to REAL (no stubs): `hugit undo`
(event-sourced compensating undo, D14 Human-only — #145), `hugit policy test` (runs the
real `Engine::house()`, local≡forge — #145), `hugit verdict approve`/`hugit verdict reject`
(single-lens wrappers over `verdict::record`, serve-parity — #146), `hugit note` (appends a
`journal.note` record to the canonical log — the verb is top-level `note`, not
`journal note`; #146). File-content reads went REAL (PS-18
reversed): `hugit_proto::resolve_blob_at_path` (path→blob git tree-walk, traversal-safe —
#147) + `GET /v1/repos/{repo}/blob|edit/{*path}` serve actual file bytes, secret-scrubbed
on read, 404-no-oracle, fail-closed boot loader (#148). **All hermetically tested; blob/edit
live-serving is now LIVE via the multi-repo git-from-CAS read path (the 2026-06-22 update
below supersedes the original `HUGIT_SERVE_GIT_DIR` gate — the engine serves blob/edit from CAS,
no baked git-dir). `symbol` (W6) is wired —
`blob.rs` calls `compute_outline` → `hugit_symbols::outline_blob`; `hugit symbol --file` is real.**

**Update 2026-06-22 (multi-repo engine + killer-data + AC LIVE; F6a delivered):** the
prod engine (`engine.githugr.com`) was redeployed (staged: code, then 2nd repo; health-verified)
to hugit `main`'s **multi-repo** build — `/readyz {"git_serving":true,"git_repos":2}`, serving
both `hugit` and **`githugr`** (F6a — githugr's git closure ingested into CAS tenant `d863fafb`).
Shipped + merged this round: the **product-refinement** (Phase 1 CLI git-proximity + serve honesty
+ additive raw-int cost contract; Phase 2 capture-on-land + review legibility + `land queue`,
#179–181), the **live CoreLink AC** (memoization, #182), the **multi-repo** `AppState` (#183), and
**git-ingest/CAS hardening** (streaming `cat-file --batch` + 429-backoff + adaptive split, + the
`memmap2`→0.9.11 RUSTSEC-2026-0186 root fix, #184). **CAUTION — verified to "route serves" only:**
the killer-data reads (code-search, real diff-counts, attested cost) return `401` unauthed (auth-gate,
route present) on the deployed engine; that they RENDER real data with a session token is the githugr
TL's pending smoke (they hold the engine dev-token + run the www), NOT yet proven from here.

**Update 2026-06-26 (git push LIVE — the forge is writable):** `git push` (receive-pack) went LIVE on
the prod engine. Two waves: (1) the `cas:rw` grant + `HUGIT_SERVE_RECEIVE_PACK=1` + the CasRw write
seam deployed, but the first push was **rejected** — the `receive_pack` core unpacked via `ScratchOdb`
→ the SYSTEM `git` binary, which the `gcr.io/distroless/cc` runtime lacks (`ReceiveError::Io`); (2)
**#198** replaced it with a **pure-Rust gix-pack unpack** (mirrors the git-free read path) — an
adversarial review caught + we fixed a BLOCKER (aggregate delta-expansion bomb) before merge. Redeploy
(image `7841611c`) → the first real `git push` returned `unpack ok` + `ok refs/heads/_pushsmoke`. The
CAS handler emits `ok` only after the fail-closed `finalize_cas_push` (objects→CAS + D1 log +
refs.json/oid-index.json rewrite), so `ok` ⇒ durable. **Honest caveats:** (a) a pushed ref serves only
after the next engine reboot (in-memory snapshot not live-refreshed — a tracked follow-up); (b)
clone-back *exposure* is the forge/owner visibility decision (`repo.meta{visibility}`), not hugit code. v0 = self-contained
packs only (thin-pack bases + incremental pushes on server-side ancestry rejected fail-closed). Reads
stayed 200 throughout (no outage). Lesson: any distroless-runtime path MUST be git-binary-free.
Follow-ups shipped the same day: **#199** panic-isolated the git/SSE handlers (a handler panic no longer
crashes the single-threaded engine) + corrected every doc that still said push was 404; **#200** wired
`hugit pr land --dispatch` (real per-PR cost from the runner fabric, fail-closed/honest — live exec waits
on the runners-TL fabricd spawn fix); **#201** the **live ref hot-swap** (a pushed ref reflects in the
advertise immediately, no reboot — deployed + verified live, image `…hotswap-29ea6e5`). Two adversarial
audits of the live write path returned: **security SOUND (no unauthorized write / CAS poisoning / known
crash)**, honesty corrected. Remaining write-path follow-ups (tracked, none a live hole): thin-pack /
CAS-base reachability (incremental pushes on server-side history); an `If-Match` conditional manifest PUT
as a HARD pre-condition before ever running `max_instances>1` (today the unconditional refs.json PUT is
safe ONLY by the single-instance + single-threaded invariant).

**Update 2026-06-26b (write-path hardening — 4-agent adversarial audit + 5-fix wave):** a fresh
4-auditor brutal sweep of the live write path returned **SOUND on the security axes that matter**
(no unauthenticated write, no CAS poisoning, no `ok`-without-durability, no crash/panic DoS, the
single-writer race genuinely closed) — but found real holes, now fixed at root in one wave
(`fix/write-path-hardening-audit`, gate-green): **(1)** a **correctness defect** — the receive-pack
compare-and-append stale-check derived the current ref view from `replay(log)`, but a CAS-ingested
branch has NO `ref.update` event on the log, so **updating an existing ingested branch was
false-rejected as non-fast-forward** (push worked LIVE only for *new* branches; the deployed prod
engine still has this until this wave deploys) — fixed by passing the authoritative `git_refs`
snapshot (the advertise projection) as the stale-check view; **(2)** an authenticated resolver
**O(n²) DoS** on reverse-ordered REF_DELTAs (capped, fail-closed); **(3)** peak unpack memory lowered
to container-safe; **(4)** lazy-CAS cache poison-parity; **(5)** a bounded decoded-object cache (was
unbounded → OOM on a long-lived instance). #2(b) (a partial-manifest-commit ref wedge on a transient
R2 fault) stays tracked under the existing pre-HA `If-Match` seam — not new debt. **DEPLOYED + verified
live 2026-06-26 (engine `/readyz version 2026-06-26-write-path-hardening-9de574b`): a real force-push
UPDATED an existing branch on prod (`_pushsmoke` `75b5715`→`f40aeb1`, `ok`) — the #2a defect is CLOSED
live (previously a false `StaleRef`); the hot-swap advertised the new tip immediately; reads stayed 200
(engine + public www blob) through the cutover, no outage.** The deploy also reconciled a config-drift:
the receive-pack enablement (`HUGIT_SERVE_RECEIVE_PACK` + engine-worker forwarding + `HUGIT_SERVE_VERSION`)
was previously set via UNCOMMITTED deploy-time edits — now committed in `../githugr/engine.wrangler.jsonc`
+ `engine-worker/index.js` so the live push config is reproducible and a redeploy never silently regresses
push to 403.

**Update 2026-07-07 (engine caught up to `main` + B5 token fungibility DEPLOYED — the HA `≥2`
blocker's code+key are live; activation is githugr+clw):** the prod engine, which had drifted far
behind (`2026-07-06-b5-recover-hdrm`), was **redeployed to current `main`** (`609b8b6` →
`/readyz version 2026-07-07-b5-token-74e5c8f`, image `afa2a5ba`), carrying ~7 merged-but-unshipped PRs:
**#278** the **stateless HMAC-signed engine token** (the REAL `#128` `≥2` blocker — the session token
was an in-process `Mutex<HashMap>`, single-host → ~50% `401` at 2 instances; now `hg1_<payload>.<hmac>`,
verified constant-time + STATELESSLY, so any instance with the shared `HUGIT_ENGINE_TOKEN_KEY` accepts
any instance's token — **owner-adjudicated (b) stateless over clw's June `459e761` D1-store, because a
D1 read per authed request DoSes the single-threaded accept loop**); **#91** (operator `/v1/me/*` no
longer leaks a GDPR-erased repo's slug); **#70** blob-history index; **#74** repo.meta boot cache; **#90**
PAT/`GET /v1/me/account`; the **GDPR1 erase route** + min-grace floor; **#265** cost-killer path-B
(agent-exec seam DTOs, byte-identical). The `HUGIT_ENGINE_TOKEN_KEY` secret is SET (one wrangler secret =
identical every instance); the deployed config is committed reproducible on githugr `main` (`e0d35d5` —
drift closed, not recreated). **Cutover VERIFIED, not asserted:** polled `/readyz` — old served until the
new container warmed (**zero downtime**), new booted `probing`→`ready:true` (no crash-loop), `www.githugr.com/`
200 with real content (no wire-drift), the new-only routes (`/v1/me/account`, `/v1/me/tokens`, GDPR erase,
`/v1/repos/{r}/prs`) answer `401` (present+gated), NOT `404` (absent). **Honest gaps (NOT mine):** the
`≥2` **activation** is githugr's flip (`ENGINE_INSTANCE_COUNT=1`→`2`, `max_instances` already `2`) + the
two-key authed smoke → clw witnesses → signs off (the D1 counterpart is DROPPED); an **authed render**
verify (the killer-data / `/v1/me/*`) is the githugr TL's session-token smoke (from anon BOTH repos `404`
— the correct private-visibility posture, `RepoMeta` default PRIVATE, NOT a regression; whether `githugr`
should be anon-cloneable is an owner/forge `repo.meta{visibility}` decision). The `✓ cas:` attestation
marker's **keyset selector** (`attest_keyset::{select_attestation_key,verify_with_keyset}`, #57) is BUILT +
conformance-pinned (`attestation_keyset_selection.json`, 5 cases + anti-downgrade), but not yet WIRED into
the live `CloseResponse` (the tracked additive change; owner holds the first `/insights` land until cost is
non-zero). Deploy runbook is now proven end-to-end (CF `wrangler login` → isolated worktree staging →
`npm install` + docker → secret-forward → poll `/readyz` cutover → commit-the-bump).

**What IS genuinely live (don't under-claim it either):** the `/v1` read+write API
against `hugit` (+ now `githugr`) — 11/20 reads serve real chain-verified R2 data;
the 9 POST verbs are code-complete + R2-CAS-persisted (proven against prod R2),
`authz`-gated (the one deployed security boundary, 404-no-oracle); `hugit check`/
`verdict` are real EXECUTE paths; `hugit export` is a real zero-dependency exit-proof;
SSE replay-then-close. The engine is **lazy git-from-CAS** (boots from the CoreLink
CAS, ~5 s cold-start) and is now MULTI-REPO. **Magnitude: the single-tenant forge is now read+WRITE
live — the `/v1` API (~20–25% across 2 repos, AC-memoized) PLUS git `push` over the wire (live
2026-06-26: a pushed ref is advertised immediately via the live ref hot-swap #201, no reboot; CREATE,
UPDATE, INCREMENTAL push on server-side history, *and* DELETE all work live + prod-verified
(#2a update-fix + #206 thin-pack/CAS-base reachability, deployed+verified 2026-06-26; #208 delete-ref with
a default-branch guard, deployed+verified 2026-06-28 — 3 test refs deleted live via the receive-pack wire,
`main` guarded, advertise dropped them immediately; #209 added the `delete-refs` capability to the
advertise — deployed+verified 2026-06-28 — so **`git push --delete` works via the REAL git client**, proven
live: `git push --delete _clidel` → ` - [deleted]` and the ref dropped from the advertise. The earlier
"remote rejected" was the client refusing to send a zero-id delete because the v0 advertise omitted
`delete-refs`; the server handler always worked); remaining item — clone-back *exposure* is the
forge/owner visibility decision (`repo.meta{visibility}`), not hugit code. **The single-tenant WRITE path is fully client-usable via standard git: create + update +
incremental + delete, all proven live end-to-end.** (Op note: the engine is behind Cloudflare bot-protection —
a non-git/non-browser UA gets `403 error 1010`; probe with a `git/`/browser UA.)
**Magnitude — measured against the WEDGE, not forge-parity (git parity is a NON-GOAL; see Principles):**
the git-native surface (clone/fetch/push over the standard git wire) **rides git and is LIVE** — a stock
`git` client works, no custom client, no account to read a public repo. The wedge that ONLY hugit builds —
the landing layer, **per-intent cost-attribution (now LIT live: $20.34 real on `/insights`, first attested
non-zero cost)**, and the orchestration primitives — is largely built, with the cost-killer now rendering
real data on the live door. The honest **wedge** gaps still open: **live runner exec** (dispatch waits on
the runners-TL fabricd spawn fix), **multi-tenant hosting/identity** (bring-your-own-repo — an account
concern, *downstream* of the wedge, not the wedge itself), and the killer-data **render unverified-from-here**
(the githugr TL's authed www smoke). **Anonymous clone is NOT a hugit build gap** — it's the forge/owner
**visibility/exposure decision** (`repo.meta{visibility}`; the engine already enforces whatever flag it's
handed — see Principles). Do NOT re-grade this as "% of a forge".

**Critical path to a usable single-tenant forge (biggest → smallest) — updated 2026-06-22:**
~~deploy current `main`~~ DONE (multi-repo + killer-data + AC live) → githugr TL render-verifies
the killer-data with a token + runs the www → ~~git `push`/receive-pack~~ **DONE (#198, live
2026-06-26 with caveats a+b)** → ~~live-snapshot refresh~~ **DONE (#201, live ref hot-swap — a pushed
ref reflects in the advertise immediately, no reboot)** → ~~thin-pack / CAS-base reachability~~ **DONE
(#206, deployed + prod-verified 2026-06-26 — incremental push on server-side history lands; the
single-tenant WRITE path is now functionally complete)** → **anonymous-read exposure** (a forge/owner
**visibility decision**, NOT a hugit build step — the engine already enforces `repo.meta{visibility}`) → identity
Clerk exchange (still gated on `HUGIT_SESSION_EXCHANGE_URL` + `hugit-prod-d1`) → runner fabric live →
GitHub App + live mirror → multi-tenant. The non-code ones are owner/infra-gated. Per-capability
status table + tracked seams: the audit doc above.

**Gate + CI:** `main` green by the local gate (fmt + clippy `--workspace --all-targets
--locked -D warnings` + test `--workspace --locked` + `cargo deny`) AND runner-verified
per code push (docs-only pushes skip CI via `paths-ignore`). **CI runner: GitHub-hosted
ubuntu-latest** (re-migrated 2026-07-09 — see `crates/hugit-app/Cargo.toml` CI history).
The 2026-06-29 self-hosted macOS revert was a transient hosted-pool outage, now past.
If the hosted pool outages again (jobs completing-failure with ZERO steps), revert
one line in `.github/workflows/ci.yml:65` to `[self-hosted, macOS]`.
PR's CI is in-flight** (it starves the runner). A hosted-runner failure with empty failed-steps
is infra, not code. A `conformance/manifest.sha256` change must run the full-workspace
`hugit-invariants/x4` validator, not just the touched crate (it pins the exact VECTORS set +
two-space format). **Never claim green without a concluded `mergeStateStatus=CLEAN` + both
checks SUCCESS — never asserted by a watcher's exit.**

Read first: `docs/review/2026-06-17-honest-delivery-audit-double-checked.md` (the TRUE
state — what's live vs hermetic vs absent) · `docs/whitepaper/hugit-v1.md` (product design) ·
`docs/product/product.md` (the product brief: ICPs, killers, positioning, pricing posture) ·
`docs/adr/` (0001 context envelope · 0002 HuGR identity) ·
`docs/interop.md` (the microscopic seam map: AC/CAS · runners · GitHub · githugr) ·
`docs/plan/decomposition.md` + `docs/plan/wp-contracts/` (the 67-WP register — the LOGIC spec,
not a delivery claim) · `docs/strategy/campaign-3-llm-native-forge.md` (founding brief) ·
`docs/research/` · `docs/handoff/` (pending cross-repo work: P2 provisioning · identity rollout).

## Principles (decided, don't relitigate without the owner)

- **Don't deviate from git.** Names, CLI shape, mental model stay git-proximate.
  Every deviation costs human adoption AND LLM affinity. (This is why the
  product is "hugit", not a fantasy name.)
- **Embrace, don't assault.** Compat ladder: git wire protocol → landing layer
  riding ON GitHub → bounded bidirectional mirror → authoritative forge. A
  broken bridge kills trust instantly; never naive symmetric sync.
- **The wedge is the landing problem** (integration/merge for agent fleets),
  not authoring, not review prose.
- **Measure completeness against the wedge, NEVER git parity.** hugit is
  git-native **symbiosis**, not a forge rebuilt from scratch. What git already
  does well — clone/fetch/push, diff, branch, merge-mechanics, history — git
  keeps doing; hugit rides the git wire with **zero conflict** and adds value
  ONLY where git doesn't solve (landing, cost-attribution, orchestration). So
  "% of a forge", "git feature parity", "single-digit % multi-tenant" are the
  **WRONG ruler** — grading against them contradicts this very thesis. This was
  the recurring drift the owner killed 2026-07-03: *"não temos que fazer
  paridade… o hugit funciona em simbiose com o git… entra onde o git não
  resolve."* Grade the **complement** (is the wedge live + does it ride git
  clean?), never a GitHub clone.
- **Visibility (public/private) is the forge's decision, not hugit's.** Reading/
  cloning a *public* repo is git-native — stock `git clone`, **no account**,
  exactly like any git remote. WHERE "public vs private" is decided belongs to
  the forge surface (githugr) or to GitHub when riding on it; **hugit-serve only
  ENFORCES** the `repo.meta{visibility}` flag it is handed — it builds no
  visibility product. So "anonymous clone" is never framed as a hugit *build*
  gap; it's an owner/forge **exposure decision** (verify the deployed posture
  live before asserting anon-open OR authed-only — it's owner-set). Hard
  invariant that never bends: **read-authz ≠ write-authz** — open reads never
  open writes; `git push` always needs a credential (like git/GitHub).
- **Memoize by content, price flat.** Never usage-billing whiplash; never
  charge for the customer's own compute.
- **Zero debt, no loose ends, impeccable repo** (same owner mandate as
  CoreLink). Verify claims; never loosen rigor without an explicit waiver.
- **"Built" ≠ "delivered".** A PR merged + the gate green means the LOGIC passes
  tests hermetically — NOT that it is served, wired to real data, or live. State
  the scope explicitly; verify with a live probe before any "live"/"done" claim.
  (The over-claim that eroded trust 2026-06-17 — see the audit doc.)
- **State the family in the correct tense.** Production-state claims about a
  sibling cite that repo at the time of writing (GA notes, runbooks) — never
  memory of a design. Cautionary tale: the cross-tenant-dedup overclaim
  (`../corelink-runners/docs/review/2026-06-09-cross-tenant-dedup-claim.md`).
- **Identity is decided (ADR-0002):** one **HuGR account** on CoreLink
  machinery (Clerk · org = tenant · PATs) behind a frozen contract; **a PAT
  never reaches a browser**. No new auth service without a forcing function.

## Relationship to the HuGR family

Same primitive stack, nothing built twice:
**HuGR → CoreLink { Cache (launch) · Runners (#1) · Workspaces (#2) } →
hugit (#3, here) → githugr (#4, the forge surface)**. The CAS, AC, manifests,
tenancy, and PAT auth live in corelink-server — hugit consumes them, it does
not fork them. **Do not let hugit work leak into CoreLink's launch route or
campaign #1/#2 critical paths.**

Two incubation repos are managed FROM hugit sessions under owner-approved
fence carve-outs: `../githugr` (campaign #4) and `../corelink-runners`
(campaign #1 — its `docs/spec/hugit-integration-contract.md` is frozen from
hugit's side; **amended to v1.2.0** 2026-06-11, WP-R6+WA4: §13 adds per-job
metrics emission + transcript capture hook obligations; §13.1 money field
renamed `cost_usd|f64` → `cost_usd_micros|u64` per the WA4 integer-micro-USD
contract amendment; the frozen v1.0 §0–§12 are otherwise unchanged).

⚠️ Sibling repos — corelink-server especially, but **also the carve-outs** —
have **other live sessions/worktrees**. Never assume sole ownership; check
status/log before acting. A relayed sibling handoff may live on that repo's
`main` or another branch, NOT its checked-out tree — `git show <branch>:path`
before claiming it's missing. **Never `git commit --amend` or rewrite history in
a sibling**: another session's commit may have become HEAD between your commit
and your amend (it happened 2026-06-09; recovered via atomic ref
compare-and-swap). Fixup commits only; even in hugit, re-check `git log -1` is
yours immediately before any amend.

## The session fence (owner mandate 2026-06-05 — MECHANIZED)

It must be **impossible** for hugit work to cross other sessions' repos,
especially corelink-server. Enforcement is physical, not behavioral:

1. **`.claude/settings.json`** (this repo) carries `permissions.deny` rules
   AND a `PreToolUse` hook (`.claude/hooks/forbid-sibling-paths.py`) that
   **blocks every Edit/Write/NotebookEdit into a sibling HuGR project and
   every Bash command referencing one unless it is provably read-only**
   (fail-closed). Every session opened in this directory — and every
   subagent it spawns — inherits the fence automatically.
2. **Open hugit sessions IN `~/Documents/HuGR/hugit`** — never from a
   sibling project's directory (a session anchored elsewhere does not load
   this fence). The founding session was corelink-anchored by historical
   accident; do not repeat it.
3. The session fence (`.claude/settings.json`) is authoritative; the TechLead
   profile (`.techlead/profile`) lists a subset of the same `neverTouch` paths
   for fleet dispatch.
4. Read-only inspection of siblings (cat/grep/git log) is allowed — context
   is fine, mutation never is. Fence changes require explicit owner approval.

## Conventions

- Commits: `Signed-off-by:` trailer (DCO) + `Co-Authored-By: Claude …` trailer.
- English for all repo documents; lean, evidence-cited strategy docs (house
  style mirrors `corelink-server/marketing/expansion/`).
- Once code exists: branch → PR → merge, gates green before merge (inherit the
  CoreLink discipline). Until then, docs may land on `main`.

## Don't touch

Other projects share the parent dir (`corelink-server`, `hugr-wallet`,
`HuGR-Smith`, `HuGR-Arsenal`, `_worktrees/`, etc.). **Only work on hugit here.**
