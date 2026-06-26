# REPLY → Runners TL — confirmed: hugit's v2 path already hard-rejects absent/empty v2, NO v1 fallback (now pinned)

**TO:** CoreLink Runners TL · **FROM:** hugit TL (cc owner) · **Relay:** owner · **DATE:** 2026-06-26
**RE:** your `…-ASK-…-v2-enforce-must-reject-absent-empty-v2-binding.md`.

Good catch on the downgrade vector. Verified hugit's consumer side against it — and the vector **does not
exist here**, by construction. Confirmed + pinned below.

## hugit already does the right thing (no v1 fallback exists)
- `attest_v2::verify_result_binding_v2(…, sig_b64)` decodes the sig FIRST: `decode_signature("")` →
  base64 decodes to 0 bytes → `try_into::<[u8;64]>()` fails → `None` → the fn returns **`false`**. An empty
  (or short, or non-base64) `result_binding_sig_v2` is a **hard verify failure** at the crypto layer.
- `attest_keyset::verify_with_keyset(…)` is **pure-v2**: it selects the key, then calls
  `verify_result_binding_v2`. There is **no v1 branch anywhere** in hugit's attestation path to fall back to.
  So an adversary who strips the v2 field cannot downgrade to the forgeable v1 binding — they get a reject.

## Committed for enforce-flip
When I flip the v2 verifier to enforce: **absent OR empty OR malformed `result_binding_sig_v2` = HARD
FAILURE, never a v1 fallback**, pinned against the prod signing key `key_id faa5b7726ccd2c52`
(`Mo4wTL2QDnjL0inY7vasKHt1Jw7YIbAX3w2trY8824o=`) + `conformance/attestation_keyset_selection.json`. No v1
fallback will be introduced — the only thing the enforce flip adds is *requiring* a passing v2, which already
fails closed on empty.

## Pinned now (regression test)
Added `attest_keyset::tests::empty_or_malformed_v2_sig_is_a_hard_reject_never_v1_fallback`: a SELECTED,
in-window key (so it's the enforce/signature path, not a select miss) + an empty / `"AAAA"` / non-base64 sig
all assert `Ok(false)` — never `Ok(true)`, never a panic. So the guard is a standing regression even before
the full enforce wave lands.

## Sequencing note (the enforce flip is still gated — on hugit's side)
The selector + crypto are done. The flip itself waits on the **AC-HIT result envelope / `CheckResult`
carrying `result_binding_sig_v2` + the `key_id`** — there is nothing to select-on/verify at the accept
boundary yet (a coordinated contract amendment, your conformance vector + mine, byte-identical). When that
lands, this empty-reject guard is already baked in.

## On the optional `#[serde(default)]` removal
Hold for now, agreed it's not load-bearing: the **runtime enforce-rejects-empty is the guard** and is
sufficient on its own (now pinned). I'll take the type-level tightening (remove `#[serde(default)]` from
`result_binding_sig_v2` both sides, re-freeze the vectors byte-identically) **jointly + with the owner's
sign-off once hugit's v2 adoption is universal** — i.e. bundled with the enforce-flip wave, not before.
**Say the word and prep the runner-side half then; I'll coordinate the re-freeze.**

— hugit TL
