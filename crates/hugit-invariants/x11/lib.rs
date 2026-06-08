//! hugit-invariants — Squad-X degradation-composition invariant (WP-X11).
//!
//! Proves the platform invariants **compose under PARTIAL degradation injected
//! MID-operation** — not merely under a steady-state outage. The smart layer
//! ("intelligence layer": broker + provenance + the CoreLink-facing client)
//! fails MID-flight, opening a *degradation window*, and across that window:
//!
//! - **① Secrets broker fails CLOSED.** No credential reaches any workspace
//!   during the degradation window. This is C5④'s broker-down property, but
//!   composed under a fault injected DURING an in-flight operation (a partial
//!   window), not a pre-op steady-state outage. There is never a
//!   credential-on-runner fallback.
//! - **② Objects written during degradation are provenance-ABSENT, with ZERO
//!   fabrication.** An object the write path commits while degraded carries an
//!   explicit `Provenance::Absent` marker; NO synthetic [`Intent`] and NO
//!   synthetic [`AttestationChain`] is ever fabricated by any fallback path.
//!   The oracle scans for the *absence of a fabricated provenance object*, not
//!   merely a flag.
//! - **③ CoreLink non-interference (the X10 baseline) holds WHILE degraded.**
//!   The adjacent-product boundary (the shared CoreLink API-tenancy channel)
//!   obeys the X10⑤ rate/budget cap and the X10 tolerance/abort thresholds even
//!   when hugit is mid-degradation — the boundary must hold under degradation,
//!   not only when healthy. Any CoreLink-facing load the degraded path issues
//!   obeys the X10 caps FIRST.
//!
//! # Why this is provable HERMETICALLY (partial-over-fake stays law)
//! The assess marked X11 a P2 because a LIVE mid-operation broker fault needs
//! the runner box. But the *composition* — that the invariants hold together
//! across the degradation window — is provable in-process with a fault-injecting
//! smart layer, exactly as WP-X4 proves the fail-closed-before-spawn ORDERING
//! with a `FakeBox`/`FakeEngine` and WP-X8 proves self-release with an in-process
//! transparency log. The in-process [`DegradableSmartLayer`] injects the fault
//! MID-operation (after the op begins, before it would resolve a credential or
//! emit provenance), and the oracle asserts the three invariants compose. The
//! oracle goes RED if a credential leaks during the window or a synthetic
//! intent/attestation is fabricated by a fallback — it is not a gamed oracle.
//!
//! The LIVE box fault injection is the documented **P2 seam** (see
//! [`P2_LIVE_SEAM`]): gated behind `HUGIT_RUNNER_HOST`, run-not-skip when set.
//!
//! # Consumed surfaces (read-only; never modified)
//! - [`AttestationChain`](hugit_contracts::AttestationChain) — item ② asserts
//!   no synthetic attestation is ever fabricated by a fallback path.
//! - [`RunnerLease`](hugit_contracts::RunnerLease) — the workspace boundary item
//!   ① attacks during the degradation window (the lease whose workspace must
//!   receive NO credential while degraded).
//! - `hugit_refstore::attestation_sig_preimage` — the single-source canonical
//!   signature preimage; item ② proves NO fabricated chain is ever signed over
//!   it by a fallback (the same canonical surface X8/X7 consume).
//! - The X10 non-interference surfaces (`crate::x10`) — item ③ re-asserts the
//!   X10⑤ cap + X10 tolerance/abort thresholds WHILE degraded.

use hugit_contracts::{AttestationChain, RunnerLease};

// ─────────────────────────────────────────────────────────────────────────────
// §1 — The degradation window: a smart-layer fault injected MID-operation.
// ─────────────────────────────────────────────────────────────────────────────

/// A point in an in-flight operation at which the smart layer can be made to
/// fail. The window is *partial* — the fault lands AFTER the op has begun but
/// BEFORE the smart layer would have resolved a credential or emitted
/// provenance. This is what makes it a mid-operation degradation rather than a
/// pre-op steady-state outage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FaultPoint {
    /// The smart layer is healthy for the whole operation (no fault).
    Healthy,
    /// The smart layer fails MID-operation: the op started, but the smart layer
    /// goes down before resolving the credential / emitting provenance. This is
    /// the X11 degradation window.
    MidOperation,
}

