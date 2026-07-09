//! The lease-scoped check ORCHESTRATION (WP-Wave-E-PR3).
//!
//! This is the glue that turns the lease client's individual verbs
//! ([`LeaseClient::acquire`] / `exec` / `poll_meta` / `close`) into a single
//! fail-closed run: acquire a runner lease, dispatch the check, collect the
//! result + the §13.1 per-job metrics, and **always** release the lease — even
//! when the exec or the collect fails. It is the orchestration half of the same
//! seam [`crate::runner::lease_exec::LiveBoxRunnerExecutor`] now wires.
//!
//! ## What it returns
//!
//! [`exec_and_collect`] assembles a frozen [`CheckResult`] from the runner's
//! result-envelope meta (stamping the supplied memo key + three axes onto the
//! record, identical to the local executor's contract) AND the §13.1
//! [`IntentMetrics`] (via [`crate::runner::metrics`]). [`dispatch_check`] wraps
//! that with the acquire/close lifecycle.
//!
//! ## Secret discipline
//!
//! The PAT never leaves the private [`RunnerConfig`] inside the [`LeaseClient`]:
//! this module only ever holds a `&LeaseClient`, mirrors the lease client's
//! fail-closed posture, and surfaces a lease-client failure as the
//! [`RunnerExecError::Lease`] wrapper (whose inner [`RunnerError`] is itself
//! secret-free by construction).
//!
//! ## Hermetic
//!
//! Every line here is proven against the fake transport (no socket is opened).
//! The only code that touches the network is `UreqRunnerTransport`, which lives
//! behind the same `RunnerTransport` seam the lease client already uses.

use hugit_contracts::check_result::Artifact;
use hugit_contracts::{CheckDef, CheckResult, IntentMetrics, RunnerLease, RunnerState};
use serde::Deserialize;

use crate::runner::lease_client::{
    AcquireLeaseRequest, AcquireResponse, CloseStatus, IngestEvent, IngestUsage, LeaseClient,
    RunnerTransport,
};
use crate::runner::lease_exec::RunnerExecError;
use crate::runner::metrics::RunnerJobMetrics;

/// The runner's result-envelope META (`GET .../envelope/meta`), interpreted.
///
/// PR1 left `poll_meta` returning a raw JSON value on purpose; THIS is the typed
/// interpretation (PR3). A PERMISSIVE projection — deliberately NOT
/// `deny_unknown_fields` — so the runner may carry envelope-level extras
/// (status, attestation refs, …) that the forge does not consume here. The
/// frozen [`CheckResult`] is then BUILT from these fields plus the
/// caller-supplied memo key/axes, never decoded directly (the frozen type is
/// `deny_unknown_fields`).
#[derive(Debug, Clone, Deserialize)]
struct RunnerResultEnvelope {
    /// Process exit code (0 = success). The VERDICT — never defaulted.
    exit: i32,
    /// Output artifacts: (path, digest) pairs.
    #[serde(default)]
    artifacts: Vec<Artifact>,
    /// Content-addressed ref to captured stdout blob.
    #[serde(default)]
    stdout_ref: String,
    /// Content-addressed ref to captured stderr blob.
    #[serde(default)]
    stderr_ref: String,
    /// Wall-clock duration of the check, ms.
    #[serde(default)]
    duration_ms: u64,
    /// Reference to the runner that executed this check.
    #[serde(default)]
    runner_ref: String,
    /// Unix epoch ms when the result was produced.
    #[serde(default)]
    produced_at: u64,
    /// The §13.1 per-job metrics payload.
    metrics: RunnerJobMetrics,
}

/// The outcome of a lease-scoped check run: the frozen [`CheckResult`] and the
/// §13.1 [`IntentMetrics`] mapped from the runner's report.
#[derive(Debug, Clone, PartialEq)]
pub struct DispatchOutcome {
    /// The byte-identity-comparable check result (memo key + axes stamped on).
    pub result: CheckResult,
    /// The per-job metrics (ADR-0001 §2.3 inputs).
    pub metrics: IntentMetrics,
    /// The lease this outcome was produced under — the VERIFY BINDING for the
    /// off-box cost attestation ([`intent_metrics_sig`](Self::intent_metrics_sig)
    /// is bound to `lease_id` + tenant). Previously this only survived as
    /// `result.runner_ref = "offbox:{id}"`; carried explicitly so the
    /// cost-attestation verdict can re-derive the fabric's signed pre-image. Both
    /// paths set it to the lease id (the B-path box lease, the A-path off-box lease).
    pub lease_id: String,
    /// The fabric's off-box **attested-cost signature** over the finalized §13.1
    /// metrics (`CloseResponse.intent_metrics_sig`), bound to `lease_id` + tenant.
    /// `Some` ONLY on the A-path when the fabric emits it (`FABRIC_EMIT_INTENT_
    /// METRICS_SIG`); `None` on the B-path and whenever emission is off. This is
    /// the ONLY thing that can attest an OFF-BOX cost — the verdict fails closed
    /// to `Unattested` when it is `None`.
    pub intent_metrics_sig: Option<String>,
    /// The fabric key id the close named (`CloseResponse.fabric_key_id`), carried
    /// for keyset selection ([`crate::attest_keyset`]). Not consumed by the v1
    /// verdict (the pubkey is supplied by config), but propagated so a later
    /// keyset-select wiring has it without a second fabric round-trip. `None` on
    /// the B-path / when the fabric omits it.
    pub fabric_key_id: Option<String>,
}

/// Execute a check on an ALREADY-HELD lease and collect its outcome.
///
/// Fail-closed: refuses to exec unless the lease is [`RunnerState::Held`], and
/// refuses to fabricate a result if the runner did not accept the exec. Does NOT
/// acquire or close the lease — that lifecycle is the caller's
/// ([`dispatch_check`] for the full path; the live executor for the
/// already-acquired path). Stamps the supplied memo key + three axes onto the
/// returned [`CheckResult`].
pub fn exec_and_collect<T: RunnerTransport>(
    client: &LeaseClient<T>,
    lease: &RunnerLease,
    def: &CheckDef,
    memo_key: &str,
    tree_root: &str,
    def_digest: &str,
    toolchain_digest: &str,
) -> Result<DispatchOutcome, RunnerExecError> {
    if lease.state != RunnerState::Held {
        return Err(RunnerExecError::LeaseNotHeld(lease.state.clone()));
    }

    let ack = client
        .exec(&lease.lease_id, def)
        .map_err(RunnerExecError::Lease)?;
    if !ack.accepted {
        return Err(RunnerExecError::Run(
            "runner refused the exec (accepted=false)".to_string(),
        ));
    }

    let meta = client
        .poll_meta(&lease.lease_id)
        .map_err(RunnerExecError::Lease)?;
    let envelope: RunnerResultEnvelope = serde_json::from_value(meta.0)
        .map_err(|e| RunnerExecError::Run(format!("result-envelope meta decode failed: {e}")))?;

    let RunnerResultEnvelope {
        exit,
        artifacts,
        stdout_ref,
        stderr_ref,
        duration_ms,
        runner_ref,
        produced_at,
        metrics,
    } = envelope;

    let result = CheckResult {
        memo_key: memo_key.to_string(),
        tree_hash: tree_root.to_string(),
        def_digest: def_digest.to_string(),
        toolchain_digest: toolchain_digest.to_string(),
        exit,
        artifacts,
        stdout_ref,
        stderr_ref,
        duration_ms,
        runner_ref,
        produced_at,
    };

    Ok(DispatchOutcome {
        result,
        metrics: metrics.into_intent_metrics(),
        // B-path: a box-exec lease carries NO off-box §13.2 attested-cost binding,
        // so the sig/key are absent by construction — the verdict is honestly
        // `Unattested` for a B-path outcome. The lease id is still carried.
        lease_id: lease.lease_id.clone(),
        intent_metrics_sig: None,
        fabric_key_id: None,
    })
}

