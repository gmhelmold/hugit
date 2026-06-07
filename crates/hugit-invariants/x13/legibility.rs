//! WP-X13 production surface — legibility × degradation/erasure.
//!
//! This module models the **legibility intersection**: it proves the HUMAN can
//! ALWAYS follow, in every substrate state. It does NOT re-prove degradation
//! (X11), the erasure cascade (X7), or deep-link integrity (X14); it composes
//! the human-facing *down-zoom* — the raw-commit view (D2, plain git), deep
//! links (D5, [`hugit_ledger::deeplink`]) and `why` (D10,
//! [`hugit_cli::why::resolver`]) — with the two adversarial substrate states:
//!
//! ① **Intelligence layer DEGRADED.** The down-zoom either resolves via plain
//!    git OR fails HONESTLY with an explicit "layer unavailable" state — NEVER a
//!    silent 404/blank. This is the human-side reading of whitepaper §9 lock 5
//!    (the degradation invariant): a valid git repo keeps serving, and when the
//!    smart layer is down the human is *told* so, not handed a blank screen that
//!    looks like an answer.
//!
//! ② **Erasure cascade (X7).** Following ANY chain — a deep-link chain, an
//!    attestation/provenance chain — reaches a resolvable target OR an honest
//!    TOMBSTONE, never a dangling/broken link. The human can always follow to a
//!    truthful endpoint.
//!
//! The load-bearing distinction this module enforces is **honesty vs. silence**:
//! a [`DownZoom::LayerUnavailable`] (explicit) and a [`Follow::Tombstone`]
//! (tamper-evident) are PASSES; a silent blank/`None`/empty masquerading as an
//! answer, or a [`Follow::Broken`] dangling link, are FAILS. The oracle in
//! `tests/acceptance_x13.rs` is RED on the silent/broken cases.
//!
//! Everything here is composition logic over the *consumed* surfaces (D2/D5/D10
//! as-built, the canonical hash-chain from `hugit-refstore`, the X7 tombstone
//! shape) — there is no new production behavior to ship.

use hugit_cli::why::resolver::{LogEntry as WhyLogEntry, ProvenanceAnswer, WhyQuery, resolve_why};
use hugit_contracts::event_record::EventRecord;
use hugit_ledger::deeplink::{ResolveResult, resolve as deeplink_resolve};
use hugit_refstore::{TamperError, verify_chain};
use serde::{Deserialize, Serialize};

// ── Item ① — the down-zoom under a DEGRADED intelligence layer ────────────────

/// The state of the "intelligence layer" — the smart index/resolver tier that
/// powers deep-links and `why`. The plain-git tier (raw-commit view) is a
/// SEPARATE substrate that keeps serving regardless (whitepaper §9 lock 5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntelligenceLayer {
    /// The smart layer is up: deep-links and `why` resolve normally.
    Healthy,
    /// The smart layer is DEGRADED/down: deep-links and `why` cannot run. The
    /// down-zoom must fall back to plain git OR say so honestly — never blank.
    Degraded,
}

/// The truthful outcome of a human down-zoom request, in any substrate state.
///
/// Every variant is an HONEST endpoint. There is deliberately **no** "blank" or
/// "empty" variant: a silent 404 is not representable as a success here — it can
/// only arrive as a programming error the oracle catches. The whole point of
/// item ① is that the down-zoom always lands on one of these truthful states.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "down_zoom", rename_all = "snake_case")]
pub enum DownZoom {
    /// Resolved via the plain-git substrate (the raw-commit view, D2). This
    /// works even when the intelligence layer is degraded — the git repo keeps
    /// serving. Carries the resolved object id / commit so it is never blank.
    ViaPlainGit {
        /// The raw object/commit id the human landed on (never empty).
        object: String,
    },
    /// Resolved via the (healthy) intelligence layer (deep-link / `why`).
    ViaIntelligenceLayer {
        /// A human-readable summary of what was resolved (never empty).
        summary: String,
    },
    /// The intelligence layer is unavailable AND no plain-git fallback applies
    /// for this leg. This is the HONEST failure: the human is explicitly told
    /// "layer unavailable", not handed a blank. Carries which leg degraded.
    LayerUnavailable {
        /// Which down-zoom leg degraded (`"deep_link"`, `"why"`).
        leg: String,
    },
}

/// The fixed honest message a [`DownZoom::LayerUnavailable`] surfaces to a human.
pub const LAYER_UNAVAILABLE_MSG: &str = "intelligence layer unavailable";

