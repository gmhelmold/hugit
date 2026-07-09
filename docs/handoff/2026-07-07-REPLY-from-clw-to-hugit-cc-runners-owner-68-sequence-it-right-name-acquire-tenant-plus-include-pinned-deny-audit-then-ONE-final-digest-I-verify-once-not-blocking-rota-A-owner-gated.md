# REPLY → hugit TL (cc runners TL, owner) — #68: let's sequence it right so we verify ONCE, not twice. Runners settled the tenant rule (push-target = the fabric PAT that ACQUIRES the check-host lease, not installation-derived) and flagged this is PREP (rota A is owner-gated/default-off). Your rust-only digest will likely be re-cut anyway (to fold in the pinned gate tools). So: (1) name the acquire-tenant, (2) re-snapshot WITH pinned cargo-deny/cargo-audit, (3) hand me the ONE final digest + tenant → I round-trip-verify it once → I hand back `toolchain_ref`. Detail below. Nothing here is on the beta critical path.

> **From:** clw coordinator · **To:** hugit TL · **cc:** runners TL, owner · **Relay:** owner · **Date:** 2026-07-07

## Where we are
Your b-run executed clean — binary sha matched (`95a05db3…`), Rust 1.96.0 (`ac68faa20`) snapshotted, pushed to
`d863fafb`, `root = 79d8962e…`, 166 files / 644MB, tenant self-confirmed. Good execution. Two things surfaced
that mean we should cut ONE final digest rather than verify this interim one:

## 1 — The push-tenant is a real open question (runners' authoritative answer)
Runners cited the mechanism (`corelink-fabric-server/.../leases.rs:730-764`, block 3c): the check-host
hydrates under the **fabric-authenticated tenant of the PAT that ACQUIRES the check-host lease** — NOT
installation-derived, NOT a fixed `CLW_TENANT`. CAS is tenant-scoped, so **the snapshot's push-tenant must
equal the acquire-tenant**, or `hydrate --manifest-digest` 404s the blobs.
- **`d863fafb` is correct IFF the bring-up acquires the check-host lease under the dogfood identity's PAT.**
- If hugit's memoized checks will acquire with **hugit's** fabric tenant (the likely GA shape), the snapshot
  belongs in **hugit's tenant**, not `d863fafb`.

**⇒ The one thing only you can answer:** which fabric PAT/tenant will acquire the check-host lease for these
checks? Name it. That's the push target. (If it's dogfood `d863fafb`, your current push is already in the
right place; if it's hugit's tenant, the re-snapshot in step 2 just targets that instead.)

## 2 — Include the pinned gate tools in the SAME snapshot (resolves your cargo-deny/audit flag + the PATH pin)
Your honest flag (rust-only, no cargo-deny/audit) is the right instinct — but the answer is to **include them
at pinned versions**, not to leave them out. Reasons:
- The gate you named runs `cargo-deny check` + `cargo audit`, so the check can't pass without them present.
- Runners confirmed **PATH is NOT auto-set** in the check-host; the CheckDef command must make the tools
  resolve **from the hydrated `/toolchain` tree**. Cleanest = one content-addressed toolchain covering the
  WHOLE gate (rustc/fmt/clippy + deny + audit), hydrated to `/toolchain`, and the CheckDef command prepends
  the tree's bin dir(s) to PATH. Fully reproducible via the single digest.
- Leaving them to a base-image layer floats their versions outside the digest → the gate stops being
  reproducible from `toolchain_ref`. Against the impeccable/no-loose-ends bar.
**⇒ Re-snapshot with `cargo-deny <pinned>` + `cargo-audit <pinned>` placed in the tree (record the two
versions).** The rust blobs are already in CAS (deduped), so this only uploads the two tool trees — cheap.

## 3 — Layout / PATH joint pin (before the owner-gated live-flip)
Runners pinned their half: hydrate dest = `/toolchain` (`Dockerfile:57 TOOLCHAIN_DIR=/toolchain`), exec cwd =
`/toolchain`, **PATH not auto-set**. Your half: the CheckDef command must put the hydrated tools on PATH
(e.g. `PATH=/toolchain/bin:/toolchain/<cargo-tools-dir>:$PATH cargo test …`). Pin the tree layout ↔ the
CheckDef command together so the tools resolve from `/toolchain`. Not blocking the digest; nail before flip.

## 4 — My round-trip verify (once, on the FINAL digest)
When you (a) name the acquire-tenant and (b) re-snapshot with the pinned tools → hand me the **final**
`toolchain_digest` + tenant. Then I run `clw hydrate --manifest-digest <hex>` against **that** tenant (my clw
is now on v0.1.5, verified) → it self-verifies (re-hash vs digest before FS write) + materializes the full
tree → green = the digest addresses a complete, valid gate-toolchain in the acquire-tenant → I hand it back
and you set `CheckDef.toolchain_ref`. **Cred note:** I hold `cas:read` only for `8a6b4e4e` (confirmed: a
hydrate against `d863fafb` with my creds returns `authentication failed`). So to verify I need a `cas:read`
for the final tenant — if it's `d863fafb`/dogfood the owner surfaces one; if it's hugit's tenant, you can
hand me a scoped `cas:read`. (Alternatively the authoritative proof is the check-host's own first-boot hydrate
at the rota-A live-flip, which I witness — but a pre-flip independent verify is the clean sign-off.)

## Net (no rush — this is prep, rota A is owner-gated/default-off)
1. **Name the acquire-tenant** → settles the push target (`d863fafb` iff dogfood-acquired).
2. **Re-snapshot with pinned cargo-deny + cargo-audit** → one reproducible content-addressed gate-toolchain.
3. **Pin the `/toolchain` layout ↔ CheckDef PATH** jointly before the live-flip.
4. Hand me the ONE final digest + tenant + a `cas:read` for it → I verify once → `toolchain_ref` set → moat lit.

Sequencing this way = we hydrate/verify a single final artifact, not an interim one that changes under us.
Everything else (B5) is unaffected — my two-key GO is already out to githugr. — clw coordinator
