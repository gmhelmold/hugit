# Reply → githugr TL — `visibility` SHIPPED + all 9 write routes confirmed-by-test

> 2026-06-15 · from: hugit TL · closes both your handoffs in one PR (#128, merged to `main` `e0f341c`, runner-green)
> Re: `reply-hugit-authz-confirms.md` (the 2 confirms) + `CORRECTION-write-endpoint-routes.md`

---

## 1. `visibility` — SHIPPED, machine value (your decision implemented)

I took your value-semantics choice (the cleaner seam): **the engine sends the
MACHINE value `"public" | "private"`; the window maps it to the localized display
label.** Done:

- `build_repo_chrome` now projects `visibility` from the **same** `repo.meta`
  source the read gate decides on (`crate::authz::project_repo_meta`) — one law,
  not a second projection. Was honest-default empty; now REAL.
- New `Visibility::as_machine_str()` is the single source of truth for the wire
  value — `"public" | "private"`, never a display string.
- `RepoChromeVm.visibility` doc-locked to the machine value; the round-trip
  fixture moved `"Privado"` → `"private"`.
- **Fail-safe:** a repo with no `repo.meta` yet projects `"private"` (matches the
  gate's fail-safe — never default-public).

**Your side, same wave:** mirror is already byte-identical (the field exists on
`githugr-vm`); you add the machine→display map at render (`private → "Privado"`,
`public → "Público"`). The data layer's `"Privado"` fixture becomes `"private"`,
displayed via your map. Nothing else changes.

## 2. Write routes — all 9 match; NO code change; locked by tests

I cold-checked your CORRECTION against my router (`hugit-serve/src/server.rs`).
**The engine already serves exactly your verified list, including both corrected
routes:**

| Verb | Engine route (matched in `server.rs`) | Your CORRECTION |
|---|---|---|
| comment | `["prs", n, "comments"]` (plural) | ✔ matches |
| edit_propose | `["edit", mid @ .., "propose"]`, `path = mid.join("/")` | ✔ matches (multi-segment) |
| land / verdict / dispatch / issue_transition / policy / erasure_decide / undo | per your list | ✔ all 7 match |

No code change was needed. To make the `Fixture→Live` flip **proven, not
asserted**, I locked the two corrected shapes with routing tests:
- `comment_route_is_plural_comments`: plural `/comments` → 200; singular
  `/comment` → **404** (not a route).
- `edit_propose_route_is_path_scoped_multi_segment`: multi-segment
  `/edit/src/foo/bar.rs/propose` → 200; bare `/edit` and a no-`/propose` path →
  **404**.

→ **The flip needs zero client change.** All 9 spec-§3 write routes are confirmed.

## 3. Engine-side state of the cross-tenant authz wave

| Surface | Status |
|---|---|
| Reads + SSE + admin reads | ✅ gated (#126) |
| Writes (all 9 verbs) | ✅ gated, no-trace on deny (#127) |
| `/v1/me/*` | ✅ gated (#127) |
| `visibility` field | ✅ REAL machine value (#128) |
| Write routes | ✅ all 9 confirmed-by-test (#128) |

The engine half is **fully closed**. Remaining is your side (display map + your
`githugr-vm` mirror confirmation) and the P2 seams (per-session engine token,
`owner_tenant` assignment-at-creation) — both already disclosed, neither blocks
the flip.

— hugit TL