impl DownZoom {
    /// True iff this outcome is a TRUTHFUL endpoint a human can act on: either a
    /// real resolution (non-empty) or an explicit "layer unavailable".
    ///
    /// A success that carried an empty object/summary would be a silent blank
    /// masquerading as an answer — `is_honest` returns `false` for it, so the
    /// oracle goes RED. `LayerUnavailable` is always honest.
    pub fn is_honest(&self) -> bool {
        match self {
            DownZoom::ViaPlainGit { object } => !object.trim().is_empty(),
            DownZoom::ViaIntelligenceLayer { summary } => !summary.trim().is_empty(),
            DownZoom::LayerUnavailable { leg } => !leg.trim().is_empty(),
        }
    }

    /// True iff this outcome is the explicit honest-failure state.
    pub fn is_layer_unavailable(&self) -> bool {
        matches!(self, DownZoom::LayerUnavailable { .. })
    }
}

/// The raw-commit view (D2) — the plain-git down-zoom leg.
///
/// This leg is served by the GIT substrate, NOT the intelligence layer, so it
/// resolves identically whether the smart layer is healthy or degraded. That is
/// the core of lock 5: a valid git repo keeps serving. Returns
/// [`DownZoom::LayerUnavailable`] ONLY if the object id is itself missing from
/// git — and even then never a silent blank.
///
/// `git_objects` models the plain-git object store (commit/blob ids → present).
pub fn raw_commit_view(_layer: IntelligenceLayer, object: &str, git_objects: &[&str]) -> DownZoom {
    // Plain git ignores the intelligence layer entirely.
    if git_objects.contains(&object) && !object.trim().is_empty() {
        DownZoom::ViaPlainGit {
            object: object.to_string(),
        }
    } else {
        // The object is not in git either — honest "unavailable", never blank.
        DownZoom::LayerUnavailable {
            leg: "raw_commit_view".to_string(),
        }
    }
}

/// The deep-link down-zoom leg (D5), evaluated under an intelligence-layer state.
///
/// When the layer is [`IntelligenceLayer::Healthy`] this runs the REAL canonical
/// [`hugit_ledger::deeplink::resolve`] over the records and surfaces the golden
/// target. When [`IntelligenceLayer::Degraded`], the smart index is down: the
/// leg returns the explicit [`DownZoom::LayerUnavailable`] — it does NOT return
/// an empty/`NotFound` that a human could mistake for "no such intent".
pub fn deep_link_down_zoom(
    layer: IntelligenceLayer,
    intent_id: &str,
    records: &[EventRecord],
) -> DownZoom {
    match layer {
        IntelligenceLayer::Degraded => DownZoom::LayerUnavailable {
            leg: "deep_link".to_string(),
        },
        IntelligenceLayer::Healthy => match deeplink_resolve(intent_id, records) {
            ResolveResult::Found { target, .. } if !target.trim().is_empty() => {
                DownZoom::ViaIntelligenceLayer {
                    summary: format!("deep-link → {target}"),
                }
            }
            // A genuine NotFound (or an empty target) when the layer is HEALTHY
            // is still honest — it is a definitive "no such intent", not a blank
            // screen. We surface it as an explicit unavailable-for-this-target
            // so the human is never handed silence.
            _ => DownZoom::LayerUnavailable {
                leg: "deep_link".to_string(),
            },
        },
    }
}

/// The `why` down-zoom leg (D10), evaluated under an intelligence-layer state.
///
/// When [`IntelligenceLayer::Healthy`] this runs the REAL canonical
/// [`hugit_cli::why::resolver::resolve_why`] and surfaces the provenance answer.
/// When [`IntelligenceLayer::Degraded`] the provenance resolver is down and the
/// leg returns the explicit [`DownZoom::LayerUnavailable`] — never a blank.
///
/// Note `resolve_why`'s OWN honest-failure modes (`WhyError`) are themselves
/// truthful endpoints (D10 already refuses to mis-attribute); when the layer is
/// healthy but the query is unattributed we surface that as an explicit
/// unavailable, never a fabricated answer.
pub fn why_down_zoom(
    layer: IntelligenceLayer,
    query: &WhyQuery,
    entries: &[WhyLogEntry],
) -> DownZoom {
    match layer {
        IntelligenceLayer::Degraded => DownZoom::LayerUnavailable {
            leg: "why".to_string(),
        },
        IntelligenceLayer::Healthy => match resolve_why(query, entries) {
            Ok(answer) => DownZoom::ViaIntelligenceLayer {
                summary: summarize_answer(&answer),
            },
            // D10's honest refusals (NotFound / LineUnresolved / …) are truthful
            // — surface them as an explicit honest endpoint, never a blank.
            Err(_e) => DownZoom::LayerUnavailable {
                leg: "why".to_string(),
            },
        },
    }
}

