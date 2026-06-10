//! `hugit intent new` — author an intent through the real refstore path
//! (WP-PC2).
//!
//! An intent is the unit of orchestrated work: a **charter**, an **acceptance**
//! list, optionally bound to a **campaign**, authored normally by a SUBAGENT.
//! `new` builds the frozen [`IntentSidecar`] (the intent record), lands it onto
//! the local event log through the real
//! [`import_sidecar`](hugit_refstore::intent::import_sidecar) path — the same
//! seam the dogfood wave drives — and prints the stable result object:
//!
//! ```json
//! {"intent_id":"…","already_exists":false}
//! ```
//!
//! ## Idempotency (decided)
//!
//! Agents retry safely. The `intent_id` is either explicit (`--id`) or a
//! deterministic content hash of the charter / acceptance / campaign / author —
//! so an identical re-run resolves to the SAME id. `import_sidecar` refuses a
//! `DuplicateIntentId`; we catch that and return the existing id with
//! `already_exists:true`, exit 0 (never a fake second landing, never an error).
//!
//! ## Authorship
//!
//! An intent is authored by a subagent in the normal flow. `--agent <type>`
//! records the authoring agent type on the landing event's principal chain;
//! the honest default is `"main"` (the orchestrator), never an invented agent.

use std::path::Path;

use hugit_contracts::IntentSidecar;
use hugit_refstore::intent::import_sidecar;
use sha2::{Digest, Sha256};

use super::error::PorcelainError;
use super::store::IntentStore;

/// The honest default authoring agent type when `--agent` is not given.
pub const DEFAULT_AGENT: &str = "main";

/// The synthetic ref an authored (not-yet-pushed) intent lands onto in the
/// local hermetic store. Labelled as synthetic — fixture world until a real
/// push binds a git ref (no fake sha, same posture as the dogfood harness's
/// `dogfood:…` commit labels).
const AUTHORED_REF: &str = "refs/hugit/intents";

/// The inputs to `intent new` (transcribed from the binary's clap args so the
/// library owns the behavior, the binary owns only parsing).
pub struct NewIntent {
    /// Human-readable charter / description of what the intent intends to do.
    pub charter: String,
    /// The campaign key this intent is bound to.
    pub campaign: String,
    /// Acceptance criteria (the repeatable `--acceptance` items).
    pub acceptance: Vec<String>,
    /// Explicit intent id (idempotency key). `None` ⇒ a deterministic content
    /// hash is derived.
    pub id: Option<String>,
    /// The authoring agent type. `None` ⇒ [`DEFAULT_AGENT`].
    pub agent: Option<String>,
    /// Optional content-addressed ref to the full context blob (the envelope).
    pub context_ref: Option<String>,
}

/// The stable result object printed by `intent new`.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct NewResult {
    /// The id of the (now-landed) intent.
    pub intent_id: String,
    /// `true` when the intent already existed (idempotent re-run), else `false`.
    pub already_exists: bool,
}

/// Derive a deterministic intent id from the authored content, so an identical
/// re-run (no explicit `--id`) resolves to the same id and is idempotent.
///
/// `intent-<sha256-prefix>` over the length-prefixed authored fields (charter /
/// campaign / each acceptance item / agent) — order-stable, collision-safe by
/// framing. Not a security boundary; an authoring convenience.
fn derive_intent_id(charter: &str, campaign: &str, acceptance: &[String], agent: &str) -> String {
    /// Length-prefixed field push (4-byte BE len ‖ bytes) — framing makes the
    /// concatenation unambiguous, mirroring the refstore `LP(s)` primitive.
    fn field(hasher: &mut Sha256, s: &str) {
        hasher.update((s.len() as u64).to_be_bytes());
        hasher.update(s.as_bytes());
    }
    let mut hasher = Sha256::new();
    field(&mut hasher, charter);
    field(&mut hasher, campaign);
    field(&mut hasher, agent);
    hasher.update((acceptance.len() as u64).to_be_bytes());
    for item in acceptance {
        field(&mut hasher, item);
    }
    let digest = hex::encode(hasher.finalize());
    format!("intent-{}", &digest[..16])
}

/// Author an intent: build the sidecar, land it through the real refstore path,
/// persist the store, and return the stable result.
///
/// Idempotent: a duplicate id (explicit or content-derived) returns the
/// existing intent with `already_exists:true` and exit 0.
pub fn run(input: NewIntent, store_path: &Path) -> Result<NewResult, PorcelainError> {
    if input.charter.trim().is_empty() {
        return Err(PorcelainError::new(
            "invalid_argument",
            "charter is empty",
            "pass a non-empty --charter describing what the intent does",
        ));
    }
    if input.campaign.trim().is_empty() {
        return Err(PorcelainError::new(
            "invalid_argument",
            "campaign is empty",
            "pass --campaign <key> binding the intent to a campaign",
        ));
    }

    let agent = input.agent.unwrap_or_else(|| DEFAULT_AGENT.to_string());
    let intent_id = input.id.clone().unwrap_or_else(|| {
        derive_intent_id(&input.charter, &input.campaign, &input.acceptance, &agent)
    });

    let mut store = IntentStore::load(store_path).map_err(PorcelainError::from_store)?;

    // Idempotency: if this id already landed, return it unchanged (exit 0).
    if store.intent_for(&intent_id).map_err(PorcelainError::from_store)?.is_some() {
        return Ok(NewResult {
            intent_id,
            already_exists: true,
        });
    }

    let context_ref = input.context_ref.unwrap_or_default();
    let sidecar = IntentSidecar {
        intent_id: intent_id.clone(),
        charter: input.charter.clone(),
        acceptance: input.acceptance.clone(),
        context_ref: context_ref.clone(),
        // Frozen invariant: the sidecar is NEVER authoritative (B6④).
        authoritative: false,
    };

    // The principal chain records authorship as given (subagent normally),
    // bound to the campaign — honest provenance, not invented.
    let principal_chain = vec![format!("campaign:{}", input.campaign), format!("agent:{agent}")];
    let recorded_at = now_ms();

    // The REAL refstore path: land the sidecar onto the event log keyed by its
    // intent_id (emits `intent.landed`). The synthetic ref/target are labelled
    // as authored-not-pushed (fixture world until a push binds a git oid).
    import_sidecar(
        &mut store.log,
        &sidecar,
        AUTHORED_REF,
        &authored_target(&intent_id),
        principal_chain,
        recorded_at,
    )
    .map_err(|e| {
        // DuplicateIntentId is already handled by the pre-check above; any
        // remaining import error (e.g. empty id) is a structured argument fault.
        PorcelainError::new(
            "invalid_intent",
            e.to_string(),
            "ensure --charter is set and the intent id is non-empty",
        )
    })?;

    // Record the non-authoritative sidecar corpus and the disclosed envelope
    // ref (when given) by intent_id — one lifecycle, one id.
    store.sidecars.insert(intent_id.clone(), sidecar);
    if !context_ref.is_empty() {
        store.envelopes.insert(intent_id.clone(), context_ref);
    }
    store.save(store_path).map_err(PorcelainError::from_store)?;

    Ok(NewResult {
        intent_id,
        already_exists: false,
    })
}

/// The synthetic target oid for an authored-not-pushed intent — a content tag,
/// honestly labelled `authored:<id>` (never a fake 40-hex git sha).
fn authored_target(intent_id: &str) -> String {
    format!("authored:{intent_id}")
}

/// Current unix time in milliseconds (the landing event's observability
/// annotation; excluded from the hash pre-image by design).
fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}
