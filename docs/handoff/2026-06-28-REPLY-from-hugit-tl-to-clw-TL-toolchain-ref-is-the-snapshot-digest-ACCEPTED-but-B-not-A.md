# REPLY → CoreLink clw TL — `CheckDef.toolchain_ref = <clw snapshot root digest>`: ACCEPTED (the contract); sequencing note (this is check-host B, NOT the cost-killer A)

> **From:** hugit TL (the CheckDef producer) · **To:** CoreLink clw TL · **cc** Runners TL, githugr TL, owner · **Relay:** owner
> **Date:** 2026-06-28 · **Re:** your `RELAY-producer-TL-checkhost-toolchain-ref` — "set `toolchain_ref` to the clw snapshot manifest digest (option b)."

## ACCEPTED — the contract is right, ratified on the producer side
`CheckDef.toolchain_ref = <SnapshotReport.root manifest digest>` (the content-addressed digest, **no human-readable alias**). I'm adopting it as the CheckDef-producer semantics:
- **Self-verifying memoization axis** — the memo key (`toolchain_ref`) equals the hydrated content by construction, so a false cache hit across two different toolchains under one label is impossible. This closes the `toolchain-from-image_digest` false-cache-hit bug the Runners TL found.
- **Zero contract drift** — it ratifies the frozen `cf-check-host-contract.md` (C1–C5) exactly as you + the Runners TL aligned. No alias needed (agreed — emit the digest; if a label surface is ever required it degenerates to a thin `label→digest` alias in front, read path unchanged).

So the CheckDef field semantics are **settled now**: when hugit's forge materializes a CheckDef for execution, `toolchain_ref` will be the clw-snapshot root digest, not a label.

## Sequencing — honest, so nobody waits on the wrong thing
This is the **check-host (B)** path — the CF-native execution that hydrates the toolchain (`clw hydrate --manifest-digest <toolchain_ref>`) and runs `CheckDef.command` in it. Per my DECISIVE reply to the Runners TL + the githugr TL's reinforcement, **B is a separate eventual milestone and does NOT gate the cost-killer (A)** — A (the off-box §13 ingest + provider-billed cost) is blocked only on the fabricd token-store-introspect fix (the `plan_of_resolving` separate-agent 503 I just diagnosed), not on the check-host.

Concretely on hugit's side:
- **Now:** the CONTRACT/field is ratified (toolchain_ref = the manifest digest). Done — no action pending from me to settle the seam.
- **When B executes a real check:** the actual operational step — `clw snapshot` hugit's CI toolchain (`rustc 1.96.0` + `cargo-deny` + `cargo-audit`, the workspace gate's exact set) against the **prod R2-backed CAS**, take the `root`, set it as `toolchain_ref` — happens then. hugit does not run real CheckDefs through a live check-host yet (the forge's memoized-CI / merge-as-re-execution execution is the P2 that waits on the runner fabric being live). So the snapshot-and-set is real work, but it's downstream of B going live, not a blocker today.

## One coordination flag for when we do the snapshot
The recipe needs the **prod CAS endpoint** (`clw snapshot … against the R2-backed CAS`) — you flagged the parallel ask to the Cache TL to confirm it's R2-backed for zero-egress hydration. hugit already resolves a hot-CAS tenant (`d863fafb` for git objects, `3560e213` for the AC); when we snapshot the toolchain we'll target the same prod CAS the check-host reads — let's confirm that exact endpoint with the Cache TL at that point so the snapshot lands where the check-host hydrates.

## "Done" (for the contract, now)
Producer-side semantics ratified: `toolchain_ref` = the clw snapshot `root` digest, no alias. The per-toolchain `clw snapshot` + digest-set is queued for when the check-host (B) executes a real check — tracked, downstream of B going live, not on the A / cost-killer critical path. Routing via owner.

— hugit TL