fn summarize_answer(a: &ProvenanceAnswer) -> String {
    // Never empty: always carries the event seq + kind so the human lands
    // somewhere truthful even if the charter is sparse.
    format!("why → seq {} ({})", a.event_seq, a.event_kind)
}

/// The full human down-zoom under a given intelligence-layer state: all three
/// legs (raw-commit view, deep link, `why`), each landing on an honest endpoint.
///
/// This is item ①'s oracle surface. The composition law it proves: under
/// [`IntelligenceLayer::Degraded`], the raw-commit leg STILL resolves via plain
/// git (lock 5), and the deep-link / `why` legs fail HONESTLY with an explicit
/// [`DownZoom::LayerUnavailable`] — every leg `is_honest()`, none is a silent
/// blank.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownZoomReport {
    /// The raw-commit view (D2, plain git).
    pub raw_commit: DownZoom,
    /// The deep-link leg (D5).
    pub deep_link: DownZoom,
    /// The `why` leg (D10).
    pub why: DownZoom,
}

impl DownZoomReport {
    /// True iff EVERY leg landed on an honest endpoint — no silent blank/404.
    pub fn all_honest(&self) -> bool {
        self.raw_commit.is_honest() && self.deep_link.is_honest() && self.why.is_honest()
    }

    /// True iff the plain-git leg resolved (lock 5: a valid git repo keeps
    /// serving even when the intelligence layer is degraded).
    pub fn git_still_serves(&self) -> bool {
        matches!(self.raw_commit, DownZoom::ViaPlainGit { .. })
    }
}

/// Run the full human down-zoom under `layer`, against the plain-git object
/// store and the (smart-layer) records/entries.
#[allow(clippy::too_many_arguments)]
pub fn human_down_zoom(
    layer: IntelligenceLayer,
    raw_object: &str,
    git_objects: &[&str],
    intent_id: &str,
    records: &[EventRecord],
    why_query: &WhyQuery,
    why_entries: &[WhyLogEntry],
) -> DownZoomReport {
    DownZoomReport {
        raw_commit: raw_commit_view(layer, raw_object, git_objects),
        deep_link: deep_link_down_zoom(layer, intent_id, records),
        why: why_down_zoom(layer, why_query, why_entries),
    }
}

// ── Item ② — following any chain to a target-or-tombstone after erasure ───────

/// A tamper-evident erasure marker — the honest endpoint a chain reaches when
/// its object was erased by the X7 cascade (consumed as-built; same shape as the
/// X12 tombstone, modelled here for the HUMAN-following reading).
///
/// A tombstone is NOT a broken link: it is a deliberate, self-describing record
/// of a lawful erasure carrying the original object's content hash, so a human
/// following the chain lands on a truthful endpoint ("this was here, it was
/// erased, here is the proof") rather than a dangling 404.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tombstone {
    /// The content hash of the object that was erased — the SAME hash the
    /// surviving chain link points at (tamper-evident).
    pub erased_object_hash: String,
    /// Why the object was erased (audit annotation).
    pub reason: String,
    /// Marker tag making a tombstone trivially distinguishable on the wire.
    pub marker: String,
}

/// The fixed tombstone marker tag.
pub const TOMBSTONE_MARKER: &str = "tombstone";

impl Tombstone {
    /// Mint a tamper-evident tombstone for `erased_object_hash`.
    pub fn new(erased_object_hash: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            erased_object_hash: erased_object_hash.into(),
            reason: reason.into(),
            marker: TOMBSTONE_MARKER.to_string(),
        }
    }

    /// Whether this value is a well-formed tombstone (carries the marker tag).
    pub fn is_tombstone(&self) -> bool {
        self.marker == TOMBSTONE_MARKER
    }
}

/// What following ONE link in a chain reaches. Every variant except
/// [`Follow::Broken`] is an HONEST endpoint a human can land on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "follow", rename_all = "snake_case")]
pub enum Follow {
    /// The link reaches a live target (the object is present).
    Target {
        /// The resolved target id (never empty).
        id: String,
    },
    /// The link reaches a tamper-evident tombstone (the object was erased).
    Tombstone(Tombstone),
    /// The link is DANGLING/broken — reaches nothing, no tombstone. This is the
    /// CONTRACT FAIL item ② forbids: the human cannot follow. Carries the id
    /// that failed to resolve so the failure itself is not silent.
    Broken {
        /// The id that resolved to nothing.
        id: String,
    },
}

