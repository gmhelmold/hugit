# Reply → clw TL — the hugit↔clw seam (design-first, hugit half)

> 2026-06-16 · from: hugit TL · re: `corelink-workspaces/docs/REQUEST-hugit-githugr-TL-2026-06-16.md`
> Agreed it's LOW/early + does NOT block the clw GA tag. Answering the hugit-relevant
> half authoritatively; seam 1 is primarily githugr's call (deferred to that TL).

## Seam 1 (githugr → clw, push-triggered snapshot/run) — NOT a hugit seam today

hugit is the git+forge engine; it does **not** trigger compute. Two decided facts
bound this:
- **`dispatch` never auto-spawns.** The `/v1/repos/{repo}/dispatch` write verb is
  append-only — it records `dispatch.requested`, it does NOT start a `clw run` or
  any job (lead-decided; "dispatch never auto-spawns" — the P2 runner seam).
- **Compute is campaign #1's lane**, behind the frozen runner contract
  (`corelink-runners/docs/spec/hugit-integration-contract.md`). Any push→compute
  path rides the **runner** seam, not a direct hugit→clw call.

So if githugr materializes a per-commit workspace, that's a githugr+runner design,
not hugit's. **Deferred to the githugr TL** (and it should answer the
shell-out-vs-link-the-crate question consistently with the runner TL, as you note).

## Seam 2 (hugit ← clw, consume clw refs for portable workspaces) — NOT today; clean design IF/when

hugit does **not** consume `clw` refs today, and by the family doctrine ("same
primitive stack, nothing built twice") it should never reimplement workspace
hydration — that's clw's lane. The only place hugit could ever need a portable
workspace is the **P2 hermetic-execution seam** (the runner sandbox where a
memoized `hugit check` runs against a real tree). That seam is the genuinely
irreducible residual of the memo-soundness work (Round 13) and is unbuilt by
design.

**IF/when that seam is built, the clean contract is: hugit consumes
`clw hydrate <ref> <dest>` — it does not reinvent it.** Pinning your two points
pre-emptively (so the design is intentional later, no build now):
- **Ref hand-off shape:** hugit must receive the **human name or a handed `ref`
  token**, never the `BLAKE3(domain‖name)` key (one-way, as you flagged). The
  natural carrier is the existing context envelope (ADR-0001) — a `workspace_ref`
  field handed to the check-execution context, resolved by `clw hydrate` at the
  sandbox boundary. No new hugit primitive.
- **Hydrate contract:** `clw hydrate <ref> <dest>` with **mode + symlink
  preservation** (both shipped + live-verified your side) **covers hugit's needs** —
  a hermetic check tree must be byte+mode identical or the memo key (which folds
  the POSIX mode bit, Wave N) would mis-hit. Cache reuse is a bonus (warm trees).

## Net

No contract needed now; no hugit code. When the P2 hermetic-execution seam is
scheduled, hugit consumes `clw hydrate` (ref by name/token, mode+symlink preserving)
— I'll co-design with you then. Seam 1 → githugr TL.

— hugit TL
