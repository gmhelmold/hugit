# CLASS 1 — REDACTION / SECRET-AT-REST — SOTA audit

Round 8, fresh adversarial fleet on the integrated state (`def8a18`, Wave K).
This is a SEVERE-CLASS audit: the goal is to enumerate the WHOLE class of
secret-at-rest leaks, find the ONE structural root, and name the deny-by-default
fix that makes the class impossible — not to patch the next instance.

**Verdict up front: the class is OPEN.** A prefix-less / unlisted-prefix
high-entropy credential (AWS secret access key, SendGrid `SG.`, Stripe
`rk_live_`, a 32-char random base64 token) rides any **identifier-address field**
verbatim into the forever event-log. Reproduced live on the real binary across
five verbs. This is the 6th instance of "an exemption is a hole" — and it is
structurally the SAME hole the prior five were, because the identifier exemption
is an **allowlist of known prefixes**, not a deny-by-default gate.

---

## 1. Scope & method

**Completeness, not sampling.** The redaction surface has exactly two scrub
engines and three scrub *modes*; I enumerated all of them from source, then
enumerated every user-controlled field of every write verb and the scrub mode it
resolves to, then reproduced every `✗`/`~` cell live on `target/debug/hugit`.

The two engines (single-sourced, kept "in lockstep" by comment):

- `crates/hugit-ledger/src/redact.rs` — `apply()` / `is_secret()`: FIVE detectors
  — (1) `SECRET:` marker, (2) known-prefix table (`KNOWN_PREFIXES`) + `sk-`,
  (3) connection-string password, (4) keyword-context (`token=…`), (5) bare-hex
  + Shannon-entropy scan with the value-gated `cas:`/`<algo>:` content-address
  exemption.
- `crates/hugit-cli/src/porcelain.rs` — `scrub_payload()` / `scrub_to_canonical()`:
  the recursive per-`(key,value)` boundary that EVERY porcelain append routes
  through. It chooses one of three `ScrubMode`s per field (`scrub_mode`,
  porcelain.rs:357):
  - `FreeText` → full engine (all 5 detectors).
  - `Verbatim` → digest-keyed AND value is digest-shaped (`is_digest_shaped`).
  - **`Structural`** → identifier-keyed (`is_identifier_key`): runs
    `structural_secret_scrub` (porcelain.rs:401), which is detectors (1)–(4)
    **with detector (5) — the entropy scan — DELIBERATELY OMITTED**
    (porcelain.rs:441-443).

The door (`crates/hugit-cli/src/ident.rs::validate_identifier`) is a SECOND copy
of the same allowlist logic — it rejects iff `structural_secret_scrub` changes
the value (ident.rs:110), i.e. it has the *same* entropy blind spot by
construction (ident.rs:38-42 documents this as intentional).

**Sinks audited (a–g from the mandate):** (a) the hash-chained `--log`,
(b) `.hugit/*` local stores (`intents.json`), (c) `<log>.ac` Action-Cache
sidecar, (d–f) refstore/queue/envelope payloads (all reached via the same
`scrub_to_canonical` / `principal_chain` paths), (g) read-surface re-emission
(`pr show`, `export`).

**Repro harness:**
`export RUSTC=…/1.96.0…/bin/rustc; cargo build -p hugit-cli --locked`, then each
verb run in a `mktemp -d` OUTSIDE the repo, with the planted token grepped out of
every written file (count must be 0). Planted specimens, chosen to be REAL
issuer shapes that carry NO token in `KNOWN_PREFIXES`:

- `AWS = wJalrXUtnFEMIK7MDENGbPxRfiCYEXAMPLEKEY123` (AWS secret access key shape — 40-char base64)
- `SENDGRID = SG.aBcDeFgHiJkLmNoPqRsTuV.wXyZ0123456789aBcDeFgHiJkLmNoPqRsTuVwXyZ012`
- `STRIPE = rk_live_51HxYzAbCdEfGhIjKlMnOpQrStUvWxYz0123456789`
- `HEXKEY = 9f86d081…0a08` (64-hex — Django `SECRET_KEY` / HMAC)
- `B64 = aB3xZ9qL2mK7pR4tY8wN6vC1dF5gH0jS` (32-char dense base64)

---

## 2. Complete inventory — THE matrix

