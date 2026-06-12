//! Porcelain shared helpers — **the one error law + the one exit-code law**
//! (WP-WB0, SOTA-fix Wave B; PC0 scaffold before it).
//!
//! # The LLM-first contract
//!
//! hugit's primary typist is an orchestrated agent, so the machine shape IS the
//! contract. Every porcelain verb — flow (`campaign`/`intent`/`pr`) AND legacy
//! (`why`/`impact`/`tournament`/`export`) — converges on ONE error envelope and
//! ONE exit-code law. An agent parsing stdout always gets a machine-parseable
//! signal, never a fake success and never a bare string.
//!
//! ## One error law (THE canonical shape)
//!
//! A structured (non-crash) error is a single JSON object on **stdout**:
//!
//! ```json
//! {"error":{"kind":"…","message":"…","fix":"…", …context}}
//! ```
//!
//! - **nested** under a top-level `"error"` key (never a flat `{"kind":…}`),
//! - `kind` — a stable, machine-matchable error class (snake_case),
//! - `message` — the human/agent-readable description,
//! - **`fix`** is THE remediation key (NEVER `suggested_fix` — the P2 audit's
//!   schism: the legacy `intent` envelope still spells it `suggested_fix`; that
//!   module is the next WP's to converge — see the note in [`PorcelainError`]),
//! - any further keys are flat **context** folded into the `error` object
//!   (e.g. `"detail"`, `"path"`, `"seq"`), via [`PorcelainError::with_context`].
//!
//! ## One exit-code law
//!
//! - **`0`** — success.
//! - **`2`** — a structured user/domain error (the envelope above on stdout).
//!   This is [`PORCELAIN_ERROR_EXIT`].
//! - **`1`** — RESERVED for an internal fault (a bug, not the caller's input).
//!   An internal fault is ALSO emitted as JSON, with `kind:"internal"`, so even
//!   a crash-class fault stays machine-parseable. This is [`INTERNAL_FAULT_EXIT`].
//!
//! Module WPs converge on this law by constructing [`PorcelainError`] (or the
//! [`internal`] helper) and rendering with [`PorcelainError::to_json`] /
//! [`PorcelainError::exit_code`]. The PC0 stub shape ([`not_implemented`]) is a
//! `PorcelainError` of `kind:"not_implemented"` carrying the owning `wp`.

use std::process::ExitCode;

use serde_json::{Value, json};

/// The process exit code for a structured (non-crash) user/domain error — the
/// `{"error":{…}}` envelope on stdout. The one error law's exit code.
pub const PORCELAIN_ERROR_EXIT: u8 = 2;

/// The process exit code RESERVED for an internal fault (a bug, not bad input).
/// Also emitted as JSON (`kind:"internal"`) so even a fault stays parseable.
pub const INTERNAL_FAULT_EXIT: u8 = 1;

/// THE canonical structured porcelain error — rendered as `{"error":{…}}` JSON
/// on stdout, exit [`PORCELAIN_ERROR_EXIT`] (or [`INTERNAL_FAULT_EXIT`] for the
/// `internal` kind).
///
/// One shape for every verb. `fix` is THE remediation key (never
/// `suggested_fix`). Extra structured context is folded flat into the `error`
/// object via [`with_context`](PorcelainError::with_context).
///
/// # Convergence note (the P2 audit)
///
/// The flow porcelain has two pre-existing error types that this law supersedes:
/// `campaign::CampaignError` (already `fix`-keyed, nested — compatible) and
/// `intent`'s `error::PorcelainError` (spells the remediation `suggested_fix` —
/// the divergence the audit flagged). Converging the `intent`/`pr`/`campaign`
/// CALL SITES onto this type is the next WP's; WB0 establishes the shape here +
/// converges the legacy verbs (`why`/`impact`/`tournament`/`export`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PorcelainError {
    /// Stable, machine-matchable error class (snake_case).
    kind: &'static str,
    /// Human/agent-readable description of what went wrong.
    message: String,
    /// THE remediation the caller (an agent) can act on. Never `suggested_fix`.
    fix: String,
    /// Extra structured context, folded flat into the `error` object on render.
    context: Vec<(&'static str, Value)>,
    /// Whether this is an internal fault (exit `1`) rather than a user/domain
    /// error (exit `2`). Set only via [`PorcelainError::internal`].
    internal: bool,
}

impl PorcelainError {
    /// A `kind` + `message` + `fix` error (the common case), exit
    /// [`PORCELAIN_ERROR_EXIT`].
    pub fn new(kind: &'static str, message: impl Into<String>, fix: impl Into<String>) -> Self {
        PorcelainError {
            kind,
            message: message.into(),
            fix: fix.into(),
            context: Vec::new(),
            internal: false,
        }
    }

    /// An **internal fault** (a bug, not the caller's input): `kind:"internal"`,
    /// exit [`INTERNAL_FAULT_EXIT`]. Still emitted as JSON so a fault stays
    /// machine-parseable.
    pub fn internal(message: impl Into<String>) -> Self {
        PorcelainError {
            kind: "internal",
            message: message.into(),
            fix: "this is an internal hugit bug; report it with the command + inputs".to_string(),
            context: Vec::new(),
            internal: true,
        }
    }

    /// Fold a flat structured context key into the `error` object (e.g.
    /// `("path", json!("src/a.rs"))`). Repeatable; insertion order preserved.
    pub fn with_context(mut self, key: &'static str, value: Value) -> Self {
        self.context.push((key, value));
        self
    }

    /// Render THE canonical `{"error":{"kind","message","fix", …context}}`
    /// envelope (the stable wire shape).
    pub fn to_json(&self) -> String {
        let mut error = json!({
            "kind": self.kind,
            "message": self.message,
            "fix": self.fix,
        });
        if let Some(map) = error.as_object_mut() {
            for (k, v) in &self.context {
                map.insert((*k).to_string(), v.clone());
            }
        }
        json!({ "error": error }).to_string()
    }

    /// The process exit code under the one exit-code law: `1` for an internal
    /// fault, else `2` for a structured user/domain error.
    pub fn exit_code(&self) -> ExitCode {
        if self.internal {
            ExitCode::from(INTERNAL_FAULT_EXIT)
        } else {
            ExitCode::from(PORCELAIN_ERROR_EXIT)
        }
    }

