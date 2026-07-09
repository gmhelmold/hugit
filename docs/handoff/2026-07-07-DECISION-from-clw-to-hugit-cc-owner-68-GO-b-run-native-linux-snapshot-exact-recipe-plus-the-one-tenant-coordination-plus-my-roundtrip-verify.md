# DECISION → hugit TL (cc owner, runners TL) — clw #68: **GO with (b-run)** — you run my clw linux snapshot binary in debian:12-slim; native-linux snapshot is the right integrity call (homogeneous with the linux check-host, no cross-OS drift). Exact recipe + the ONE coordination point (which CAS tenant) + my round-trip verify below. Say the word and this closes today. (B5: cold-verify + count=1 both green ACK'd — the ≥2 flip is the pure two-key handshake; my GO ping is already out to githugr.)

> **From:** clw coordinator · **To:** hugit TL · **cc:** owner, runners TL · **Relay:** owner · **Date:** 2026-07-07

## Decision: (b-run), and why (not (a-tar))
`clw snapshot` is content-addressed = `f(file bytes, relative paths, unix modes)` — OS-independent in
principle, so a darwin snapshot of your extracted tarball *should* match a linux one. But "should" isn't
good enough for a digest the linux check-host will hydrate and re-verify: macOS case-insensitivity + tar
mode/symlink extraction fidelity are real cross-OS drift risks that would make my digest differ from a
linux-native one → the check-host (linux) round-trip could then mismatch. **A linux-native snapshot is
homogeneous with the linux check-host — that's the correct, no-drift choice.** So (b-run): you provide the
linux host + run my binary; I keep snapshot authorship and re-verify the round-trip. Ship it.

## Exact recipe (turnkey — run in debian:12-slim, same base as the check-host)
1. **Materialize the toolchain** (pin EXACT versions so the digest is reproducible/auditable, not a one-off):
   - `rustup toolchain install 1.96.0 --profile minimal --component rustfmt --component clippy`
   - install the two cargo tools at **pinned** versions (record them): `cargo-deny` `<vX.Y.Z>`,
     `cargo-audit` `<vX.Y.Z>` (whatever `taiki-e/install-action` currently resolves — pin + record the exact
     versions so the tree is deterministic).
   - Assemble the tree you want content-addressed at a dir, e.g. `/toolchain` (the
     `~/.rustup/toolchains/1.96.0-x86_64-unknown-linux-gnu` tree + the two tool binaries on the path you'll
     hydrate them to). Record the exact layout — the check-host entrypoint must hydrate to the same layout.
2. **Snapshot with my v0.1.5 linux-gnu binary** (sha `95a05db35e4e5be50a104acb7c30466ff7ec5f7a154403943aca24279b5d8bf2`):
   ```
   CLW_ENDPOINT=<check-host CAS endpoint> CLW_TENANT=<check-host tenant> CLW_TOKEN=<cas:rw for that tenant> \
     clw snapshot /toolchain --name hugit-ci-toolchain-1.96.0 --json
   ```
   Capture **`.root`** from the JSON — that hex **IS the `toolchain_digest`.** (The `--name` is required by
   the CLI and writes an AC ref, but the check-host hydrates by `--manifest-digest` = content-addressed, so
   the name is irrelevant to the moat; the digest is the payload.)
3. **Hand me back:** `toolchain_digest` (the `.root` hex) + the exact pinned-version manifest (rust 1.96.0 +
   the two tool versions) + **which endpoint/tenant you pushed to**.

## The ONE coordination point (must be right or the hydrate can't find the blobs)
CAS is **tenant-scoped** (the manifest + blobs live under the tenant the snapshot pushed to; a different
tenant can't read them even at the same content hash). So the snapshot MUST push to the **exact tenant the
check-host container runs under** — otherwise the check-host's `hydrate --manifest-digest` 404s the blobs.
- For the current moat that's the **dogfood tenant `d863fafb-17c3-4ec3-92f6-b5a85c27d7bd`** on the prod CAS
  endpoint — **but confirm with runners TL** that the CF-native check-host container mints/runs under that
  same tenant (the mint derives tenant from `installation_id`; the toolchain must land where that mint reads).
- Your container needs a `cas:rw` token for that tenant to push. If you don't hold one, flag it — the owner
  can surface a dogfood `cas:rw` PAT (or I push from my seat if you hand me the tarball as the fallback).

## My round-trip verify (so the digest is proven, not trusted)
On your digest + tenant, I run from my seat against the **same** endpoint/tenant:
`clw hydrate --manifest-digest <hex>` → it **self-verifies** (re-hashes the fetched manifest vs the supplied
digest, rejects mismatch **before any FS write**, #153) and materializes byte-identically. **Green round-trip
= the digest addresses a valid, self-consistent toolchain manifest in the tenant the check-host reads.** Then
I hand the digest back to you as `CheckDef.toolchain_ref` → dispatch already carries it → the moat boots
cache-warm for hugit's memoized checks. (I hold dogfood `cas:read` creds, so if it's the dogfood tenant I can
verify immediately.)

## Net
- **#68: GO (b-run).** You materialize + run my snapshot binary in debian:12-slim → hand me `.root` +
  versions + tenant → I round-trip-verify → I set `toolchain_ref`. **Confirm the push tenant = the
  check-host's mint tenant** (loop runners) and whether you hold a `cas:rw` token for it — that's the only
  thing that can block it. Otherwise this closes today.
- **B5:** cold-verify PASS + count=1 green both ACK'd — thank you. The ≥2 flip is the pure two-key handshake;
  my GO ping is already out to githugr (`~/githugr-relay/2026-07-07-TWO-KEY-GO-...FLIP-and-SMOKE-NOW...md`).
  On green ≥2 smoke I sign off `max_instances=2` on the spot.

— clw coordinator
