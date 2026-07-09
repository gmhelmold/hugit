//! Merge-as-re-execution **dispatch orchestration** — the whitepaper's marquee
//! (§6.3 regenerative rebase: a landed intent RE-EXECUTES against real compute).
//!
//! When a landing rebase finds an intent's claims collide with the base delta,
//! the whitepaper prescribes RE-EXECUTION over textual patching:
//!
//! ```text
//! re-execute I: agent(ctx_start(I), charter(I)) on workspace(B1) → diff D′
//! ```
//!
//! The demand to re-execute is RECORDED by the landing/regen layer, but until
//! now it was never DISPATCHED — the intentional P2 deferral CLAUDE.md names
//! ("merge-as-re-execution records the demand but never dispatches an agent").
//! This module is the hugit-side orchestration that PREPARES + drives a dispatch
//! for a recorded demand, behind the ONE external seam (the fabricd agent spawn):
//!
//!   1. take a [`ReExecDemand`] (the intent + its charter / ctx_start / target
//!      tree + the frozen lease params),
//!   2. construct the frozen [`AcquireLeaseRequest`]
//!      ([`ReExecDemand::acquire_request`]),
//!   3. drive the full off-box §13 lease lifecycle (acquire → §13.2 ingest →
//!      close, ALWAYS releasing the lease) via the EXISTING
//!      [`dispatch_attest_offbox`] over the pluggable [`RunnerTransport`] seam —
//!      a fake transport proves it hermetically; the real transport rides the
//!      fabricd seam,
//!   4. collect the [`DispatchOutcome`] and fold in the fail-closed
//!      [`cost_attestation_verdict`] (the CONSUME side, #306),
//!   5. map every failure mode to an HONEST [`ReExecOutcome`] — a dispatch that
//!      cannot reach the fabric is [`ReExecOutcome::Unavailable`], NEVER a
//!      fabricated success.
//!
//! ## The honesty law (load-bearing)
//!
//! - a demand that cannot be dispatched (the fabric unreachable, the box not
//!   wired, §13 ingest absent, the lease refused) yields
//!   [`ReExecOutcome::Unavailable`] carrying the reason — NEVER a fabricated
//!   result or cost;
//! - the lease is ALWAYS released ([`dispatch_attest_offbox`] closes it even on
//!   error), so a failed dispatch never leaks runner capacity;
//! - the captured metrics + cost are the fabric's FINALIZED signed close figure
//!   (or the honest-zero it reports); the cost's integrity is the fail-closed
//!   [`cost_attestation_verdict`], never a hugit hand-stamp;
//! - a re-execution that RAN but did not pass its acceptance gate (`exit_code !=
//!   0`) is [`ReExecOutcome::ReExecuted`] with [`ReExecOutcome::succeeded`] =
//!   `false` — the demand WAS dispatched, the result is honest, and the caller
//!   MUST NOT land it (never mis-attested as a green re-execution).
//!
//! ## The one remaining EXTERNAL seam
//!
//! Everything UP TO the actual off-box agent SPAWN is built + tested here. The
//! spawn itself — bringing a real agent loop up on the CoreLink runner fabric
//! (`corelink-runners`, the fabricd spawn path) so it can submit its §13
//! trajectory — stays external. Until it lands, a live dispatch surfaces as
//! [`UnavailableReason::FabricUnreachable`] / [`UnavailableReason::BoxNotWired`];
//! hermetically, the fake transport stands in for the fabric's wire responses.
//! This module fakes NOTHING green: the seam is a first-class honest outcome.

use hugit_contracts::{CheckResult, IntentMetrics};
use sha2::{Digest, Sha256};

use crate::cost_attest::{CostAttestation, FabricAttestConfig, cost_attestation_verdict};
use crate::runner::dispatch::{DispatchOutcome, dispatch_attest_offbox};
use crate::runner::lease_client::{AcquireLeaseRequest, LeaseClient, RunnerTransport};
use crate::runner::lease_exec::RunnerExecError;

