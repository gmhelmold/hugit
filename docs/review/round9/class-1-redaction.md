# CLASS 1 — REDACTION — Round 9 re-audit

Fresh-context convergence re-audit of CLASS 1 (REDACTION / SECRET-AT-REST) on
`integ/wave-l` (HEAD `95353f9`, Wave L applied). Read-only on all code/docs;
the only write is this file. Live repro on the real `target/debug/hugit`, all
stores/logs in `mktemp` dirs OUTSIDE the repo.

## 1. Scope & method

Wave L (commit `22a15d5`, L-A) INVERTED the identifier scrub to deny-by-default:
`structural_secret_scrub` (porcelain.rs:421) now returns the value verbatim ONLY
if `is_safe_identifier_shape` (porcelain.rs:454) proves a bounded safe-address
shape; otherwise `[REDACTED]`. The door (`ident.rs::validate_identifier`) is
defined as "reject iff the scrub changes the value," so the same gate closes the
input path (`secret_in_identifier`, exit 2).

The address allowlist is: (a) `is_structural_secret` ⇒ redact (prefix table /
`sk-` / PEM / conn-string / keyword); (b) `is_digest_shaped` (bare 40/64-hex OR
value-gated `cas:`/`<algo>:` ref) ⇒ survive; (c) `is_ulid_shaped` (26-char
Crockford base32) ⇒ survive; (d) bounded identifier charset `[A-Za-z0-9._@:/+-]`
that is NOT (len ≥ `IDENT_ENTROPY_MIN_LEN`=20 AND Shannon entropy ≥
`IDENT_ENTROPY_THRESHOLD`=4.5) ⇒ survive; else redact.

Method: (1) re-ran the full Round-8 leaking matrix on the real binary;
(2) attacked the new deny-by-default boundary for a secret that masquerades as a
safe shape; (3) probed the entropy-threshold edges; (4) checked completeness
across verbs/sinks/error-echo and over-scrub regressions.

## 2. Matrix re-verification

Every Round-8 ✗ cell is now ✓ for the original PREFIX-LESS HIGH-ENTROPY threat.
Live, grep-count of the planted AWS key (`wJalr…KEY123`, entropy 4.76) in every
written file = 0:

| Round-8 cell | Round-8 | Round-9 result (AWS key) | repro |
|---|---|---|---|
| `intent new --id` | ✗ leak (count 2) | ✓ door-reject exit 2, count 0 | T1 |
| `pr open --pr` | ✗ leak (count 1) | ✓ door-reject exit 2, count 0 | T4 |
| `pr open --run-id` (+chain) | ✗ leak ×2 | ✓ door-reject exit 2, count 0 | T5 |
| `pr open --principal` | ✗ leak (count 1) | ✓ payload `[REDACTED]`, count 0 | T6 |
| `campaign open --campaign` | ✗ leak (count 1) | ✓ door-reject exit 2, count 0 | T7 |
| `check --pr` | ✗ leak (count 1) | ✓ count 0 (memo-keyed) | T10b |
| `tournament --intent` (err echo) | n/a | ✓ `[REDACTED]` in `intent_not_found` | — |
| `verdict --intent` (err echo) | n/a | ✓ `[REDACTED]` in `intent_not_found` | — |
| `context-ref` (free-text) | ✓ | ✓ count 0 | — |

All four Round-8 P0 specimens (AWS 4.76 / SendGrid 5.24 / Stripe 5.28 / dense
B64 5.00) reject or redact, count 0. **The Round-8 P0 (F1) is CLOSED.**

Over-scrub regression sweep — every legit address SURVIVES verbatim, 0 collapsed
to the sentinel: `feature/login`, `auth-hardening`, a ULID
`01HQXW8ZK4M9P2N7R3T5V6Y8BC`, `pr-9f8e7d6c`, `run-0011-2233`, `42`,
`user@host.com`, two distinct 40-hex addresses. **No over-scrub regression.**

