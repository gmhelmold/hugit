# MCP catalog — the servers a hugit agent may wire

> **Honesty caveat (read first):** this catalog names the surfaces a hugit-driving agent reaches.
> A surface listed as **LIVE** has been verified end-to-end against the deployed engine; one listed
> **DEFERRED** is built-but-not-served or transferred, and an agent must NOT assume it. A `401`/`404`
> from any `/v1` route is the auth/visibility gate — **not** proof the route is absent (the honesty
> law, `hugit/SKILL.md` Section 4). Ground truth for the API shape is `crates/hugit-serve` +
> `crates/hugit-http-contracts`; the live-vs-deferred state is the honest delivery audit
> (`docs/review/2026-06-17-honest-delivery-audit-double-checked.md`).

hugit does not (today) ship a bespoke MCP server; an agent reaches the forge through three real
surfaces. This catalog tells the agent which server/transport to wire for each, the auth it needs,
and the honesty caveat that governs what a response *means*.

---

## Section 0 — Catalog at a glance

| # | Surface | Transport | Auth | State | Use it for |
|---|---|---|---|---|---|
| 1 | The hugit CLI (local `--log`) | the `hugit` binary (stdio) | none (a local file) | **LIVE** | the full verb table — campaign/intent/pr/check/verdict/ledger/… over a local log |
| 2 | The `/v1` engine read API | HTTPS (`/v1/...`) | Bearer (session token / dev-token) | **LIVE (auth-gated)** | chain-verified reads: repos, blob/edit, code-search, diff-counts, fleet, SSE |
| 3 | The `/v1` engine write API | HTTPS POST (`/v1/...`) | Bearer (`authz`-gated) | **LIVE (single-tenant)** | the 9 POST verbs (intent/pr/verdict/… persisted to R2-CAS) |
| 4 | git over the wire (upload-pack) | git smart-HTTP | Bearer (owner) / anon (public) | **LIVE (owner-clone)** | `git clone`/`fetch` — owner-authed clone of a private repo; anon = public-only |
| 5 | git over the wire (receive-pack) | git smart-HTTP | `cas:rw` Bearer | **LIVE (push)** | `git push` — create/update/incremental/delete a ref (single-tenant) |
| 6 | The runner fabric (off-box exec) | corelink-fabricd | runner PAT | **EXTERNAL / OUTSIDE CLI V1** | live agent dispatch + real per-PR cost — NOT served from hugit |

---

## Section 1 — The hugit CLI over a local log (LIVE)

The first-class surface. Every verb in `HUGIT_VERBS` runs locally against a canonical, hash-chained
`--log`. No network, no auth — just stdio + a file. This is what `/hugit-worker` executes by default.

- **Wire it as:** the `hugit` binary invoked over stdio; the result is the one JSON envelope on
  stdout (`hugit/SKILL.md` Section 3).
- **Auth:** none (local file ownership).
- **Honesty caveat:** the `--log` is the source of truth; a `log_not_found`/`parse_log` error is
  REAL (never an empty world). A `not_implemented` stub is honest, not a failure.

---

## Section 2 — The `/v1` engine API (LIVE, auth-gated)

`hugit-serve` (deployed at the engine host, e.g. `engine.githugr.com`) speaks the same logic over
HTTPS. It is **multi-repo** (serves `hugit` + `githugr`) and **lazy git-from-CAS** (boots from the
CoreLink CAS). The githugr window and remote agents read it.

- **Wire it as:** an HTTP MCP server (or direct HTTPS) to `/v1/...`; health is `/readyz`.
- **Auth:** `Authorization: Bearer <token>`. A per-session token comes from `/v1/token` (the
  CoreLink session exchange); a dev-token stub is the current identity. **A PAT never reaches a
  browser** (ADR-0002).
- **Reads (LIVE):** ~11/20 read routes serve real chain-verified R2 data — repo listing, `blob`/
  `edit` file bytes (secret-scrubbed on read), code-search, diff-counts, `fleet`, plus SSE
  (replay-then-close).
- **Writes (LIVE, single-tenant):** the 9 POST verbs are R2-CAS-persisted, `authz`-gated (the one
  deployed security boundary, 404-no-oracle).