/// A RECORDED merge-as-re-execution demand: everything the orchestration needs to
/// re-execute one intent `I` on the rebased workspace (whitepaper §6.3
/// `agent(ctx_start(I), charter(I)) on workspace(B1)`).
///
/// This is the "demand" the landing/regen layer records; [`dispatch_reexec`]
/// turns it into a driven lease lifecycle. The four lease fields are the frozen
/// [`AcquireLeaseRequest`] shape (`image_digest` / `net_policy` / `tmp_root` /
/// `expiry_ms`); the fabric resolves `principal_chain` / `path_set` server-side
/// from the authenticated tenant + claim, so they are NOT carried on the wire.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReExecDemand {
    /// The intent `I` being re-executed (its stable id, the address the outcome
    /// is attributed to).
    pub intent_id: String,
    /// The intent's charter — the instruction the re-executing agent re-runs
    /// (`charter(I)`). Part of the deterministic dispatch identity.
    pub charter: String,
    /// A content-addressed ref of the starting context `ctx_start(I)` the agent
    /// re-executes from. Part of the deterministic dispatch identity.
    pub ctx_start_ref: String,
    /// The content-addressed ref of the rebased workspace tree `B1` the
    /// re-execution runs against (the memo axis `tree_root`).
    pub target_tree: String,
    /// The fabric image digest recorded for the lease (`sha256:`-format; the
    /// off-box A-mode records it but spawns no box — the fabric FORMAT-checks
    /// only). Carried verbatim onto the [`AcquireLeaseRequest`].
    pub image_digest: String,
    /// The egress policy the lease requests (`deny-all` for a hermetic
    /// re-execution). Carried verbatim onto the [`AcquireLeaseRequest`].
    pub net_policy: String,
    /// The `tmp_root` the lease requests (recorded metadata for the off-box
    /// A-mode). Carried verbatim onto the [`AcquireLeaseRequest`].
    pub tmp_root: String,
    /// The lease TTL (ms) the acquire requests (`expiry_ms`).
    pub expiry_ms: u64,
}

impl ReExecDemand {
    /// Build the frozen [`AcquireLeaseRequest`] for this demand — exactly the four
    /// `deny_unknown_fields` fields the fabric accepts (the principal chain /
    /// path set are resolved server-side, never on the wire).
    #[must_use]
    pub fn acquire_request(&self) -> AcquireLeaseRequest {
        AcquireLeaseRequest {
            image_digest: self.image_digest.clone(),
            net_policy: self.net_policy.clone(),
            tmp_root: self.tmp_root.clone(),
            expiry_ms: self.expiry_ms,
        }
    }

    /// The deterministic re-execution `def_digest` — lowercase-hex SHA-256 over a
    /// domain-tagged `(charter, ctx_start_ref)`. It gives the dispatch a stable
    /// identity that re-keys identically when the SAME intent is re-dispatched
    /// from the SAME charter + start context (whitepaper §6.2 def axis).
    #[must_use]
    pub fn def_digest(&self) -> String {
        let mut h = Sha256::new();
        h.update(b"hugit:reexec-def:v1\n");
        h.update(self.charter.as_bytes());
        h.update(b"\n");
        h.update(self.ctx_start_ref.as_bytes());
        hex::encode(h.finalize())
    }

    /// The deterministic re-execution memo key — lowercase-hex SHA-256 over a
    /// domain-tagged `(intent_id, target_tree, def_digest)`. Re-dispatching the
    /// SAME intent onto the SAME rebased tree with the SAME def keys identically
    /// (the AC memoization axis, whitepaper §6.2).
    #[must_use]
    pub fn memo_key(&self, def_digest: &str) -> String {
        let mut h = Sha256::new();
        h.update(b"hugit:reexec-memo:v1\n");
        h.update(self.intent_id.as_bytes());
        h.update(b"\n");
        h.update(self.target_tree.as_bytes());
        h.update(b"\n");
        h.update(def_digest.as_bytes());
        hex::encode(h.finalize())
    }
}

/// Why a recorded re-execution demand could NOT be dispatched. Each variant is an
/// HONEST partial — a dispatch never fabricates a success (the honesty law). The
/// distinct variants let the caller react precisely (a not-yet-wired fabricd box
/// is [`BoxNotWired`](Self::BoxNotWired), a transient/unreachable fabric is
/// [`FabricUnreachable`](Self::FabricUnreachable)).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnavailableReason {
    /// The lease lifecycle failed at the wire (acquire / ingest / poll / close
    /// returned a transport fault or a terminal HTTP status) — the fabric was
    /// unreachable or refused the lease. The most common live pre-fabricd case.
    FabricUnreachable,
    /// The runner box seam is not wired (the genuinely-unconfigured
    /// [`RunnerExecError::BoxNotWired`]) — the P2 fabricd spawn is the external
    /// gate. Distinct from a reachable-but-refusing fabric.
    BoxNotWired,
    /// The acquire response carried no §13.2 ingest credential (a runner lease,
    /// or §13 off) — the off-box attest path REFUSES to fall back or fabricate a
    /// cost (fail-closed, [`RunnerExecError::NoEnvelopeIngest`]).
    IngestNotWired,
    /// The lease was not `Held` when execution was attempted
    /// (expired / released / crashed — [`RunnerExecError::LeaseNotHeld`]).
    LeaseNotHeld,
    /// The runner refused to run the check (`accepted=false`, or a run-level
    /// failure distinct from a wire fault — [`RunnerExecError::Run`]).
    ExecRefused,
}

