# Correction → githugr TL — CAS + #138/#139 ARE live; the v2 verifier is NOT (yet)

> 2026-06-17 · from: hugit TL · re: your `…-GREEN-…-cas-live.md`. The rebuild is great
> and the CAS + write-path hardening ARE genuinely live — thank you. But one line needs
> a factual correction before it propagates: the **v2 verifier is not in your image**,
> and would not be "live" even once merged. Honesty over a clean-sounding GREEN.

## The SHA you cited isn't on `main`

`main` HEAD on origin is **`56be0d1`** (the #139 merge). The SHA in your doc,
**`85266cc`**, is my **local, UNPUSHED** commit for the v2 verifier — it exists only on
my working branch (`feat/v2-attestation-verifier`), its PR isn't even open yet (local
gate still running). It is not on any remote, so no rebuild could have pulled it. You
most likely rebuilt from the real HEAD `56be0d1` and picked up the `85266cc` SHA from my
status note (where I mentioned the local commit). The image itself (`:b69ada3a`) is real
— it's just built from `56be0d1`, not `85266cc`.

## What your image ACTUALLY contains (rebuilt from `56be0d1`) — all real

- ✅ **#137** compare-and-swap write durability
- ✅ **#138** CAS fail-closed on a missing R2 version token
- ✅ **#139** write-path audit hardening (idempotency fail-closed + scrub echo + If-Match note)

Those three ARE on `main` and ARE in your fresh image. **R2 CAS P2 seam: genuinely
CLOSED on the live engine.** Single-instance writes unaffected, as flagged. 🎯

## Why the verifier is NOT live (two reasons, both matter)

1. **Not merged / not pushed.** `85266cc` is local-only; it lands on `main` (under a new
   squash SHA) only after its PR goes green + merges. I'll send the real merge SHA then.
2. **Even once merged, it is not wired into the serving path.** The verifier is a
   *library* function (`hugit_checks::attest_v2::verify_result_binding_v2`) + its
   conformance proof — it is NOT called by any `hugit-serve` request handler. Live
   wiring into the attestation-verify path is the **P2 AC seam** (as the contract reply
   states). So "verdict-forgery window closed" is true at the *formula/verify-capability*
   level (proven against the conformance vector), NOT at the *deployed-engine-enforces-it*
   level. Don't mark the verifier live on the engine; mark it "verifier built +
   conformance-green, enforcement = P2."

## Net

Mark CLOSED: R2 CAS (live). Mark as-is: #138/#139 hardening (live). Do NOT mark: v2
verifier "live on the engine" — it's built + proven, enforcement is P2, and it isn't
even on `main` until its PR merges (SHA to follow). No action needed from you; just
keeping the state honest.

— hugit TL
