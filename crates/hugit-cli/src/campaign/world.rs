//! The hermetic world seam for the campaign porcelain (WP-PC1, canonical seam PC4).
//!
//! The `--log` file is the **one canonical on-disk seam every porcelain verb
//! shares**: a JSON `[EventRecord, …]` array — exactly the engine's
//! [`hugit_refstore::EventLog`] shape (the hash-chained record array the
//! refstore, dogfood, `hugit pr` and `hugit intent` all read/write). The log is
//! the single source of truth; the campaign projects everything it needs —
//! captured envelopes, PR bundles, the campaign envelope ref — **off the records
//! on that log**, never from a parallel side-document. Captured
//! [`ContextEnvelope`]s ride as `campaign.envelope` / `pr.envelope` /
//! `intent.envelope` records (the same kinds the PC3 PR porcelain already
//! appends), so a PR opened by `hugit pr open` and an intent landed by `hugit
//! intent new --log` compose with `campaign show`/`close` on one shared file.

use std::path::Path;

use hugit_contracts::context_envelope::{Altitude, CampaignRollup, CiCost, ContextEnvelope};
use hugit_contracts::event_record::EventRecord;
use hugit_contracts::verdict_object::VerdictObject;
use hugit_ledger::Ledger;
use hugit_ledger::rollup::{PrPhase, PrQueueInput, campaign_rollup, pr_record};
use hugit_refstore::intent::intents_from_log;
use hugit_refstore::{EventLog, verify_chain};

use super::output::CampaignError;

/// One PR bundle of a campaign — projected from the log's `pr.opened` records.
///
/// The bundle is the queue's truth: which intents a PR proposes to land. PC4
/// derives it from the canonical event log (the `pr.opened` payload carries
/// `pr_id` + `intent_ids` + `campaign`) rather than a hand-authored side list,
/// so `hugit pr open` and `hugit campaign` agree by construction.
#[derive(Debug, Clone)]
pub struct Bundle {
    /// The PR id.
    pub pr_id: String,
    /// The intent ids bundled in this PR.
    pub intent_ids: Vec<String>,
    /// Landing-queue phase: `landed` | `in_flight` | `blocked`.
    pub phase: PrPhase,
}

/// The loaded + projected world for one campaign run.
pub struct World {
    /// The local event log, hash-chained through the real append path.
    pub log: EventLog,
    /// The asked→done→proven ledger projection.
    pub ledger: Ledger,
}

/// The canonical event kinds the campaign porcelain appends (additive over the
/// frozen generic [`hugit_contracts::EventRecord`] — `kind` is a free string,
/// so a new kind is additive without unfreezing any contract).
pub const KIND_CAMPAIGN_OPENED: &str = "campaign.opened";
/// `kind` for the close seal record.
pub const KIND_CAMPAIGN_CLOSED: &str = "campaign.closed";
/// `kind` carrying a captured campaign-altitude [`ContextEnvelope`] (payload is
/// the envelope as canonical JSON), matching the PR porcelain's
/// `pr.envelope` / `intent.envelope` convention.
pub const KIND_CAMPAIGN_ENVELOPE: &str = "campaign.envelope";
/// `kind` carrying the campaign envelope's CAS ref (the F2b seal pointer), as a
/// `{"campaign","envelope_ref"}` payload — distinct from the envelope record
/// itself, since [`ContextEnvelope`] denies unknown fields.
pub const KIND_CAMPAIGN_ENVELOPE_REF: &str = "campaign.envelope_ref";
/// `kind` carrying a captured PR-altitude envelope (shared with `hugit pr`).
pub const KIND_PR_ENVELOPE: &str = "pr.envelope";
/// `kind` carrying a captured intent-altitude envelope (shared with `hugit pr`).
pub const KIND_INTENT_ENVELOPE: &str = "intent.envelope";

impl World {
    /// Read + parse the canonical `[EventRecord, …]` log and build the real
    /// projections. A missing file is an EMPTY log (so `open` may bootstrap it);
    /// any other read/parse/chain fault fails closed.
    pub fn load(path: &Path) -> Result<World, CampaignError> {
        let log = load_canonical_log(path)?;
        let ledger = Ledger::from_records(log.records());
        Ok(World { log, ledger })
    }

    /// Whether a `campaign.opened` record already names this key (idempotency).
    pub fn campaign_opened(&self, key: &str) -> bool {
        self.has_campaign_record(KIND_CAMPAIGN_OPENED, key)
    }

    /// Whether a `campaign.closed` record already names this key.
    pub fn campaign_closed(&self, key: &str) -> bool {
        self.has_campaign_record(KIND_CAMPAIGN_CLOSED, key)
    }