### 2a. The three scrub modes and what each exempts

| Mode | Where decided | Detectors run | Entropy scan (det. 5)? | Hole? |
|------|---------------|---------------|------------------------|-------|
| `FreeText` | every non-id, non-digest field | 1–5 | **yes** | none |
| `Verbatim` | digest key **AND** `is_digest_shaped` value | none | n/a | none — value-gated since WH-SCRUB; a non-hex value falls back to `FreeText` (proven T9/T12) |
| `Structural` | identifier key (`campaign`/`intent_id`/`pr_id`/`run_id`/`id`) + the `pr` principal-chain | 1–4 | **NO (omitted)** | **THE HOLE** |

### 2b. Field × sink coverage (every write verb)

Cells: ✓ scrubbed/redacted · ✗ leaks verbatim · `id`=identifier(Structural mode) ·
`ft`=free-text · `dg`=digest(value-gated). Sink (a)=forever-log is the
load-bearing one; (g)=read re-emission inherits (a).

| Verb · field | mode | prefixed secret (ghp_/xoxb-/conn) | **prefix-less high-entropy secret** | repro |
|---|---|---|---|---|
| `intent new --id` | id | ✓ door-reject | **✗ LEAK** | T1 (count 2) |
| `intent new --campaign` | ft | ✓ | ✓ | T2/T3 (REDACTED) |
| `intent new --charter/--acceptance` | ft | ✓ | ✓ | T(charter) |
| `intent new --owner/--agent` | ft | ✓ | ✓ | — |
| `campaign open --campaign` | id | ✓ door-reject | **✗ LEAK** | T7 (count 1) |
| `campaign open --owner` | ft | ✓ | ✓ | T8 (REDACTED) |
| `campaign open --charter` | ft | ✓ | ✓ | — |
| `pr open --pr` | id | ✓ door-reject | **✗ LEAK** | T4 (count 1) |
| `pr open --campaign` | id | ✓ door-reject | **✗ LEAK** | (same class as T4) |
| `pr open --run-id` | id + **principal_chain** | ✓ door-reject | **✗ LEAK ×2** | T5 (count 2: payload + chain) |
| `pr open --principal` | id + **principal_chain** | ✓ structural | **✗ LEAK** | T6 (count 1) |
| `pr open --intent` | ft | ✓ | ✓ | — |
| `verdict --intent` | ft | ✓ | ✓ | matrix |
| `verdict --lens` | ft | ✓ | ✓ | matrix |
| `verdict --tree-hash` | dg (value-gated) | ✓ | ✓ | T9 (REDACTED — non-hex falls to ft) |
| `check --pr` | id | ✓ door? (not door-validated) → boundary | **✗ LEAK** | T10 (count 1) |
| `check --principal` | **ft** (checks scrubs principal with full engine) | ✓ | ✓ | T11 (REDACTED) |
| `check --toolchain` | dg (value-gated) | ✓ | ✓ | T12 (REDACTED) |
| `check --def/--cmd` | ft | ✓ | ✓ | matrix |

**Matrix verdict: 7 leaking cells**, all on `Structural`-mode identifier fields
(plus the `pr` principal-chain duplication). Every `FreeText`/`Verbatim` cell is
clean. The asymmetry is total and exactly predicts the root: a field leaks iff it
is routed through the entropy-skipping `Structural` mode.

Note the *inconsistency* that itself flags the root: `check --principal` is SAFE
(checks/run.rs:1140 uses `crate::redaction::scrub` = full engine) while
`pr --principal` LEAKS (pr/mod.rs:1469 uses `structural_secret_scrub`). Two
sibling verbs treat the same conceptual field with opposite rigor — a symptom of
a hand-maintained, per-call-site exemption rather than one gate.

### 2c. Every scrub exemption branch (enumerated)

1. `ScrubMode::Verbatim` — digest key + digest-shaped value. **Value-gated**
   (WH-SCRUB). A non-digest value falls to `FreeText`. Proven safe (T9/T12).
   *Can a secret wear this shape?* Only a 40/64-hex or `cas:<CID>` value — which
   is content-address shaped and not a credential charset. The `cas:` payload is
   double-gated (`is_cas_payload_shaped` + `!is_structural_secret`, the K-SCRUB
   Round-7 fix). **Not a hole.**