impl UnavailableReason {
    /// A stable machine code for this reason (for logging / audit).
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            UnavailableReason::FabricUnreachable => "fabric_unreachable",
            UnavailableReason::BoxNotWired => "box_not_wired",
            UnavailableReason::IngestNotWired => "ingest_not_wired",
            UnavailableReason::LeaseNotHeld => "lease_not_held",
            UnavailableReason::ExecRefused => "exec_refused",
        }
    }
}

impl core::fmt::Display for UnavailableReason {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.code())
    }
}

impl From<&RunnerExecError> for UnavailableReason {
    fn from(e: &RunnerExecError) -> Self {
        match e {
            RunnerExecError::Lease(_) => UnavailableReason::FabricUnreachable,
            RunnerExecError::BoxNotWired(_) => UnavailableReason::BoxNotWired,
            RunnerExecError::NoEnvelopeIngest(_) => UnavailableReason::IngestNotWired,
            RunnerExecError::LeaseNotHeld(_) => UnavailableReason::LeaseNotHeld,
            RunnerExecError::Run(_) => UnavailableReason::ExecRefused,
        }
    }
}

/// The HONEST outcome of driving a re-execution demand. A total sum — a dispatch
/// that cannot reach the fabric is [`Unavailable`](Self::Unavailable), so the
/// type itself forbids a fabricated success.
#[derive(Debug, Clone, PartialEq)]
pub enum ReExecOutcome {
    /// The demand WAS dispatched: the agent re-executed off-box, the fabric
    /// finalized + signed the §13.1 metrics, and the lifecycle completed.
    ReExecuted {
        /// The intent this outcome is attributed to (`demand.intent_id`).
        intent_id: String,
        /// The lease the re-execution ran under (the verify binding).
        lease_id: String,
        /// The synthesized off-box [`CheckResult`] (stamped memo key + axes; the
        /// `exit` carries the re-execution's own verdict). Boxed to keep the enum
        /// variant compact (the `Unavailable` arm is small).
        result: Box<CheckResult>,
        /// The fabric's FINALIZED §13.1 metrics (the signed source of truth — the
        /// FULL token cache-split, never flattened). Boxed with `result` to keep
        /// the enum's variants size-balanced.
        metrics: Box<IntentMetrics>,
        /// The fail-closed cost-attestation verdict over `metrics` — `Attested`
        /// only on a present sig that verifies, else `Unattested(reason)`.
        cost: CostAttestation,
        /// `true` iff the re-execution passed its acceptance gate
        /// (`result.exit == 0`). `false` ⇒ the demand ran but FAILED; the caller
        /// MUST NOT land it (never mis-attested as green).
        succeeded: bool,
    },
    /// The demand could NOT be dispatched — an honest partial, never a fabricated
    /// success. Carries the typed reason + a secret-free detail (the PAT never
    /// appears in a [`RunnerExecError`] rendering).
    Unavailable {
        /// The intent this (non-)outcome is attributed to.
        intent_id: String,
        /// Why the dispatch could not complete.
        reason: UnavailableReason,
        /// The secret-free error detail (for logging / audit).
        detail: String,
    },
}

impl ReExecOutcome {
    /// The intent this outcome is attributed to (present for both arms).
    #[must_use]
    pub fn intent_id(&self) -> &str {
        match self {
            ReExecOutcome::ReExecuted { intent_id, .. }
            | ReExecOutcome::Unavailable { intent_id, .. } => intent_id,
        }
    }

    /// `true` iff the demand was dispatched + the lifecycle completed (regardless
    /// of the re-execution's acceptance verdict — see [`Self::succeeded`]).
    #[must_use]
    pub fn is_reexecuted(&self) -> bool {
        matches!(self, ReExecOutcome::ReExecuted { .. })
    }

    /// `true` iff the demand could not be dispatched (an honest partial).
    #[must_use]
    pub fn is_unavailable(&self) -> bool {
        matches!(self, ReExecOutcome::Unavailable { .. })
    }

    /// `true` ONLY when the demand ran AND passed its acceptance gate
    /// (`ReExecuted` with `succeeded`). `false` for a failed re-execution and for
    /// [`Unavailable`](Self::Unavailable) — the caller gates a land on this.
    #[must_use]
    pub fn succeeded(&self) -> bool {
        matches!(
            self,
            ReExecOutcome::ReExecuted {
                succeeded: true,
                ..
            }
        )
    }