    fn has_campaign_record(&self, kind: &str, key: &str) -> bool {
        self.log.records().iter().any(|r| {
            r.kind == kind
                && serde_json::from_str::<serde_json::Value>(&r.payload)
                    .ok()
                    .and_then(|v| {
                        v.get("campaign")
                            .and_then(|c| c.as_str())
                            .map(str::to_string)
                    })
                    .as_deref()
                    == Some(key)
        })
    }

    /// The PR phases of this campaign, projected from the event log.
    ///
    /// A PR enters **in-flight** when proposed — whether by the legacy
    /// `pr.submitted` kind or the live `hugit pr open` kind (`pr.opened`);
    /// `pr.queued` (entered the landing queue via `hugit pr land`) keeps it
    /// in-flight (queued is not yet landed). `pr.landed` settles it **landed**;
    /// `pr.excluded` settles it **blocked**. Only PRs whose payload names this
    /// campaign are counted. Returned sorted by pr_id for deterministic output.
    pub fn pr_phases(&self, key: &str) -> Vec<(String, PrPhase)> {
        use std::collections::BTreeMap;
        let mut phase: BTreeMap<String, PrPhase> = BTreeMap::new();
        for r in self.log.records() {
            let (pr_id, campaign) = match pr_payload_fields(&r.payload) {
                Some(f) => f,
                None => continue,
            };
            if campaign.as_deref() != Some(key) {
                continue;
            }
            match r.kind.as_str() {
                // Proposed — by either porcelain; first sight enters in-flight.
                // `pr.queued` (entered the landing queue) is still unsettled.
                "pr.submitted" | "pr.opened" | "pr.queued" => {
                    phase.entry(pr_id).or_insert(PrPhase::InFlight);
                }
                "pr.landed" => {
                    phase.insert(pr_id, PrPhase::Landed);
                }
                "pr.excluded" => {
                    phase.insert(pr_id, PrPhase::Blocked);
                }
                _ => {}
            }
        }
        phase.into_iter().collect()
    }

    /// The PRs of this campaign still **in-flight** (proposed/queued, not settled).
    pub fn in_flight_prs(&self, key: &str) -> Vec<String> {
        self.pr_phases(key)
            .into_iter()
            .filter(|(_, p)| *p == PrPhase::InFlight)
            .map(|(id, _)| id)
            .collect()
    }