2. `is_content_address_ref` `cas:`/`<algo>:` exemption (redact.rs:410,
   porcelain.rs:625) — value-gated since K-SCRUB. **Not a hole** (a
   `cas:<64hex>` IS high-entropy and must survive; a `cas:ghp_…` redacts).
3. **`ScrubMode::Structural` — identifier fields, entropy scan omitted
   (porcelain.rs:441-443).** *Can a secret wear this shape?* **YES** — any
   high-entropy credential with no listed prefix. **THE HOLE.**
4. The door allowlist (`ident.rs`) — same omission, so it cannot compensate.

---

## 3. Findings

### F1 — P0 · CODE · Prefix-less high-entropy secret leaks verbatim through every identifier-address field

- **Repro (T1):**
  `hugit intent new --store S --id wJalrXUtnFEMIK7MDENGbPxRfiCYEXAMPLEKEY123 --charter x --campaign c --acceptance y`
  → `--id` echoed verbatim; `grep -c` of the AWS key in the store = **2**.
- **Repro (T4):** `hugit pr open … --pr wJalr…KEY123 …` → log count **1**.
- **Repro (T5):** `hugit pr open … --run-id wJalr…KEY123 …` → log count **2**
  (payload value + the `principal_chain` entry hashed verbatim by
  `append_authorized`).
- **Repro (T6):** `hugit pr open … --author-kind human --principal wJalr…KEY123`
  → log count **1**.
- **Repro (T7):** `hugit campaign open --campaign wJalr…KEY123 …` → log count **1**.
- **Repro (T10):** `hugit check --def fmt --pr wJalr…KEY123 --store` → log count **1**.
- **Control proving it is the EXEMPTION, not a detector gap:** the IDENTICAL
  string in a `FreeText` field is redacted —
  `hugit intent new --charter "deploy with wJalr…KEY123 now" …` → count **0**;
  `hugit intent new --campaign wJalr…KEY123 …` → `"[REDACTED]"`, count **0**.
  The full engine's entropy detector (5) catches this credential; the identifier
  mode deliberately skips detector (5), so the same value survives.
- **Generality:** SendGrid `SG.…` and Stripe `rk_live_…` keys in `--id` also
  leak (count 2 each) — real issuer formats whose prefixes are not in
  `KNOWN_PREFIXES`. The class is not AWS-specific; it is *any* credential the
  finite allowlist does not name.
- **`file:line`:** `porcelain.rs:441-443` (entropy scan omitted in
  `is_structural_secret`); `porcelain.rs:357-360` (`scrub_mode` → `Structural`);
  `ident.rs:38-42` + `:110` (the door inherits the same blind spot).
- **TYPE:** code (a live verbatim leak into the forever, hash-chained log — same
  severity class as the Round-2 hex leak and the Round-7 `cas:` PAT leak).
- **ROOT:** the identifier exemption is an ALLOWLIST of known credential prefixes,
  so it is open by default to every secret not on the list. See §4.

### F2 — P1 · CODE/honesty · `pr --principal` vs `check --principal` rigor split

`check`'s principal is scrubbed with the FULL engine
(`crate::redaction::scrub`, checks/run.rs:1140 — SAFE), but `pr`'s principal and
run-id use `structural_secret_scrub` (pr/mod.rs:1465-1472 — LEAKS, F1). The same
conceptual field is protected to two different standards because each call site
chose its own scrub. **ROOT:** there is no single choke point — every append site
re-decides redaction. (Proven: T6 leaks, T11 clean.)

### F3 — P1 · honesty (verification root) · the per-verb MATRIX cannot catch this class

`crates/hugit-cli/tests/acceptance_wj_matrix.rs` is the permanent guard the owner
banked after Round 6 ("the class can never silently regress"). Its `secrets()`
set (lines 81-90) is **exclusively prefixed/structural**: `ghp_`, `xoxb-`,
`clp_`, `Bearer`, `sk-proj-`, conn-string. It contains **no prefix-less
high-entropy specimen** in any identifier column. So the matrix is GREEN while F1
is wide open — the guard tests only the inputs the exemption already catches.
**TYPE:** honesty/verification. **ROOT:** the test enumerates the allowlist, not
the threat model; it can only ever confirm the allowlist, never find its gaps.