`acceptance_wj_matrix.rs` (the F3 guard) was re-authored to add the prefix-less
high-entropy specimens (AWS/SendGrid/Stripe/B64) across every identifier column
— the matrix now enumerates the threat model, not the secret allowlist. (Gap:
it contains no LOW-per-char-entropy specimen — see R9-1.)

## 3. Break attempts on the new deny-by-default boundary

Each: input → entropy → live result (grep count of the secret at rest).

- **B1 — 64-hex secret** (Django `SECRET_KEY` / HMAC), `--id`. SURVIVES (count 3).
  Exits early via `is_bare_hex_digest` (40/64-hex). *Documented Round-8 §6
  irreducible residual* (indistinguishable from a content address).
- **B2 — 40-hex secret**, `--id`. SURVIVES (count 3). Same digest path. *Same
  residual.*
- **B3 — `cas:`-wrapped 51-char base32 value**, `--id`. SURVIVES (count 3).
  Value-gated `cas:` payload-shape path. A `cas:`-wrapped *structural* secret
  (`cas:ghp_…`) still redacts (K-SCRUB, re-confirmed). *Same residual class.*
- **B4 — ULID-shaped value**, `--id`. SURVIVES (count 3). `is_ulid_shaped`.
  *Documented narrow residual.*
- **B5 — 32-char hex token** `7d8f…9b0d` (entropy **3.91**), `--id`. **SURVIVES
  (count 3).** NOT 40/64-hex, so NOT a content-address shape — passes the
  charset check, then clears the entropy gate (3.91 < 4.5). → **R9-1.**
- **B6 — 50-char hex secret** (entropy 3.88), `--id`. **SURVIVES (count 3).**
- **B7 — 30-char lowercase base32 token** (entropy 4.37), `--id`. **SURVIVES
  (count 3).**
- **B8 — 24-char lowercase-letters token** (entropy **4.585** > 4.5), `--pr`.
  **REJECTED** (door exit 2, count 0). Confirms the gate fires above 4.5.
- **B9 — 32-hex as `--run-id`** (principal_chain path). **SURVIVES (count 2)** —
  payload value + the hashed chain entry. The chain path inherits the same gate.
- **B10 — TOTP base32 secret** `JBSWY3DP…` (uppercase A-Z2-7, entropy 3.38),
  `--id`. **SURVIVES (count 3).** A real RFC-6238 shared-secret shape.
- **B11 — 24-digit numeric secret** (entropy 3.29), `--id`. **SURVIVES.**
- **B12 — low-entropy-by-repetition 32-char "tokeny" base64**
  `aBaBaB…xYxY` (entropy 3.78), `--id`. **SURVIVES (count 3).**

**Strongest break (B5/B9/B10):** the entropy gate is the SOLE discriminator for
charset-(d) values, and its ceiling math is exploitable. A pure-hex string has a
per-char entropy ceiling of **log2(16) = 4.0 < 4.5**, so *any-length* hex value
ALWAYS passes — not just the 40/64-hex content-address shapes the Round-8
residual accepted. Numeric (ceiling 3.32) and structured base32 (TOTP 3.38)
likewise sit permanently under 4.5. So the deny-by-default gate, as tuned,
admits a broad band of REAL low-per-char-entropy credential formats (hex API
keys of non-digest length, TOTP seeds, numeric secrets, repetitive tokens) that
are not 40/64-hex/ULID/cas-shaped. These survive verbatim into the forever
hash-chained log AND are re-emitted in tournament output and the
`intent_not_found` error echo (the high-entropy masking does not fire on them).

## 4. Residual findings

### R9-1 — P1 · honesty/seam (with a code-tunable knob) · the entropy gate admits a broad band of low-per-char-entropy credential formats beyond the documented 40/64-hex residual

- **Repro:** `hugit intent new --store S --id 7d8f3a2b1c9e4f6a8b2d5e7c1a3f9b0d
  --charter x --campaign c --acceptance y` → exit 0, grep count **3** at rest.
  Same for a 50-hex secret, a 30-char base32 token, a TOTP base32 seed
  (`JBSWY3DPEHPK3PXPJBSWY3DPEHPK3PXP`), a 24-digit numeric secret, and a
  repetitive 32-char base64 blob. Via `--run-id` the value also enters the
  hashed `principal_chain` (count 2).