    /// The error's stable `kind` (for call-site convergence + tests).
    pub fn kind(&self) -> &'static str {
        self.kind
    }

    /// An I/O fault reading/writing a `--log`/`--store` path (user/domain).
    pub fn io(action: &str, path: &std::path::Path, e: &std::io::Error) -> Self {
        PorcelainError::new(
            "io",
            format!("{action} {}: {e}", path.display()),
            "check the --log path exists and is readable/writable",
        )
    }

    /// A malformed/truncated `--log` file (not valid canonical JSON). The one
    /// input-error law's `parse_log` kind — never silently an empty world.
    ///
    /// This helper is the **flow porcelain's** loader fault (campaign / intent /
    /// pr / checks / queue — the verbs that share the canonical
    /// `[EventRecord, …]` log). The legacy verbs `why`/`export` read their OWN
    /// distinct on-disk shapes (a why-entry wrapper array / an `{events:[…]}`
    /// object) and build their own `parse_log` error with a verb-specific shape
    /// hint in `main.rs` — they do NOT call this helper, so its fix text is
    /// accurate for every verb that DOES (no verb claims a shared format it does
    /// not honor — see `--log` consistency, WF item 4).
    pub fn parse_log(path: &std::path::Path, e: &serde_json::Error) -> Self {
        PorcelainError::new(
            "parse_log",
            format!("--log file {} is not valid JSON: {e}", path.display()),
            "the --log file must be a canonical JSON [EventRecord, …] array \
             (the engine's EventLog shape, shared by every FLOW porcelain verb — \
             campaign/intent/pr/checks/queue; why/export read their own shapes); \
             a truncated/corrupt file is rejected, never read as an empty world",
        )
        .with_context("path", json!(path.display().to_string()))
    }

    /// A `--log` FILE that does not exist (the path is absent on disk). Explicit
    /// — NEVER silently an empty world (the P5 audit finding). Exit `2`.
    ///
    /// Shared by the flow porcelain AND the legacy `why`/`export` reads (a
    /// missing file is the same canonical `log_not_found`/exit-2 everywhere);
    /// the canonical-shape hint below is the flow shape — `why`/`export` add
    /// their own verb-specific shape hint on the `parse_log` path in `main.rs`.
    pub fn log_not_found(path: &std::path::Path) -> Self {
        PorcelainError::new(
            "log_not_found",
            format!("--log file does not exist: {}", path.display()),
            "create the log first (e.g. `hugit intent new --log <path>` \
             bootstraps it) or point --log at an existing log in the shape this \
             verb reads (the flow porcelain shares the canonical \
             [EventRecord, …] array; why/export read their own shapes)",
        )
        .with_context("path", json!(path.display().to_string()))
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Scrub-on-append — THE structural redaction seam (WP-WG-SCRUB)
// ─────────────────────────────────────────────────────────────────────────────
//
// THE forbidden path is a porcelain verb that appends a USER-supplied string to
// the hash-chained, append-only `--log` WITHOUT routing it through the redaction
// engine first. The chain is forever: a `ghp_…`/JWT/conn-string appended verbatim
// is unredactable (re-hashing would break the chain), so the secret leaks for the
// life of the log.
//
// Per-FIELD redaction (Wave E) was fragile: each new verb had to remember to
// scrub each new field, and the wedge wave's new verbs (`check`, `verdict`,
// `pr abandon/queued/landed`, …) forgot — two fresh adversaries reproduced a live
// `ghp_…` leak. The fix is STRUCTURAL: every porcelain append builds a
// `serde_json::Value` payload and hands it to [`scrub_payload`], which recursively
// replaces EVERY user string value with the engine's wholesale
// [`REDACTED`](hugit_ledger::redact::REDACTED) sentinel when any detector fires —
// BEFORE the bytes reach the chain. A verb physically cannot forget a field: the
// whole tree is scrubbed by construction.
//
// **Keys are structural, not user content** — but the digest exemption is
// VALUE-GATED, not key-gated (WH-SCRUB, adversarial Round 4). The envelope's own
// content-address survival rule (see `hugit_ledger::redact`) says a digest VALUE
// must survive verbatim because it is load-bearing, never a secret. That holds
// ONLY when the value is ACTUALLY digest-shaped. A digest-NAMED field
// (`memo_key`/`tree_hash`/`commit`/`*_digest`/`hash`) carrying a non-digest value
// is the Round-4 hole: `hugit check --toolchain <ghp_…>` / `verdict --tree-hash
// <ghp_…>` route a RAW user flag into a digest-named field, and a pure key-name
// exemption persisted the secret VERBATIM in the forever-log (same class as the
// Round-2 hex leak). The fix: a digest-named field is exempt from the scrub ONLY
// IF its value is digest-shaped ([`is_digest_shaped`] — 64/40-hex OR a
// `sha256:`/`cas:`/`<algo>:` content-address prefix). A real `memo_key` /
// `sha256:abc…` survives; a smuggled secret in a digest-named field scrubs like
// any other value. A bare digest in a non-digest-named free-text field is (still)
// subject to the engine's entropy scan and may redact — the engine's WF-1 "err
// toward redaction in free text" law, not a regression.
//
// **Identifier fields are ADDRESSES, not free text** — {`campaign`, `intent_id`,
// `pr_id`, `run_id`} are keys the rest of the flow looks up by. Free-text-scrubbing
// them collapses two distinct 40-hex/high-entropy keys to one `[REDACTED]` (silent
// data loss) AND breaks addressing asymmetrically (open scrubs the key,
// abandon/close look up the raw key). But BLANKET-exempting them (the WH-SCRUB
// design) is a hole: the Round-5 adversaries reproduced a live leak —
// `campaign open --campaign <xoxb-…>` / `check --pr <secret>` smuggled a
// structurally-shaped credential into an identifier field and it reached the
// forever-log VERBATIM. The exemption leaned on the `ident.rs` input validator,
// which was both WEAKER than the engine (omitted `xoxb-`/`clp_`/`Bearer`, used
// `starts_with` not substring, no trim) AND not called on every write path.
//
// WI-SCRUB makes the boundary itself safe: identifier values get a STRUCTURAL
// scrub ([`structural_secret_scrub`] — the engine's prefix / connection-string /
// JWT / PEM / keyword detectors), EXEMPT from only the bare-hex + entropy scan.
// A `xoxb-`/`clp_`/`Bearer`/conn-string/JWT/PEM in an identifier REDACTS (no verb
// can leak it — the security boundary no longer depends on per-verb validation);
// a 40/64-hex content address or a dense high-entropy slug id SURVIVES verbatim
// (no collapse, addressing stays symmetric). The `ident.rs` validator is now a UX
// nicety (a clear error at input), not the security boundary; another WP owns it.
// (Free-text fields — charter / reason / owner / summary / lens names — are NOT
// identifiers and still get the full free-text scrub.)

/// Recursively scrub EVERY user-supplied string VALUE in a porcelain payload
/// through the redaction engine ([`crate::redaction::scrub`] →
/// [`hugit_ledger::redact::apply`]), IN PLACE, BEFORE the payload is serialized
/// and appended to the hash-chained log.
///
/// Object KEYS are structural and never scrubbed. The per-value scrub mode is
/// decided per `(key, value)` as the tree descends ([`scrub_mode`]): a digest-NAMED
/// field survives verbatim ONLY IF its value is digest-SHAPED ([`is_digest_shaped`]);
/// an identifier-address field ([`is_identifier_key`]) gets the STRUCTURAL-secret
/// scrub ([`structural_secret_scrub`] — a prefixed/conn-string/JWT/PEM secret
/// redacts, a 40/64-hex address survives); every other string value gets the full
/// free-text scrub. Arrays inherit their parent key's mode; nested objects
/// re-evaluate per child key.
///
/// This is THE seam: route every porcelain append through [`scrub_payload`]
/// (directly or via [`scrub_to_canonical`]). A raw append of an un-scrubbed user
/// payload is the forbidden path the WG-SCRUB / WH-SCRUB tests guard against.
pub fn scrub_payload(value: &mut Value) {
    scrub_value(value, ScrubMode::FreeText);
}

/// Inner recursion. `mode` is the scrub decision for the value reached at this
/// node (re-evaluated per object child by [`scrub_mode`]):
/// [`FreeText`](ScrubMode::FreeText) routes the string through the full engine,
/// [`Verbatim`](ScrubMode::Verbatim) leaves a digest-shaped value untouched, and
/// [`Structural`](ScrubMode::Structural) routes an identifier value through the
/// structural detectors only (so a prefixed secret REDACTS but an address
/// SURVIVES). Arrays propagate the parent's mode.
fn scrub_value(value: &mut Value, mode: ScrubMode) {
    match value {
        Value::String(s) => match mode {
            ScrubMode::FreeText => *s = crate::redaction::scrub(s),
            ScrubMode::Structural => *s = structural_secret_scrub(s),
            ScrubMode::Verbatim => {}
        },
        Value::Array(items) => {
            for item in items {
                scrub_value(item, mode);
            }
        }
        Value::Object(map) => {
            for (key, child) in map.iter_mut() {
                // Re-evaluate the mode at EACH (key, value): a digest key keeps
                // only a digest-SHAPED value verbatim; an identifier key gets the
                // structural-secret scrub (a prefixed secret redacts, an address
                // survives). Everything else (including a secret smuggled into a
                // digest-named field) scrubs as free text.
                scrub_value(child, scrub_mode(key, child));
            }
        }
        // Numbers / bools / null carry no user free-text — nothing to scrub.
        _ => {}
    }
}

/// The per-value scrub decision as the tree descends: SCRUB the value through the
/// free-text engine, leave it VERBATIM, or apply the STRUCTURAL-secret scrub
/// (identifier fields). Decided per `(key, value)`.
#[derive(Clone, Copy)]
enum ScrubMode {
    /// Route the string through the full free-text engine ([`crate::redaction::scrub`]).
    FreeText,
    /// Leave the string verbatim (a digest-SHAPED value under a digest key).
    Verbatim,
    /// Identifier-ADDRESS field: route through the STRUCTURAL detectors only
    /// (prefix / connection-string / JWT / PEM / keyword), EXEMPT from the
    /// bare-hex + entropy scan, so a `xoxb-`/`clp_`/`Bearer`/conn-string in an
    /// identifier REDACTS while a 40/64-hex address or high-entropy slug SURVIVES
    /// (no collapse, stays addressable). [WI-SCRUB]
    Structural,
}

/// Decide how the string under `key` (carrying `value`) is scrubbed as the tree
/// descends. Three modes ([`ScrubMode`]):
///
/// 1. **Digest fields, value-gated** ([`is_digest_key`] AND [`is_digest_shaped`])
///    → [`ScrubMode::Verbatim`]: a content address is load-bearing, never a
///    secret — but ONLY when its value is actually digest-shaped. A digest-NAMED
///    field carrying a non-digest value (a secret smuggled via
///    `--toolchain`/`--tree-hash`) falls through to [`ScrubMode::FreeText`] and
///    scrubs (WH-SCRUB, the Round-4 hole).
/// 2. **Identifier-address fields** ([`is_identifier_key`]) →
///    [`ScrubMode::Structural`]: keys the flow looks up by. They are NOT
///    blanket-exempt (the Round-5 hole: a `xoxb-`/`clp_`/`Bearer`/conn-string in
///    an identifier field reached the forever-log VERBATIM because the exemption
///    relied on an input validator that was both WEAKER than the engine and not
///    called on every write path). Instead the value is routed through the
///    engine's STRUCTURAL detectors at the scrub boundary itself — no verb can
///    leak a prefixed secret in an identifier — while the bare-hex/entropy scan
///    is skipped so a 40/64-hex address (or a high-entropy slug id) SURVIVES
///    verbatim (no collapse; addressing stays symmetric across open/abandon/close).
/// 3. Everything else → [`ScrubMode::FreeText`].
///
/// Identifier exemption is decided as a KEY (an identifier value is always a
/// string in practice); the digest branch only fires for a string value.
fn scrub_mode(key: &str, value: &Value) -> ScrubMode {
    if is_identifier_key(key) {
        return ScrubMode::Structural;
    }
    if is_digest_key(key) {
        // VALUE-GATED: a string value must be digest-shaped to survive; a
        // non-string (array/object) under a digest key recurses and is decided
        // per descendant, so do not blanket-exempt it here.
        return match value {
            Value::String(s) if is_digest_shaped(s) => ScrubMode::Verbatim,
            _ => ScrubMode::FreeText,
        };
    }
    ScrubMode::FreeText
}

/// Apply the STRUCTURAL-secret scrub to an identifier-field value (WI-SCRUB).
///
/// The free-text engine ([`hugit_ledger::redact::apply`]) fires on FIVE detector
/// classes; this routes an identifier value through the four **structural** ones —
/// known-prefix credentials (`ghp_`/`xoxb-`/`clp_`/`AKIA`/`sk-`/…), the JWT
/// (`eyJ`) and `Bearer ` prefixes, PEM private-key blocks, connection-string
/// passwords, and the keyword-context detector (`token=…`) — but DELIBERATELY
/// SKIPS the bare-hex + high-entropy scan. The result: a structurally-shaped
/// secret smuggled into an identifier field REDACTS (no verb can leak it), while a
/// 40/64-hex content-address or a dense high-entropy slug used as an address
/// SURVIVES verbatim (so two distinct addresses never collapse to one
/// `[REDACTED]` and the rest of the flow can still look them up).
///
/// Single-source-of-truth check: the engine has no public hook to run the
/// structural detectors WITHOUT the entropy scan (only the all-in `apply` is
/// public — verified 2026-06-11), and the engine MUST NOT be weakened, so the
/// minimal composition lives here. Each branch mirrors a private `redact::apply`
/// detector exactly; the bare-hex/entropy branch is the only one omitted.
///
/// **Public for the principal-chain seam (WJ-UNIFY).** Most identifier fields
/// reach the log inside the JSON payload, where [`scrub_payload`] applies this
/// scrub automatically by key ([`is_identifier_key`]). But a few identifier
/// values (`pr`'s `--run-id`/`--principal`) are stamped into the event's
/// `principal_chain`, which `append`/`append_authorized` hash VERBATIM (it is
/// NOT a JSON payload, so [`scrub_payload`] never sees it). Those call sites
/// apply this SAME structural scrub before building the chain, so a prefixed
/// secret in `--run-id`/`--principal` REDACTS while a bare-hex/slug address
/// SURVIVES — identical treatment to the payload, one boundary, no collapse.
///
/// ## DENY-BY-DEFAULT (L-A, adversarial Round 8 — the class-killing inversion)
///
/// The polarity is INVERTED. The historical design was a secret ALLOWLIST:
/// "redact iff the value matches a known credential prefix, else survive." That
/// is open by default — every prefix-less high-entropy credential (an AWS
/// 40-char base64 secret key, a SendGrid `SG.`, a Stripe `rk_live_`, a dense
/// 32-char base64 token) rode an identifier field VERBATIM into the forever
/// hash-chained log. It was the 6th instance of "an exemption is a hole": a
/// positive secret-recognition gate is open to everything it fails to recognise,
/// and secrets are an OPEN set (every SaaS mints a new prefix).
///
/// We now allowlist ADDRESSES (a CLOSED set) instead: an identifier value
/// survives verbatim ONLY if it PROVES it is a bounded safe-address shape
/// ([`is_safe_identifier_shape`]); anything else REDACTS. A new credential
/// format invented tomorrow does not open a hole, because the gate never asks
/// "is this a known secret?" — it asks "is this a known address?", and a random
/// credential is not. Address survival (distinct keys must not collapse —
/// WJ-UNIFY) is honoured: every legitimate address shape (40/64-hex, ULID,
/// `cas:<digest>`, kebab/snake slug, integer, short human name) passes.
pub fn structural_secret_scrub(s: &str) -> String {
    if is_safe_identifier_shape(s) {
        // Provably a bounded safe-address shape — survives verbatim (an address
        // must not collapse, or the wedge lands the wrong PR — WJ-UNIFY).
        s.to_string()
    } else {
        // Deny-by-default: not provably an address ⇒ REDACT. This catches every
        // prefix-less high-entropy credential the old secret-allowlist missed.
        hugit_ledger::redact::REDACTED.to_string()
    }
}

/// The CLOSED set of address shapes an identifier may legitimately take and
/// still survive unredacted (L-A). Deny-by-default: NOT on this list ⇒ redact.
///
/// Designed to be GENEROUS (a legit id wrongly redacted breaks the address), so
/// it accepts every shape the flow actually addresses by:
///
/// - a 40/64-hex git-sha / content address, or a `cas:`/`<algo>:` content-address
///   ref ([`is_digest_shaped`] — exits early; a real CID is high-entropy and MUST
///   survive);
/// - a ULID (Crockford base32, 26 chars), git short-hash (7–40 hex), pure
///   integers, kebab/snake/dotted slugs and short human names — the bounded
///   identifier charset (`[A-Za-z0-9._@:/+-]`), gated so it does NOT admit a
///   high-entropy credential blob.
///
/// REJECTS (→ redact): anything that trips a STRUCTURAL secret detector
/// ([`is_structural_secret`] — prefix/JWT/PEM/conn-string/keyword), AND any
/// value that is BOTH long (≥ the credential length floor) AND high Shannon
/// entropy AND not digest-shaped — i.e. a 40-char dense base64 AWS key, a
/// SendGrid/Stripe token, a 32-char dense base64 blob. When in doubt we prefer
/// redaction (security); the entropy floor is tuned so the legitimate address
/// shapes above all clear it.
pub fn is_safe_identifier_shape(s: &str) -> bool {
    let t = s.trim();
    if t.is_empty() {
        // An empty/whitespace value carries no secret and MUST pass through
        // UNCHANGED (return safe → the scrub is a no-op). Turning `""` into the
        // sentinel would mask emptiness from downstream checks — e.g. `pr open`'s
        // empty-`--run-id` binding test reads an empty run-id as "unbound" and
        // refuses; a `[REDACTED]` would look bound. Emptiness is the door's
        // separate `invalid_argument` rule, not the secret scrub's job.
        return true;
    }
    // A structural credential shape is never an address — redact.
    if is_structural_secret(t) {
        return false;
    }
    // A 40/64-hex digest or a `cas:`/`<algo>:` content-address ref is the
    // canonical high-entropy address that MUST survive (a real CID clears the
    // entropy floor — exit early before the entropy gate below would reject it).
    if is_digest_shaped(t) {
        return true;
    }
    // A ULID is the canonical intent-id shape — 26-char Crockford base32 — and is
    // high-entropy by construction (~4.6 bits/char), so it would trip the generic
    // entropy gate below. Recognise it EXPLICITLY as a structured address so it
    // survives. The Crockford base32 charset (no `I`/`L`/`O`/`U`) and the exact
    // length-26 are a narrow, closed shape; a credential is overwhelmingly not
    // length-26 base32 (the same accepted-residual class as a bare hex CID).
    if is_ulid_shaped(t) {
        return true;
    }
    // Bounded identifier charset only — a value outside it is not an address the
    // flow uses. `@`, `:`, `/`, `.` are the address punctuation the flow uses
    // (emails, branch refs, scoped ids); `_`/`-` are slug separators. (`+`/`=`
    // base64 padding chars are admitted to the charset but caught by the entropy
    // gate below if they form a dense blob.)
    if !t
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '@' | ':' | '/' | '+' | '-'))
    {
        return false;
    }
    // The one place the entropy signal belongs for identifiers: an identifier is
    // not allowed to BE a long, dense, high-entropy non-hex blob (an AWS /
    // SendGrid / Stripe key). A long LOW-entropy slug
    // (`feature/long-descriptive-branch-name`) survives; a long dense random run
    // redacts. Short values (< the credential floor) and structured hex
    // addresses (handled above) always survive.
    if t.len() >= IDENT_ENTROPY_MIN_LEN && ident_shannon_entropy(t) >= IDENT_ENTROPY_THRESHOLD {
        return false;
    }
    true
}