### F4 — P2 · seam · the two engines are kept "in lockstep" by hand

`redact.rs` and `porcelain.rs` carry duplicated detector code (`has_sk_key` /
`contains_sk_key`, `KNOWN_PREFIXES` / `KNOWN_SECRET_PREFIXES`,
`is_content_address_ref` ×2) with "kept in lockstep" comments. The structural
scrub exists ONLY because "the engine has no public hook to run the structural
detectors WITHOUT the entropy scan" (porcelain.rs:386-390). That missing hook IS
why mode 3 hand-rolls a weaker copy. **TYPE:** seam/maintainability — a standing
drift risk and the enabling condition for F1.

---

## 4. Root-cause analysis — the ONE structural reason

**The class recurs because redaction is ALLOWLIST-shaped at the one place it must
be deny-by-default.**

Every prior instance (Rounds 2–7: bare-hex, digest-key, `cas:`, `xoxb-` in
ident, `sk-proj-`) and this one (F1) are the same failure: a value is exempted
from the full entropy scan based on a **positive recognition** —
"is it a known prefix?", "is it digest-shaped?", "is it `cas:`?". A positive
recognition gate is open to everything it fails to recognize. Secrets are an
OPEN set (every SaaS mints a new prefix); an allowlist of secret shapes can never
close.

The identifier exemption (mode 3) is the purest form of it. Its stated reason is
real physics: an identifier is an ADDRESS — two distinct 40-hex keys must not
collapse to one `[REDACTED]`, or `pr land --pr B` lands `A` (the WJ-UNIFY
wrong-PR bug). So the entropy scan was dropped for identifiers. But "drop the
entropy scan" was implemented as "redact ONLY if it matches a known credential
prefix" — which silently lets EVERY prefix-less high-entropy credential through.
The address-survival requirement does NOT actually require letting arbitrary
high-entropy secrets survive; it only requires that *content-address-SHAPED*
values survive. Those are a narrow, recognizable, deny-by-default-able set
(40/64-hex, ULID/Crockford-base32, `<algo>:<hex>`, `cas:<CID>`). The code
inverted the gate: instead of "survive iff PROVABLY an address (else redact)" it
wrote "redact iff PROVABLY a known secret (else survive)."

The verification layer (F3) has the same inversion: the matrix enumerates known
secret prefixes and confirms they redact, instead of enumerating address shapes
and confirming everything else redacts.

So the single root is: **the identifier scrub allowlists secrets instead of
allowlisting addresses.** Flip that one polarity and the class is closed.

---

## 5. Recommended structural remediation — the class-killing fix

**Make the identifier exemption deny-by-default: redact unless the value is
PROVABLY a content-address / safe-identifier shape.** One surgical change, one
function.

Replace the body of `structural_secret_scrub` (porcelain.rs:401-410) so the
survival decision is a *positive address allowlist*, not a negative secret
allowlist:

```rust
pub fn structural_secret_scrub(s: &str) -> String {
    // Deny-by-default: an identifier value SURVIVES verbatim ONLY if it is
    // provably a safe address shape (a content address or a bounded
    // identifier charset). Anything else — including a prefix-less
    // high-entropy credential — REDACTS. This inverts the polarity that made
    // the exemption a hole: we no longer try to recognize every secret; we
    // recognize the small, closed set of legitimate addresses.
    if is_safe_identifier_shape(s) {
        s.to_string()
    } else {
        hugit_ledger::redact::REDACTED.to_string()
    }
}

/// The CLOSED set of address shapes an identifier may legitimately take and
/// still survive unredacted. Deny-by-default: not on this list ⇒ redact.
fn is_safe_identifier_shape(s: &str) -> bool {
    let t = s.trim();
    // 40/64-hex git-sha / content address, or <algo>:<hex>/cas:<CID> ref.
    if is_digest_shaped(t) { return true; }
    // Bounded identifier charset: ASCII alnum plus the address punctuation the
    // flow actually uses (`- _ / . @ :`), AND below the credential-length band
    // OR low-entropy. A ULID, `feature/login`, `pr-9f8e`, `user@host`,
    // `intent-abc123` all pass; a 40-char base64 AWS key (mixed case, dense,
    // long) does NOT — it clears the entropy floor and is not digest-shaped.
    is_bounded_address(t)
}
```