/// Drive the full lease lifecycle for one check: acquire → exec → poll → close.
///
/// The lease is ALWAYS closed, regardless of how the exec/collect step fared
/// (fail-closed: a leaked lease holds runner capacity and a filesystem scope).
/// The run error takes precedence over a close error (the close failure is only
/// surfaced when the run itself succeeded, so a real failure is never masked by
/// a best-effort cleanup hiccup).
pub fn dispatch_check<T: RunnerTransport>(
    client: &LeaseClient<T>,
    acquire: &AcquireLeaseRequest,
    def: &CheckDef,
    memo_key: &str,
    tree_root: &str,
    def_digest: &str,
    toolchain_digest: &str,
) -> Result<DispatchOutcome, RunnerExecError> {
    // The fabric returns the AcquireResponse WRAPPER; the B-path needs only the
    // inner lease (the exec endpoint + any §13 ingest credential are ignored here).
    let lease = client
        .acquire(acquire)
        .map_err(RunnerExecError::Lease)?
        .lease;
    let lease_id = lease.lease_id.clone();

    let outcome = exec_and_collect(
        client,
        &lease,
        def,
        memo_key,
        tree_root,
        def_digest,
        toolchain_digest,
    );

    // The close status is the run's honest verdict (exit 0 ⇒ succeeded). On a
    // collect error we still close, claiming `failed` (the box ran but produced
    // no usable result) — never `succeeded`.
    let status = match &outcome {
        Ok(o) if o.result.exit == 0 => CloseStatus::Succeeded,
        _ => CloseStatus::Failed,
    };

    // ALWAYS close — even when the run errored. Explicit (not a Drop guard) so
    // the close call is observable in the hermetic transport assertions. The
    // B-path KEEPS the box-measured `poll_meta` metrics (the `outcome`): a
    // box-exec lease has no §13.2 capture hook, so its close returns the fabric's
    // honest-ZERO projection — preferring it would WIPE the figure the box
    // actually measured. (Contrast the A-path, where the lease carries a §13.2
    // hook and the close metrics ARE the finalized source of truth.)
    // B-path: a box-exec lease has no off-box provider-cost source, so the close
    // submits no `cost_usd_micros` — the box-measured `poll_meta` metrics already
    // carry the figure this path keeps, and the fabric's honest-zero floor is
    // correct for the close itself.
    let close = client
        .close(&lease_id, status, None)
        .map_err(RunnerExecError::Lease);

    match outcome {
        Ok(o) => {
            // Surface a close error; otherwise the box-measured outcome wins.
            close?;
            Ok(o)
        }
        // The run error wins; the lease was still closed above.
        Err(e) => Err(e),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Cost-killer path "A" — the OFF-BOX §13 attest dispatch.
//
// Where the B-path (`dispatch_check`) drives `exec` on a fabric BOX (the box
// measures + returns the §13.1 metrics on poll_meta), the A-path is for an
// agent loop that ran OFF-BOX (hugit's own dispatch client): it SUBMITS its
// trajectory to the lease's §13.2 ingest endpoint with the SCOPED ingest
// credential the acquire response now carries, then reads the fabric's signed
// envelope back via poll (PAT) and closes (PAT). The fabric hosts + signs the
// attestation over what hugit submits.
// ─────────────────────────────────────────────────────────────────────────────

/// Project an off-box-measured §13.1 [`IntentMetrics`] into the §13.2
/// trajectory-event batch the ingest endpoint accepts (the body
/// [`LeaseClient::submit_envelope`] POSTs).
///
/// The measured AGGREGATE is carried faithfully: a `tool_call` event per
/// `tool_breakdown` count (reproducing `tool_calls` + the per-tool split), and a
/// `model_turn` event per `model_turns` (≥1) with the FULL token cache-split
/// usage + the `active_ms` busy span on the FIRST turn. `bytes_b64` is empty —
/// hugit submits METRICS at this seam, not transcript CONTENT (the forge owns
/// transcript capture/redaction). The aggregate-only `wall_ms` and
/// `cost_usd_micros` are fabric-derived at finalize (cost = submitted tokens ×
/// the fabric price card) and are NOT representable as a per-turn event — which
/// is exactly why the attested figure is the fabric's to compute, not hugit's.
pub fn project_intent_metrics(m: &IntentMetrics) -> Vec<IngestEvent> {
    // Bound the per-count expansion: the counts come from `measured` (a hugit-side
    // off-box measurement, NOT a fabric response), but a corrupt/hostile
    // measurement with `count`/`model_turns` near `u64::MAX` would OOM here. The
    // projection is only the SUBMITTED trajectory — the ATTESTED figure is the
    // fabric's finalized CLOSE metrics — so clamping the discrete-event count
    // never affects the source-of-truth cost; it just caps a self-DoS.
    const MAX_PROJECTED_PER_KIND: u64 = 10_000;
    let mut events = Vec::new();
    for tc in &m.tool_breakdown {
        for _ in 0..tc.count.min(MAX_PROJECTED_PER_KIND) {
            events.push(IngestEvent {
                kind: "tool_call".to_string(),
                bytes_b64: String::new(),
                tool: Some(tc.tool.clone()),
                usage: None,
                busy_ms: 0,
            });
        }
    }
    let turns = m.model_turns.clamp(1, MAX_PROJECTED_PER_KIND);
    for i in 0..turns {
        let (usage, busy_ms) = if i == 0 {
            (
                Some(IngestUsage {
                    input: m.tokens.input,
                    output: m.tokens.output,
                    cache_read: m.tokens.cache_read,
                    cache_write: m.tokens.cache_write,
                }),
                m.active_ms,
            )
        } else {
            (None, 0)
        };
        events.push(IngestEvent {
            kind: "model_turn".to_string(),
            bytes_b64: String::new(),
            tool: None,
            usage,
            busy_ms,
        });
    }
    events
}

/// Drive the full OFF-BOX §13 attest lifecycle for one lease: acquire → SUBMIT
/// the off-box trajectory to the §13.2 ingest endpoint (scoped credential) →
/// poll the signed envelope (PAT, for the `CheckResult` fields) → close (PAT).
/// The `measured` metrics are the off-box agent loop's §13.1 figures; they are
/// projected into the §13.2 event batch and submitted. The returned
/// [`DispatchOutcome`] carries the metrics from the fabric's FINALIZED **close**
/// response (`CloseResponse.metrics`) — the signed source of truth — NOT the
/// modeled `poll_meta` figure (this lease carries a §13.2 capture hook, so its
/// close yields the real finalized metrics; if both exist, close wins).
///
/// FAIL-CLOSED (the load-bearing rules):
/// - if the acquire response has NO `envelope_ingest` (a runner lease, or §13
///   off), this returns [`RunnerExecError::NoEnvelopeIngest`] — it NEVER silently
///   falls back to the exec+poll B-path or fabricates an envelope;
/// - the lease is ALWAYS closed (even on a submit / poll / no-ingest error), so
///   a lease never leaks runner capacity;
/// - the SCOPED credential is used ONLY for the ingest submit; acquire / poll /
///   close use the tenant PAT (held privately in the [`LeaseClient`]).
///
/// `cost_usd_micros` is the total cost of the off-box run, submitted VERBATIM on
/// the close (#64), where the fabric (#226) records it into
/// `CloseResponse.metrics.cost_usd_micros`. It MUST be a figure that is TRUE for
/// THIS intent — the per-PR honesty law forbids a FABRICATED, ESTIMATED, or
/// MISATTRIBUTED (another intent's) number. Two truthful sources qualify:
///   - the provider's billed `/usage` figure read by a live off-box agent loop
///     (the most direct; that loop is the not-yet-built P2 seam), OR
///   - the owner-ratified **WP-COST-3 Option A** figure the shipped land caller
///     passes: `Σ(this PR's REAL `ctx.usage` token records × the stamped
///     published price-card rate)` — priced from THIS intent's real measurements
///     at a versioned rate, so it is real + attributable + reproducible, NOT a
///     stand-in. (It is a price-CARD figure, not the invoice, which is why a
///     provider `/usage` bill supersedes it when the loop lands.)
///
/// Pass `None` for the honest-zero floor when neither exists (no `ctx.usage`
/// records / an unknown model / an overflow — the caller's complete-or-nothing).
///
/// `exit_code` is the off-box run's own verdict (`0` = completed successfully):
/// it is carried onto the synthesized off-box `CheckResult.exit` and DECIDES the
/// close status, so a FAILED off-box run (`exit_code != 0`) closes `failed` and is
/// never mis-attested as a success. Callers with no live off-box loop yet (the P2
/// seam) that only reach this path AFTER their own success gate pass `0`.
// The lease-lifecycle inputs (acquire spec, measured metrics, the three memo axes,
// the toolchain digest) plus the #64 provider cost are each load-bearing and
// distinct; a params struct would only obscure the call sites — allow the count.
#[allow(clippy::too_many_arguments)]
pub fn dispatch_attest_offbox<T: RunnerTransport>(
    client: &LeaseClient<T>,
    acquire: &AcquireLeaseRequest,
    measured: &IntentMetrics,
    memo_key: &str,
    tree_root: &str,
    def_digest: &str,
    toolchain_digest: &str,
    cost_usd_micros: Option<u64>,
    exit_code: i32,
) -> Result<DispatchOutcome, RunnerExecError> {
    let acquired = client.acquire(acquire).map_err(RunnerExecError::Lease)?;
    let lease_id = acquired.lease.lease_id.clone();

    let collected = attest_and_collect(
        client,
        &acquired,
        measured,
        memo_key,
        tree_root,
        def_digest,
        toolchain_digest,
        exit_code,
    );

    // The close status is the run's honest verdict: the off-box run's own
    // `exit_code` (carried onto `result.exit`), or `failed` on a collect error.
    // So a FAILED off-box run (`exit_code != 0`) closes `failed` — never
    // mis-attested as a success.
    let status = match &collected {
        Ok(o) if o.result.exit == 0 => CloseStatus::Succeeded,
        _ => CloseStatus::Failed,
    };

    // ALWAYS close — even when the attest step errored (incl. a missing ingest
    // credential). Explicit so the close is observable in the transport asserts.
    // A-path: submit the off-box provider-billed cost (#64) on the close. The
    // fabric records it verbatim into `CloseResponse.metrics.cost_usd_micros`.
    // `None` ⇒ the fabric keeps its honest-zero derived floor.
    let close = client
        .close(&lease_id, status, cost_usd_micros)
        .map_err(RunnerExecError::Lease);

    match collected {
        Ok(o) => {
            // A-MODE ATTESTED FIGURE: the per-job cost is the fabric's FINALIZED
            // §13.1 metrics delivered on the CLOSE response — the signed source of
            // truth — NOT the modeled `poll_meta` figure collected above. The
            // off-box lease carries a §13.2 capture hook, so its close yields the
            // real finalized metrics (the hook-less B-path would get the zero
            // projection — exactly why B keeps poll_meta and A prefers close). If
            // both exist we PREFER close: it is the finalized number the fabric
            // signs and the forge attests. Honesty law preserved — the figure is
            // the fabric's, never a hugit hand-stamp: an honest-zero close yields
            // an honest-zero captured cost.
            let close_resp = close?;
            Ok(DispatchOutcome {
                result: o.result,
                // The attested figure is the fabric's finalized CLOSE metrics.
                metrics: close_resp.metrics.into_intent_metrics(),
                // Carry the verify binding + the fabric's OFF-BOX attested-cost
                // signature (+ key id) OFF the close so the cost-attestation verdict
                // can fail-closed-verify it. `intent_metrics_sig`/`fabric_key_id` are
                // whatever the fabric emitted (both `None` when emission is off — the
                // honest-unattested default). Nothing here CLAIMS attested cost; the
                // verdict does, only after a present sig verifies.
                lease_id,
                intent_metrics_sig: close_resp.intent_metrics_sig,
                fabric_key_id: close_resp.fabric_key_id,
            })
        }
        Err(e) => Err(e),
    }
}

/// Submit the off-box trajectory under an ALREADY-ACQUIRED lease and collect the
/// fabric's signed envelope. Does NOT acquire or close — that lifecycle is
/// [`dispatch_attest_offbox`]'s. Fail-closed: refuses a non-`Held` lease and a
/// lease with no §13.2 ingest credential.
// Mirrors `dispatch_attest_offbox`'s arg set (the memo axes + toolchain + the
// off-box `exit_code` verdict are each load-bearing + distinct); a params struct
// would only obscure the single call site — allow the count.
#[allow(clippy::too_many_arguments)]
fn attest_and_collect<T: RunnerTransport>(
    client: &LeaseClient<T>,
    acquired: &AcquireResponse,
    measured: &IntentMetrics,
    memo_key: &str,
    tree_root: &str,
    def_digest: &str,
    toolchain_digest: &str,
    exit_code: i32,
) -> Result<DispatchOutcome, RunnerExecError> {
    if acquired.lease.state != RunnerState::Held {
        return Err(RunnerExecError::LeaseNotHeld(acquired.lease.state.clone()));
    }

    // Fail-closed: a check lease MUST carry the §13.2 ingest credential for the
    // A-path. Absent → a clear error, never a silent B-path fall-back.
    let ingest = acquired.envelope_ingest.as_ref().ok_or_else(|| {
        RunnerExecError::NoEnvelopeIngest(
            "acquire response carried no envelope_ingest (runner lease, or §13 off)".to_string(),
        )
    })?;

    // Submit the off-box trajectory to the §13.2 ingest endpoint with the SCOPED
    // credential (NEVER the tenant PAT — the lease client uses the PAT for the
    // acquire/poll/close calls only).
    let events = project_intent_metrics(measured);
    client
        .submit_envelope(&ingest.ingest_path, &ingest.credential, &events)
        .map_err(RunnerExecError::Lease)?;

    // OFF-BOX: there is NO box-exec result envelope to poll. The off-box agent
    // loop ran hugit-side; the fabric hosts + signs the §13 attestation but never
    // runs a box, so `GET .../envelope/meta` returns the §13.2 event echo
    // (`{"meta":[]}`) — NOT a `RunnerResultEnvelope` — and the close response's
    // `check_result` is `null`. (Verified LIVE 2026-07-07 against the CF fabricd:
    // the prior code decoded meta as a box `RunnerResultEnvelope` and failed
    // `missing field exit`; it only ever passed against the fake transport — the
    // never-run-live blind spot that also hid #204/#205/#214.) So DON'T poll a box
    // result: synthesize the off-box `CheckResult` from the stamped axes. The
    // CALLER (`hugit pr land --dispatch`) uses ONLY `DispatchOutcome.metrics`,
    // which `dispatch_attest_offbox` supersedes with the fabric's FINALIZED close
    // metrics (the signed source of truth); the `result`/fallback `metrics` here
    // are never surfaced, so they only need to be honest + valid.
    //
    // The off-box run's honest verdict rides `exit_code` (0 = the run completed
    // successfully): the outer `dispatch_attest_offbox` derives the close status
    // from `result.exit`, so a FAILED off-box run (`exit_code != 0`) closes
    // `failed` and never mis-attests a failure as a success (the honesty law).
    let result = CheckResult {
        memo_key: memo_key.to_string(),
        tree_hash: tree_root.to_string(),
        def_digest: def_digest.to_string(),
        toolchain_digest: toolchain_digest.to_string(),
        exit: exit_code,
        artifacts: Vec::new(),
        stdout_ref: String::new(),
        stderr_ref: String::new(),
        duration_ms: measured.wall_ms,
        runner_ref: format!("offbox:{}", acquired.lease.lease_id),
        produced_at: 0,
    };

    Ok(DispatchOutcome {
        result,
        metrics: measured.clone(),
        // INTERMEDIATE outcome: `dispatch_attest_offbox` supersedes `metrics` with
        // the fabric's finalized close metrics AND attaches the close's
        // `intent_metrics_sig`/`fabric_key_id`. The signed figure rides the CLOSE,
        // not this poll-less collect step, so the sig/key are honestly `None` here.
        lease_id: acquired.lease.lease_id.clone(),
        intent_metrics_sig: None,
        fabric_key_id: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runner::lease_client::{
        AcquireResponse, EnvelopeIngest, ExecAck, RunnerConfig, RunnerError,
    };
    use hugit_contracts::context_envelope::{TokenCounts, ToolCount};
    use std::collections::VecDeque;
    use std::sync::Mutex;

    const SENTINEL_PAT: &str = "pat-SUPER-SECRET-do-not-leak-7f3a";

    #[derive(Debug, Clone)]
    struct FakeCall {
        method: &'static str,
        url: String,
        bearer: String,
        body: Vec<u8>,
    }

    /// In-memory transport: FIFO queued `(status, body)` responses, records every
    /// call (incl. the bearer, so the A-path scoped-cred-vs-PAT split is provable).
    /// NEVER opens a socket (mirrors the lease-client fake).
    #[derive(Debug, Default)]
    struct FakeTransport {
        responses: Mutex<VecDeque<(u16, Vec<u8>)>>,
        calls: Mutex<Vec<FakeCall>>,
    }

    impl FakeTransport {
        fn with_responses(responses: Vec<(u16, Vec<u8>)>) -> Self {
            Self {
                responses: Mutex::new(responses.into()),
                calls: Mutex::new(Vec::new()),
            }
        }
        fn next(&self) -> Result<(u16, Vec<u8>), RunnerError> {
            self.responses
                .lock()
                .unwrap()
                .pop_front()
                .ok_or_else(|| RunnerError::Transport("fake: no queued response".into()))
        }
        fn calls(&self) -> Vec<FakeCall> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl RunnerTransport for FakeTransport {
        fn post(
            &self,
            url: &str,
            bearer: &str,
            body: &[u8],
        ) -> Result<(u16, Vec<u8>), RunnerError> {
            self.calls.lock().unwrap().push(FakeCall {
                method: "POST",
                url: url.to_string(),
                bearer: bearer.to_string(),
                body: body.to_vec(),
            });
            self.next()
        }
        fn get(&self, url: &str, bearer: &str) -> Result<(u16, Vec<u8>), RunnerError> {
            self.calls.lock().unwrap().push(FakeCall {
                method: "GET",
                url: url.to_string(),
                bearer: bearer.to_string(),
                body: Vec::new(),
            });
            self.next()
        }
    }

    /// Build the acquire-response WRAPPER bytes for a held lease, optionally
    /// carrying the §13.2 ingest credential (the A-path needs it; a runner-lease
    /// response omits it).
    fn acquire_wrapper(lease_id: &str, ingest: Option<(&str, &str)>) -> Vec<u8> {
        let resp = AcquireResponse {
            lease: sample_lease(lease_id, RunnerState::Held),
            exec_endpoint: format!("/v1/leases/{lease_id}/exec"),
            envelope_ingest: ingest.map(|(path, cred)| EnvelopeIngest {
                ingest_path: path.to_string(),
                credential: cred.to_string(),
            }),
        };
        serde_json::to_vec(&resp).unwrap()
    }

    fn sample_lease(lease_id: &str, state: RunnerState) -> RunnerLease {
        RunnerLease {
            lease_id: lease_id.to_string(),
            principal_chain: vec!["agent:tester".to_string()],
            path_set: vec!["/work".to_string()],
            expiry: 1_000_000,
            net_policy: "deny-all".to_string(),
            tmp_root: "/tmp/runner".to_string(),
            state,
        }
    }

    fn sample_def() -> CheckDef {
        CheckDef {
            def_digest: "a".repeat(64),
            command: "cargo test".to_string(),
            inputs: vec!["src/**".to_string()],
            toolchain_ref: "rust-1.96".to_string(),
            env_manifest: "blob:abc".to_string(),
            glob_set: vec!["**/*.rs".to_string()],
        }
    }

    fn acquire_req() -> AcquireLeaseRequest {
        AcquireLeaseRequest {
            image_digest: "alpine@sha256:d9e853af2c8e".to_string(),
            net_policy: "deny-all".to_string(),
            tmp_root: "/work/tmp".to_string(),
            expiry_ms: 60_000,
        }
    }

    /// A complete result-envelope meta body: the CheckResult fields + §13.1
    /// metrics (with a runner-specific extra to prove it is tolerated).
    fn envelope_body() -> Vec<u8> {
        br#"{
            "exit": 0,
            "artifacts": [{"path": "check.out", "digest": "deadbeef"}],
            "stdout_ref": "blob:out",
            "stderr_ref": "blob:err",
            "duration_ms": 4200,
            "runner_ref": "runner:box-7",
            "produced_at": 1718000000000,
            "status": "done",
            "metrics": {
                "tokens": {"input": 1000, "output": 200, "cache_read": 50, "cache_write": 25, "total": 1275},
                "wall_ms": 9000,
                "active_ms": 7500,
                "tool_calls": 3,
                "tool_breakdown": [{"tool": "Bash", "count": 3}],
                "model_turns": 2,
                "cost_usd_micros": 4281900,
                "cpu_ms": 1234
            }
        }"#
        .to_vec()
    }

    /// A fabric `CloseResponse` body carrying the §13.1 finalized metrics PLUS
    /// the fabric extras hugit tolerates-and-ignores (attestation, sigs,
    /// fabric_key_id, echoed check_result) — proving the liberal decode. The
    /// `cost_usd_micros` + `cache_read` are parameterised so a test can give the
    /// CLOSE a figure DISTINCT from the poll envelope and prove close wins.
    fn close_body(lease_id: &str, cost_usd_micros: u64, cache_read: u64) -> Vec<u8> {
        format!(
            r#"{{
                "lease_id": "{lease_id}",
                "released": true,
                "capture_incomplete": false,
                "metrics": {{
                    "tokens": {{"input": 1000, "output": 200, "cache_read": {cache_read}, "cache_write": 25, "total": 1275}},
                    "wall_ms": 9000,
                    "active_ms": 7500,
                    "tool_calls": 3,
                    "tool_breakdown": [{{"tool": "Bash", "count": 3}}],
                    "model_turns": 2,
                    "cost_usd_micros": {cost_usd_micros}
                }},
                "check_result": null,
                "attestation": {{"tree":"","def":"","runner":"","model":"","principal":["tenant:t"],"sig":"c2ln"}},
                "result_binding_sig": "c2ln",
                "result_binding_sig_v2": "c2lnMg==",
                "fabric_key_id": "0011223344556677"
            }}"#
        )
        .into_bytes()
    }

    /// A fabric `CloseResponse` whose finalized metrics are the HONEST ZERO the
    /// fabric reports when nothing was observed — every meter reads 0.
    fn close_body_zero(lease_id: &str) -> Vec<u8> {
        format!(
            r#"{{
                "lease_id": "{lease_id}",
                "released": true,
                "capture_incomplete": false,
                "metrics": {{
                    "tokens": {{"input": 0, "output": 0, "cache_read": 0, "cache_write": 0, "total": 0}},
                    "wall_ms": 0,
                    "active_ms": 0,
                    "tool_calls": 0,
                    "tool_breakdown": [],
                    "model_turns": 0,
                    "cost_usd_micros": 0
                }}
            }}"#
        )
        .into_bytes()
    }

    /// A fabric `CloseResponse` carrying the off-box attested-cost binding: the
    /// finalized metrics PLUS an `intent_metrics_sig` + `fabric_key_id` (the fields
    /// the fabric emits when `FABRIC_EMIT_INTENT_METRICS_SIG` is on). Used to prove
    /// `dispatch_attest_offbox` PROPAGATES them off the close onto the outcome. The
    /// `sig`/`key` are opaque strings here — propagation carries them verbatim; the
    /// cryptographic verify is exercised in `cost_attest`'s verdict tests.
    fn close_body_with_sig(lease_id: &str, cost_usd_micros: u64, sig: &str, key: &str) -> Vec<u8> {
        format!(
            r#"{{
                "lease_id": "{lease_id}",
                "released": true,
                "capture_incomplete": false,
                "metrics": {{
                    "tokens": {{"input": 1000, "output": 200, "cache_read": 50, "cache_write": 25, "total": 1275}},
                    "wall_ms": 9000,
                    "active_ms": 7500,
                    "tool_calls": 3,
                    "tool_breakdown": [{{"tool": "Bash", "count": 3}}],
                    "model_turns": 2,
                    "cost_usd_micros": {cost_usd_micros}
                }},
                "intent_metrics_sig": "{sig}",
                "fabric_key_id": "{key}"
            }}"#
        )
        .into_bytes()
    }

    fn client(transport: FakeTransport) -> LeaseClient<FakeTransport> {
        let config = RunnerConfig::new("https://runner.example/", SENTINEL_PAT).unwrap();
        LeaseClient::with_transport(config, transport)
    }

    #[test]
    fn full_acquire_exec_poll_close_returns_result_and_metrics() {
        let ack = ExecAck {
            lease_id: "lease-abc123".to_string(),
            accepted: true,
        };
        let transport = FakeTransport::with_responses(vec![
            (201, acquire_wrapper("lease-abc123", None)), // acquire (wrapper)
            (202, serde_json::to_vec(&ack).unwrap()),     // exec
            (200, envelope_body()),                       // poll_meta (box-measured)
            // close — the B-path KEEPS the poll_meta metrics; the close body here
            // is parsed (200 + required metrics) but its figure is discarded.
            (200, close_body("lease-abc123", 999, 999)),
        ]);
        let client = client(transport);

        let out = dispatch_check(
            &client,
            &acquire_req(),
            &sample_def(),
            "f".repeat(64).as_str(),
            "1".repeat(64).as_str(),
            "2".repeat(64).as_str(),
            "3".repeat(64).as_str(),
        )
        .expect("orchestration succeeds");

        // CheckResult: envelope fields + the stamped memo key/axes.
        assert_eq!(out.result.memo_key, "f".repeat(64));
        assert_eq!(out.result.tree_hash, "1".repeat(64));
        assert_eq!(out.result.def_digest, "2".repeat(64));
        assert_eq!(out.result.toolchain_digest, "3".repeat(64));
        assert_eq!(out.result.exit, 0);
        assert_eq!(out.result.artifacts.len(), 1);
        assert_eq!(out.result.artifacts[0].path, "check.out");
        assert_eq!(out.result.artifacts[0].digest, "deadbeef");
        assert_eq!(out.result.duration_ms, 4200);
        assert_eq!(out.result.runner_ref, "runner:box-7");

        // IntentMetrics mapped from §13.1.
        assert_eq!(out.metrics.tokens.total, 1275);
        assert_eq!(out.metrics.tokens.cache_read, 50);
        assert_eq!(out.metrics.cost_usd_micros, 4281900);
        assert_eq!(out.metrics.tool_calls, 3);
        assert_eq!(out.metrics.tool_breakdown.len(), 1);

        // The lifecycle hit all four endpoints in order, ending in close.
        let calls = client.transport().calls();
        assert_eq!(calls.len(), 4);
        assert_eq!(calls[0].url, "https://runner.example/v1/leases");
        assert_eq!(
            calls[1].url,
            "https://runner.example/v1/leases/lease-abc123/exec"
        );
        assert_eq!(
            calls[2].url,
            "https://runner.example/v1/leases/lease-abc123/envelope/meta"
        );
        assert_eq!(
            calls[3].url,
            "https://runner.example/v1/leases/lease-abc123/close"
        );
        assert_eq!(calls[3].method, "POST");
    }

    #[test]
    fn lease_is_always_closed_even_when_exec_errors() {
        let transport = FakeTransport::with_responses(vec![
            (201, acquire_wrapper("lease-err", None)), // acquire (wrapper)
            (500, Vec::new()),                         // exec → terminal Status(500)
            (204, Vec::new()),                         // close MUST still be attempted
        ]);
        let client = client(transport);

        let err = dispatch_check(
            &client,
            &acquire_req(),
            &sample_def(),
            "f".repeat(64).as_str(),
            "1".repeat(64).as_str(),
            "2".repeat(64).as_str(),
            "3".repeat(64).as_str(),
        )
        .expect_err("exec failure surfaces");
        assert!(
            matches!(err, RunnerExecError::Lease(RunnerError::Status(500))),
            "the run error must win; got {err:?}"
        );

        // The lease was STILL closed despite the exec error.
        let calls = client.transport().calls();
        assert_eq!(calls.len(), 3);
        let closed = calls
            .iter()
            .any(|c| c.url.ends_with("/lease-err/close") && c.method == "POST");
        assert!(
            closed,
            "the lease must be closed on the error path: {calls:?}"
        );
    }

    #[test]
    fn exec_and_collect_refuses_a_non_held_lease() {
        let transport = FakeTransport::with_responses(vec![]);
        let client = client(transport);
        let lease = sample_lease("lease-x", RunnerState::Expired);
        let err = exec_and_collect(
            &client,
            &lease,
            &sample_def(),
            "f".repeat(64).as_str(),
            "1".repeat(64).as_str(),
            "2".repeat(64).as_str(),
            "3".repeat(64).as_str(),
        )
        .expect_err("must refuse an expired lease");
        assert_eq!(err, RunnerExecError::LeaseNotHeld(RunnerState::Expired));
        // Fail-closed: no transport call was made.
        assert!(client.transport().calls().is_empty());
    }

    #[test]
    fn refused_exec_is_not_fabricated() {
        // accepted=false MUST NOT be turned into a phantom CheckResult.
        let ack = ExecAck {
            lease_id: "lease-r".to_string(),
            accepted: false,
        };
        let transport = FakeTransport::with_responses(vec![
            (201, acquire_wrapper("lease-r", None)),
            (200, serde_json::to_vec(&ack).unwrap()),
            (204, Vec::new()), // close still happens
        ]);
        let client = client(transport);
        let err = dispatch_check(
            &client,
            &acquire_req(),
            &sample_def(),
            "f".repeat(64).as_str(),
            "1".repeat(64).as_str(),
            "2".repeat(64).as_str(),
            "3".repeat(64).as_str(),
        )
        .expect_err("a refused exec is an error, not a fabricated pass");
        assert!(matches!(err, RunnerExecError::Run(_)));
        // Closed anyway.
        assert!(
            client
                .transport()
                .calls()
                .iter()
                .any(|c| c.url.ends_with("/lease-r/close"))
        );
    }

    #[test]
    fn no_pat_in_dispatch_error_rendering() {
        let transport = FakeTransport::with_responses(vec![
            (201, acquire_wrapper("lease-s", None)),
            (401, Vec::new()), // exec rejected
            (204, Vec::new()),
        ]);
        let client = client(transport);
        let err = dispatch_check(
            &client,
            &acquire_req(),
            &sample_def(),
            "f".repeat(64).as_str(),
            "1".repeat(64).as_str(),
            "2".repeat(64).as_str(),
            "3".repeat(64).as_str(),
        )
        .unwrap_err();
        assert!(!format!("{err:?}").contains(SENTINEL_PAT));
        assert!(!format!("{err}").contains(SENTINEL_PAT));
    }

    // ── Cost-killer path "A" — the OFF-BOX §13 attest dispatch. ──────────────

    /// A §13.1 IntentMetrics with a distinctive cache split + tool breakdown, the
    /// off-box-measured input to the A-path.
    fn measured_metrics() -> IntentMetrics {
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
            tool_breakdown: vec![
                ToolCount {
                    tool: "Bash".to_string(),
                    count: 3,
                },
                ToolCount {
                    tool: "Edit".to_string(),
                    count: 2,
                },
            ],
            model_turns: 2,
            cost_usd_micros: 5_555_555,
        }
    }

    /// (A-1) The full A-path lifecycle: acquire → submit (SCOPED cred) → poll
    /// (PAT) → close (PAT). The scoped ingest credential is used ONLY on the
    /// ingest submit; acquire/poll/close use the tenant PAT. The lease is closed.
    #[test]
    fn attest_offbox_lifecycle_scoped_cred_for_ingest_pat_for_rest() {
        const SCOPED: &str = "scoped-ingest-cred-A1";
        let transport = FakeTransport::with_responses(vec![
            (
                201,
                acquire_wrapper(
                    "lease-attest-1",
                    Some(("/v1/leases/lease-attest-1/envelope/ingest", SCOPED)),
                ),
            ), // acquire
            (200, Vec::new()), // submit_envelope
            // close — carries the FINALIZED §13.1 metrics (the attested figure).
            // The off-box A-path does NOT poll a box result envelope (no box ran;
            // the live fabric returns `{"meta":[]}` + `check_result:null`).
            (200, close_body("lease-attest-1", 4281900, 50)),
        ]);
        let client = client(transport);

        let out = dispatch_attest_offbox(
            &client,
            &acquire_req(),
            &measured_metrics(),
            "f".repeat(64).as_str(),
            "1".repeat(64).as_str(),
            "2".repeat(64).as_str(),
            "3".repeat(64).as_str(),
            // A representative provider-billed cost (#64): it must ride the close
            // body with the exact `cost_usd_micros` key the frozen fabric expects.
            Some(9_900_000),
            0, // exit_code 0 ⇒ close claims succeeded
        )
        .expect("A-path lifecycle succeeds");

        // The returned metrics are the fabric's finalized CLOSE metrics.
        assert_eq!(out.metrics.cost_usd_micros, 4281900);
        assert_eq!(out.metrics.tokens.cache_read, 50);
        // The synthesized off-box result carries the stamped axes + an off-box
        // marker (the caller ignores `result`; it must still be honest + valid).
        assert_eq!(out.result.exit, 0);
        assert_eq!(out.result.runner_ref, "offbox:lease-attest-1");
        assert!(out.result.artifacts.is_empty());

        let calls = client.transport().calls();
        assert_eq!(calls.len(), 3, "acquire → submit → close (NO box poll)");
        // The close body carried the submitted provider-billed cost verbatim (#64).
        let close_body: serde_json::Value = serde_json::from_slice(&calls[2].body).unwrap();
        assert_eq!(close_body["status"], "succeeded");
        assert_eq!(close_body["cost_usd_micros"], 9_900_000);
        // acquire — PAT.
        assert_eq!(calls[0].url, "https://runner.example/v1/leases");
        assert_eq!(calls[0].bearer, format!("Bearer {SENTINEL_PAT}"));
        // submit — the SCOPED credential, to the §13.2 ingest path.
        assert_eq!(
            calls[1].url,
            "https://runner.example/v1/leases/lease-attest-1/envelope/ingest"
        );
        assert_eq!(calls[1].method, "POST");
        assert_eq!(calls[1].bearer, format!("Bearer {SCOPED}"));
        assert!(
            !calls[1].bearer.contains(SENTINEL_PAT),
            "the tenant PAT must NEVER reach the ingest endpoint"
        );
        // No box-result poll on the off-box A-path — `envelope/meta` is never hit.
        assert!(
            !calls.iter().any(|c| c.url.ends_with("/envelope/meta")),
            "off-box A-path must not poll a box result envelope"
        );
        // close — PAT.
        assert_eq!(
            calls[2].url,
            "https://runner.example/v1/leases/lease-attest-1/close"
        );
        assert_eq!(calls[2].bearer, format!("Bearer {SENTINEL_PAT}"));
    }

    /// (A-1b) HONESTY: a FAILED off-box run (`exit_code != 0`) closes the lease
    /// `failed`, never mis-attested as a success. The synthesized off-box
    /// `CheckResult.exit` carries the caller's verdict and the close status
    /// derives from it — so a wired off-box loop that fails cannot silently claim
    /// a green attestation.
    #[test]
    fn attest_offbox_failed_run_closes_failed() {
        const SCOPED: &str = "scoped-ingest-cred-A1b";
        let transport = FakeTransport::with_responses(vec![
            (
                201,
                acquire_wrapper(
                    "lease-attest-fail",
                    Some(("/v1/leases/lease-attest-fail/envelope/ingest", SCOPED)),
                ),
            ), // acquire
            (200, Vec::new()),                            // submit_envelope
            (200, close_body("lease-attest-fail", 0, 0)), // close
        ]);
        let client = client(transport);

        let out = dispatch_attest_offbox(
            &client,
            &acquire_req(),
            &measured_metrics(),
            "f".repeat(64).as_str(),
            "1".repeat(64).as_str(),
            "2".repeat(64).as_str(),
            "3".repeat(64).as_str(),
            None,
            1, // exit_code 1 ⇒ the off-box run FAILED
        )
        .expect("A-path lifecycle still completes (a failed run is closed, not errored)");

        // The synthesized off-box result carries the failed verdict.
        assert_eq!(out.result.exit, 1, "the failed exit rides the CheckResult");
        // And the CLOSE claimed `failed` — the honesty invariant.
        let calls = client.transport().calls();
        let close_body: serde_json::Value = serde_json::from_slice(&calls[2].body).unwrap();
        assert_eq!(
            close_body["status"], "failed",
            "a failed off-box run must close `failed`, never `succeeded`"
        );
    }

    /// (A-2) FAIL-CLOSED: a check lease whose acquire response carries NO
    /// `envelope_ingest` (a runner lease, or §13 off) errors with
    /// `NoEnvelopeIngest` — never a silent B-path fall-back — and the lease is
    /// STILL closed (no leaked capacity).
    #[test]
    fn attest_offbox_fail_closed_when_no_envelope_ingest() {
        let transport = FakeTransport::with_responses(vec![
            (201, acquire_wrapper("lease-no-ingest", None)), // acquire, NO ingest cred
            (204, Vec::new()),                               // close MUST still happen
        ]);
        let client = client(transport);

        let err = dispatch_attest_offbox(
            &client,
            &acquire_req(),
            &measured_metrics(),
            "f".repeat(64).as_str(),
            "1".repeat(64).as_str(),
            "2".repeat(64).as_str(),
            "3".repeat(64).as_str(),
            None,
            0,
        )
        .expect_err("absent envelope_ingest is a fail-closed error");
        assert!(
            matches!(err, RunnerExecError::NoEnvelopeIngest(_)),
            "must be NoEnvelopeIngest, got {err:?}"
        );

        // No submit was attempted (fail-closed before ingest); the lease WAS closed.
        let calls = client.transport().calls();
        assert_eq!(calls.len(), 2, "acquire → close (no submit, no poll)");
        assert!(
            calls
                .iter()
                .any(|c| c.url.ends_with("/lease-no-ingest/close") && c.method == "POST"),
            "the lease must be closed even on the fail-closed path: {calls:?}"
        );
        assert!(
            !calls.iter().any(|c| c.url.contains("/envelope/ingest")),
            "no ingest submit on the fail-closed path"
        );
    }

    /// (A-3) `project_intent_metrics` carries the measured AGGREGATE faithfully
    /// into the §13.2 event batch: tokens on the first model_turn, model_turns as
    /// the model_turn count, tool_calls + the per-tool split as tool_call events.
    #[test]
    fn project_intent_metrics_is_faithful_to_the_aggregate() {
        let events = project_intent_metrics(&measured_metrics());

        let tool_calls: Vec<_> = events.iter().filter(|e| e.kind == "tool_call").collect();
        assert_eq!(tool_calls.len(), 5, "tool_calls reproduced");
        assert_eq!(
            tool_calls
                .iter()
                .filter(|e| e.tool.as_deref() == Some("Bash"))
                .count(),
            3
        );
        assert_eq!(
            tool_calls
                .iter()
                .filter(|e| e.tool.as_deref() == Some("Edit"))
                .count(),
            2
        );

        let model_turns: Vec<_> = events.iter().filter(|e| e.kind == "model_turn").collect();
        assert_eq!(model_turns.len(), 2, "model_turns reproduced");
        // The FIRST turn carries the full token cache-split + the active_ms busy span.
        let usage = model_turns[0]
            .usage
            .as_ref()
            .expect("first turn carries usage");
        assert_eq!(usage.input, 111);
        assert_eq!(usage.output, 222);
        assert_eq!(usage.cache_read, 333);
        assert_eq!(usage.cache_write, 444);
        assert_eq!(model_turns[0].busy_ms, 7600);
        // Subsequent turns carry no usage (the aggregate is on turn 0).
        assert!(model_turns[1].usage.is_none());
    }

    /// (A-4) The A-mode ATTESTED figure comes from the CLOSE response — the
    /// fabric's finalized, signed §13.1 metrics (`7000001`, cache_read `4242`) —
    /// with the FULL cache-split preserved (never flattened). The off-box path
    /// polls no box result envelope; close is the sole source of truth.
    #[test]
    fn attest_offbox_attested_figure_comes_from_close_metrics() {
        const SCOPED: &str = "scoped-ingest-cred-A4";
        let transport = FakeTransport::with_responses(vec![
            (
                201,
                acquire_wrapper(
                    "lease-attest-4",
                    Some(("/v1/leases/lease-attest-4/envelope/ingest", SCOPED)),
                ),
            ),
            (200, Vec::new()), // submit_envelope
            // close — the FINALIZED, signed figure (the off-box path polls no box
            // result; close is the sole source of truth).
            (200, close_body("lease-attest-4", 7_000_001, 4242)),
        ]);
        let client = client(transport);

        let out = dispatch_attest_offbox(
            &client,
            &acquire_req(),
            &measured_metrics(),
            "f".repeat(64).as_str(),
            "1".repeat(64).as_str(),
            "2".repeat(64).as_str(),
            "3".repeat(64).as_str(),
            None,
            0,
        )
        .expect("A-path lifecycle succeeds");

        // The attested figure is the fabric's finalized CLOSE figure.
        assert_eq!(
            out.metrics.cost_usd_micros, 7_000_001,
            "the attested cost is the fabric's finalized close figure"
        );
        // The full cache-split is preserved from the close metrics (never flattened).
        assert_eq!(out.metrics.tokens.cache_read, 4242);
        assert_eq!(out.metrics.tokens.cache_write, 25);
        assert_eq!(out.metrics.tokens.total, 1275);
    }

    /// (A-5) HONEST ZERO: when the fabric's close reports zero metrics (nothing
    /// observed), the attested figure is zero — never a hugit stand-in. Even
    /// though the off-box run had `measured` non-zero inputs, the attested figure
    /// is strictly the fabric's finalized close number.
    #[test]
    fn attest_offbox_honest_zero_when_fabric_reports_zero() {
        const SCOPED: &str = "scoped-ingest-cred-A5";
        let transport = FakeTransport::with_responses(vec![
            (
                201,
                acquire_wrapper(
                    "lease-attest-5",
                    Some(("/v1/leases/lease-attest-5/envelope/ingest", SCOPED)),
                ),
            ),
            (200, Vec::new()), // submit_envelope
            // close — the fabric honestly reports ZERO (no box result poll).
            (200, close_body_zero("lease-attest-5")),
        ]);
        let client = client(transport);

        let out = dispatch_attest_offbox(
            &client,
            &acquire_req(),
            &measured_metrics(),
            "f".repeat(64).as_str(),
            "1".repeat(64).as_str(),
            "2".repeat(64).as_str(),
            "3".repeat(64).as_str(),
            None,
            0,
        )
        .expect("A-path lifecycle succeeds");

        // Honest zero — the fabric reported zero, so the attested figure is zero
        // (never the `measured` input, never a fabricated number).
        assert_eq!(out.metrics.cost_usd_micros, 0);
        assert_eq!(out.metrics.tokens.total, 0);
        assert_eq!(out.metrics.tokens.cache_read, 0);
        assert_eq!(out.metrics.tool_calls, 0);
        assert_eq!(out.metrics.model_turns, 0);
        assert!(out.metrics.tool_breakdown.is_empty());
    }

    /// (A-6) PROPAGATION: `dispatch_attest_offbox` carries the close's
    /// `intent_metrics_sig` + `fabric_key_id` (+ the `lease_id` verify binding) OUT
    /// on the `DispatchOutcome` — the plumbing the cost-attestation verdict needs.
    #[test]
    fn attest_offbox_propagates_intent_metrics_sig_and_key() {
        const SCOPED: &str = "scoped-ingest-cred-A6";
        let transport = FakeTransport::with_responses(vec![
            (
                201,
                acquire_wrapper(
                    "lease-attest-6",
                    Some(("/v1/leases/lease-attest-6/envelope/ingest", SCOPED)),
                ),
            ),
            (200, Vec::new()), // submit_envelope
            (
                200,
                close_body_with_sig("lease-attest-6", 7_000_001, "c2lnLXY2", "0011223344556677"),
            ),
        ]);
        let client = client(transport);

        let out = dispatch_attest_offbox(
            &client,
            &acquire_req(),
            &measured_metrics(),
            "f".repeat(64).as_str(),
            "1".repeat(64).as_str(),
            "2".repeat(64).as_str(),
            "3".repeat(64).as_str(),
            None,
            0,
        )
        .expect("A-path lifecycle succeeds");

        assert_eq!(
            out.lease_id, "lease-attest-6",
            "the verify binding is carried"
        );
        assert_eq!(
            out.intent_metrics_sig.as_deref(),
            Some("c2lnLXY2"),
            "the close's intent_metrics_sig is propagated onto the outcome"
        );
        assert_eq!(
            out.fabric_key_id.as_deref(),
            Some("0011223344556677"),
            "the close's fabric_key_id is propagated onto the outcome"
        );
        // The attested figure is still the fabric's finalized close cost.
        assert_eq!(out.metrics.cost_usd_micros, 7_000_001);
    }

    /// (A-7) A close WITHOUT the sig (emission off) leaves the outcome's
    /// `intent_metrics_sig` `None` — the honest-unattested default. The B-path is
    /// likewise always `None` (no off-box binding on a box-exec lease).
    #[test]
    fn attest_offbox_sig_absent_when_fabric_does_not_emit() {
        const SCOPED: &str = "scoped-ingest-cred-A7";
        let transport = FakeTransport::with_responses(vec![
            (
                201,
                acquire_wrapper(
                    "lease-attest-7",
                    Some(("/v1/leases/lease-attest-7/envelope/ingest", SCOPED)),
                ),
            ),
            (200, Vec::new()),
            // A close body with NO intent_metrics_sig (emission off) — it does carry
            // a `fabric_key_id` (the two are independent: a key id may ride the close
            // even when the metrics sig is not emitted).
            (200, close_body("lease-attest-7", 4281900, 50)),
        ]);
        let client = client(transport);

        let out = dispatch_attest_offbox(
            &client,
            &acquire_req(),
            &measured_metrics(),
            "f".repeat(64).as_str(),
            "1".repeat(64).as_str(),
            "2".repeat(64).as_str(),
            "3".repeat(64).as_str(),
            None,
            0,
        )
        .expect("A-path lifecycle succeeds");

        // The load-bearing invariant: NO sig ⇒ the cost stays honestly unattested.
        assert!(
            out.intent_metrics_sig.is_none(),
            "emission off ⇒ no sig carried (the cost is unattested)"
        );
        assert_eq!(out.lease_id, "lease-attest-7");
    }
}
