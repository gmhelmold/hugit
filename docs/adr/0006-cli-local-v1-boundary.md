# ADR-0006: CLI-local v1 boundary

## Status

Accepted — 2026-09-07

## Context

hugit started as a forge-plus-runner design. The shipped product is now the
git-local CLI: capture, provenance, local memoized checks, review, and union
landing inside an existing repository. Old plans still described remote
workspace execution, context snapshots, and automatic jj observation as if
they were unfinished hugit work.

## Decision

1. `hugit` v1 does not implement `ws` or `dispatch`. Workspace lifecycle and
   off-box execution belong to `corelink-runners` or a future orchestration
   product. The tokens remain reserved so they cannot collide with git or be
   mistaken for live commands.
2. `ctx resume` is live. `ctx snap` is not part of v1. The canonical log,
   `hugit note`, and `hugit export` are the local persistence surfaces.
3. jj capture uses the explicit MCP `capture` tool. Automatic observation after
   `jj git export` is not implemented; it is an optional future design only if
   real users need it.
4. `hugit serve`, multi-tenant hosting, and GitHub App activation are separate
   remote-product or owner/infra concerns, not CLI release blockers.

## Consequences

- No local code backlog remains for these surfaces.
- Skills and docs must mark these capabilities as non-goals or external seams,
  never as silently deferred hugit implementation work.
- A future scope expansion requires a new decision and a frozen cross-product
  contract before code starts.