`is_bounded_address` is the address allowlist: it accepts the identifier charset
**and** rejects values that simultaneously (a) exceed the credential length floor
(`ENTROPY_MIN_LEN`, 20) and (b) clear the Shannon-entropy threshold — i.e. it
runs the *entropy detector as a NEGATIVE gate on identifiers* (a long dense
random run is rejected) while still letting an equally-long but LOW-entropy slug
(`feature/long-descriptive-branch-name`) or a structured hex address survive.
This is the one place the entropy signal belongs for identifiers: not "redact the
whole field on any entropy" (that collapses addresses) but "an identifier is not
allowed to BE a high-entropy non-hex blob." A real 40/64-hex address is
hex-shaped and exits early via `is_digest_shaped`; a ULID is base32 and
length-26, under the band; an AWS/SendGrid/Stripe key is long + dense + mixed
charset and redacts.

Because `ident.rs::validate_identifier` (ident.rs:110) is defined as "reject iff
`structural_secret_scrub` changes the value," this ONE change automatically
closes the door too — the AWS-shaped `--id` becomes an exit-2
`secret_in_identifier` at input, no separate edit needed. And because every
identifier write path already funnels through `scrub_to_canonical` /
`structural_secret_scrub` (the WJ-UNIFY unification — payload AND `pr`
principal-chain both call it, pr/mod.rs:1460-1472), the fix covers all 7 leaking
cells at one site. F2 collapses out for free once `pr` and `check` principals are
both routed through this one boundary.

**Make the fix permanent (closes F3):** add a prefix-less high-entropy specimen
(the AWS/SendGrid/Stripe/40-char-base64 shape) to `acceptance_wj_matrix.rs`'s
`secrets()` set, asserted against EVERY identifier column. Better: re-author the
matrix's column-(b) "address survives" set to be the *authoritative* definition
of `is_safe_identifier_shape`, so the test enumerates the address allowlist (the
closed set) rather than the secret allowlist (the open set) — the verification
layer then has the same deny-by-default polarity as the code.

**Why this kills the CLASS, not the instance:** after the flip there is exactly
one survival path for an identifier — "provably a bounded address shape" — and it
is a CLOSED set. A new SaaS prefix invented tomorrow does not open a new hole,
because the gate never asked "is this a known secret?"; it asks "is this a known
address?", and a random credential is not. The recurring "an exemption is a hole"
pattern ends because the exemption stops being a secret-allowlist.

---

## 6. Residual / accepted

- **Address-survival is irreducible physics (the constraint, not a hole).**
  Identifier addresses MUST survive verbatim (distinct keys must not collapse, or
  the wedge lands the wrong PR — WJ-UNIFY). The §5 fix HONORS this: it still lets
  every legitimate address shape through; it only refuses the high-entropy
  non-address blob. This is an accepted seam only in that *some* high-entropy
  value (a 64-hex content address) survives — but that value is, by definition,
  not a credential it could be confused with at rest (it is hash-shaped, and the
  digest path already blesses it everywhere).
- **A LOW-entropy secret deliberately used as an identifier** (e.g. a password
  `hunter2` jammed into `--id`) — `hunter2` is short + low-entropy, so it both
  passes `is_bounded_address` AND is not caught by detector (5) anywhere. This is
  the residual the keyword-context detector (4) covers in free text but cannot in
  a bare identifier with no `key=` context. Accepted as P2: a bare low-entropy
  string in an address field is indistinguishable from a legitimate short slug;
  the mitigation is the door's UX error + operator discipline (ident.rs SECRET_HINT),
  not the scrubber. This is genuinely irreducible (you cannot tell `hunter2` the
  password from `hunter2` the codename without context).
- **`cas:` / digest value-gates** — already deny-by-default since K-SCRUB
  (Round 7); re-verified clean here (T9/T12, and the `cas:ghp_…` unit tests).
  Not in scope to change.
- **The two-engine duplication (F4)** — accepted as a P2 maintainability seam,
  but the §5 fix REDUCES it: once `structural_secret_scrub` is the one
  address-gate, the engine's private detectors no longer need a hand-mirrored
  copy for the "structural minus entropy" use; a single public
  `redact::is_safe_identifier_shape` could be hoisted to the engine and called
  from both the door and the boundary, ending the lockstep-by-comment risk.