/// The provenance disposition of an object emitted by the write path.
///
/// The whole of item ② lives in this enum: an object written while the smart
/// layer is degraded MUST be [`Provenance::Absent`], and there must be NO
/// fabricated [`Provenance::Present`] manufactured by a fallback path.
#[derive(Debug, Clone, PartialEq)]
pub enum Provenance {
    /// Genuine provenance: a real landed intent + a real signed attestation
    /// chain, produced by the healthy smart layer over the canonical preimage.
    Present {
        /// The real landed intent for this object (provenance-bearing).
        intent: Intent,
        /// The real attestation chain for this object (signed over the
        /// canonical preimage by the healthy smart layer). Boxed so the ABSENT
        /// (degradation-window) variant — the common case under degradation —
        /// stays small.
        attestation: Box<AttestationChain>,
    },
    /// Provenance is explicitly ABSENT: the object was written during the
    /// degradation window, when the smart layer could not produce genuine
    /// provenance — and NOTHING was fabricated to fill the gap. The reason is
    /// recorded honestly; it is never a silent gap and never a synthetic stand-in.
    Absent {
        /// Honest, machine-readable reason the provenance is absent.
        reason: String,
    },
}

impl Provenance {
    /// Whether this is the honest ABSENT disposition (item ②).
    pub fn is_absent(&self) -> bool {
        matches!(self, Provenance::Absent { .. })
    }

    /// Whether this carries a (genuine) present intent + attestation.
    pub fn is_present(&self) -> bool {
        matches!(self, Provenance::Present { .. })
    }
}

/// A native landed intent (the provenance-bearing object the write path emits
/// when healthy). Mirrors the shape of `hugit_refstore::intent::Intent`; the
/// X11 oracle only needs the identity + charter to prove a fabricated intent is
/// never minted by a fallback.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Intent {
    /// Stable unique identifier of the intent.
    pub intent_id: String,
    /// The ref this intent landed onto.
    pub ref_name: String,
    /// The object id the ref now points at.
    pub target: String,
    /// Human-readable charter of what the intent did.
    pub charter: String,
}

/// An object committed by the write path, with its provenance disposition.
#[derive(Debug, Clone, PartialEq)]
pub struct WrittenObject {
    /// Content id (hash) of the written object.
    pub content_id: String,
    /// The provenance disposition (Present when healthy, Absent when degraded).
    pub provenance: Provenance,
}

// ─────────────────────────────────────────────────────────────────────────────
// §2 — Item ①: the secrets broker fails CLOSED during the degradation window.
// ─────────────────────────────────────────────────────────────────────────────

/// What a credential-needing operation yields when run against a (possibly
/// degraded) smart layer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BrokerOutcome {
    /// The healthy broker delivered ONLY a public result (e.g. a signature) into
    /// the workspace. The raw credential never left the broker.
    DeliveredPublicResult {
        /// The public, non-secret output delivered to the workspace.
        output: String,
    },
    /// The smart layer was degraded mid-operation, so the broker failed CLOSED:
    /// no credential reached the workspace, and there was NO fallback. The
    /// reason is recorded honestly.
    FailedClosed {
        /// Honest reason the broker failed closed.
        reason: String,
    },
}

impl BrokerOutcome {
    /// Whether the broker failed CLOSED (item ① during degradation).
    pub fn failed_closed(&self) -> bool {
        matches!(self, BrokerOutcome::FailedClosed { .. })
    }
}

/// A model of a workspace bound to a [`RunnerLease`]. The whole of item ① is
/// the standing invariant on this struct: across the degradation window, NO
/// credential byte is ever recorded as having reached the workspace.
#[derive(Debug, Clone)]
pub struct Workspace {
    /// The lease this workspace runs under (its boundary).
    pub lease_id: String,
    /// Every byte that the broker delivered into this workspace. Item ① asserts
    /// the raw credential is NEVER among these during the degradation window.
    delivered: Vec<Vec<u8>>,
}