/// True iff `s` is a ULID — exactly 26 chars of Crockford base32
/// (`0-9A-HJKMNP-TV-Z`, i.e. no `I`/`L`/`O`/`U`). The canonical hugit intent-id
/// shape; high-entropy by construction so it needs an EXPLICIT survival path
/// (the generic entropy gate would otherwise redact it). A credential is
/// overwhelmingly not length-26 Crockford base32, so this is a narrow, closed
/// address shape (the same accepted-residual class as a bare hex content-id).
fn is_ulid_shaped(s: &str) -> bool {
    s.len() == 26
        && s.bytes().all(|b| {
            b.is_ascii_digit()
                || matches!(b,
                    b'A'..=b'H' | b'J' | b'K' | b'M' | b'N' | b'P'..=b'T' | b'V'..=b'Z')
        })
}

/// Minimum length before the identifier entropy gate considers a value a
/// possible credential blob. Mirrors the engine's `ENTROPY_MIN_LEN` (20) so a
/// short id is never entropy-rejected; a 40-char AWS key clears it.
const IDENT_ENTROPY_MIN_LEN: usize = 20;

/// Shannon-entropy threshold (bits/char) above which a long identifier run is
/// treated as a credential blob rather than an address. A ULID (Crockford
/// base32, structured) and human slugs sit well under; a dense random base64
/// AWS/SendGrid/Stripe key clears it. Set slightly above the engine's free-text
/// 4.0 so a ULID's structured base32 still SURVIVES (it is a real address) while
/// a 40-char dense mixed-case base64 key redacts.
const IDENT_ENTROPY_THRESHOLD: f64 = 4.5;