- **Root:** `is_safe_identifier_shape` (porcelain.rs:501) gates charset-(d)
  values ONLY on `len ≥ 20 AND entropy ≥ 4.5`. Pure-hex (ceiling 4.0), numeric
  (3.32), and structured base32/TOTP (≤3.4–4.4) can never reach 4.5, so they are
  never rejected. The Round-8 §6 residual was scoped to **40/64-hex content
  addresses**; Wave L's gate is strictly WIDER — it accepts hex of *any* length
  plus base32/numeric/repetitive tokens.
- **TYPE:** honesty/seam. This is NOT a regression of the closed P0 (dense
  high-entropy SaaS keys are caught), and these surviving values genuinely
  overlap the legitimate low-entropy-address space (short hashes, abbreviated
  ids, numeric ids) — a 32-hex value is, at rest, hard to distinguish from a
  truncated content hash. It is a code-tunable threshold, not a structural
  inversion, so it is a P1 honesty gap (the residual is wider than documented),
  not a P0 class re-opening.
- **What would tighten it (not required for convergence, owner call):** the
  threshold 4.5 + the hex-digest exemption together mean "hex of any length
  survives." If the intent is "only *content-address-shaped* hex survives," the
  bare-hex exemption should be length-pinned to {40,64} (it already is in
  `is_bare_hex_digest`, but charset-(d) re-admits other-length hex under the
  entropy gate). A per-charset entropy ceiling, or rejecting long pure-hex that
  is NOT exactly 40/64, would close the hex band. The numeric/base32/repetitive
  bands are closer to genuinely-irreducible (a numeric id and a numeric secret
  are indistinguishable without context — the Round-8 `hunter2` argument).

### R9-2 — P2 · honesty/verification · the F3 guard matrix has no low-per-char-entropy specimen

`acceptance_wj_matrix.rs` added AWS/SendGrid/Stripe/B64 — all entropy ≥ 4.76.
It contains no 32-hex / TOTP / numeric / repetitive specimen, so the matrix is
green while the R9-1 band survives. Same shape as Round-8 F3 (the test
enumerates what the gate already catches), one octave down: it now covers the
high-entropy threat but still does not probe the threshold's lower edge.
**TYPE:** verification.

### R9-3 — P2 · seam · two-engine duplication persists (Round-8 F4 unchanged)

`is_structural_secret`, `KNOWN_SECRET_PREFIXES`, `is_content_address_ref`,
`ident_shannon_entropy` are still a hand-mirrored copy of the ledger engine
("kept in lockstep" by comment). Standing drift risk; unchanged by L-A.

## 5. CONVERGENCE VERDICT

**CONVERGED on the Round-8 P0; the residual band is WIDER than documented (P1
honesty, not a P0 re-open).** The class-killing inversion is real and live: the
identifier scrub now allowlists addresses, every Round-8 high-entropy leak cell
is closed (door-reject or `[REDACTED]`, count 0), no legit address over-scrubs,
and the door + payload + principal-chain + error-echo all route through the one
gate. **No P0 residual.** The one residual worth flagging (R9-1, P1) is that the
entropy threshold (4.5) is permanently un-reachable by pure-hex (ceiling 4.0),
numeric, and structured-base32 charsets, so the surviving set is broader than
the "40/64-hex content address" the Round-8 baseline accepted as irreducible —
real hex/TOTP/numeric credential formats ride through. This is a tunable-knob
honesty gap on the boundary of irreducible physics (a 32-hex value is hard to
distinguish from a short content hash at rest), not a structural reopening of the
class. Recommend an owner decision on tightening the hex/numeric band (R9-1) plus
adding a low-entropy specimen to the guard matrix (R9-2); the deny-by-default
polarity itself is sound and holds.