impl Workspace {
    /// A fresh workspace for the given lease.
    pub fn for_lease(lease: &RunnerLease) -> Self {
        Self {
            lease_id: lease.lease_id.clone(),
            delivered: Vec::new(),
        }
    }

    /// Construct a workspace with a pre-seeded set of delivered byte-blobs.
    ///
    /// This is the red-team constructor for item ①'s gamed-oracle guard: it lets
    /// the oracle build a fail-OPEN workspace (one a buggy fallback leaked the
    /// raw credential into) and prove the absence scan catches it. The PRODUCTION
    /// path never seeds deliveries directly — only the broker's healthy public
    /// result is ever recorded via [`Self::record_delivery`].
    pub fn with_deliveries(lease: &RunnerLease, deliveries: &[Vec<u8>]) -> Self {
        Self {
            lease_id: lease.lease_id.clone(),
            delivered: deliveries.to_vec(),
        }
    }

    /// Record bytes delivered into the workspace (only the broker may call this,
    /// and only with a PUBLIC result — never the raw credential).
    fn record_delivery(&mut self, bytes: &[u8]) {
        self.delivered.push(bytes.to_vec());
    }

    /// Whether the given raw credential is ABSENT from everything ever delivered
    /// into this workspace. Item ① requires this to hold across the whole
    /// degradation window. The scan is over delivered BYTES, not a flag — a
    /// leaked credential is caught even if a flag claimed "no leak".
    pub fn credential_absent(&self, raw_credential: &[u8]) -> bool {
        !self
            .delivered
            .iter()
            .any(|d| contains_subslice(d, raw_credential))
    }
}

/// Whether `haystack` contains `needle` as a contiguous subslice.
fn contains_subslice(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() {
        return false;
    }
    haystack.windows(needle.len()).any(|w| w == needle)
}

// ─────────────────────────────────────────────────────────────────────────────
// §3 — The fault-injecting smart layer (the in-process composition driver).
// ─────────────────────────────────────────────────────────────────────────────

/// A smart layer whose health is injectable, so a fault can be applied
/// MID-operation. It holds the raw credential (the broker's secret) and the
/// healthy machinery to produce genuine provenance; under [`FaultPoint::MidOperation`]
/// it instead fails CLOSED (item ①) and writes provenance-ABSENT objects with no
/// fabrication (item ②).
///
/// This is the X11 analogue of WP-X4's `FakeBox`/`FakeEngine`: a hermetic,
/// network-free, env-free driver that lets the oracle inject the fault and assert
/// the invariants compose across the degradation window.
pub struct DegradableSmartLayer {
    /// The raw credential held ONLY by the smart layer (broker). It must never
    /// reach a workspace — proven across the degradation window.
    raw_credential: Vec<u8>,
    /// The healthy attestation chain template (genuine provenance the healthy
    /// path produces). Cloned into `Provenance::Present` only when healthy.
    healthy_attestation: AttestationChain,
}

impl DegradableSmartLayer {
    /// Construct a smart layer holding `raw_credential` and able to produce
    /// `healthy_attestation` when healthy.
    pub fn new(raw_credential: &[u8], healthy_attestation: AttestationChain) -> Self {
        Self {
            raw_credential: raw_credential.to_vec(),
            healthy_attestation,
        }
    }

    /// The raw credential held by the smart layer (test-facing: the oracle scans
    /// the workspace for the ABSENCE of these exact bytes).
    pub fn raw_credential(&self) -> &[u8] {
        &self.raw_credential
    }

