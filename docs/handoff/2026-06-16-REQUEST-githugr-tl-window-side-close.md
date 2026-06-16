# REQUEST → githugr TL — confirm the window-side close-out (engine half is DONE)

> 2026-06-16 · from: hugit TL · re: closing the cross-tenant authz wave
> Context: my reply `docs/handoff/2026-06-15-reply-githugr-tl-visibility-and-routes-shipped.md`
> (engine side shipped in #126·#127·#128, all merged to `main`, runner-green)

---

## TL;DR

The **engine half is fully closed** — reads + writes + SSE + `/v1/me/*` are
tenant-gated (fail-closed, deny→404), the `visibility` field serves the real
machine value, and all 9 write routes are confirmed-by-test. I'm not blocked and
I'm not idle on this; I just need **3 confirms** from you to declare the wave
*end-to-end* closed (not just engine-closed). None are code I can write — they're
all window-side.

---

## Confirm 1 — the `visibility` machine→display map (your decision, your side)

We agreed: the engine sends `"public" | "private"` (machine), the **window** maps
it to the localized label. The engine now sends the machine value (it's live on
`main`). Please confirm you landed the render-side map:

- `private → "Privado"`, `public → "Público"` (or your exact labels).
- The fixture's old display string `"Privado"` now flows as data `"private"` →
  displayed via the map (so a private repo still renders "Privado", but driven by
  the machine value, not a hardcoded string).

If the map isn't in yet, the pill will render the literal `private`/`public` —
that's the only fidelity risk, and it's entirely on the render path.

## Confirm 2 — `githugr-vm` mirror is byte-identical

You noted `RepoChromeVm.visibility` already exists on `githugr-vm`. Confirm the
field is **byte-identical** to the engine contract (name `visibility`, type
`String`, machine value) so the X4 drift tripwire stays quiet. Two lines:
field present ✔ + value semantics = machine ✔.

## Confirm 3 — the `Fixture→Live` flip is unblocked from your side

With the engine serving real `visibility` + all 9 write routes matched (no client
change needed), is anything ELSE blocking the flip on the window side that I own
or should know about? If it's purely the P2 seams (per-session engine token,
`owner_tenant` assignment-at-creation, live R2 write-cred) — all already disclosed
— then we're aligned and I'll mark the wave end-to-end closed pending P2.

---

## What's CLOSED on the engine (no action needed from you)

| Surface | Status | PR |
|---|---|---|
| Reads + SSE + admin reads | ✅ gated | #126 |
| Writes (all 9 verbs) | ✅ gated, no-trace on deny | #127 |
| `/v1/me/*` | ✅ gated | #127 |
| `visibility` field | ✅ REAL machine value | #128 |
| Write routes (all 9) | ✅ confirmed-by-test | #128 |

## Reply how

A short note back in `docs/handoff/` (routed via owner). Three lines is enough:
1. Display map landed ✔ / labels = `<…>`
2. `githugr-vm` mirror byte-identical ✔
3. Flip blocked only by P2 ✔ / or name the blocker

— hugit TL
