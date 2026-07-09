# ADR-0004: Art.17 erasure must render provenance cleartext PII unrecoverable (keyed-pseudonym + key-shred)

Status: Accepted-and-implemented (decision + full migration landed — durable CSPRNG key
store, hash-preserving redaction + redaction-aware `verify_chain`, executor shred-on-complete,
forward-pseudonymisation primitive; see "Migration plan" below for the per-leg status)
Date: 2026-07-09
Context-of: GDPR1 erasure completeness audit (hugit-serve — `writes::erasure`, `provenance_pii`)
Applies to: hugit · githugr (DSR anchor)

## Context

The GDPR1 erasure cascade ([`writes::erasure`]) operates on the OBJECT/content store:
it tombstones the subject's repos (an append-only terminal `repo.erased`) and physically
GCs the account-EXCLUSIVE CAS objects (410-Gone via the CoreLink erase seam). It
deliberately **never** touches the append-only provenance chain
(`erasure.rs:18-19`) — a repo is tombstoned by APPENDING a terminal record, never by a
rewrite, because the chain is tamper-evident.

The completeness critic (verified) found the residual hole: the provenance records
themselves carry the subject's **cleartext personal identifiers** —

- the `account` slug in event payloads (e.g. `erasure.requested` writes
  `{"account": "<slug>", "subject": "<slug>"}`), and
- `clerk:<org>:<user>` in the **`principal_chain`** of every record the subject
  authored, across all their repo logs and the account log.

`org` == the account slug (one identity, audit B2). These fields PERSIST forever after a
"completed" Art.17 erasure. Residual cleartext PII after erasure = an Art.17
completeness hole for an arbitrary real user.

### The constraint that makes this hard