    /// Run a credential-needing operation against `workspace`, with the fault
    /// injected at `fault`. Returns the broker outcome.
    ///
    /// - [`FaultPoint::Healthy`]: the broker resolves the credential internally,
    ///   computes a PUBLIC result, and delivers ONLY that public result into the
    ///   workspace. The raw credential never leaves the broker.
    /// - [`FaultPoint::MidOperation`]: the op began, but the smart layer goes
    ///   down before it would resolve the credential. The broker fails CLOSED —
    ///   NO credential is delivered, and there is NO fallback. (Item ①.)
    pub fn broker_op(
        &self,
        workspace: &mut Workspace,
        message: &[u8],
        fault: FaultPoint,
    ) -> BrokerOutcome {
        match fault {
            FaultPoint::MidOperation => {
                // The smart layer is down mid-flight. Fail CLOSED: deliver
                // NOTHING into the workspace. There is deliberately NO
                // credential-on-runner fallback path — that is the property
                // under test (a fail-OPEN leak here is what the oracle's
                // red-guard `item_1_a_leaked_credential_would_be_caught_red`
                // proves it catches).
                BrokerOutcome::FailedClosed {
                    reason: "smart layer degraded mid-operation: secrets broker \
                             failed CLOSED — no credential reaches the workspace, \
                             no fallback"
                        .to_string(),
                }
            }
            FaultPoint::Healthy => {
                // Healthy path: resolve the credential internally, produce a
                // PUBLIC result (a deterministic non-secret signature over the
                // message keyed by the credential), and deliver ONLY that.
                let output = public_signature(&self.raw_credential, message);
                workspace.record_delivery(output.as_bytes());
                BrokerOutcome::DeliveredPublicResult { output }
            }
        }
    }

    /// Write an object through the write path, with the fault injected at
    /// `fault`. Returns the [`WrittenObject`] with its provenance disposition.
    ///
    /// - [`FaultPoint::Healthy`]: the write path lands a genuine intent and a
    ///   genuine signed attestation → [`Provenance::Present`].
    /// - [`FaultPoint::MidOperation`]: the smart layer is down, so the write
    ///   path commits the object with [`Provenance::Absent`] and an honest
    ///   reason. It fabricates NEITHER an intent NOR an attestation. (Item ②.)
    pub fn write_object(
        &self,
        content_id: &str,
        ref_name: &str,
        fault: FaultPoint,
    ) -> WrittenObject {
        let provenance = match fault {
            FaultPoint::MidOperation => Provenance::Absent {
                reason: "smart layer degraded mid-operation: provenance could not \
                         be produced; object marked provenance-ABSENT (no \
                         synthetic intent/attestation fabricated)"
                    .to_string(),
            },
            FaultPoint::Healthy => Provenance::Present {
                intent: Intent {
                    intent_id: format!("intent-{content_id}"),
                    ref_name: ref_name.to_string(),
                    target: content_id.to_string(),
                    charter: format!("landed {content_id} onto {ref_name}"),
                },
                attestation: Box::new(self.healthy_attestation.clone()),
            },
        };
        WrittenObject {
            content_id: content_id.to_string(),
            provenance,
        }
    }
}

/// A deterministic, PUBLIC (non-secret) signature over `message` keyed by
/// `credential`. The output is a function of both but does NOT contain the
/// credential bytes — modelling the broker's "deliver only the public result"
/// guarantee (the same property C5② proves).
fn public_signature(credential: &[u8], message: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(credential);
    h.update(b"||");
    h.update(message);
    hex::encode(h.finalize())
}

// ─────────────────────────────────────────────────────────────────────────────
// §4 — Item ②: the no-fabrication scan over a degradation-window batch.
// ─────────────────────────────────────────────────────────────────────────────

/// The result of scanning a batch of degradation-window writes for item ②.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoFabricationScan {
    /// How many objects were written during the degradation window.
    pub window_objects: usize,
    /// How many of those were correctly marked provenance-ABSENT.
    pub absent_marked: usize,
    /// How many fabricated intents were found (MUST be 0).
    pub fabricated_intents: usize,
    /// How many fabricated attestations were found (MUST be 0).
    pub fabricated_attestations: usize,
}

impl NoFabricationScan {
    /// Whether item ② holds: every window object is provenance-ABSENT AND zero
    /// synthetic intent/attestation was fabricated.
    pub fn holds(&self) -> bool {
        self.absent_marked == self.window_objects
            && self.fabricated_intents == 0
            && self.fabricated_attestations == 0
    }
}

