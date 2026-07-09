# ADR-0003: Owner-scoped repo routing is deferred until cross-tenant public discovery ships

Status: Accepted (scoped deferral)
Date: 2026-07-09
Context-of: G11 (hugit-serve PR #298 — user-scoped repo slug namespace)

## Context

G11 user-scoped the stored repo slug to `<owner_tenant>/<name>` to close a
name-squatting vector and a create-time 201-vs-409 existence oracle. To avoid a
migration, the entire request surface still addresses a repo by a **single-segment
bare name**, resolved per-caller: `AppState::resolve_repo_slug(name, principal)`
maps a bare name to the *caller's* `<tenant>/<name>` key, falling back to the legacy
flat key on a miss. This is uniform across the git smart-HTTP wire
(advertise/upload-pack/receive-pack), the `/v1` read + write dispatch, the SSE
stream, and the githugr UI (`/r/{repo}`, all single-segment).

Consequence (G11's flagged limitation): a NEW (post-G11, scoped) **public** repo is
addressable by bare name ONLY by its owning tenant. A composite `<owner>/<name>`
identity is not routable over the single-segment wire, so an anonymous or
cross-tenant caller cannot reach another tenant's new public repo by URL. Legacy
(pre-G11, flat-slug) public repos — including the dogfood showcase set — are
unaffected.

## Decision

Defer owner-scoped `/{owner}/{repo}` routing. Keep the single-segment,
per-caller-resolved addressing shipped in G11.

Rationale — this is future-work, not a go-live gap:
- The open-beta CoreLink Workspaces model is each user's OWN isolated
  private/workspace repos. No launch flow requires addressing another tenant's
  user-created repo by URL.
- githugr surfaces NO cross-tenant public repo discovery: `/r/{repo}` is
  single-segment; there is no `/{owner}/{repo}` route; `/explore` features a single
  showcase repo, not a public index; a cross-tenant `list_public_repos` does not
  exist (it is explicitly forward-compat, unwired).
- The only cross-tenant public display is the fixed showcase carve-out
  (`GITHUGR_SHOWCASE_REPOS`, dogfood `hugit`/`githugr`), served via the operator
  provider over LEGACY-FLAT slugs — reachable, and unaffected by G11.
- Every other repo is read as the caller (anonymous → public 200 / private 404),
  so isolation is already correct; nothing is leaking and nothing addressable was
  lost.

## When to revisit (the trigger)

Reopen this ADR when cross-tenant public repo **discovery** becomes a product goal —
concretely, when a `list_public_repos` cross-tenant index is specified. At that
point the routing is a coordinated M–L change across three seams, sequenced behind
the index feature (not before it):
1. git wire URL router — parse `/{owner}/{repo}.git` → composite slug
   (`is_git_path`, `respond_git`, `is_receive_pack_path`).
2. `/v1` API — composite-key or `/v1/repos/{owner}/{repo}` read/write/SSE dispatch.
3. githugr — `/r/{owner}/{repo}` routes + the `repo_from_{read,write}_path`
   extractors + link/`clone_cmd` builders.
Resolution stays backward-compatible: bare single-segment addressing must keep
resolving the caller's own + legacy repos unchanged (no migration).

## Consequences

- No code change now; zero regression surface on the just-shipped live forge.
- A new public repo remains fully usable by its owner (browse/clone/push by bare
  name) and correctly private to everyone else — the intended beta behavior.
- The composite `<owner>/<name>` identity is durably stored (G11), so the future
  routing is additive over data that already exists — no re-slug/migration debt is
  accruing while deferred.
