# CLASS 1 — REDACTION — Round 10 confirmation re-audit (post M-2 unification)

Fresh-context confirmation re-audit of CLASS 1 (REDACTION / SECRET-AT-REST) on
`integ/wave-m` (HEAD `e85bcf5`), after WP **M-2** (`9301b56`, R9-3) unified the
two hand-mirrored scrub engines onto ONE shared primitive module. Read-only on
all code/docs; the only write is this file. Live repro on real `target/debug/hugit`
binaries; all stores/logs in `mktemp` dirs OUTSIDE the repo.

## 1. Scope & method

M-2 extracted the shared structural-secret / content-address-shape / ULID /
entropy primitives into ONE module — `crates/hugit-ledger/src/secret_shape.rs`
(516 LOC) — and made BOTH layers consume it:

- the **free-text engine** `hugit-ledger/src/redact.rs` deletes its private
  copies and `use`s `secret_shape::{is_structural_secret, is_content_address_ref,
  is_bare_hex_digest_shape, is_digest_algo, is_token_char, shannon_entropy,
  ENTROPY_MIN_LEN}`; it KEEPS its own free-text policy — `ENTROPY_THRESHOLD =
  4.0` + the bare-hex exemption logic in `high_entropy_token` (a BARE 40/64-hex
  run REDACTS in free text; an `<algo>:`/`cas:`-prefixed one survives).
- the **identifier door / boundary** `hugit-cli/src/porcelain.rs` deletes its
  hand-mirrored `KNOWN_SECRET_PREFIXES`/`contains_*`/`is_*`; the policy fns
  `is_safe_identifier_shape` (deny-by-default, `IDENT_ENTROPY_THRESHOLD = 4.5`,
  PS-14 hybrid) and `is_digest_shaped` are now `pub use` re-exports of the shared
  module. `ident.rs` still calls `porcelain::structural_secret_scrub`, now built
  on the shared gate.

The two layers keep DIFFERENT top-level policies (door: 4.5 + {40,64}-hex pin +
deny-by-default; free-text: 4.0 + bare-hex-redacts) but build on identical
primitives. M-2's risk is therefore **behavior drift**, not a new hole.

Method: (1) re-ran the full identifier-door + free-text matrix on the NEW binary;
(2) **built the PRE-M2 binary** (detached worktree at `9301b56^` = `f982499`, the
hand-mirrored two-engine state — confirmed no `secret_shape.rs`) and diffed
EVERY outcome OLD-vs-NEW across 42 curated vectors + 120 random fuzz tokens × both
layers; (3) probed every boundary the unified primitive touches (entropy edges,
the {40,64} pin, the cas: value-gate, floor edges); (4) proved the test set is
load-bearing by mutation.

## 2. Matrix re-verification (door + free-text, before-M2 vs now)

**Identifier door — secrets REJECT exit 2, count 0 at rest** (all ✓):
prefix-less 32-hex, prefix-less 50-hex, 24-digit-numeric, AWS, SendGrid, Stripe,
`ghp_`, `xoxb-`, JWT, conn-string → `secret_in_identifier` exit 2, grep count 0.

**Identifier door — addresses SURVIVE exit 0** (all ✓): 40-hex, 64-hex, ULID,
UUID, integer `42`, `intent-<16hex>`, slug `auth-hardening`, `cas:<64-hex>`,
CI run-id `123456789` → exit 0, verbatim at rest.

**PS-14 hybrid intact** (all ✓): {40,64}-hex SURVIVE; 32-hex / 50-hex /
24-numeric REDACT (door-reject). Pin edges 39/41/63/65-hex → REJECT; exactly
40/64 → SURVIVE. Floor: 19-hex/19-numeric SURVIVE (< len floor 20), 20-hex/
20-numeric REDACT. Uppercase `A1B2…` 32-hex REDACT (hexdigit covers A-F).

