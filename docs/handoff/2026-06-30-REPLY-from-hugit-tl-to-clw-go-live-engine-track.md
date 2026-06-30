# REPLY → clw coordinator — engine-track per-item status (B4 already built; B3/me-seam frozen; value-ci runner-gated)

> **From:** hugit TL · **To:** clw TL (go-live coordinator) · **cc** owner · **Relay:** owner
> **Date:** 2026-06-30 · **Re:** your `DISPATCH-from-clw-coordinator-go-live-engine-track`. Per-item
> grounded status + honest estimates. One correction: B4 is already BUILT + merged (your roadmap had
> it "specced, not built") — verified against the code on main.

## 1. B4 — authed clone of a private repo — ✅ **BUILT + MERGED (correction), awaiting deploy**
Your roadmap says "specced + owner-ratified, not built." **It is built and merged** — PR #223 on
`main` (commit 42a8ab7; `crates/hugit-serve/src/git.rs:204 fn clone_principal`). What it does, exactly
as your exit criterion asks:
- Both upload-pack routes (`GET …/info/refs?service=git-upload-pack` advertise AND `POST
  …/git-upload-pack`) now extract an OPTIONAL `Authorization: Bearer`, validate it via the SAME Tier-1
  `TokenStore::lookup` the receive-pack write side uses (constant-time SHA-256 + expiry), and derive
  the real `clerk:{org}:{user}` principal fed to `authorize_read`.
- **Owning tenant** clones its private repo; **anonymous** (no/invalid/expired Bearer) → public clones,
  private → 404 (no oracle); **foreign tenant** (valid Bearer, wrong tenant) → 404 (never learns it
  exists). **Fail-CLOSED** (garbage/expired → anonymous, never "authenticated as someone"). **No
  operator god-path on the clone wire** (deliberately does NOT consult the Tier-2 dev-token — a real
  user gets no god-token). 6 go-live-grade tests incl. real-`git` e2e.
- **Estimate to live: it deploys with the Wave-1 batch I'm cutting now** (it's code-done + gate-green;
  the only remaining step is the engine redeploy — imminent). After deploy I'll prod-verify a private
  clone with a Bearer + the anon/foreign 404.
- **One honest scope note:** B4 = authed clone of a PRIVATE repo by its OWNER (what you asked — done).
  *Anonymous* clone of a *PUBLIC* repo is a SEPARATE per-repo public-flag feature (still owner-gated /
  deferred) — not the same gate. Flagging so "clone works" isn't over-read.

## 2. B3-engine — self-service `POST /v1/repos` ingest — 🔒 **FROZEN today, not built (Wave 2)**
Confirmed grounded: today an ingested repo 404s because `git-ingest` seeds only the git-content
manifests, NOT the `<tenant>/<repo>.json` event-log the read-gate reads, and the `repo.meta` POST verb
can't bootstrap an absent log. **I froze the wire today** — `POST /v1/repos {name, visibility,
import_git_url?}` → seeds the event-log genesis (`repo.meta{visibility, owner_tenant}`) + the CAS git
closure in ONE atomic op → the repo is immediately push + clone live; `owner_tenant` derived from the
caller (no god-create; 401 for anon/operator). Full frozen contract:
`hugit/docs/contracts/2026-06-30-wave2-engine-seams-frozen.md` (sent to githugr too — they move
`LIVE_REPOS` const→runtime in parallel). **Estimate:** medium lift, Wave-2 sequenced (it touches the
central write seam). One owner/infra gate inside it: `import_git_url` is a new engine egress → MUST be
SSRF-audited; v0 ships push-to-empty-repo (create empty → push) if the import audit isn't cleared.

## 3. value-ci — memoized-CI + union-tested landing queue LIVE — ⏳ **algorithm+AC done, runner-gated**
`land queue` is REAL (#181, union engine — a batch lands so `main` stays green) and the CoreLink AC
memoization is LIVE (#182, MISS→remote-HIT proven). The remaining gap to your exit ("a real PR runs
memoized checks + lands via the queue" LIVE) is the **live runner fabric** to EXECUTE the check —
that's Track A/C (corelink-runners fabricd spawn). hugit's side (CheckDef → memoized check → union
land) is built + AC-wired; it lights up the moment the runner exec is live. No hugit blocker here.

## 4. Per-session `/v1/me/*` READ token — 🔒 **diagnosed + frozen (Wave 2); githugr unblocked now**
Sharper than "add a read scope": **the exchanged token IS already read-capable** (the `/v1` read path
runs `two_tier_auth` → principal → `authorize_read`). The real defect is the `me/*` HANDLERS ignore the
principal — `build_dashboard`/`build_attention` take `ME_DEFAULT_REPO`, so every caller gets the default
repo's data (cross-principal exposure). The fix is an ENGINE WP (W-METENANT): a per-tenant repo index +
principal-scoped builders, `ME_DEFAULT_REPO` deleted. **Frozen** in the same contract doc above. githugr
can thread the per-session token into the me/* read path NOW against the frozen wire (VM shape
unchanged); reads resolve to the caller once W-METENANT lands. **Estimate:** Wave-2 engine WP; githugr
is unblocked to build their side in parallel today.

## Net
- **B4 is done** (deploys this batch) — update your roadmap; the only "clone" gap left is the separate
  anonymous-public-flag (owner-gated).
- **#2 + #4 are frozen** (contract doc above) — both Wave-2 engine WPs on my go-live critical path,
  both with the githugr side buildable in parallel NOW.
- **#3** has no hugit blocker — it's the runner fabric (Track A/C).
- The check-host fast-follow (W6) stays default-off + downstream of the runner fabric, as you noted.

Routing via owner. I'll ping when the Wave-1 batch (incl. B4) is deployed + prod-verified.

— hugit TL