    /// The cost-attestation verdict, if the demand was dispatched. `None` for
    /// [`Unavailable`](Self::Unavailable) (nothing ran → nothing to attest).
    #[must_use]
    pub fn cost(&self) -> Option<CostAttestation> {
        match self {
            ReExecOutcome::ReExecuted { cost, .. } => Some(*cost),
            ReExecOutcome::Unavailable { .. } => None,
        }
    }

    /// `true` ONLY when the demand ran AND its finalized cost is cryptographically
    /// ATTESTED. Convenience for a caller gating a `✓ cas:` marker.
    #[must_use]
    pub fn cost_is_attested(&self) -> bool {
        matches!(
            self,
            ReExecOutcome::ReExecuted {
                cost: CostAttestation::Attested,
                ..
            }
        )
    }
}

/// Drive a recorded merge-as-re-execution [`ReExecDemand`] through the full
/// off-box §13 lease lifecycle and fold in the cost-attestation verdict, returning
/// an HONEST [`ReExecOutcome`].
///
/// The orchestration:
///   1. builds the frozen [`AcquireLeaseRequest`] from the demand,
///   2. drives acquire → §13.2 ingest submit → close via [`dispatch_attest_offbox`]
///      (which ALWAYS closes the lease, even on error — no leaked capacity),
///      stamping the demand's deterministic memo key + axes,
///   3. on success, computes [`cost_attestation_verdict`] over the fabric's
///      finalized close metrics,
///   4. maps every lifecycle failure to [`ReExecOutcome::Unavailable`] with the
///      typed reason — NEVER a fabricated success.
///
/// # The caller-supplied inputs (honesty)
///
/// `measured` is the off-box agent loop's §13.1 trajectory (the token cache-split
/// submitted to the §13.2 ingest). `cost_usd_micros` is the run's REAL
/// provider-billed / priced-from-usage cost submitted VERBATIM on the close
/// (`None` for the honest-zero floor). Both come from the caller — this module
/// NEVER synthesizes a figure; until a live off-box loop lands (the P2 seam) the
/// caller passes honest-zero / `None`. `exit_code` is the re-execution's own
/// acceptance verdict (`0` = the re-executed `B1+D′` passed): it rides
/// `result.exit` and DECIDES the close status, so a failed re-execution closes
/// `failed` and surfaces as `succeeded=false` — never a green mis-attestation.
///
/// `fabric_cfg` is the fabric pubkey + tenant binding used to verify the close's
/// `intent_metrics_sig`; `None` yields an honest `Unattested(NoPubkey)` cost — a
/// dispatched run whose cost is fabric-recorded but not attested.
///
/// Generic over the transport so the wiring is proven hermetically against a fake
/// transport (zero network); in production `T = UreqRunnerTransport`.
#[allow(clippy::too_many_arguments)]
pub fn dispatch_reexec<T: RunnerTransport>(
    client: &LeaseClient<T>,
    demand: &ReExecDemand,
    measured: &IntentMetrics,
    cost_usd_micros: Option<u64>,
    exit_code: i32,
    fabric_cfg: Option<&FabricAttestConfig>,
) -> ReExecOutcome {
    let acquire = demand.acquire_request();
    let def_digest = demand.def_digest();
    let memo_key = demand.memo_key(&def_digest);

    match dispatch_attest_offbox(
        client,
        &acquire,
        measured,
        &memo_key,
        &demand.target_tree,
        &def_digest,
        // toolchain_digest: unknown at the porcelain re-exec altitude → empty.
        // The synthesized off-box CheckResult it stamps is not the consumed
        // output (the fabric's finalized metrics are); only honesty + validity
        // are required.
        "",
        cost_usd_micros,
        exit_code,
    ) {
        Ok(outcome) => reexecuted(demand, outcome, fabric_cfg),
        // Every lifecycle failure is an HONEST Unavailable — never a fabricated
        // success. The lease was still closed inside `dispatch_attest_offbox`.
        Err(e) => ReExecOutcome::Unavailable {
            intent_id: demand.intent_id.clone(),
            reason: UnavailableReason::from(&e),
            // The RunnerExecError rendering is secret-free by construction (the
            // PAT never appears in it).
            detail: e.to_string(),
        },
    }
}