**Free-text engine** (`--charter`, all ✓): `ghp_`/AWS/JWT/conn-string →
`[REDACTED]`, raw count 0; **bare 40/64-hex → `[REDACTED]`** (the documented
free-text law — distinct from the door, which lets bare 40/64-hex survive);
`cas:ghp_…` → `[REDACTED]` (value-gated); `cas:<64-hex>` and `sha256:<64-hex>`
→ SURVIVE; prose → SURVIVE.

**Before-M2 vs now:** identical. The OLD (`f982499`, hand-mirrored) and NEW
(`e85bcf5`, unified) binaries produced byte-identical (exit, at-rest-count)
results on EVERY one of the 42 curated vectors across BOTH layers. **Zero drift.**

## 3. Drift / bypass attempts

| Attack on the unified primitive | Result |
|---|---|
| **Curated 42-vector OLD-vs-NEW diff**, door + free-text (84 comparisons) | **ZERO DRIFT** — every (exit,count) identical |
| **Randomized fuzz**, 120 tokens × 6 charsets (hex/HEX/num/base32/base64/slug), len 6–70, door OLD-vs-NEW | **ZERO DRIFT** — 120/120 identical |
| {40,64}-hex pin edges (39/41/63/65) — try to ride odd-length hex | all REJECT (door) — pin holds, no widening |
| cas: value-gate — `cas:ghp_…`, `cas:<32-hex>`, `cas:<64-hex>`, `cas:<base32-CID>` | `cas:ghp_` REJECT; the others survive exactly as pre-M2 (the 32-hex-cas survivor is H=4.02<4.5 low-entropy slug-band, identical OLD/NEW) |
| entropy edge — 24-char lowercase blob (>4.5) vs low-entropy hex/numeric/base32 | door fires >4.5; the documented low-entropy band survives — same boundary as pre-M2 |
| Make a secret survive in BOTH engines / an address redact that survived before | **none found** |

Strongest attack — the **OLD-vs-NEW differential over 162 inputs × 2 layers** —
is the decisive drift probe: if any shared primitive (the prefix table, the
`sk-` gate, the conn-string scan, the keyword scan, the cas value-gate, the
shannon fn, the {40,64} shape, the ULID shape, the entropy thresholds) had
drifted in the extract-and-rewire, at least one of those inputs would diverge.
None did.

## 4. Findings

| id | sev | repro | TYPE |
|---|---|---|---|
| (none — no drift/regression/new bypass) | — | — | — |

Test set confirmed **load-bearing** (not merely present): in a throwaway copy
OUTSIDE the repo, dropping `ghp_` from `secret_shape::KNOWN_PREFIXES` turned RED
the unit `secret_shape::structural_secret_classes` + `…::safe_identifier_shape_…`
(2 fail), the `hugit-ledger` `redact` units, AND the CLI `acceptance_wj_matrix`
(**9 fail**: `intent_id_field_matrix`, `campaign_campaign_field_matrix`,
`check_toolchain_*`, `pr_*_field_matrix`, `la_every_prefixless_specimen_…`) —
proving the door routes end-to-end through the unified module. Green baseline:
secret_shape 5/5, redact 59/59, porcelain 27/27, acceptance_wj_matrix 29/29.

**Residual (unchanged, documented):** the PS-14 low-entropy-base32 physics residual
— a low-entropy base32 value (Shannon ≈3.4, indistinguishable from a legit slug)
survives both layers. Accepted irreducible boundary; M-2 did not touch it.

## 5. CONVERGENCE VERDICT

**CONVERGED.** M-2 is a behavior-preserving refactor: it single-sources the
shared primitives without changing ANY survive/redact/reject decision in either
layer. Proven by a binary-level OLD-vs-NEW differential (162 inputs × 2 layers,
zero drift) plus the full curated matrix (every secret redacts/rejects, every
address survives, both layers, count 0/verbatim as required), the {40,64} pin
and cas value-gate intact, and a mutation test confirming the unified module is
wired and guarded by the test set end-to-end. R9-3's hand-duplication drift risk
is structurally CLOSED (one source of truth) with no behavior change. Only the
documented PS-14 low-entropy-base32 physics residual remains; not a drift, not a
new hole.