/// Shannon entropy (bits/char) of an identifier value — a local mirror of the
/// engine's private `shannon_entropy`, used only by the identifier address gate.
fn ident_shannon_entropy(s: &str) -> f64 {
    let bytes = s.as_bytes();
    if bytes.is_empty() {
        return 0.0;
    }
    let mut counts = [0usize; 256];
    for &b in bytes {
        counts[b as usize] += 1;
    }
    let len = bytes.len() as f64;
    let mut entropy = 0.0;
    for &count in counts.iter() {
        if count == 0 {
            continue;
        }
        let p = count as f64 / len;
        entropy -= p * p.log2();
    }
    entropy
}

/// True iff `s` trips a STRUCTURAL secret detector — the four `redact::apply`
/// detector classes that do NOT depend on entropy/bare-hex shape. Mirrors the
/// engine's private `is_secret` minus its detector (5). Kept in lockstep with
/// `hugit_ledger::redact`; the engine remains the law for free text.
fn is_structural_secret(s: &str) -> bool {
    // (1) the planted marker (same as the engine).
    if s.contains(hugit_ledger::redact::SECRET_MARKER) {
        return true;
    }
    // (2) known credential prefixes (substring match, as the engine does).
    if KNOWN_SECRET_PREFIXES.iter().any(|p| s.contains(p)) {
        return true;
    }
    // (2b) `sk-` with the engine's ≥20-token-char length gate.
    if contains_sk_key(s) {
        return true;
    }
    // PEM private-key blocks.
    if s.contains("-----BEGIN") && s.contains("PRIVATE KEY") {
        return true;
    }
    // (3) connection-string password.
    if contains_connection_string_password(s) {
        return true;
    }
    // (4) keyword-context secret (`token=…`, `password: …`).
    if contains_keyword_context_secret(s) {
        return true;
    }
    // (5) bare-hex + high-entropy scan — DELIBERATELY OMITTED so an address
    //     (40/64-hex content address or a dense high-entropy slug) survives.
    false
}

/// Known credential prefixes that are secrets by construction — the structural
/// mirror of the engine's `KNOWN_PREFIXES` (case-sensitive, as issuers mint
/// them). `sk-` is handled separately ([`contains_sk_key`]) with a length gate.
const KNOWN_SECRET_PREFIXES: &[&str] = &[
    "ghp_",
    "gho_",
    "ghs_",
    "github_pat_",
    "AKIA",
    "xoxb-",
    "xoxp-",
    "xoxo-",
    "xoxa-",
    "xoxs-",
    "clp_",
    "Bearer ",
    "eyJ",
];

/// Minimum run of base64/hex chars after `sk-` for the engine's `sk-` gate.
const SK_MIN_SUFFIX_LEN: usize = 20;

/// Recognised credential keywords (lowercase) for the keyword-context detector —
/// mirrors the engine's `KEYWORD_PREFIXES`.
const SECRET_KEYWORDS: &[&str] = &["password", "passwd", "secret", "token", "api_key", "pwd"];

/// A char that can appear inside a base64/hex token run (engine's `is_token_char`).
fn is_secret_token_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '+' || c == '/' || c == '=' || c == '-' || c == '_'
}

/// True iff `s` contains an `sk-` API key (mirrors the engine's `has_sk_key`):
/// the suffix run (including `-`/`_`) is ≥ [`SK_MIN_SUFFIX_LEN`] token chars, OR
/// it is the OpenAI project-key marker `proj-<non-empty id>` (so a short
/// `sk-proj-leaklens99999` redacts). Short non-key ids (`sk-256`, `sk-learn`)
/// do NOT fire. Kept in lockstep with `hugit_ledger::redact::has_sk_key`.
fn contains_sk_key(s: &str) -> bool {
    let needle = "sk-";
    let mut search = s;
    while let Some(pos) = search.find(needle) {
        let after = &search[pos + needle.len()..];
        let run: String = after
            .chars()
            .take_while(|&c| is_secret_token_char(c))
            .collect();
        if run.chars().count() >= SK_MIN_SUFFIX_LEN {
            return true;
        }
        if let Some(id) = run.strip_prefix("proj-")
            && !id.is_empty()
        {
            return true;
        }
        let advance = pos + needle.len();
        if advance >= search.len() {
            break;
        }
        search = &search[advance..];
    }
    false
}

/// True iff `s` carries a URL with an embedded `user:password@host` (mirrors the
/// engine's `has_connection_string_password`). A bare-hex address has no `://`,
/// so this never fires on one.
fn contains_connection_string_password(s: &str) -> bool {
    let mut search = s;
    while let Some(scheme_end) = search.find("://") {
        let authority_start = scheme_end + 3;
        if authority_start >= search.len() {
            break;
        }
        let authority_str = &search[authority_start..];
        let authority_len = authority_str
            .find(['/', '?', '#'])
            .unwrap_or(authority_str.len());
        let authority = &authority_str[..authority_len];
        if let Some(at_pos) = authority.find('@') {
            let userinfo = &authority[..at_pos];
            if let Some(colon_pos) = userinfo.find(':')
                && !userinfo[colon_pos + 1..].is_empty()
            {
                return true;
            }
        }
        search = &search[authority_start..];
    }
    false
}

/// True iff `s` contains a credential keyword immediately followed by `=`/`:` and
/// a non-empty value (mirrors the engine's `has_keyword_context_secret`). An
/// address has no `keyword=value` shape, so this never fires on one.
fn contains_keyword_context_secret(s: &str) -> bool {
    let lower = s.to_ascii_lowercase();
    let bytes = lower.as_bytes();
    for kw in SECRET_KEYWORDS {
        let mut pos = 0usize;
        while pos < lower.len() {
            let Some(kw_pos) = lower[pos..].find(kw) else {
                break;
            };
            let abs_kw_start = pos + kw_pos;
            let after_kw = abs_kw_start + kw.len();
            let before_ok = abs_kw_start == 0 || !bytes[abs_kw_start - 1].is_ascii_alphanumeric();
            if before_ok {
                let mut sep_idx = after_kw;
                while bytes
                    .get(sep_idx)
                    .is_some_and(|b| *b == b' ' || *b == b'\t')
                {
                    sep_idx += 1;
                }
                if let Some(sep_byte) = bytes.get(sep_idx)
                    && (*sep_byte == b'=' || *sep_byte == b':')
                {
                    let value = s.get(sep_idx + 1..).unwrap_or("").trim_start();
                    if !value.is_empty() {
                        return true;
                    }
                }
            }
            let advance = abs_kw_start + kw.len();
            if advance <= pos {
                break;
            }
            pos = advance;
        }
    }
    false
}