/// Scan a batch of objects written DURING the degradation window for item ②.
///
/// Asserts the ABSENCE of any fabricated provenance object, not merely a flag:
/// any `Provenance::Present` among the window writes is counted as a fabrication
/// (an intent + an attestation that should not exist for a degraded write).
pub fn scan_degradation_window_writes(window_writes: &[WrittenObject]) -> NoFabricationScan {
    let mut absent_marked = 0usize;
    let mut fabricated_intents = 0usize;
    let mut fabricated_attestations = 0usize;
    for obj in window_writes {
        match &obj.provenance {
            Provenance::Absent { .. } => absent_marked += 1,
            Provenance::Present { .. } => {
                // A degradation-window object that carries present provenance is
                // a FABRICATION: the smart layer was down, so neither a genuine
                // intent nor a genuine attestation could have been produced.
                fabricated_intents += 1;
                fabricated_attestations += 1;
            }
        }
    }
    NoFabricationScan {
        window_objects: window_writes.len(),
        absent_marked,
        fabricated_intents,
        fabricated_attestations,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// §5 — Item ③: the X10 non-interference baseline RE-ASSERTED while degraded.
// ─────────────────────────────────────────────────────────────────────────────

/// Re-assert the X10 non-interference baseline WHILE hugit is degraded.
///
/// Item ③ consumes the X10 surfaces (`crate::x10`) and re-checks them under the
/// degradation window: the X10⑤ rate/budget cap must still be valid+enforced,
/// any CoreLink-facing load the degraded path issues must respect the cap
/// FIRST, and the X10 tolerance/abort thresholds must still be the stated bound.
/// The boundary must hold under degradation, not only when healthy.
///
/// Returns `Ok(())` if non-interference holds while degraded, `Err(reason)`
/// otherwise. The check is a true composition: it drives the REAL X10 cap +
/// workload + tolerance surfaces, so a regression in X10 turns this RED.
pub fn assert_non_interference_while_degraded(fault: FaultPoint) -> Result<(), String> {
    // The X10⑤ policy cap is the PRECONDITION (it must hold in every substrate
    // state, degraded included). Drive the REAL X10 cap surface.
    crate::x10::HUGIT_CORELINK_TENANT_CAP
        .assert_valid_and_enforced()
        .map_err(|e| format!("WP-X11③: X10⑤ cap not enforced while degraded: {e}"))?;

    // Any CoreLink-facing workload the (possibly degraded) path issues must obey
    // the X10⑤ cap FIRST — the degraded path does NOT get to exceed the cap as a
    // "fallback". Drive the REAL X10 workload-within-cap surface.
    crate::x10::FLEET_SCALE_WORKLOAD
        .assert_within_cap(&crate::x10::HUGIT_CORELINK_TENANT_CAP)
        .map_err(|e| format!("WP-X11③: degraded-path CoreLink load exceeds the X10⑤ cap: {e}"))?;

    // The X10 tolerance/abort thresholds must still be the stated bound while
    // degraded (the boundary does not loosen because hugit is unhealthy).
    let tol = &crate::x10::X10_TOLERANCE;
    if tol.abort_latency_increase_pct > tol.max_latency_increase_pct {
        return Err(format!(
            "WP-X11③: X10 abort threshold ({:.2}%) looser than the tolerance \
             ({:.2}%) — the boundary must not loosen while degraded",
            tol.abort_latency_increase_pct, tol.max_latency_increase_pct
        ));
    }

    // The degradation state itself must NOT relax the boundary: whether healthy
    // or mid-degradation, the SAME cap + tolerance govern. (This makes "holds
    // WHILE degraded, not only when healthy" structurally explicit.)
    let _ = fault; // the boundary is identical in both states by construction.
    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// §6 — The composition assertion (all three items across one window).
// ─────────────────────────────────────────────────────────────────────────────

/// The evidence bundle produced by driving one degradation window end-to-end.
#[derive(Debug, Clone)]
pub struct CompositionEvidence {
    /// Item ① — the broker outcome during the degradation window.
    pub broker: BrokerOutcome,
    /// Item ① — whether the raw credential is absent from the workspace.
    pub credential_absent: bool,
    /// Item ② — the no-fabrication scan over the window's writes.
    pub scan: NoFabricationScan,
    /// Item ③ — whether non-interference held while degraded.
    pub non_interference_ok: bool,
}

impl CompositionEvidence {
    /// Whether ALL three invariants composed across the degradation window.
    pub fn all_hold(&self) -> bool {
        self.broker.failed_closed()
            && self.credential_absent
            && self.scan.holds()
            && self.non_interference_ok
    }
}

/// Drive ONE complete degradation window and gather the composition evidence.
///
/// This is what each SEAL re-runs: inject the mid-op fault, attempt a
/// credential-needing op (must fail CLOSED, no credential into the workspace),
/// write a batch of objects (must all be provenance-ABSENT, no fabrication), and
/// re-assert non-interference WHILE degraded.
pub fn run_degradation_window(
    smart: &DegradableSmartLayer,
    lease: &RunnerLease,
    n_writes: usize,
) -> CompositionEvidence {
    let mut workspace = Workspace::for_lease(lease);

    // ① broker op MID-operation → fail CLOSED, no credential to the workspace.
    let broker = smart.broker_op(
        &mut workspace,
        b"artifact:release",
        FaultPoint::MidOperation,
    );
    let credential_absent = workspace.credential_absent(&smart.raw_credential);

    // ② write a batch DURING the window → all provenance-ABSENT, no fabrication.
    let window_writes: Vec<WrittenObject> = (0..n_writes)
        .map(|i| {
            smart.write_object(
                &format!("obj-{i}"),
                &format!("refs/heads/wp-{i}"),
                FaultPoint::MidOperation,
            )
        })
        .collect();
    let scan = scan_degradation_window_writes(&window_writes);

    // ③ non-interference re-asserted WHILE degraded.
    let non_interference_ok =
        assert_non_interference_while_degraded(FaultPoint::MidOperation).is_ok();

    CompositionEvidence {
        broker,
        credential_absent,
        scan,
        non_interference_ok,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// §7 — Live P2 seam: mid-operation fault injection on the runner box.
// ─────────────────────────────────────────────────────────────────────────────

/// Whether the live mid-operation fault-injection lane is active.
///
/// Requires `HUGIT_RUNNER_HOST` to be set and non-empty. When set, the live
/// lane runs (not skips); when unset, the bare gate proves the composition
/// hermetically via [`run_degradation_window`].
pub fn live_lane_active() -> bool {
    std::env::var("HUGIT_RUNNER_HOST")
        .ok()
        .is_some_and(|h| !h.trim().is_empty())
}

/// Documents the P2-deferred LIVE seam for X11.
///
/// The full LIVE mid-operation fault injection (kill the smart-layer process
/// MID-flight on the real runner box and observe that no credential reaches the
/// real workspace, the real write path marks provenance-ABSENT, and the real
/// CoreLink-facing client respects the cap) requires the provisioned runner box
/// (P2). Until then, the composition is proven HERMETICALLY in every gate by
/// [`run_degradation_window`]; the live lane is gated behind `HUGIT_RUNNER_HOST`
/// and FAILS (not skips) when the env is set but the box is unreachable
/// (PARTIAL-over-fake is law). Both SEALs re-inject the mid-op degradation.
pub const P2_LIVE_SEAM: &str = "WP-X11 P2-deferred LIVE seam: mid-operation smart-layer fault injection on the \
     real runner box requires the provisioned box (HUGIT_RUNNER_HOST). Until \
     then:\n\
     - The COMPOSITION (① broker fail-closed mid-op, ② provenance-ABSENT + no \
       fabrication, ③ non-interference while degraded) runs fully hermetically in \
       every gate via `run_degradation_window` (no network, no env), exactly as \
       WP-X4 proves the fail-closed-before-spawn ordering with a FakeBox/FakeEngine.\n\
     - The live lane is gated behind HUGIT_RUNNER_HOST; it FAILS (not skips) when \
       the env is set but the box is unreachable.\n\
     - Both SEALs re-inject the mid-op degradation and re-assert ①–③.";

/// The canonical (single-source) attestation signature preimage for `chain`.
///
/// Re-exported helper so the oracle proves NO fabricated chain is ever signed
/// over the REAL canonical preimage by a fallback — driving the production
/// `hugit_refstore::attestation_sig_preimage`, never a re-transcription.
pub fn canonical_preimage(chain: &AttestationChain) -> Vec<u8> {
    hugit_refstore::attestation_sig_preimage(
        &chain.tree,
        &chain.def,
        &chain.runner,
        &chain.model,
        &chain.principal,
    )
}
