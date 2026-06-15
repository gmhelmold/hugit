# Handoff → githugr TL — the operator admin area (`/admin`)

**From:** hugit TL · **To:** githugr TL · **Date:** 2026-06-15 · routed via owner.

> Owner decision: hugit builds the ENGINE; githugr builds the SURFACE. The engine
> half of the operator admin area is shipped + green on hugit `main`; the screen is
> yours. This doc is what it is, the engine contract you consume, the design
> direction (hard-won this session), a starting UI draft, and what's explicitly
> OUT of scope.

## 1. What it is (and who it's for)

`/admin` = the **operator control-plane** for ONE forge: the surface for whoever
**operates/administers** a forge day-to-day. NOT the end-user dev (they use the
normal screens), and NOT the business owner (see §6 — that's a different,
out-of-scope thing).

It answers the operator's two questions: **"what needs me right now?"** and **"is
the forge healthy + trustworthy?"** — and lets them **act** on it.

## 2. The engine contract (READY on hugit `main` — consume these)

Four `/v1` reads, all pure projections over the chain-verified event log, all
Bearer-gated (PRs #123 + #124, merged):

| Endpoint | VM | Serves |
|---|---|---|
| `GET /v1/repos/{repo}/admin/overview` | `AdminOverviewVm` | one-call snapshot (queue depth, attention, campaigns, policy count, …) |
| `GET /v1/repos/{repo}/audit?since=&limit=&kind=&principal=` | `AuditVm` | paginated event timeline (who/what/when + integrity `hash_short`); raw payload NEVER echoed, `summary` is a scrubbed one-liner |
| `GET /v1/repos/{repo}/erasure` | `ErasureHistoryVm` | erasure decisions (approved + denied; exec always `pending` = X12 P2) |
| `GET /v1/admin/tokens` | `AdminTokensVm` | active engine-token sessions (sanitized; single-host) |

VMs are in `hugit-http-contracts::admin` (mirror them in `githugr-vm`).

**For the ACTIONS (the "tools/admin" half), the engine already has the write
verbs** (Wave-2, #115) — wire buttons to these:
`POST …/prs/{n}/land · …/prs/{n}/verdict · …/policy (step-up) ·
…/erasure/{id}/decide (step-up) · …/dispatch · …/undo`. Idempotency-Key mandatory;
`policy`/`erasure` are step-up-gated; see the spec §3 + the write-door.

## 3. Design direction (the hard-won lessons — please don't repeat my mistakes)

I first built it as a **report of vanity numbers** and the owner (rightly) killed it.
The two corrections that define a SOTA operator panel:

1. **It's a CONTROL plane (act), NOT a report (look).** Lead with **what needs
   action** (PRs approved-not-landed, rejected, stuck-in-queue, erasures awaiting a
   call) and let the operator **act inline** (the §2 verbs). The audit/governance
   sit below as the trust/forensics layer.
2. **No vanity KPIs.** DROP cumulative counters — "total PRs ever", "number of
   policy rules", "events in log", "erasure decisions count" are useless. Keep ONLY
   signals an operator acts on:
   - **what needs you** (approved-not-landed + rejected) — actionable
   - **queue stuck?** — the oldest queued PR + how long it's waited
   - **cache hit-rate / saved** — *the product's whole value* (memoize-by-content);
     from `check.recorded` (the `/checks` read already computes the local KPIs)
   - the genuinely-best ones (**main green?**, **$ saved / cost**) are P2-gated
     (need a live main-CI status + the live AC) — disclose honestly, never fake.

**Honesty bar (owner law):** real data or a documented honest default — NEVER a
fabricated number. The engine reads already follow this.

## 3.5 Recommended v1 layout (act-first — a concrete target, not just principles)

Top-to-bottom, by what the operator needs in that order:

1. **"Precisa de você"** (the headline — an ACTION queue, not a number). One list
   of items awaiting an operator call, each with the inline action:
   | Item | Source | Inline action → verb |
   |---|---|---|
   | PR aprovado, não-landado | `overview` / `attention` | **Landar** → `POST …/prs/{n}/land` |
   | PR rejeitado | verdict on `review`/`audit` | **Ver** → review screen |
   | Erasure aguardando decisão | `erasure` (state) | **Aprovar / Negar** → `POST …/erasure/{id}/decide` *(step-up)* |
   Empty = "tudo em dia" (a calm honest empty state, not a fake zero).
2. **Health strip** (a THIN row of only-what-matters — replaces the vanity cards):
   `fila: N · mais antigo há Xh` · `cache hit-rate: Y%` · `precisam atenção: Z`.
   That's it. (main-green + $-saved go here when their P2 seams land.)
3. **Trilha de auditoria** (read) — the trust/forensics timeline (`audit`).
4. **Governança** — policy posture with a **toggle** (`POST …/policy`, step-up) +
   erasure history (`erasure`).
5. **Sessões ativas** (`/v1/admin/tokens`) — read (revoke is P2, see §2/§6).

**Auth for actions:** every action posts with a mandatory `Idempotency-Key`;
`policy` + `erasure` require step-up (a fresh Clerk session / `X-Step-Up`). The
window renders the POST + handles the `STEP_UP_REQUIRED` / `409` envelopes (spec §3).

## 4. A starting UI draft (use as a skeleton, then rework per §3)

`hugit/docs/staging/githugr-admin-area/` has a maud screen draft + `INTEGRATION.md`
(VM → screen → route → fixture → live → hybrid, account-level `page_account`,
kit.css). It was render-verified for **layout** (the screenshot looked clean), BUT
its **KPI content is exactly the vanity set §3 says to drop** — so take the
structure/wiring as a head-start and **redo the KPIs + add the actions**. It's a
scaffold, not the answer.

## 5. Why hugit isn't building the screen

The doctrine (`hugit-session-scope-engine-only`): githugr screens are the githugr
TL's lane. (I briefly built directly in `../githugr` under an owner override; a
concurrent session's `git reset` wiped my uncommitted edits — confirming the lane
split exists for a reason. The engine half + the staged draft are the clean
hand-off.)

## 6. OUT of scope — the owner/business dashboard (parked, separate)

Mid-discussion the idea of **owner/business KPIs** came up (how many users, their
**countries**, usage volume, growth). That is a **fundamentally different surface**
for a **different audience** (the githugr-business founder, across ALL forges) —
NOT this per-forge operator panel. It is **parked**, and it needs things that do
not exist yet: a **user-analytics / telemetry pipeline** (signup events, usage
events), **geo-IP capture** (countries aren't in the event log), and — honestly —
**users** (pre-launch). When pulled, it's its own design + data-pipeline project,
not a tweak to `/admin`. Do not conflate the two.

## 7. TL;DR for the githugr TL

- Engine reads + write verbs are READY on hugit `main` (§2) — build the screen on them.
- Make it a **control plane** (act-first), not a report; **no vanity KPIs** (§3).
- **Build to the concrete v1 layout in §3.5** — "Precisa de você" action queue first,
  thin health strip, then audit/governance/sessions. Each action maps to a verb there.
- Starting scaffold in `docs/staging/githugr-admin-area/` (keep the wiring; rework the
  KPIs + add the actions).
- Business/founder dashboard is a separate parked project (§6) — not this.