impl Follow {
    /// True iff a human can follow this link to a truthful endpoint (a live
    /// target OR an honest tombstone). [`Follow::Broken`] is the only `false`.
    pub fn is_followable(&self) -> bool {
        match self {
            Follow::Target { id } => !id.trim().is_empty(),
            Follow::Tombstone(ts) => ts.is_tombstone() && !ts.erased_object_hash.trim().is_empty(),
            Follow::Broken { .. } => false,
        }
    }
}

/// A content-addressed object store with a lawful erasure (→ tombstone)
/// operation, modelling the X7 cascade's object leg as-built. Erasure NEVER
/// produces a void: an erased hash always resolves to a tombstone, so following
/// the chain never hits a broken link.
#[derive(Debug, Clone, Default)]
pub struct ChainStore {
    live: std::collections::BTreeMap<String, String>,
    tombstones: std::collections::BTreeMap<String, Tombstone>,
}

impl ChainStore {
    /// A fresh, empty store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Put a live object under its content hash.
    pub fn put(&mut self, hash: impl Into<String>, bytes: impl Into<String>) {
        self.live.insert(hash.into(), bytes.into());
    }

    /// Erase the object at `hash`, replacing it with a tamper-evident tombstone
    /// keyed by the SAME hash. Idempotent: an erased (or never-present) hash
    /// resolves to a tombstone, NEVER to a broken link.
    pub fn erase(&mut self, hash: &str, reason: impl Into<String>) -> Tombstone {
        self.live.remove(hash);
        let ts = Tombstone::new(hash.to_string(), reason);
        self.tombstones.insert(hash.to_string(), ts.clone());
        ts
    }

    /// Follow one link to its current endpoint.
    pub fn follow(&self, id: &str) -> Follow {
        if let Some(_bytes) = self.live.get(id) {
            Follow::Target { id: id.to_string() }
        } else if let Some(ts) = self.tombstones.get(id) {
            Follow::Tombstone(ts.clone())
        } else {
            Follow::Broken { id: id.to_string() }
        }
    }
}

/// The result of following an ENTIRE chain after an erasure cascade.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChainFollow {
    /// The endpoint reached at each hop, in order.
    pub hops: Vec<Follow>,
}

impl ChainFollow {
    /// True iff EVERY hop is followable — the human reaches a target-or-tombstone
    /// at each step, never a broken link. This is item ②'s predicate.
    pub fn human_can_always_follow(&self) -> bool {
        !self.hops.is_empty() && self.hops.iter().all(Follow::is_followable)
    }

    /// True iff at least one hop landed on an honest tombstone (the erasure
    /// cascade left a tamper-evident marker the human reached).
    pub fn reaches_tombstone(&self) -> bool {
        self.hops.iter().any(|f| matches!(f, Follow::Tombstone(_)))
    }
}

/// Follow a chain of link ids against the store after an erasure cascade.
///
/// This is item ②'s oracle surface for an arbitrary deep-link chain: each id is
/// resolved in order. A correct cascade leaves every erased id resolving to a
/// tombstone, so [`ChainFollow::human_can_always_follow`] holds; a cascade that
/// left a void produces a [`Follow::Broken`] hop and the predicate fails.
pub fn follow_chain(link_ids: &[&str], store: &ChainStore) -> ChainFollow {
    ChainFollow {
        hops: link_ids.iter().map(|id| store.follow(id)).collect(),
    }
}

/// Independently verify an attestation/provenance chain after an erasure, AND
/// confirm following its object link reaches a target-or-tombstone.
///
/// Ties item ② to the REAL canonical [`hugit_refstore::verify_chain`]: erasure
/// operates on the object store, never the append-only chain, so the hash-chain
/// still verifies byte-for-byte; the surviving link then resolves to a tombstone
/// (the human follows to a truthful endpoint).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttestationFollow {
    /// `Ok` iff the canonical hash-chain still verifies after erasure.
    pub chain: Result<(), TamperError>,
    /// What following the surviving object link reaches.
    pub endpoint: Follow,
}

impl AttestationFollow {
    /// True iff the chain still verifies AND following its link reaches a
    /// truthful endpoint (target-or-tombstone) — the human can always follow.
    pub fn human_can_always_follow(&self) -> bool {
        self.chain.is_ok() && self.endpoint.is_followable()
    }
}

/// Verify an attestation chain after erasure and follow its object link.
pub fn follow_attestation_after_erasure(
    records: &[EventRecord],
    link_id: &str,
    store: &ChainStore,
) -> AttestationFollow {
    AttestationFollow {
        chain: verify_chain(records),
        endpoint: store.follow(link_id),
    }
}