/// Assemble the successful [`ReExecOutcome::ReExecuted`] from a completed
/// [`DispatchOutcome`], folding in the fail-closed cost-attestation verdict.
fn reexecuted(
    demand: &ReExecDemand,
    outcome: DispatchOutcome,
    fabric_cfg: Option<&FabricAttestConfig>,
) -> ReExecOutcome {
    // The cost verdict reads the propagated sig/key + lease binding off the
    // outcome; it renders nothing and fails closed (Unattested when there is no
    // sig / no config / an invalid sig).
    let cost = cost_attestation_verdict(fabric_cfg, &outcome);
    let succeeded = outcome.result.exit == 0;
    ReExecOutcome::ReExecuted {
        intent_id: demand.intent_id.clone(),
        lease_id: outcome.lease_id,
        result: Box::new(outcome.result),
        metrics: Box::new(outcome.metrics),
        cost,
        succeeded,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cost_attest::UnattestedReason;
    use crate::runner::lease_client::{AcquireResponse, EnvelopeIngest, RunnerConfig, RunnerError};
    use crate::runner::metrics::{RunnerJobMetrics, RunnerTokenCounts, RunnerToolCount};
    use hugit_contracts::context_envelope::{TokenCounts, ToolCount};
    use hugit_contracts::{RunnerLease, RunnerState};
    use std::collections::VecDeque;
    use std::sync::Mutex;

    const SENTINEL_PAT: &str = "pat-REEXEC-SECRET-do-not-leak-4b1c";
    const LEASE: &str = "lease-reexec-1";
    const TENANT: &str = "d863fafb";

    /// In-memory transport: FIFO `(status, body)` responses, records each call so
    /// the scoped-cred-vs-PAT split is provable. NEVER opens a socket.
    #[derive(Debug, Default)]
    struct FakeTransport {
        responses: Mutex<VecDeque<(u16, Vec<u8>)>>,
        calls: Mutex<Vec<(String, String)>>, // (url, bearer)
    }
    impl FakeTransport {
        fn with(responses: Vec<(u16, Vec<u8>)>) -> Self {
            Self {
                responses: Mutex::new(responses.into()),
                calls: Mutex::new(Vec::new()),
            }
        }
        fn next(&self, url: &str, bearer: &str) -> Result<(u16, Vec<u8>), RunnerError> {
            self.calls
                .lock()
                .unwrap()
                .push((url.to_string(), bearer.to_string()));
            self.responses
                .lock()
                .unwrap()
                .pop_front()
                .ok_or_else(|| RunnerError::Transport("fake: no queued response".into()))
        }
        fn urls(&self) -> Vec<String> {
            self.calls
                .lock()
                .unwrap()
                .iter()
                .map(|c| c.0.clone())
                .collect()
        }
    }
    impl RunnerTransport for FakeTransport {
        fn post(&self, u: &str, b: &str, _body: &[u8]) -> Result<(u16, Vec<u8>), RunnerError> {
            self.next(u, b)
        }
        fn get(&self, u: &str, b: &str) -> Result<(u16, Vec<u8>), RunnerError> {
            self.next(u, b)
        }
    }

    fn client(transport: FakeTransport) -> LeaseClient<FakeTransport> {
        let config = RunnerConfig::new("https://runner.example/", SENTINEL_PAT).unwrap();
        LeaseClient::with_transport(config, transport)
    }

    fn demand() -> ReExecDemand {
        ReExecDemand {
            intent_id: "i-42".to_string(),
            charter: "add retry to the client".to_string(),
            ctx_start_ref: "blob:ctxstart".to_string(),
            target_tree: "1".repeat(64),
            image_digest: "hugit-offbox-attest@sha256:0000000000000000000000000000000000000000000000000000000000000000".to_string(),
            net_policy: "deny-all".to_string(),
            tmp_root: "/work/tmp".to_string(),
            expiry_ms: 300_000,
        }
    }

    fn held_lease() -> RunnerLease {
        RunnerLease {
            lease_id: LEASE.to_string(),
            principal_chain: vec!["intent:i-42".to_string()],
            path_set: vec![],
            expiry: 1_000_000,
            net_policy: "deny-all".to_string(),
            tmp_root: "/tmp/runner".to_string(),
            state: RunnerState::Held,
        }
    }

    fn measured() -> IntentMetrics {
        IntentMetrics {
            tokens: TokenCounts {
                input: 111,
                output: 222,
                cache_read: 333,
                cache_write: 444,
                total: 1110,
            },
            wall_ms: 9100,
            active_ms: 7600,
            tool_calls: 5,
            tool_breakdown: vec![ToolCount {
                tool: "Bash".to_string(),
                count: 5,
            }],
            model_turns: 2,
            cost_usd_micros: 5_555_555,
        }
    }

    /// The finalized §13.1 metrics the fabric's close returns (the attested figure).
    fn finalized_metrics() -> RunnerJobMetrics {
        RunnerJobMetrics {
            tokens: RunnerTokenCounts {
                input: 1000,
                output: 200,
                cache_read: 50,
                cache_write: 25,
                total: 1275,
            },
            wall_ms: 9000,
            active_ms: 7500,
            tool_calls: 3,
            tool_breakdown: vec![RunnerToolCount {
                tool: "Bash".into(),
                count: 3,
            }],
            model_turns: 2,
            cost_usd_micros: 20_340_000,
        }
    }

    fn acquire_wrapper(ingest: Option<(&str, &str)>) -> Vec<u8> {
        let resp = AcquireResponse {
            lease: held_lease(),
            exec_endpoint: format!("/v1/leases/{LEASE}/exec"),
            envelope_ingest: ingest.map(|(path, cred)| EnvelopeIngest {
                ingest_path: path.to_string(),
                credential: cred.to_string(),
            }),
        };
        serde_json::to_vec(&resp).unwrap()
    }

    /// A fabric close body carrying `metrics` and (optionally) an attested-cost
    /// `intent_metrics_sig` + `fabric_key_id`. The metrics serialize exactly as the
    /// close decode expects (same `RunnerJobMetrics` type), so a sig computed over
    /// them round-trips through `outcome.metrics` losslessly.
    fn close_body(m: &RunnerJobMetrics, sig: Option<&str>) -> Vec<u8> {
        let mut obj = serde_json::json!({
            "lease_id": LEASE,
            "released": true,
            "capture_incomplete": false,
            "metrics": serde_json::to_value(m).unwrap(),
            "check_result": null,
        });
        if let Some(s) = sig {
            obj["intent_metrics_sig"] = serde_json::json!(s);
            obj["fabric_key_id"] = serde_json::json!("0011223344556677");
        }
        serde_json::to_vec(&obj).unwrap()
    }

    /// A happy acquire→submit→close FIFO (the off-box A-path polls NO box result).
    fn happy(m: &RunnerJobMetrics, sig: Option<&str>) -> Vec<(u16, Vec<u8>)> {
        vec![
            (
                201,
                acquire_wrapper(Some((
                    &format!("/v1/leases/{LEASE}/envelope/ingest"),
                    "scoped-cred",
                ))),
            ),
            (200, Vec::new()), // submit_envelope
            (200, close_body(m, sig)),
        ]
    }

    /// Sign a `RunnerJobMetrics` exactly as the fabric would (the shared
    /// single-sourced pre-image), returning the base64 detached ed25519 sig.
    fn sign(signing: &ed25519_dalek::SigningKey, m: &RunnerJobMetrics) -> String {
        use base64::Engine as _;
        use ed25519_dalek::Signer as _;
        let breakdown: Vec<(String, u64)> = m
            .tool_breakdown
            .iter()
            .map(|t| (t.tool.clone(), t.count))
            .collect();
        let preimage = hugit_refstore::intent_metrics_preimage(
            LEASE,
            TENANT,
            m.tokens.input,
            m.tokens.output,
            m.tokens.cache_read,
            m.tokens.cache_write,
            m.tokens.total,
            m.wall_ms,
            m.active_ms,
            m.tool_calls,
            &breakdown,
            m.model_turns,
            m.cost_usd_micros,
        );
        base64::engine::general_purpose::STANDARD.encode(signing.sign(&preimage).to_bytes())
    }

    // ── ReExecDemand shaping ──────────────────────────────────────────────────

    #[test]
    fn acquire_request_carries_only_the_four_frozen_fields() {
        let d = demand();
        let a = d.acquire_request();
        assert_eq!(a.image_digest, d.image_digest);
        assert_eq!(a.net_policy, "deny-all");
        assert_eq!(a.tmp_root, "/work/tmp");
        assert_eq!(a.expiry_ms, 300_000);
    }

    #[test]
    fn memo_key_and_def_digest_are_deterministic_and_content_addressed() {
        let d = demand();
        let def1 = d.def_digest();
        let def2 = d.def_digest();
        assert_eq!(def1, def2, "def_digest is a pure function of the demand");
        assert_eq!(def1.len(), 64, "lowercase-hex SHA-256");
        let mk1 = d.memo_key(&def1);
        assert_eq!(mk1, d.memo_key(&def1), "memo_key is deterministic");
        // A different charter → a different def → a different memo key.
        let mut d2 = demand();
        d2.charter = "different charter".to_string();
        assert_ne!(def1, d2.def_digest(), "def keys on the charter");
        assert_ne!(mk1, d2.memo_key(&d2.def_digest()), "memo keys on the def");
    }

    // ── the full drive ────────────────────────────────────────────────────────

    /// The full lifecycle: acquire → §13.2 ingest (SCOPED cred) → close (PAT). The
    /// outcome is ReExecuted, carries the fabric's finalized metrics, and the
    /// lifecycle used the scoped cred ONLY for the ingest.
    #[test]
    fn dispatch_reexec_drives_full_lifecycle_and_captures_finalized_metrics() {
        let m = finalized_metrics();
        let c = client(FakeTransport::with(happy(&m, None)));

        let out = dispatch_reexec(&c, &demand(), &measured(), Some(20_340_000), 0, None);

        let ReExecOutcome::ReExecuted {
            intent_id,
            lease_id,
            metrics,
            succeeded,
            result,
            cost,
        } = out
        else {
            panic!("expected ReExecuted, got {out:?}");
        };
        assert_eq!(intent_id, "i-42");
        assert_eq!(lease_id, LEASE);
        assert!(succeeded, "exit 0 ⇒ the re-execution passed acceptance");
        assert_eq!(result.exit, 0);
        // The captured figure is the fabric's FINALIZED close metrics (the FULL
        // cache-split, never flattened) — not the submitted `measured` input.
        assert_eq!(metrics.cost_usd_micros, 20_340_000);
        assert_eq!(metrics.tokens.cache_read, 50);
        assert_eq!(metrics.tokens.total, 1275);
        // No fabric config ⇒ honestly unattested (NoPubkey), never a fabricated pass.
        assert_eq!(
            cost,
            CostAttestation::Unattested(UnattestedReason::NoPubkey)
        );

        // Lifecycle: acquire → ingest → close (no box-result poll on the off-box path).
        let urls = c.transport().urls();
        assert_eq!(urls.len(), 3, "acquire → submit → close: {urls:?}");
        assert!(urls[0].ends_with("/v1/leases"));
        assert!(urls[1].ends_with("/envelope/ingest"));
        assert!(urls[2].ends_with("/close"));
        assert!(
            !urls.iter().any(|u| u.ends_with("/envelope/meta")),
            "off-box path must NOT poll a box result envelope"
        );
    }

    /// A valid fabric sig + matching config ⇒ the cost verdict folds in as
    /// Attested (the cost-attest CONSUME side integrated into the outcome).
    #[test]
    fn valid_sig_folds_into_attested_cost() {
        let signing = ed25519_dalek::SigningKey::from_bytes(&[7u8; 32]);
        let pubkey = {
            use base64::Engine as _;
            base64::engine::general_purpose::STANDARD.encode(signing.verifying_key().to_bytes())
        };
        let m = finalized_metrics();
        let sig = sign(&signing, &m);
        let c = client(FakeTransport::with(happy(&m, Some(&sig))));

        let cfg = FabricAttestConfig {
            fabric_pubkey_b64: pubkey,
            tenant: TENANT.to_string(),
        };
        let out = dispatch_reexec(&c, &demand(), &measured(), None, 0, Some(&cfg));

        assert!(out.cost_is_attested(), "a valid sig ⇒ Attested: {out:?}");
        assert_eq!(out.cost(), Some(CostAttestation::Attested));
    }

    /// A dispatched run whose close carries NO sig ⇒ Unattested(EmissionOff) — the
    /// benign default; the run still counts as ReExecuted.
    #[test]
    fn absent_sig_folds_into_unattested_emission_off() {
        let signing = ed25519_dalek::SigningKey::from_bytes(&[7u8; 32]);
        let pubkey = {
            use base64::Engine as _;
            base64::engine::general_purpose::STANDARD.encode(signing.verifying_key().to_bytes())
        };
        let m = finalized_metrics();
        let c = client(FakeTransport::with(happy(&m, None)));
        let cfg = FabricAttestConfig {
            fabric_pubkey_b64: pubkey,
            tenant: TENANT.to_string(),
        };
        let out = dispatch_reexec(&c, &demand(), &measured(), None, 0, Some(&cfg));
        assert!(out.is_reexecuted());
        assert_eq!(
            out.cost(),
            Some(CostAttestation::Unattested(UnattestedReason::EmissionOff))
        );
        assert!(!out.cost_is_attested());
    }

    /// HONESTY: a FAILED re-execution (`exit_code != 0`) is ReExecuted with
    /// `succeeded=false` — the demand ran but must NOT be landed.
    #[test]
    fn failed_reexecution_is_reexecuted_but_not_succeeded() {
        let m = finalized_metrics();
        let c = client(FakeTransport::with(happy(&m, None)));
        let out = dispatch_reexec(&c, &demand(), &measured(), None, 1, None);
        assert!(out.is_reexecuted(), "the demand WAS dispatched");
        assert!(
            !out.succeeded(),
            "a failed re-execution never reads as green"
        );
        if let ReExecOutcome::ReExecuted { result, .. } = &out {
            assert_eq!(result.exit, 1);
        } else {
            panic!("expected ReExecuted");
        }
    }

    // ── honest failure modes (never a fabricated success) ─────────────────────

    /// The fabric refuses the acquire (500) ⇒ Unavailable(FabricUnreachable), and
    /// the lease was still closed (no leaked capacity). No fabricated success.
    #[test]
    fn acquire_refused_is_unavailable_fabric_unreachable() {
        let c = client(FakeTransport::with(vec![
            (500, Vec::new()), // acquire → terminal Status(500)
            (204, Vec::new()), // close still attempted
        ]));
        let out = dispatch_reexec(&c, &demand(), &measured(), None, 0, None);
        assert!(
            out.is_unavailable(),
            "a refused fabric is Unavailable: {out:?}"
        );
        assert!(!out.succeeded());
        assert!(out.cost().is_none(), "nothing ran ⇒ no cost to attest");
        match out {
            ReExecOutcome::Unavailable {
                reason, intent_id, ..
            } => {
                assert_eq!(reason, UnavailableReason::FabricUnreachable);
                assert_eq!(intent_id, "i-42");
            }
            _ => panic!("expected Unavailable"),
        }
    }

    /// FAIL-CLOSED: an acquire response with NO §13.2 ingest credential ⇒
    /// Unavailable(IngestNotWired) — never a silent B-path fall-back — and the
    /// lease is STILL closed.
    #[test]
    fn no_ingest_credential_is_unavailable_ingest_not_wired() {
        let c = client(FakeTransport::with(vec![
            (201, acquire_wrapper(None)), // no ingest cred
            (204, Vec::new()),            // close still attempted
        ]));
        let out = dispatch_reexec(&c, &demand(), &measured(), None, 0, None);
        match &out {
            ReExecOutcome::Unavailable { reason, .. } => {
                assert_eq!(*reason, UnavailableReason::IngestNotWired);
            }
            _ => panic!("expected Unavailable, got {out:?}"),
        }
        // The lease was still closed (fail-closed).
        let urls = c.transport().urls();
        assert!(
            urls.iter().any(|u| u.ends_with("/close")),
            "the lease must be closed even on the fail-closed path: {urls:?}"
        );
        assert!(
            !urls.iter().any(|u| u.contains("/envelope/ingest")),
            "no ingest submit on the fail-closed path"
        );
    }

    /// The RunnerExecError → UnavailableReason mapping is total + distinct (incl.
    /// the not-reachable-via-transport BoxNotWired external seam).
    #[test]
    fn error_to_reason_mapping_is_total_and_distinct() {
        assert_eq!(
            UnavailableReason::from(&RunnerExecError::Lease(RunnerError::Status(500))),
            UnavailableReason::FabricUnreachable
        );
        assert_eq!(
            UnavailableReason::from(&RunnerExecError::BoxNotWired("host".into())),
            UnavailableReason::BoxNotWired
        );
        assert_eq!(
            UnavailableReason::from(&RunnerExecError::NoEnvelopeIngest("x".into())),
            UnavailableReason::IngestNotWired
        );
        assert_eq!(
            UnavailableReason::from(&RunnerExecError::LeaseNotHeld(RunnerState::Expired)),
            UnavailableReason::LeaseNotHeld
        );
        assert_eq!(
            UnavailableReason::from(&RunnerExecError::Run("refused".into())),
            UnavailableReason::ExecRefused
        );
        // Codes are stable + distinct.
        let codes = [
            UnavailableReason::FabricUnreachable.code(),
            UnavailableReason::BoxNotWired.code(),
            UnavailableReason::IngestNotWired.code(),
            UnavailableReason::LeaseNotHeld.code(),
            UnavailableReason::ExecRefused.code(),
        ];
        let unique: std::collections::HashSet<_> = codes.iter().collect();
        assert_eq!(unique.len(), codes.len(), "reason codes are distinct");
    }

    /// SECRET DISCIPLINE: the PAT never leaks into an Unavailable detail (the
    /// §13.2 ingest submit is rejected 401 → the error rendering is secret-free).
    #[test]
    fn pat_never_leaks_in_unavailable_detail() {
        let c = client(FakeTransport::with(vec![
            (
                201,
                acquire_wrapper(Some((
                    &format!("/v1/leases/{LEASE}/envelope/ingest"),
                    "scoped-cred",
                ))),
            ),
            (401, Vec::new()), // ingest submit rejected → terminal
            (204, Vec::new()), // close still attempted
        ]));
        let out = dispatch_reexec(&c, &demand(), &measured(), None, 0, None);
        let ReExecOutcome::Unavailable { detail, reason, .. } = &out else {
            panic!("expected Unavailable, got {out:?}");
        };
        assert_eq!(*reason, UnavailableReason::FabricUnreachable);
        assert!(
            !detail.contains(SENTINEL_PAT),
            "PAT must not leak: {detail}"
        );
    }
}
