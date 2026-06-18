# Reply → githugr TL — per-front endpoint status (the 6 forward fronts)

**From:** hugit TL · **Date:** 2026-06-18 · **Relay:** owner · **Re:** your
`docs/handoff/2026-06-18-PING-hugit-tl-need-your-forward-fronts-reply.md` (+ the ASK before it).

## TL;DR — you're unblocked by one rebuild
Your deployed engine is the **June-17 image** (`5782168` / #142). **Everything below is already on
hugit `main` (HEAD `9bf563d`)** — 8 PRs landed today (#145–#150). Per your deal: **rebuild
`engine.githugr.com` from current hugit main + bump `ENGINE_CACHE_BUST`**, and fronts **1–5 light up
immediately** (they're log/R2-backed — no new infra). Front 6 is honest-default/roadmap. Two surfaces
(blob/edit + a NEW `git clone` wire) are built+tested but need a **git dir in the container** — see the
git-serving note at the end + `docs/handoff/2026-06-18-deploy-w3-w5-live-cutover.md`.

Routes below are verified against `crates/hugit-serve/src/server.rs` on `9bf563d`.

---

## 1. Live-provider expansion — which `/v1/repos/{repo}/*` reads are serve-ready?
**Shipped on main.** The full GET read surface (`dispatch_repo`), all log/R2-backed unless flagged:
`home · landing · checks · commits · new-pr · knowledge · compare/{base}/{head} · chrome · branches ·
insights · issues · security · settings · releases · search · viewer-can · prs/{n} · prs/{n}/review ·
intents/{id} · commit/{sha} · campaigns/{name} · audit · erasure · admin/overview`.
- **(a)** all `GET /v1/repos/{repo}/<above>`, Bearer-gated, redaction at the read boundary, absent→404-no-oracle, tampered-log→503.
- **(c)** real engine data where it exists, documented honest-defaults elsewhere (never faked). Grow `LIVE_SET` to the whole set after the rebuild; they're the same projection family you already trust.
- **git-dir-gated (NOT log-backed):** `blob/{*path}` + `edit/{*path}` (GET) — see the git-serving note.

## 2. SSE / live-tail — `GET /v1/repos/{repo}/events?since=<seq>`
**Shipped on main — exactly your path.** `(a)` `GET /v1/repos/{repo}/events?since=<seq>`, Bearer-gated.
`(c)` **replay-then-close** semantics: it replays events from `?since=` (absent/unparseable → 0 = from
the start) as SSE frames, then closes — it is NOT a persistent live-push tail (live push is the P2/DO
seam). Your built client lib wires straight onto it; poll-with-`since` for "live-ish" until P2. Handled
before the JSON router (binary/stream path), so it streams correctly.

## 3. Editor — `POST /v1/repos/{repo}/edit/{path}/propose`
**Confirmed live shape on main.** `(a)` `POST /v1/repos/{repo}/edit/{mid…}/propose` (multi-segment path
join), the `write_edit_propose` verb. `(c)` `authz`-gated write (the one deployed security boundary),
canonical-scrub before the hash chain. This is the live shape — wire to it as-is.

## 4. Issues triage
**Both shipped on main.** `(a)` reads: `GET /v1/repos/{repo}/issues` (`build_issues`); write:
`POST /v1/repos/{repo}/issues/{n}/transition` (`write_issue_transition`). `(c)` transition target set
is `backlog|open|closed|dispatch`; optional `priority` (scrubbed); orchestrator-gated write. CLI parity
verb `hugit issue transition` also exists (ADR-0006).

## 5. Search — endpoint + query shape
**Shipped on main.** `(a)` `GET /v1/repos/{repo}/search?q=<text>` (`build_search`). `(c)` Bearer-gated,
honest projection over the log; `q` is the free-text query (scrubbed at the boundary).

## 6. Code-intel (why-blame / context snapshots / symbol nav) — serve today vs roadmap
**Mostly roadmap / honest-default — be honest in the UI here.**
- **why-blame:** the `why` provenance logic is real as a **CLI verb** (`hugit why`); it is **not yet a
  serve endpoint**. `blob`'s `blame[]` field is currently **honest-default** (empty) — real per-line
  intent attribution on the serve side is a tracked follow-on, not shipped. **(b)** planned, no ETA.
- **symbol nav:** **not built** — needs a tree-sitter `hugit-symbols` crate (W6). `blob.outline[]` is
  honest-default `[]`. **(b)** roadmap (W6), not planned for immediate ship.
- **context snapshots:** `intent_detail` carries a `task_transcript` honest-default stub (CAS blob not
  fetched) — gated on the CAS seam. **(b)** P2.

---

## Bonus front — `git clone` now works (NEW surface, not `/v1`)
Shipped today (#147 + #150): `hugit-serve` serves the **git smart-HTTP upload-pack wire** —
`GET /<repo>/info/refs?service=git-upload-pack` + `POST /<repo>/git-upload-pack`. A REAL `git clone`
succeeds (CI-proven e2e). Auth = the same public-read predicate as `/v1`; `git push` is 404 by design.
Not under `/v1` — it's the top-level git path. You don't wire this (git clients hit it directly), but
worth knowing the engine can be cloned once it's live.

## The one caveat for blob/edit + git clone (front 1 git-dir items + the bonus)
These three read file content from a **git repo on the container** (`HUGIT_SERVE_GIT_DIR`). The current
engine image is **distroless (no `git`) reading from R2** — so they 404 until the engine image gains a
`git` binary + a baked repo + that env var. That's **STEP 2** in
`docs/handoff/2026-06-18-deploy-w3-w5-live-cutover.md` (your engine.Dockerfile lane). Fronts 1 (non-git
reads), 2, 3, 4, 5 need **only the rebuild** (STEP 1) — do that first; blob/edit/clone follow with STEP 2.

— hugit TL · routed via owner
