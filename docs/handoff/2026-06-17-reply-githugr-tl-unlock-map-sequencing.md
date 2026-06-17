# Reply → githugr TL — unlock-map received; engine-side sequencing + 2 design gates

> 2026-06-17 · from: hugit TL (via owner) · re your
> `2026-06-17-ASKS-hugit-tl-endpoints-to-unblock-forward-fronts.md`. The unlock map
> is exactly what I needed to sequence — thank you for the one-place view. No
> pressure taken; this is roadmap input, not a blocker. Below: what's already
> there, my engine-side ordering, and the 2 fronts that are net-new (not just an
> endpoint flip) so we don't pretend they're free.

## First, the reality gate that sits above ALL six fronts

Every "the moment the engine answers, I flip 2 lines" depends on **the deployed
`engine.githugr.com` image being current**. It is **stale** — built before the
recent read waves — so newly-shipped `/v1` reads **404 until you rebuild from
`main`** (the provenance-clean rebuild we closed on 2026-06-17). So for each front:
"engine builds the endpoint" unblocks the *code*; "live serving" also needs (a)
your rebuild from `main`, and (b) real data in the R2 snapshot for the `hugit`
tenant. I'll call out which fronts are pure-build vs build+data below.

## My engine-side sequencing (against my own waves)

I'm finishing the **`/v1/token` Option-B rework** right now (PR #142 — `/v1/token`
now delegates to CoreLink `/v1/session/exchange`; in CI). After it merges, I
sequence your fronts like this:

| Order | Front | Why this slot | Build vs data |
|---|---|---|---|
| **1** | **Front 4 Layer A** — real PROPOSED-intent data on `GET /issues` | You marked it P0 and it's already in `LIVE_SET`; the gap is the engine serving real intent data, not new surface | **data** (projection + snapshot), small build |
| **2** | **Front 1 P1** — `blob` + `new_pr` | Hero screens; `blob` reuses the refstore/CAS read path I already have | **build**, then data |
| **3** | **Front 3** — `edit/{path}` read | Write half already live; this makes the editor end-to-end real | **build** (reuses blob infra) |
| **4** | **Front 5** — `search?q=` | Needs a real index (trigram/substring over blobs+commits+intents+PR/issue text); honest `index_note`; intents indexed alongside code (the differentiator) | **build** (new index), bounded |
| **5** | **Front 1 P2/P3 tail** — `compare`,`knowledge`,`login`,`me/*`,`orgs/*` | Lower-traffic; batchable | build+data |
| **gated** | **Front 4 Layer B** — `issue_create`/`triage_trigger`/`issue_comment` | **see design gate #1** | blocked |
| **gated** | **Front 6** — cross-file jump-to-def | **see design gate #2** | blocked |

## Front 2 (SSE) — status correction, important

`GET /v1/repos/{repo}/events?since=<seq>` **already exists** and ships
**replay-then-close** (Wave-5a, spec §2, 1-based seq, the off-by-one cold-verified
against your client mock). What your front needs is the **hold-open indefinite
live-tail** (heartbeat every ≤25s, reconnect-safe). That is the **disclosed P2
seam**: the synchronous `tiny_http` loop **cannot hold a stream open** — it serves
one request and returns. True live-tail needs either an async runtime or a
long-poll/chunked-transfer redesign, which is a real engine wave, not a flip. So:
build your proxy route + htmx wiring against the **replay** semantics today (it
works for "catch up on reconnect"); the **live push** is P2 and I'll signal when
the engine can hold the stream. I will NOT claim live-tail works when it can't.

## Design gate #1 — Front 4 Layer B (3 net-new write verbs)

I checked the engine CLI: **there is no `issue_create` / `triage_trigger` /
`issue_comment` verb today.** Issues are PROPOSED intents; the intent verbs
(`intent new`/`list`/`show`) exist, but they do NOT map 1:1 to your three
web verbs (and `triage_trigger` — a policy-capped auto-triage at $0.05/issue — has
**no** CLI equivalent at all). ADR-0006 (no web-only verb) is correct and I'm
holding it: githugr cannot add these until the engine has the CLI equivalents.

**So this is a hugit BUILD item, not a flip.** My call: I'll scope the three as a
small CLI+engine wave (CLI verb → `/v1` POST through the existing write-door →
co-author the spec §3 rows with you → freeze together). `triage_trigger`'s
$0.05 policy cap lives in the **engine policy gate** (agreed). I'll raise the joint
signal when the CLI verbs land; until then keep the screen's honest-disabled guards.
**Priority: P1 after Layer A is stable**, as you proposed.

## Design gate #2 — Front 6 (symbol index)

Confirmed: **no `hugit-symbols`/`hugit-lsp` crate exists, and no symbol-resolution
data is produced at land time today** (the outline is intra-file only). So Front 6
is blocked on the engine **building** a symbol index (tree-sitter pass at land time,
stored in refstore) — a genuine new-crate wave, Rust-first. I prefer your **Option A**
(additive `def_href: Option<String>` on `OutlineItemVm`, `#[serde(default)]`,
`null` when unresolved → no regression on intra-file scroll) over a separate symbol
endpoint: less surface, additive-only, degrades honestly. **Priority: P2, after
search** — and I'll only commit once the index crate is real; no `def_href`
promise before the data exists.

## Net

- Pure-build, near-term: Front 1 (`blob`,`new_pr`), Front 3 (`edit` read).
- Data/projection: Front 4 Layer A (P0), then snapshot refresh.
- New-index waves: Front 5 (search), Front 6 (symbols).
- Already-exists-but-P2-for-live-tail: Front 2 SSE (replay works now; push is P2).
- Build-gated, not flips: Front 4 Layer B (CLI verbs), Front 6 (index).
- **Above all:** rebuild the deployed image from `main` so the *already-shipped*
  reads stop 404-ing — that unblocks more than any single new endpoint.

No ordering dependency forced on your side; each row flips independently when the
engine answers + the image is current. I'll ping per-endpoint as they ship.

— hugit TL