The chain is append-only + tamper-evident. `this_hash` is a frozen SHA-256 over
`(prev_hash, kind, principal_chain, payload, seq)` (`hugit_refstore::compute_this_hash`,
byte-exact, single-sourced), and `hugit_refstore::verify_chain` **recomputes** `this_hash`
from those exact bytes (check #3) on top of the seq-monotonicity (check #1) and
prev→this linkage (check #2). So you cannot mutate a past record's `principal_chain` /
`payload` to scrub cleartext without breaking `this_hash` → `verify_chain` fails closed →
replay refuses to serve. Cleartext PII cannot simply be deleted in place.

## Decision

A completed erasure MUST render the subject's cleartext identifiers in the durable
provenance **unrecoverable**, while RETAINING a **pseudonymous accountability record**
("subject `<pseudonym>` erasure requested@T / executed@T'") — lawful and required under
**Art.5(2)** (you must be able to demonstrate you honoured the erasure) and consistent
with the existing DSR audit trail (the `erasure.requested`/`cancelled`/`executed`
lifecycle and the retained DSR legitimacy id).

### Approach chosen: B (pseudonymise-from-origin), hardened with A's key-shred

Provenance records store a **pseudonymous subject-ref** — `subj:<hex>` where
`<hex> = HMAC-SHA256(per_subject_key, account_slug)` — in place of the cleartext account
and `clerk:…` principal. The **chain hashes the pseudonym**, so integrity and
tamper-evidence are preserved by construction (nothing is rewritten; `verify_chain`
passes unchanged). The cleartext↔pseudonym linkability lives ONLY in the per-subject
**key** held by a `SubjectKeyStore`. Erasure **shreds** that key. After the shred the
pseudonym bytes remain in the chain, but the identity is unrecoverable:

- it cannot be **reversed** to the account (HMAC is one-way, and the key is gone), and
- it cannot be **re-derived** from a guessed account slug — the per-subject key defeats
  the brute force that a plain hash would fall to (account slugs are `[a-z0-9-]`, ≤64,
  trivially enumerable, so an *unsalted* `hash(slug)` is reversible by a dictionary sweep;
  this is exactly why **C alone is insufficient** — see below).

This is GDPR-recognised **crypto-shredding**: the erasable per-subject key IS the
"mapping store" of approach B, and "delete the mapping" == "shred the key" of approach A.

### Why B (+ key-shred) over pure A, and why not C alone

- **Pure A (ciphertext-in-chain + decrypt-on-read)** — store the cleartext *encrypted*
  under the per-subject key in `principal_chain`/`payload`; shred the key to lose it.
  Technically preserves the chain (this_hash over ciphertext), but it forces
  **decrypt-on-every-read** across every identity consumer — authz
  (`derive_owner_tenant`, `asserted_class`), ownership enumeration, projection, replay,
  the git wire. That is the maximum blast radius, and the ciphertext still needs the SAME
  erasable per-subject key. Rejected as the lead: it buys nothing over B while touching
  far more of the hot read path. (B's pseudonym is an *opaque stable key* most consumers
  can use directly; only the rare cleartext-display path resolves via the key store.)
- **B (chosen)** — the pseudonym is a stable opaque token; consumers key on it directly;
  a single erasable secret per subject; the chain hashes the pseudonym so integrity holds
  through erase.
- **C (append-only tombstone + redacted-read) ALONE is INSUFFICIENT** and is NOT shipped
  by itself: a `redact` marker that hides cleartext at read time leaves the past cleartext
  *bytes* (already hashed into `this_hash`) intact in durable storage — recoverable by
  anyone with store access. C is only sound COMBINED with A/B (the retained
  `erasure.pii_shredded` accountability record IS a C-style tombstone, but it works
  because B has already made the identity pseudonymous).

## Why this does not fit one PR (scope + risk)

The **full** implementation is L-effort, migration-heavy, and high-risk on the LIVE
tamper-evidence core:

1. **Hot write path (every verb)** — pseudonymise the identity at the origin of every
   `principal_chain`/payload write (`write_pr_create`, `write_land`, `write_provision`,
   `write_account_erase`, …): fetch-or-mint the per-subject key, derive `subj:<hex>`,
   write that instead of `clerk:…`.
2. **Every identity reader** — authz (`derive_owner_tenant` extracts `org` from
   `clerk:org:user` today; must accept a pseudonym + resolve via the key store where a
   cleartext org is genuinely needed), ownership enumeration, projections, the SSE/UI
   surfaces. Getting one reader wrong is an authorization regression on a live forge.
3. **Existing cleartext records** — already hashed into `this_hash`, append-only,
   immutable. They CANNOT be retroactively pseudonymised without rewriting history. The
   only sound path is **redaction-with-hash-preservation**: replace the stored
   `principal_chain`/PII-payload bytes with the pseudonym while **preserving the original
   `this_hash`**, and teach `verify_chain` a redaction-aware check #3 (a record flagged
   redacted verifies its linkage via the preserved `this_hash`, and check #1/#2 — the
   ordering + append-immutability that ARE the tamper-evidence — stay enforced). This is
   surgery on the shared `hugit-refstore` tamper/replay/coldtier core.
4. **Durable key store** — a KMS- or secrets-table-backed `SubjectKeyStore` with real
   key-material hygiene (CSPRNG, zeroization, per-subject rows) and a shred that is
   durable + auditable.

Any of 1–4 alone is a reviewable PR; together, on the live engine, they are a
multi-PR programme. Shipping them as one change would be the high-risk move the
tiered-flow discipline forbids.

## What ships now (this PR — the smallest safe, additive first step)

A new, self-contained `hugit_serve::provenance_pii` module — **additive**, touching
none of the contested core files (`erasure.rs`, `git.rs`, `cas.rs`, `state.rs`,
`server.rs`) and none of the hot write/read paths:

- `SubjectPseudonym::derive(key, account)` — the keyed-HMAC pseudonym primitive
  (`subj:<hex>`) over the workspace's existing `hmac`+`sha2`+`hex` pins (zero new crypto
  dep, mirroring `sigv4.rs`).
- `SubjectKey` (redacting `Debug`) + `SubjectKeyStore` trait (`key_for` / `ensure_key` /
  `shred`) + an `InMemorySubjectKeyStore` reference impl — the erasable secret and the
  one-method (`shred`) seam the executor will drive.
- `PseudonymousErasureRecord` + `ERASURE_PII_SHREDDED_KIND` — the retained Art.5(2)
  accountability record builder: pseudonym + opaque DSR id + requested_at/executed_at +
  `cleartext_shredded: true`, carrying **no** cleartext.