    /// The PR bundles of this campaign, projected from `pr.opened` records on
    /// the log (the queue's truth: pr_id → intent_ids), paired with each PR's
    /// projected phase. Sorted by pr_id (deterministic).
    pub fn bundles(&self, key: &str) -> Vec<Bundle> {
        use std::collections::BTreeMap;
        let phases: BTreeMap<String, PrPhase> = self.pr_phases(key).into_iter().collect();
        let mut by_id: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for r in self.log.records() {
            if r.kind != "pr.opened" {
                continue;
            }
            let v: serde_json::Value = match serde_json::from_str(&r.payload) {
                Ok(v) => v,
                Err(_) => continue,
            };
            if v.get("campaign").and_then(|c| c.as_str()) != Some(key) {
                continue;
            }
            let pr_id = match v.get("pr_id").and_then(|p| p.as_str()) {
                Some(p) => p.to_string(),
                None => continue,
            };
            let intent_ids = v
                .get("intent_ids")
                .and_then(|x| x.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default();
            // Latest pr.opened for the id wins (re-open robustness).
            by_id.insert(pr_id, intent_ids);
        }
        by_id
            .into_iter()
            .map(|(pr_id, intent_ids)| {
                let phase = phases.get(&pr_id).copied().unwrap_or(PrPhase::InFlight);
                Bundle {
                    pr_id,
                    intent_ids,
                    phase,
                }
            })
            .collect()
    }

    /// The captured campaign envelope ref for `key`, if a `campaign.envelope`
    /// record carries an `envelope_ref` (the F2b seal ref). `None` until
    /// captured — then `close` includes it honestly; absent it the seal is
    /// `not_captured`.
    pub fn campaign_envelope_ref(&self, key: &str) -> Option<String> {
        self.log
            .records()
            .iter()
            .filter(|r| r.kind == KIND_CAMPAIGN_ENVELOPE_REF)
            .filter_map(|r| serde_json::from_str::<serde_json::Value>(&r.payload).ok())
            .filter(|v| v.get("campaign").and_then(|c| c.as_str()) == Some(key))
            .filter_map(|v| {
                v.get("envelope_ref")
                    .and_then(|c| c.as_str())
                    .map(str::to_string)
            })
            .next_back()
    }

    /// The captured envelopes (campaign + PR + intent altitudes), projected off
    /// the `*.envelope` records on the log.
    fn envelopes(&self) -> Vec<ContextEnvelope> {
        self.log
            .records()
            .iter()
            .filter(|r| {
                matches!(
                    r.kind.as_str(),
                    KIND_CAMPAIGN_ENVELOPE | KIND_PR_ENVELOPE | KIND_INTENT_ENVELOPE
                )
            })
            .filter_map(|r| serde_json::from_str::<ContextEnvelope>(&r.payload).ok())
            .collect()
    }

    /// Find the campaign-altitude envelope for `key`, if captured.
    fn campaign_envelope(&self, key: &str) -> Option<ContextEnvelope> {
        self.envelopes()
            .into_iter()
            .find(|e| e.altitude == Altitude::Campaign && e.intent_id == key)
    }

    /// Find the PR-altitude envelope for `pr_id`, if captured.
    fn pr_envelope(&self, pr_id: &str) -> Option<ContextEnvelope> {
        self.envelopes()
            .into_iter()
            .find(|e| e.altitude == Altitude::Pr && e.intent_id == pr_id)
    }

    /// The captured intent-altitude envelopes whose id is in `intent_ids`.
    fn intent_envelopes(&self, intent_ids: &[String]) -> Vec<ContextEnvelope> {
        self.envelopes()
            .into_iter()
            .filter(|e| e.altitude == Altitude::Intent && intent_ids.contains(&e.intent_id))
            .collect()
    }

    /// The intent ids of `intent_ids` the event log projects as **landed** —
    /// the queue's truth via [`intents_from_log`], not the bundle list itself.
    fn landed_intent_ids(&self, intent_ids: &[String]) -> Result<Vec<String>, CampaignError> {
        let projected = intents_from_log(&self.log).map_err(|e| {
            CampaignError::new(
                "bad_log",
                format!("event log does not project: {e}"),
                "fix the malformed intent.landed payload in the --log file",
            )
        })?;
        Ok(projected
            .intents()
            .iter()
            .filter(|i| intent_ids.contains(&i.intent_id))
            .map(|i| i.intent_id.clone())
            .collect())
    }

    /// The verdict panels recorded for `intent_ids` (the D5 seam the Ledger
    /// itself reads), parsed from the log's `verdict.recorded` records.
    fn verdicts_of(&self, intent_ids: &[String]) -> Vec<VerdictObject> {
        self.log
            .records()
            .iter()
            .filter(|r| r.kind == "verdict.recorded")
            .filter_map(|r| serde_json::from_str::<VerdictObject>(&r.payload).ok())
            .filter(|vo| intent_ids.contains(&vo.intent))
            .collect()
    }

    /// Compute the REAL F3 [`CampaignRollup`] for `key` — the seal's cost
    /// decomposition + progress.
    ///
    /// Drives the WP-F3 projections (`pr_record` per bundle → `campaign_rollup`)
    /// over the captured envelopes and the projected PR bundles. Returns:
    /// - `Ok(Some(rollup))` when a campaign envelope is captured;
    /// - `Ok(None)` when no campaign envelope is captured (sealed progress-only,
    ///   honestly, with no fabricated cost);
    /// - `Err(..)` when a captured envelope violates the F3 rules (D14).
    ///
    /// `phases` is the projected PR phase map (so the rollup's progress agrees
    /// with `show`). A bundle whose pr_id has no projected phase falls back to
    /// the bundle's own projected phase.
    pub fn build_rollup(
        &self,
        key: &str,
        phases: &[(String, PrPhase)],
    ) -> Result<Option<CampaignRollup>, CampaignError> {
        let campaign_env = match self.campaign_envelope(key) {
            Some(e) => e,
            None => return Ok(None),
        };
        let campaign_ref = self
            .campaign_envelope_ref(key)
            .unwrap_or_else(|| "not_captured".to_string());

        let bundles = self.bundles(key);
        let mut prs: Vec<(_, PrPhase)> = Vec::with_capacity(bundles.len());
        for b in &bundles {
            let pr_env = self.pr_envelope(&b.pr_id).ok_or_else(|| {
                CampaignError::new(
                    "missing_pr_envelope",
                    format!("bundle '{}' has no captured pr-altitude envelope", b.pr_id),
                    "capture the PR envelope (pr.envelope record) before sealing",
                )
            })?;
            // landed truth is PROJECTED from the log, never the bundle list.
            let landed = self.landed_intent_ids(&b.intent_ids)?;
            let intent_envs = self.intent_envelopes(&b.intent_ids);
            let verdicts = self.verdicts_of(&b.intent_ids);
            let pr_ref = format!("cas:pr-envelope/{}", b.pr_id);
            let rec = pr_record(
                &pr_env,
                &pr_ref,
                &intent_envs,
                &landed,
                &verdicts,
                zero_ci(),
                PrQueueInput::default(),
            )
            .map_err(rollup_error)?;
            // Prefer the projected phase (agrees with `show`); else the bundle's.
            let phase = phases
                .iter()
                .find(|(id, _)| id == &b.pr_id)
                .map(|(_, p)| *p)
                .unwrap_or(b.phase);
            prs.push((rec, phase));
        }

        let rollup = campaign_rollup(&campaign_env, &campaign_ref, &prs).map_err(rollup_error)?;
        Ok(Some(rollup))
    }
}

/// Load the canonical `[EventRecord, …]` log at `path`, rehydrating + verifying
/// the hash chain. A non-existent path is an EMPTY log (so the first `open`
/// bootstraps it); any other read/parse/chain fault fails closed.
fn load_canonical_log(path: &Path) -> Result<EventLog, CampaignError> {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(EventLog::new()),
        Err(e) => return Err(CampaignError::io("read log file", path, &e)),
    };
    let records: Vec<EventRecord> =
        serde_json::from_slice(&bytes).map_err(|e| CampaignError::parse(&e))?;
    let mut log = EventLog::new();
    for record in records {
        log.push_record(record).map_err(|e| {
            CampaignError::new(
                "rehydrate",
                format!("event log does not rehydrate: {e}"),
                "the --log file's records must form a gap-free, monotonic chain",
            )
        })?;
    }
    verify_chain(log.records()).map_err(|e| {
        CampaignError::new(
            "chain_broken",
            format!("event log failed integrity verification: {e}"),
            "the --log file's hash chain is tampered or corrupt",
        )
    })?;
    Ok(log)
}

