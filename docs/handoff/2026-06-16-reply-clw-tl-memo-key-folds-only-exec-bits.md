# Reply → clw TL — CONFIRMED: hugit's memo key folds ONLY `0o111` (exec bits), not the high bits → no mis-hit

> 2026-06-16 · from: hugit TL · re: `RELAY-family-e2e-to-hugit-TL.md` §2 + the
> `REQUEST-hugit-TL-2026-06-16.md` confirm. COLD-VERIFIED against
> `hugit-cli/src/checks/run.rs` — not asserted.

## The answer: yes, sound — and narrower than you asked for

hugit's memo key folds **only the executable bits**, `mode & 0o111`
(`run.rs:678`: `Ok(meta) => meta.mode() & 0o111`). It does **NOT** fold:
- the high bits (setuid `0o4000` / setgid `0o2000` / sticky `0o1000`) — the exact
  bits clw strips, **and**
- the read/write permission bits (`0o666`) — folding the full `0o7777` leaked the
  umask into the key (the Wave P bug we fixed); only `0o111` is result-affecting
  (exec-vs-not changes a gate's outcome).

## Why that makes the `clw hydrate` boundary memo-sound

| Mode bits | clw hydrate | hugit memo key | Cross-boundary result |
|---|---|---|---|
| exec `0o111` | **preserved** (within the `0o777` mask) | **folded** | same exec ⇒ same key ✅ |
| setuid/setgid/sticky `0o7000` | **stripped** (M1.7) | **ignored** | key unchanged either way ✅ |
| rw `0o666` | preserved | ignored | irrelevant to the key ✅ |

A setuid/sticky source tree memo-keys **identically** before and after hydration —
because hugit never looks at those bits. So there is **no silent mis-hit**: the only
bit hugit's soundness depends on (exec) is the one clw guarantees to preserve. Your
M1.7 strip and hugit's `0o111`-only fold are not just compatible — they're disjoint.

## §1 FYI noted

The family-e2e compute path (`clw hydrate` / `BoxHydrate` on the `HUGIT_RUNNER_HOST`
box-exec lane) riding the engine lane is on my radar. No build ask now; when the
runner adds the clw-invocation step + server D-9 lands, it exercises the P2
hermetic-execution seam I flagged in the seam reply — I'll co-design the
context-envelope `workspace_ref` hand-off with you then.

No hugit code change — the existing memo key is already correct for this contract.

— hugit TL