- **Honesty caveat (critical):** an **unauthed `401` is the Worker auth-gate, not route-absence**;
  a `404` on a private repo is the visibility gate (404-no-oracle). To prove a route is live, use a
  PAT-authed `200`-vs-`405`, or verify from the **real consumer** (the public www that decodes the
  engine VM), NEVER an engine-direct privileged proxy. The killer-data reads (code-search, diff,
  cost) RENDER real data only with a session token — that render is verified by the githugr TL, not
  asserted from a status code.
- **Op note:** the engine is behind Cloudflare bot-protection — a non-git/non-browser UA gets
  `403 error 1010`. Probe with a `git/` or browser UA. Do not casually probe heavy reads
  (code-search) against the single prod engine — it is single-threaded and a heavy read can wedge
  the accept loop.

---

## Section 3 — git over the wire (LIVE)

The engine serves the git smart-HTTP wire over the same CAS.

- **Clone/fetch (upload-pack):** `git clone`/`fetch`. **Owner-authed clone of a PRIVATE repo is
  LIVE** (an `Authorization: Bearer <owner-token>` derives the `clerk:{org}:{user}` principal).
  **Anonymous = public-only** (private → `404`, no oracle); a **foreign tenant** → `404`
  (cross-tenant isolation). Fail-closed: a malformed/expired token degrades to anonymous, never to
  "authenticated as someone". No operator-bypass on the read wire (no god-principal over clone).
- **Push (receive-pack):** `git push` is **LIVE** (single-tenant, `cas:rw` PAT). CREATE, UPDATE,
  INCREMENTAL (on server-side history), and DELETE all work via the standard git client; a pushed
  ref is advertised immediately (live ref hot-swap, no reboot). `ok` ⇒ durable (objects→CAS + D1
  log + manifest rewrite committed before `ok`).
- **Honesty caveat:** the remaining single-tenant READ gap is **anonymous clone of a private repo**
  (owner-gated on the public-flag, deferred). Push is safe ONLY by the single-instance +
  single-threaded invariant today (an `If-Match` conditional manifest PUT is the tracked
  pre-condition before `max_instances>1`).

---

## Section 4 — The runner fabric (DEFERRED — P2)

Off-box agent execution (the compute that runs a check/merge-as-re-execution and produces a real
per-PR cost) lives in **`corelink-runners`** (campaign #1), reached via `corelink-fabricd`. hugit
consumes it across the frozen lease-DTO contract; it does NOT fork or serve it.

- **State:** the cost WIRE is proven live (acquire→§13 ingest→close, attestation signed), and the
  four lease DTOs are frozen byte-identical across both repos (conformance vectors + tripwire). But
  **live agent dispatch (merge-as-re-execution) is external** — `hugit pr land --dispatch` records
  the demand and submits `cost_usd_micros: None`.
- **Honesty caveat (the cost law):** a per-PR cost is **honest-zero (`null`)** until a real
  provider-`/usage` source feeds it. An agent MUST NOT substitute a derived `IntentMetrics` COGS or
  a demo figure — a misattributed real number is the same violation as an invented one
  (`hugit/SKILL.md` Section 4.2). Do not wire this surface as if it returns a live cost.

---

## Section 5 — Wiring discipline

| Rule | Why |
|---|---|
| Prefer surface #1 (local CLI) for the flow verbs | no auth, no network, deterministic; the worker's default |
| Use #2/#3 only with a real session token | an unauthed call returns `401` (the gate) — meaningless as a liveness probe |
| Treat every `401`/`404` as auth/visibility, never route-absence | the honesty law (Section 4 of `hugit/SKILL.md`) |
| Never wire #4 (runner fabric) as a live cost source | cost is `null` until a real provider-`/usage` source exists (P2) |
| Probe the engine with a `git/`/browser UA | Cloudflare bot-protection returns `403 1010` to other UAs |

---

## Section 6 — Change log

| Version | Date | Change |
|---|---|---|
| 0.1.0 | 2026-06-30 | Initial creation (WP W-SKILLS). The six surfaces an agent reaches (local CLI · `/v1` reads · `/v1` writes · clone · push · the deferred runner fabric), each with its auth + the honesty caveat (401/404 ≠ route-existence; cost null-until-real-source). Grounded on `hugit-serve` + the honest delivery audit. |