/// Lift an F3 [`hugit_ledger::rollup::RollupError`] into a structured campaign
/// error (subagent-authored envelope, wrong altitude — both fail-closed).
fn rollup_error(e: hugit_ledger::rollup::RollupError) -> CampaignError {
    use hugit_ledger::rollup::RollupError;
    match &e {
        RollupError::SubagentAuthor { .. } => CampaignError::new(
            "subagent_author",
            e.to_string(),
            "a campaign/PR is authored by the orchestrator or a human, never a subagent (D14)",
        ),
        RollupError::WrongAltitude { .. } => CampaignError::new(
            "wrong_altitude",
            e.to_string(),
            "supply the envelope at the altitude the rollup position requires",
        ),
        RollupError::Overflow { .. } => CampaignError::new(
            "cost_overflow",
            e.to_string(),
            "a cost/token accumulator overflowed u64 — the inputs are corrupt or \
             tampered; the rollup refuses to emit a wrapped figure (fail-closed)",
        ),
    }
}

/// The `(pr_id, campaign)` of a PR event payload, if it carries `pr_id`.
fn pr_payload_fields(payload: &str) -> Option<(String, Option<String>)> {
    let v: serde_json::Value = serde_json::from_str(payload).ok()?;
    let pr_id = v.get("pr_id")?.as_str()?.to_string();
    let campaign = v
        .get("campaign")
        .and_then(|c| c.as_str())
        .map(str::to_string);
    Some((pr_id, campaign))
}

/// A zero CI cost — the checks-seam economics are out of PC1's scope (the
/// rollup tolerates it: hit/exec counts are `0`, savings honestly `0.0`).
fn zero_ci() -> CiCost {
    CiCost {
        cache_hit: 0,
        exec: 0,
        cost_usd_micros: 0,
        saved_usd_micros: 0,
    }
}

/// Append a record to the canonical log and persist it back to `path` as a
/// pretty `[EventRecord, …]` array — the one canonical on-disk seam. The append
/// goes through the REAL [`EventLog::append`] (no forged hashes); the payload is
/// canonicalised before chaining, per the EventRecord freeze.
pub fn append_and_persist(
    world: &World,
    path: &Path,
    kind: &str,
    principal_chain: Vec<String>,
    payload: String,
    recorded_at: u64,
) -> Result<(), CampaignError> {
    let mut log = world.log.clone();
    let payload = hugit_refstore::canonical_json(&payload).unwrap_or(payload);
    log.append(kind.to_string(), principal_chain, payload, recorded_at);
    persist_log(path, &log)
}

/// Persist the canonical event log back to `path` as a pretty `[EventRecord, …]`
/// array (the same shape [`World::load`] reads).
pub fn persist_log(path: &Path, log: &EventLog) -> Result<(), CampaignError> {
    let bytes = serde_json::to_vec_pretty(log.records()).map_err(|e| {
        CampaignError::new(
            "serialize",
            format!("could not serialise the event log: {e}"),
            "this is an internal error — report it",
        )
    })?;
    std::fs::write(path, bytes).map_err(|e| CampaignError::io("write log file", path, &e))
}
