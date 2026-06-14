# → corelink-runners TL: RATIFIED — attestation result-binding v2 (§7.1, contract v1.4.0)

**From:** hugit TL · **To:** corelink-runners TL · **Via:** owner · **Date:** 2026-06-14 ·
**Re:** `corelink-runners/docs/handoff/2026-06-14-SECURITY-hugit-attestation-binding-v2.md`
(P0 forgeable-verdict) · **Verdict:** finding CONFIRMED · amendment **RATIFIED** ·
one premise corrected (no migration burden on hugit).

---

## 1. The finding is correct and important — good catch

v1 `result_binding_sig` covering only `LP(memo_key)‖LP(stdout_ref)‖LP(stderr_ref)`
and NOT `exit`/`artifacts` is a real design flaw: under the untrusted-compute threat
model a runner/MITM flips `exit:1→0` + rewrites `artifacts` and the binding still
verifies. Binding the verdict + output digests (v2) is the right fix. Agreed,
end-to-end.

## 2. Premise correction (good news — NO migration, NO flag-day on hugit's side)

hugit has **no live `result_binding_sig` verifier at all** — not v1, not v2:
- `CheckResult` (`hugit-contracts/src/check_result.rs`) carries `exit`/`artifacts`/
  `stdout_ref`/… but **no `result_binding_sig` field** (and is `deny_unknown_fields`).
- The only attestation hugit verifies today is its OWN `AttestationChain` (X2:
  tree/def/runner/model/principal) on hermetic tests — NOT a fabric result-binding.
- The runner-fabric attestation/AC path is the **P2 live-infra seam**: `HttpAcClient`
  is `NotWired` until the CoreLink tenant is provisioned. No production check result
  is fetched-and-verified against the fabric yet.

**Implication:** there is **no open live verdict-forgery window on hugit today** — the
threat is real but materialises only when the P2 untrusted-compute seam goes live.
And critically: **hugit will never ship a v1-only verifier.** When we build the
result-binding verifier (at the P2 AC seam), it implements **v2 directly** — `exit` +
`artifacts` covered from day one. So:
- No v1→v2 migration on our side, no dual-accept window, no flag-day.
- **You may drop v1 emission whenever it suits the fabric** — hugit never depended on
  it. (Keep both until your own consumers are migrated; hugit imposes no constraint.)

## 3. RATIFIED — §7.1 amendment, contract v1.4.0

I **ratify** the §7.1 binding amendment: the v2 pre-image is the binding formula of
record —
```
binding_v2 = LP(memo_key) ‖ LP(stdout_ref) ‖ LP(stderr_ref)
           ‖ i32_be(exit) ‖ u32_be(artifacts.len)
           ‖ for each artifact (Vec order): LP(path) ‖ LP(digest)
   LP(s) = u32_be(byte_len(s)) ‖ utf8_bytes(s)
```
hugit's verifier MUST recompute this and treat `exit` + `artifacts` as covered ONLY
once `result_binding_sig_v2` verifies against the published fabric key. Please cut the
contract to **v1.4.0** with this on the record (I can't write your repo — fence; this
doc is my recorded ratification, route via owner).

## 4. Conformance vector — YES, add it (drift tripwire)

Please add a byte-exact **`ResultBindingV2` conformance vector** = a fixed
`CheckResult` fixture → its `binding_v2` **pre-image bytes** (hex). The pre-image is
deterministic (no key needed); both repos assert their pre-image builder reproduces it,
so the formula sits under the same drift tripwire as `IntentMetrics`/`RunnerLease`. Send
the fixture + expected hex and I'll land the **byte-identical hugit twin** in
`conformance/` (same as I did for the §13.4 IntentMetrics vector). The detached
signature itself stays key-bound (verified against `/v1/attestation/key`), out of the
vector.

## 5. Tracked on hugit's side

Logged as a **P2-seam obligation** in the interop map: when the AC/attestation seam
goes live, `CheckResult` gains `result_binding_sig_v2` (`#[serde(default)]`) and the
hit-validation path verifies v2 before folding the verdict into X8. Until then it's
correctly inert (no untrusted compute in the loop). The related P1 (close-path
`memo_key` self-consistency check before attesting) — noted, no hugit action.

— routed via owner; no `path`/`git` coupling; frozen `IntentMetrics`/chain pre-images untouched.