/// True iff `key` names a content-address / digest field. NAME-only — pair with
/// [`is_digest_shaped`] (the value gate) before exempting; a digest NAME alone no
/// longer exempts (WH-SCRUB).
///
/// The rule: exact `memo_key` / `tree_hash` / `commit`, any `*_digest` suffix
/// (`def_digest`, `toolchain_digest`, `prompt_digest`, …), or a bare `hash`
/// field (`files_read[].hash`).
pub fn is_digest_key(key: &str) -> bool {
    matches!(key, "memo_key" | "tree_hash" | "commit" | "hash") || key.ends_with("_digest")
}

/// True iff `value` is actually digest-SHAPED — the value gate for the digest
/// exemption (WH-SCRUB). A field is exempt from scrub ONLY when BOTH its key names
/// a digest AND its value matches this shape:
///
/// - a **bare** 40- or 64-char run of hex digits (sha-1 / sha-256), OR
/// - a content-address ref with an explicit algorithm prefix: `sha256:<hex>` /
///   `sha1:<hex>` / `cas:<payload>` / any recognised `<algo>:<40|64-hex>`.
///
/// A real `memo_key` (64-hex), a `sha256:abc…` ref, or a 40-hex `commit` survives;
/// a `ghp_…` / JWT / connection-string smuggled into a digest-named field does NOT
/// match → it falls through to the scrub. This is the local mirror of the ledger's
/// own `is_bare_hex_digest_shape` / `is_content_address_ref` (private there);
/// kept tight so a real digest survives and a secret-shaped value scrubs.
fn is_digest_shaped(value: &str) -> bool {
    is_bare_hex_digest(value) || is_content_address_ref(value)
}

/// A **bare** 40- or 64-char run of hex digits (no prefix) — the sha-1 / sha-256
/// content-address shapes. Mirrors `hugit_ledger::redact::is_bare_hex_digest_shape`.
fn is_bare_hex_digest(value: &str) -> bool {
    matches!(value.len(), 40 | 64) && value.bytes().all(|b| b.is_ascii_hexdigit())
}

/// A content-address ref carrying an explicit algorithm prefix: `cas:<payload>`
/// or `<algo>:<40|64-hex>` where `<algo>` names a recognised hash family.
///
/// VALUE-GATED (K-SCRUB, the Round-7 hole — the 5th "an exemption is a hole"):
/// the `cas:` prefix no longer blanket-exempts ANY payload. The payload after the
/// `cas:` prefix must be a genuine content-address SHAPE
/// ([`is_cas_payload_shaped`]) — a bare 40/64-hex run or a base32 CID — AND must
/// not trip a structural-secret detector ([`is_structural_secret`]). A
/// `cas:ghp_…` / `cas:<JWT>` / `cas:<conn-string>` is NOT a content address: it
/// falls through here and scrubs to `[REDACTED]`, never stored verbatim. A real
/// `cas:<64-hex>` (or base32 CID) still survives — addressability preserved.
///
/// Mirrors `hugit_ledger::redact::is_content_address_ref` (kept in lockstep).
fn is_content_address_ref(value: &str) -> bool {
    if let Some(payload) = value.strip_prefix("cas:") {
        // VALUE-GATE: a `cas:` ref survives only when its payload is genuinely
        // content-address shaped AND not structurally a secret. Both gates: the
        // shape rejects credential charsets (`_`, uppercase prefixes); the
        // structural detector is the authoritative belt-and-braces guard.
        return is_cas_payload_shaped(payload) && !is_structural_secret(payload);
    }
    let Some((algo, hex)) = value.split_once(':') else {
        return false;
    };
    is_digest_algo(algo)
        && matches!(hex.len(), 40 | 64)
        && hex.bytes().all(|b| b.is_ascii_hexdigit())
}

/// True iff `payload` (the part after a `cas:` prefix) is a genuine
/// content-address SHAPE: either a bare 40/64-char hex run (sha-1 / sha-256) or a
/// base32 content-id (lowercase `[a-z2-7]`, the CIDv1/multibase-b charset, of a
/// content-address-plausible length). A credential smuggled behind `cas:` (`ghp_…`,
/// a JWT, a connection string) does NOT match this charset/length, so it is not a
/// content address. Mirrors `hugit_ledger::redact::is_cas_payload_shaped`.
fn is_cas_payload_shaped(payload: &str) -> bool {
    // Bare hex content address (sha-1 / sha-256).
    if matches!(payload.len(), 40 | 64) && payload.bytes().all(|b| b.is_ascii_hexdigit()) {
        return true;
    }
    // base32 content-id: lowercase RFC-4648 base32 charset (`a-z2-7`), of a
    // content-address-plausible length (a CIDv1 base32 of a sha-256 is ~59
    // chars; allow the 32–64 band that real content ids fall in).
    matches!(payload.len(), 32..=64)
        && payload
            .bytes()
            .all(|b| b.is_ascii_lowercase() || (b'2'..=b'7').contains(&b))
}

/// Recognised content-address algorithm tags (case-insensitive). Mirrors the
/// ledger's `is_digest_algo`. A bare unknown prefix does NOT exempt.
fn is_digest_algo(algo: &str) -> bool {
    matches!(
        algo.to_ascii_lowercase().as_str(),
        "sha1" | "sha256" | "sha-1" | "sha-256" | "sha512" | "sha-512" | "blake3" | "cas" | "oid"
    )
}

/// True iff `key` names an identifier-ADDRESS field — a key the flow looks up by,
/// not free text. These get the STRUCTURAL-secret scrub ([`structural_secret_scrub`]),
/// NOT the full free-text scrub: a prefixed/conn-string/JWT/PEM secret REDACTS,
/// but a 40/64-hex content address (or a high-entropy slug id) SURVIVES verbatim
/// so distinct keys never collapse to one `[REDACTED]` and addressing stays
/// symmetric (open/abandon/close all see the same raw key).
///
/// WI-SCRUB (adversarial Round 5) made the scrub boundary itself safe for these
/// fields. The earlier WH-SCRUB design BLANKET-exempted them, leaning on WH-IDENT
/// to validate at input — but that input validator was both WEAKER than the engine
/// (omitted `xoxb-`/`clp_`/`Bearer`, used `starts_with` not substring, no trim) AND
/// not called on every write path (`check --pr <secret>` never validated `pr_id`),
/// so a secret in an identifier reached the forever-log VERBATIM. The structural
/// scrub here closes that leak for ALL verbs at the boundary, independent of any
/// per-verb validation; the `ident.rs` input validator remains a UX nicety (a clear
/// error at input), owned by another WP. Free-text fields (charter / reason / owner
/// / summary / lens names) are NOT identifiers and still scrub in full.
pub fn is_identifier_key(key: &str) -> bool {
    // `id` is the bare identifier key a payload may carry (e.g. `intent new
    // --id`'s explicit id, before it is renamed `intent_id`); WJ-UNIFY adds it
    // so the explicit `--id` address gets the same structural-not-collapse scrub
    // as `intent_id`/`pr_id`/`campaign`/`run_id`. A prefixed secret redacts; a
    // ULID/40-hex/slug address survives — one boundary, every identifier key.
    matches!(key, "campaign" | "intent_id" | "pr_id" | "run_id" | "id")
}

/// Scrub a payload [`Value`] ([`scrub_payload`]) and return it as a **canonical**
/// JSON string (sorted keys, no insignificant whitespace — the byte shape the
/// hash chain covers), ready to hand to `append`/`append_authorized`.
///
/// This is the one-call convenience for the porcelain append sites: build the
/// `Value`, call [`scrub_to_canonical`], append the result. The canonicalisation
/// re-uses [`hugit_refstore::canonical_json`] so the appended bytes match what the
/// rest of the flow porcelain emits; if (impossibly) re-canonicalisation fails the
/// compact serialization is returned unchanged.
pub fn scrub_to_canonical(mut value: Value) -> String {
    scrub_payload(&mut value);
    let compact = value.to_string();
    hugit_refstore::canonical_json(&compact).unwrap_or(compact)
}