Tests (unit, hermetic) prove the DoD properties on the FORWARD path via the public chain
API only: the pseudonym is stable under its key, distinct per subject, not recomputable
without the key; `shred` makes the key unrecoverable (idempotent); the accountability
record carries no cleartext; and — the end-state property — a record written
pseudonymously **verifies** (`verify_chain`), and after `shred` the cleartext is
unrecoverable while the chain STILL verifies and the pseudonymous record REMAINS
(happy path unchanged).

## Interim posture (until the follow-up lands)

Existing and still-forward cleartext records retain the cleartext account/principal — an
honest, disclosed residual risk that joins the existing `ResidualDisclosure` legs in the
erasure plan (`cas-shared`, `github-mirror`, `context-store`). The erasure executor today
tombstones repos and (behind the audited seam) GCs exclusive CAS objects; **provenance-PII
shredding is a declared additional leg, not yet claimed complete** — the executor must not
represent the provenance cleartext as erased until the follow-up ships. The pseudonymous
`erasure.pii_shredded` record and the `shred` seam exist now so the executor can begin
retaining the accountability record and shredding keys as soon as the write path emits
pseudonyms.

## Migration plan (the follow-up legs, sequenced) — IMPLEMENTED

Landed in `golive/art17-full-migration` (`feat(hugit-serve): complete Art.17
provenance-PII erasure`). Per-leg status:

1. **Durable `SubjectKeyStore`** (CSPRNG + zeroization) — ✅ DONE. `LogSource` grows a
   reserved `_subject_keys/{subject}.key` keyspace (Local file + R2 conditional-PUT); a
   CSPRNG (`rand::rng()`, OS-seeded) mints 32-byte keys create-only (survives restart);
   `shred` deletes (Local) / tombstone-overwrites (R2) — durable + irreversible; `SubjectKey`
   is `zeroize`-on-drop. Exposed as `AppState::subject_key_{for,ensure,shred}`.
2. **Pseudonymise the write path** — ◑ PRIMITIVE DONE, live-verb wiring SCOPED. The pure
   forward transform `pseudonymize_write_principal_chain` (ensure-key → `subj:<hex>`,
   idempotency-stable, fail-closed) is implemented + tested. Wiring it into all ~12 live
   write verbs + the idempotency ledger match is deliberately kept as its own flagged PR
   (hot-path blast radius + the principal-keyed idem-match hazard the ADR foresaw) — and is
   NOT required for the end-state guarantee, because leg 4 renders ALL cleartext (forward
   records included) unrecoverable at erase.
3. **Identity readers** — ✅ VERIFIED (no code change needed). Confirmed `authorize_read`/
   `authorize_write` key on the projected `owner_tenant` + the LIVE request principal, NEVER
   on the stored `principal_chain` (G11). Pseudonymising the stored chain is therefore authz-
   neutral; regression-tested that a non-erased owner still resolves.
4. **Redaction-with-hash-preservation + redaction-aware `verify_chain`** — ✅ DONE.
   `hugit_refstore::redact_record` rewrites `principal_chain`/`payload` to the pseudonym while
   PRESERVING `this_hash`; an append-only `provenance.redaction` marker carries the
   `(original_this_hash, redacted_this_hash)` commitment; `verify_chain` does a first pass
   collecting markers, keeps #1/#2 for every record, and swaps #3 for the redacted branch. A
   log with no markers verifies BYTE-IDENTICALLY (zero regression; all golden pins green).
5. **Wire the executor** — ✅ DONE. On `Executed` ONLY (never `partial`/`cancelled`), the
   executor ensures the key, retains the `erasure.pii_shredded` accountability record, redacts
   the account log + every tombstoned repo log (hash-preserving), THEN shreds the key —
   fail-closed throughout, idempotent under retry.

## Consequences

- **Now:** zero regression surface — a new module + tests, no edits to live paths; the
  primitive + seam that the whole programme builds on exist and are proven.
- **End-state:** a completed Art.17 erasure leaves the durable provenance with NO
  recoverable cleartext PII, a preserved + verifiable tamper-evident chain, and a
  retained pseudonymous accountability record satisfying Art.5(2).
- The pseudonym is durably stored, so the future write-path/backfill work is additive
  over a primitive that already exists — no re-work debt accrues while phased.