/// Emit the canonical NOT-IMPLEMENTED error as JSON on **stdout** and return the
/// structured-error exit code.
///
/// Shape (stable contract for callers): `{"error":{"kind":"not_implemented",
/// "wp":"…","message":…,"fix":…}}`. Honest stub — never a fake success. `wp`
/// names the work package that will replace the stub with the real projection.
pub fn not_implemented(wp: &str) -> ExitCode {
    println!("{}", not_implemented_json(wp));
    ExitCode::from(PORCELAIN_ERROR_EXIT)
}

/// The canonical NOT-IMPLEMENTED [`PorcelainError`] for `wp` — a `PorcelainError`
/// of `kind:"not_implemented"` carrying the owning WP token as flat context.
fn not_implemented_error(wp: &str) -> PorcelainError {
    PorcelainError::new(
        "not_implemented",
        "this verb is a scaffold stub; its projection is not implemented yet",
        "track the owning work package; the stub never returns a fake success",
    )
    .with_context("wp", json!(wp))
}

/// The canonical NOT-IMPLEMENTED JSON line for `wp` (the stable wire shape).
///
/// Split out from [`not_implemented`] so the exact contract can be asserted in
/// tests without capturing stdout. `wp` is a fixed ASCII WP token, never
/// untrusted input.
pub fn not_implemented_json(wp: &str) -> String {
    not_implemented_error(wp).to_json()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_json_is_the_one_canonical_shape() {
        let e = PorcelainError::new("k", "m", "f");
        let v: Value = serde_json::from_str(&e.to_json()).unwrap();
        // nested under "error", with fix (NOT suggested_fix) as THE key.
        assert_eq!(v["error"]["kind"], "k");
        assert_eq!(v["error"]["message"], "m");
        assert_eq!(v["error"]["fix"], "f");
        assert!(v["error"].get("suggested_fix").is_none());
        assert!(v.get("kind").is_none(), "must be nested, never flat");
    }

    #[test]
    fn context_folds_flat_into_the_error_object() {
        let e = PorcelainError::new("k", "m", "f")
            .with_context("path", json!("src/a.rs"))
            .with_context("seq", json!(7));
        let v: Value = serde_json::from_str(&e.to_json()).unwrap();
        assert_eq!(v["error"]["path"], "src/a.rs");
        assert_eq!(v["error"]["seq"], 7);
    }

    #[test]
    fn user_error_is_exit_two() {
        let e = PorcelainError::new("k", "m", "f");
        assert_eq!(e.exit_code(), ExitCode::from(PORCELAIN_ERROR_EXIT));
        assert_eq!(PORCELAIN_ERROR_EXIT, 2);
    }

    #[test]
    fn internal_fault_is_kind_internal_and_exit_one() {
        let e = PorcelainError::internal("the rollup accumulator overflowed");
        let v: Value = serde_json::from_str(&e.to_json()).unwrap();
        assert_eq!(v["error"]["kind"], "internal");
        assert!(v["error"]["fix"].is_string());
        assert_eq!(e.exit_code(), ExitCode::from(INTERNAL_FAULT_EXIT));
        assert_eq!(INTERNAL_FAULT_EXIT, 1);
    }

    #[test]
    fn log_not_found_is_explicit_not_an_empty_world() {
        let e = PorcelainError::log_not_found(std::path::Path::new("/no/such.json"));
        let v: Value = serde_json::from_str(&e.to_json()).unwrap();
        assert_eq!(v["error"]["kind"], "log_not_found");
        assert_eq!(v["error"]["path"], "/no/such.json");
    }

    #[test]
    fn parse_log_carries_the_path_context() {
        let bad: serde_json::Error = serde_json::from_str::<Value>("{trunc").unwrap_err();
        let e = PorcelainError::parse_log(std::path::Path::new("/x/log.json"), &bad);
        let v: Value = serde_json::from_str(&e.to_json()).unwrap();
        assert_eq!(v["error"]["kind"], "parse_log");
        assert_eq!(v["error"]["path"], "/x/log.json");
    }

    #[test]
    fn not_implemented_json_is_the_canonical_envelope() {
        let v: Value = serde_json::from_str(&not_implemented_json("WB2")).unwrap();
        assert_eq!(v["error"]["kind"], "not_implemented");
        assert_eq!(v["error"]["wp"], "WB2");
        assert!(v["error"]["fix"].is_string());
    }

    // ── WG-SCRUB: scrub-on-append structural seam ────────────────────────────

    const GHP: &str = "ghp_16C7e42F292c6912E7710c838347Ae178B4a";
    const REDACTED: &str = hugit_ledger::redact::REDACTED;

    #[test]
    fn scrub_payload_redacts_every_user_string_value() {
        // A planted PAT in a free-text value redacts; benign prose survives;
        // KEYS are never touched (they are structural).
        let mut v = json!({
            "name": format!("deploy with {GHP}"),
            "reason": "abandoned for cause",
            "ghp_key_as_name": "value",
        });
        scrub_payload(&mut v);
        assert_eq!(v["name"], REDACTED, "secret in a value must redact");
        assert_eq!(v["reason"], "abandoned for cause", "benign prose survives");
        // The key `ghp_key_as_name` contains `ghp_` but a KEY is structural — it
        // is NOT scrubbed (only the VALUE is), and the benign value survives.
        assert!(
            v.get("ghp_key_as_name").is_some(),
            "structural key is preserved verbatim"
        );
        assert_eq!(v["ghp_key_as_name"], "value");
    }

    #[test]
    fn scrub_payload_exempts_digest_keyed_values() {
        // A content-address / digest VALUE must SURVIVE verbatim — even a bare
        // 64-hex run that free text would (correctly) redact. The exemption is
        // by KEY: memo_key / tree_hash / commit / *_digest / hash.
        let bare_64 = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
        let bare_40 = "da39a3ee5e6b4b0d3255bfef95601890afd80709";
        let mut v = json!({
            "memo_key": bare_64,
            "tree_hash": bare_64,
            "commit": bare_40,
            "def_digest": bare_64,
            "toolchain_digest": bare_64,
            "prompt_digest": bare_64,
            "hash": bare_64,
            // A non-digest free-text field carrying the SAME bare hex MUST redact.
            "name": bare_64,
        });
        scrub_payload(&mut v);
        for key in [
            "memo_key",
            "tree_hash",
            "commit",
            "def_digest",
            "toolchain_digest",
            "prompt_digest",
            "hash",
        ] {
            assert_ne!(v[key], REDACTED, "digest field `{key}` must survive");
        }
        assert_eq!(v["commit"], bare_40);
        assert_eq!(
            v["name"], REDACTED,
            "the SAME bare hex in a non-digest field redacts (engine's free-text law)"
        );
    }

    #[test]
    fn is_digest_key_matches_the_envelope_rule() {
        for k in [
            "memo_key",
            "tree_hash",
            "commit",
            "hash",
            "def_digest",
            "toolchain_digest",
            "prompt_digest",
        ] {
            assert!(is_digest_key(k), "`{k}` must be exempt");
        }
        for k in [
            "name", "reason", "intent", "lens", "pr_id", "campaign", "charter",
        ] {
            assert!(!is_digest_key(k), "`{k}` must NOT be exempt");
        }
    }

    #[test]
    fn scrub_payload_recurses_into_arrays_and_nested_objects() {
        let mut v = json!({
            "claims_checked": ["security:approve", format!("note {GHP}")],
            "nested": { "reason": format!("see {GHP}"), "tree_hash": "abc" },
            "intent_ids": [GHP, "i-2"],
        });
        scrub_payload(&mut v);
        assert_eq!(v["claims_checked"][0], "security:approve");
        assert_eq!(v["claims_checked"][1], REDACTED, "array element scrubs");
        assert_eq!(
            v["nested"]["reason"], REDACTED,
            "nested object value scrubs"
        );
        // A digest key inside a nested object is still exempt.
        assert_eq!(v["nested"]["tree_hash"], "abc");
        assert_eq!(v["intent_ids"][0], REDACTED);
        assert_eq!(v["intent_ids"][1], "i-2");
    }

    #[test]
    fn scrub_to_canonical_is_sorted_and_scrubbed() {
        // The forbidden path (a raw, un-scrubbed append) is closed: the helper
        // returns canonical (sorted-key) JSON with the secret already redacted.
        let out = scrub_to_canonical(json!({
            "tree_hash": "deadbeef",
            "name": format!("token {GHP}"),
        }));
        // Sorted keys: name before tree_hash.
        assert_eq!(
            out,
            format!(r#"{{"name":"{REDACTED}","tree_hash":"deadbeef"}}"#)
        );
        // Re-canonicalises to itself (stable wire bytes).
        assert_eq!(hugit_refstore::canonical_json(&out).unwrap(), out);
    }

    // ── WH-SCRUB (adversarial Round 4): the digest exemption is VALUE-gated ───

    /// A real 64-hex content address (sha-256 of the empty string).
    const DIGEST_64: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
    /// A real 40-hex content address (sha-1 of the empty string).
    const DIGEST_40: &str = "da39a3ee5e6b4b0d3255bfef95601890afd80709";

    #[test]
    fn digest_named_field_with_a_secret_value_now_scrubs() {
        // THE Round-4 hole: `check --toolchain <ghp_…>` routes a RAW user flag
        // into `toolchain_digest`, `verdict --tree-hash <ghp_…>` into `tree_hash`.
        // A pure key-name exemption persisted the secret VERBATIM. Value-gated:
        // the value is NOT digest-shaped → it scrubs like any other value.
        let mut v = json!({
            "toolchain_digest": GHP,                       // check --toolchain <secret>
            "tree_hash": format!("smuggled {GHP}"),        // verdict --tree-hash <secret>
            "memo_key": "postgres://u:S3cr3tP4ssw0rdVeryLongRandomToken9999@h:5432/d",
            "def_digest": GHP,
            "hash": GHP,
            "commit": GHP,
        });
        scrub_payload(&mut v);
        for key in [
            "toolchain_digest",
            "tree_hash",
            "memo_key",
            "def_digest",
            "hash",
            "commit",
        ] {
            assert_eq!(
                v[key], REDACTED,
                "a non-digest-shaped value in digest-named `{key}` MUST scrub"
            );
        }
    }

    #[test]
    fn real_digest_values_still_survive_the_value_gate() {
        // The exemption MUST still hold for an actual content address: a 64-hex
        // memo_key, a 40-hex commit, and prefixed `sha256:`/`cas:` refs survive.
        let mut v = json!({
            "memo_key": DIGEST_64,
            "tree_hash": format!("sha256:{DIGEST_64}"),
            "commit": DIGEST_40,
            "toolchain_digest": format!("sha256:{DIGEST_64}"),
            "def_digest": format!("cas:{DIGEST_64}"),
            "hash": DIGEST_64,
        });
        scrub_payload(&mut v);
        assert_eq!(v["memo_key"], DIGEST_64, "real 64-hex memo_key survives");
        assert_eq!(
            v["tree_hash"],
            format!("sha256:{DIGEST_64}"),
            "sha256: ref survives"
        );
        assert_eq!(v["commit"], DIGEST_40, "real 40-hex commit survives");
        assert_eq!(v["def_digest"], format!("cas:{DIGEST_64}"), "cas: survives");
        assert_eq!(v["hash"], DIGEST_64, "bare-hex hash survives");
    }

    #[test]
    fn identifier_address_keys_are_exempt_and_do_not_collapse() {
        // {campaign, intent_id, pr_id, run_id} are ADDRESSES, not free text:
        // scrubbing them would collapse distinct keys to one [REDACTED] (silent
        // data loss) and break addressing. Under L-A deny-by-default, a value
        // survives iff it is a PROVABLE address shape (40/64-hex, ULID, slug, …).
        let camp_a = "a1b2c3d4e5f60718293a4b5c6d7e8f9012345678"; // 40-hex
        let camp_b = "ffeeddccbbaa99887766554433221100ffeeddcc"; // distinct 40-hex
        let mut v = json!({
            "campaign": camp_a,
            "intent_id": "01HQXW8ZK4M9P2N7R3T5V6Y8BC", // ULID address survives
            "pr_id": "pr-9f8e7d6c",
            "run_id": "run-0011-2233",
        });
        scrub_payload(&mut v);
        // Two distinct campaign keys must stay distinct (no collapse).
        assert_eq!(v["campaign"], camp_a, "campaign address survives (A)");
        assert_ne!(v["campaign"], camp_b, "distinct keys do NOT collapse");
        assert_eq!(
            v["intent_id"], "01HQXW8ZK4M9P2N7R3T5V6Y8BC",
            "intent_id ULID address survives (explicit ULID shape)"
        );
        assert_eq!(v["pr_id"], "pr-9f8e7d6c", "pr_id address survives");
        assert_eq!(v["run_id"], "run-0011-2233", "run_id address survives");
    }

    #[test]
    fn a_secret_in_a_free_text_charter_still_redacts() {
        // The identifier/digest exemptions must NOT widen into free text: a PAT
        // in `charter`/`reason`/`owner`/`summary` still redacts.
        let mut v = json!({
            "charter": format!("ship with {GHP}"),
            "reason": format!("rotating {GHP}"),
            "owner": format!("ops {GHP}"),
            "summary": format!("note {GHP}"),
            "lens": format!("security {GHP}"),
        });
        scrub_payload(&mut v);
        for key in ["charter", "reason", "owner", "summary", "lens"] {
            assert_eq!(v[key], REDACTED, "free-text `{key}` still scrubs");
        }
    }

    #[test]
    fn is_digest_shaped_predicate() {
        // Bare 40/64-hex and prefixed content-address refs are digest-shaped.
        assert!(is_digest_shaped(DIGEST_64));
        assert!(is_digest_shaped(DIGEST_40));
        assert!(is_digest_shaped(&format!("sha256:{DIGEST_64}")));
        assert!(is_digest_shaped(&format!("sha1:{DIGEST_40}")));
        // K-SCRUB: a `cas:` ref is digest-shaped ONLY with a content-address
        // payload (64-hex / base32 CID), not any string.
        assert!(is_digest_shaped(&format!("cas:{DIGEST_64}")));
        assert!(is_digest_shaped(
            "cas:bafybeigdyrzt5sfp7udm7hu76uh7y26nf3efuylqabf3oclgtqy55fbzdi"
        ));
        assert!(!is_digest_shaped("cas:anything-here")); // not a content-address shape
        // K-SCRUB: a credential smuggled behind `cas:` is NOT digest-shaped.
        assert!(!is_digest_shaped(&format!("cas:{GHP}")));
        // Secrets / short / unknown-prefix are NOT digest-shaped.
        assert!(!is_digest_shaped(GHP));
        assert!(!is_digest_shaped("deadbeef")); // 8 hex, too short
        assert!(!is_digest_shaped(&format!("token:{DIGEST_64}"))); // unknown algo
        assert!(!is_digest_shaped("postgres://u:p@h:5432/d"));
    }

    #[test]
    fn is_content_address_ref_value_gates_the_cas_prefix() {
        // K-SCRUB unit proof: `cas:<64hex>` / `cas:<base32 CID>` ARE refs; a
        // `cas:<credential>` is NOT (the 5th "an exemption is a hole" closed).
        assert!(is_content_address_ref(&format!("cas:{DIGEST_64}")));
        assert!(is_content_address_ref(&format!("cas:{DIGEST_40}")));
        assert!(is_content_address_ref(
            "cas:bafybeigdyrzt5sfp7udm7hu76uh7y26nf3efuylqabf3oclgtqy55fbzdi"
        ));
        assert!(!is_content_address_ref(&format!("cas:{GHP}")));
        assert!(!is_content_address_ref("cas:anything-here"));
        // Non-cas prefixed forms unchanged.
        assert!(is_content_address_ref(&format!("sha256:{DIGEST_64}")));
        assert!(!is_content_address_ref(&format!("token:{DIGEST_64}")));
    }

    // ── WI-SCRUB (adversarial Round 5): structural-secret scrub for identifiers ─

    /// A realistic Slack bot token (the prefix `ident.rs` omitted).
    const SLACK: &str = "xoxb-2222222222-3333333333-abcdefghijklmnop";
    /// A realistic CoreLink PAT (a prefix `ident.rs` omitted).
    const CLP: &str = "clp_live_9f8e7d6c5b4a3210fedcba9876543210";

    #[test]
    fn prefixed_secret_in_an_identifier_field_now_redacts() {
        // THE Round-5 hole: a structurally-shaped credential smuggled into an
        // identifier field reached the forever-log VERBATIM under the blanket
        // exemption. Every structural class now redacts AT THE BOUNDARY — no
        // verb (check --pr, campaign --campaign, …) can leak it.
        let mut v = json!({
            "pr_id": SLACK,                                  // check --pr <xoxb-…>
            "campaign": GHP,                                 // campaign open --campaign <ghp_…>
            "intent_id": CLP,                                // <clp_…>
            "run_id": "Bearer abc123def456ghi789jkl",        // Bearer token
        });
        scrub_payload(&mut v);
        for key in ["pr_id", "campaign", "intent_id", "run_id"] {
            assert_eq!(
                v[key], REDACTED,
                "a structural secret in identifier `{key}` MUST redact"
            );
        }
    }

    #[test]
    fn connection_string_jwt_and_pem_in_identifiers_redact() {
        let mut v = json!({
            "campaign": "postgres://u:S3cr3tP4ssw0rdVeryLongRandomToken9999@h:5432/d",
            "intent_id": "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0In0.abc123def",
            "pr_id": "-----BEGIN RSA PRIVATE KEY-----\nMIIEpAIBA...",
            "run_id": "token=ghs_16C7e42F292c6912E7710c838347Ae178B4a",
        });
        scrub_payload(&mut v);
        for key in ["campaign", "intent_id", "pr_id", "run_id"] {
            assert_eq!(v[key], REDACTED, "structural secret in `{key}` redacts");
        }
    }

    #[test]
    fn hex_and_slug_addresses_survive_the_structural_scrub() {
        // The address-survival half: a 40/64-hex content address, a ULID, and a
        // plain slug MUST survive in an identifier field — distinct keys stay
        // distinct + lookup-able. (L-A: a dense high-entropy NON-address blob no
        // longer survives — see `dense_high_entropy_blob_in_identifier_redacts`.)
        let camp_a = "a1b2c3d4e5f60718293a4b5c6d7e8f9012345678"; // 40-hex
        let camp_b = "ffeeddccbbaa99887766554433221100ffeeddcc"; // distinct 40-hex
        let memo_64 = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
        let mut v = json!({
            "campaign": camp_a,
            "intent_id": "01HQXW8ZK4M9P2N7R3T5V6Y8BC",        // ULID address
            "pr_id": memo_64,                                 // 64-hex
            "run_id": "run-0011-2233",                        // plain slug
        });
        scrub_payload(&mut v);
        assert_eq!(
            v["campaign"], camp_a,
            "40-hex address survives (no collapse)"
        );
        assert_ne!(
            v["campaign"], camp_b,
            "distinct 40-hex keys do NOT collapse"
        );
        assert_eq!(
            v["intent_id"], "01HQXW8ZK4M9P2N7R3T5V6Y8BC",
            "ULID address survives (explicit ULID shape, not entropy-redacted)"
        );
        assert_eq!(v["pr_id"], memo_64, "64-hex address survives");
        assert_eq!(v["run_id"], "run-0011-2233", "plain slug survives");
    }

    #[test]
    fn dense_high_entropy_blob_in_identifier_redacts() {
        // L-A deny-by-default: a prefix-less high-entropy credential riding an
        // identifier field (the Round-8 root — AWS/SendGrid/Stripe/dense base64
        // keys carry no listed prefix, so the old secret-allowlist let them
        // through VERBATIM) now REDACTS. It is not a provable address shape.
        let aws = "wJalrXUtnFEMIK7MDENGbPxRfiCYEXAMPLEKEY123"; // 41-char AWS key shape
        let b64 = "aB3xZ9qL2mK7pR4tY8wN6vC1dF5gH0jS"; // 32-char dense base64
        let blob = "8Kp2mZ9qLx4vTn7wRj3sYb6cFd1gHe0"; // 31-char dense base64
        for (key, val) in [("campaign", aws), ("intent_id", b64), ("pr_id", blob)] {
            let mut v = json!({ key: val });
            scrub_payload(&mut v);
            assert_eq!(
                v[key], REDACTED,
                "a dense high-entropy blob in identifier `{key}` MUST redact (L-A)"
            );
        }
    }

    #[test]
    fn is_safe_identifier_shape_splits_addresses_from_blobs() {
        // Addresses SURVIVE (true).
        assert!(is_safe_identifier_shape(
            "a1b2c3d4e5f60718293a4b5c6d7e8f9012345678"
        )); // 40-hex
        assert!(is_safe_identifier_shape(
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        )); // 64-hex
        assert!(is_safe_identifier_shape("01HQXW8ZK4M9P2N7R3T5V6Y8BC")); // ULID
        assert!(is_safe_identifier_shape("auth-hardening")); // slug
        assert!(is_safe_identifier_shape("feature/login")); // branch ref
        assert!(is_safe_identifier_shape(
            "feature/long-descriptive-branch-name"
        )); // long low-entropy slug
        assert!(is_safe_identifier_shape("pr-9f8e7d6c"));
        assert!(is_safe_identifier_shape("run-0011-2233"));
        assert!(is_safe_identifier_shape("42")); // pure integer
        assert!(is_safe_identifier_shape("gustavo@humangr.com")); // email
        assert!(is_safe_identifier_shape("sk-256")); // short non-key sk- id
        // Blobs / credentials REDACT (false).
        assert!(!is_safe_identifier_shape(
            "wJalrXUtnFEMIK7MDENGbPxRfiCYEXAMPLEKEY123"
        )); // AWS
        assert!(!is_safe_identifier_shape(
            "aB3xZ9qL2mK7pR4tY8wN6vC1dF5gH0jS"
        )); // 32-char base64
        assert!(!is_safe_identifier_shape("8Kp2mZ9qLx4vTn7wRj3sYb6cFd1gHe0")); // 31-char dense blob
        assert!(!is_safe_identifier_shape(GHP)); // structural secret
        assert!(!is_safe_identifier_shape(
            "SG.aBcDeFgHiJkLmNoPqRsTuV.wXyZ0123456789aBcDeFgHiJkLmNoPqRsTuVwXyZ012"
        )); // SendGrid
        assert!(!is_safe_identifier_shape(
            "rk_live_51HxYzAbCdEfGhIjKlMnOpQrStUvWxYz0123456789"
        )); // Stripe
    }

    #[test]
    fn structural_secret_scrub_predicate_splits_secrets_from_addresses() {
        // Structural secrets redact.
        assert!(is_structural_secret(GHP));
        assert!(is_structural_secret(SLACK));
        assert!(is_structural_secret(CLP));
        assert!(is_structural_secret("Bearer abc123def456ghi789jkl"));
        assert!(is_structural_secret(
            "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0In0.abc"
        ));
        assert!(is_structural_secret(
            "postgres://u:S3cr3tP4ssw0rdVeryLongRandomToken9999@h:5432/d"
        ));
        assert!(is_structural_secret("token=hunter2"));
        assert!(is_structural_secret(
            "sk-abcdefghijklmnopqrstuvwxyz0123456789ABCDEF"
        ));
        assert!(is_structural_secret(
            "-----BEGIN RSA PRIVATE KEY-----\nMIIE"
        ));
        // Addresses + benign ids are NOT structural secrets (entropy/hex skipped).
        assert!(!is_structural_secret(
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        ));
        assert!(!is_structural_secret(
            "a1b2c3d4e5f60718293a4b5c6d7e8f9012345678"
        ));
        assert!(!is_structural_secret("8Kp2mZ9qLx4vTn7wRj3sYb6cFd1gHe0"));
        assert!(!is_structural_secret("run-0011-2233"));
        assert!(!is_structural_secret("sk-256")); // short sk- id survives
    }

    #[test]
    fn digest_named_field_with_a_real_digest_still_survives_alongside_identifiers() {
        // Cross-check: a sha256: digest field survives, and an identifier with a
        // secret redacts, in the SAME payload (the two exemption classes compose).
        let mut v = json!({
            "tree_hash": format!("sha256:{DIGEST_64}"),
            "memo_key": DIGEST_64,
            "campaign": SLACK,
        });
        scrub_payload(&mut v);
        assert_eq!(
            v["tree_hash"],
            format!("sha256:{DIGEST_64}"),
            "sha256: survives"
        );
        assert_eq!(v["memo_key"], DIGEST_64, "64-hex digest survives");
        assert_eq!(v["campaign"], REDACTED, "secret in identifier redacts");
    }

    #[test]
    fn is_identifier_key_set_is_exact() {
        for k in ["campaign", "intent_id", "pr_id", "run_id", "id"] {
            assert!(is_identifier_key(k), "`{k}` is an identifier address");
        }
        // Free-text fields are NOT identifiers (`intent` is the reviewed change,
        // `intent_ids` is plural — both still scrub).
        for k in [
            "charter",
            "reason",
            "owner",
            "summary",
            "lens",
            "intent",
            "intent_ids",
            "name",
        ] {
            assert!(!is_identifier_key(k), "`{k}` is NOT an identifier address");
        }
    }
}
