//! The hermetic world seam for the campaign porcelain (WP-PC1).
//!
//! Mirrors the `export`/`why` file contract: a `--log` JSON file holding the
//! orchestrator's local state. Events are **un-hashed intents to append** — the
//! hash chain is computed by the REAL [`hugit_refstore::EventLog::append`], so a
//! caller never forges `this_hash`/`prev_hash`. Captured [`ContextEnvelope`]s
//! (the F1/F2 capture path's output) and the queue-bundle truth ride alongside,
//! exactly as the F3 acceptance fixture assembles them — never hand-faked sums.

use std::path::Path;

use serde::{Deserialize, Serialize};

use hugit_contracts::context_envelope::{Altitude, CampaignRollup, CiCost, ContextEnvelope};
use hugit_contracts::verdict_object::VerdictObject;
use hugit_ledger::Ledger;
use hugit_ledger::rollup::{PrPhase, PrQueueInput, campaign_rollup, pr_record};
use hugit_refstore::intent::intents_from_log;
use hugit_refstore::{EventLog, canonical_json};

use super::output::CampaignError;

/// The on-disk world: the local event log + captured envelopes + queue truth.
///
/// Only `events` is required. `envelopes` / `bundles` / `campaign_envelope_ref`
/// are the capture + queue seams; absent them, `close`/`show` degrade honestly
/// (a progress-only seal, `"envelope":"not_captured"`, `"rollup":null`) rather
/// than fabricating a rollup.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WorldInput {
    /// Un-hashed events to append, in order — the local event log.
    #[serde(default)]
    pub events: Vec<EventInput>,
    /// Captured envelopes (campaign + PR + intent altitudes), if available.
    #[serde(default)]
    pub envelopes: Vec<ContextEnvelope>,
    /// Queue-bundle truth: which intents belong to which PR + its phase. The
    /// event payload deliberately does not carry PR↔intent membership (queue
    /// state), so the orchestrator supplies it (PC3 owns its authoring).
    #[serde(default)]
    pub bundles: Vec<Bundle>,
    /// `cas:` ref to the campaign's own captured envelope (the F2b envelope
    /// seal). `null` until F2b emits it — then `close` includes it honestly.
    #[serde(default)]
    pub campaign_envelope_ref: Option<String>,
}

/// One un-hashed event (the `append` inputs; the chain is computed locally).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventInput {
    /// Event kind discriminator (e.g. `"pr.submitted"`, `"campaign.opened"`).
    pub kind: String,
    /// Ordered principal chain that produced the event.
    #[serde(default)]
    pub principal_chain: Vec<String>,
    /// Opaque JSON payload, carried as a string.
    pub payload: String,
    /// Unix epoch ms when recorded (observability; excluded from the chain).
    #[serde(default)]
    pub recorded_at: u64,
}

/// One PR bundle of a campaign — the queue's truth (PC3 authors these).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Bundle {
    /// The PR id.
    pub pr_id: String,
    /// The intent ids bundled in this PR.
    #[serde(default)]
    pub intent_ids: Vec<String>,
    /// Landing-queue phase: `"landed"` | `"in_flight"` | `"blocked"`.
    pub phase: String,
    /// Landing-queue wait, ms.
    #[serde(default)]
    pub queue_wait_ms: u64,
    /// Human decisions/comments on the PR.
    #[serde(default)]
    pub human_touches: u64,
    /// Unix ms when the PR landed; `0` when not landed.
    #[serde(default)]
    pub landed_at: u64,
}

/// The loaded + projected world for one campaign run.
pub struct World {
    /// The local event log, hash-chained through the real append path.
    pub log: EventLog,
    /// The asked→done→proven ledger projection.
    pub ledger: Ledger,
    /// Captured envelopes, as loaded.
    pub envelopes: Vec<ContextEnvelope>,
    /// Queue-bundle truth, as loaded.
    pub bundles: Vec<Bundle>,
    /// Campaign envelope seal ref, if present.
    pub campaign_envelope_ref: Option<String>,
}

/// The canonical event kinds the campaign porcelain appends (additive over the
/// frozen generic [`hugit_contracts::EventRecord`] — `kind` is a free string,
/// so a new kind is additive without unfreezing any contract).
pub const KIND_CAMPAIGN_OPENED: &str = "campaign.opened";
/// `kind` for the close seal record.
pub const KIND_CAMPAIGN_CLOSED: &str = "campaign.closed";

impl World {
    /// Read + parse the world file and build the real EventLog + projections.
    pub fn load(path: &Path) -> Result<World, CampaignError> {
        let bytes =
            std::fs::read(path).map_err(|e| CampaignError::io("read world file", path, &e))?;
        let input: WorldInput =
            serde_json::from_slice(&bytes).map_err(|e| CampaignError::parse(&e))?;
        World::from_input(input)
    }

    /// Build the world from a parsed input — the real append + projection path.
    pub fn from_input(input: WorldInput) -> Result<World, CampaignError> {
        let mut log = EventLog::new();
        for e in &input.events {
            // append computes the hash chain — no forged hashes accepted. Payload
            // is canonicalised (sorted keys, no whitespace) before chaining, per
            // the EventRecord freeze; a non-JSON payload is chained verbatim.
            let payload = canonical_json(&e.payload).unwrap_or_else(|| e.payload.clone());
            log.append(
                e.kind.clone(),
                e.principal_chain.clone(),
                payload,
                e.recorded_at,
            );
        }
        let ledger = Ledger::from_records(log.records());
        Ok(World {
            log,
            ledger,
            envelopes: input.envelopes,
            bundles: input.bundles,
            campaign_envelope_ref: input.campaign_envelope_ref,
        })
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
    /// `pr.submitted` ∖ (`pr.landed` ∪ `pr.excluded`) is **in-flight**;
    /// `pr.excluded` is **blocked**; `pr.landed` is **landed**. Only PRs whose
    /// payload names this campaign are counted (PRs from other campaigns are
    /// ignored). Returned sorted by pr_id for deterministic output.
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
                "pr.submitted" => {
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

    /// The PRs of this campaign still **in-flight** (submitted, not settled).
    pub fn in_flight_prs(&self, key: &str) -> Vec<String> {
        self.pr_phases(key)
            .into_iter()
            .filter(|(_, p)| *p == PrPhase::InFlight)
            .map(|(id, _)| id)
            .collect()
    }

    /// Find the campaign-altitude envelope for `key`, if captured.
    pub fn campaign_envelope(&self, key: &str) -> Option<&ContextEnvelope> {
        self.envelopes
            .iter()
            .find(|e| e.altitude == Altitude::Campaign && e.intent_id == key)
    }

    /// Find the PR-altitude envelope for `pr_id`, if captured.
    pub fn pr_envelope(&self, pr_id: &str) -> Option<&ContextEnvelope> {
        self.envelopes
            .iter()
            .find(|e| e.altitude == Altitude::Pr && e.intent_id == pr_id)
    }

    /// The captured intent-altitude envelopes whose id is in `intent_ids`
    /// (includes every retry / discarded attempt sharing an id).
    pub fn intent_envelopes(&self, intent_ids: &[String]) -> Vec<ContextEnvelope> {
        self.envelopes
            .iter()
            .filter(|e| e.altitude == Altitude::Intent && intent_ids.contains(&e.intent_id))
            .cloned()
            .collect()
    }

    /// The intent ids of `bundle` that the event log projects as **landed** —
    /// the queue's truth via [`intents_from_log`], not the bundle list itself.
    pub fn landed_intent_ids(&self, intent_ids: &[String]) -> Result<Vec<String>, CampaignError> {
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
    pub fn verdicts_of(&self, intent_ids: &[String]) -> Vec<VerdictObject> {
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
    /// over the captured envelopes and the queue-bundle truth. Returns:
    /// - `Ok(Some(rollup))` when a campaign envelope is captured — the rollup is
    ///   computed, never faked;
    /// - `Ok(None)` when no campaign envelope is captured (capture below the
    ///   level that emits it, or pre-F2b) — the caller seals progress-only,
    ///   honestly, with no fabricated cost;
    /// - `Err(..)` when a captured envelope violates the F3 rules (e.g. a
    ///   subagent-authored campaign/PR envelope — D14, fail-closed).
    ///
    /// `phases` is the projected PR phase map (so the rollup's progress agrees
    /// with what `show` reports). A bundle whose pr_id has no projected phase
    /// falls back to its declared `phase` string.
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
            .campaign_envelope_ref
            .clone()
            .unwrap_or_else(|| "not_captured".to_string());

        let mut prs: Vec<(_, PrPhase)> = Vec::with_capacity(self.bundles.len());
        for b in &self.bundles {
            let pr_env = self.pr_envelope(&b.pr_id).ok_or_else(|| {
                CampaignError::new(
                    "missing_pr_envelope",
                    format!("bundle '{}' has no captured pr-altitude envelope", b.pr_id),
                    "capture the PR envelope (F2) or drop the bundle from the world file",
                )
            })?;
            // landed truth is PROJECTED from the log, never the bundle list.
            let landed = self.landed_intent_ids(&b.intent_ids)?;
            let intent_envs = self.intent_envelopes(&b.intent_ids);
            let verdicts = self.verdicts_of(&b.intent_ids);
            let pr_ref = format!("cas:pr-envelope/{}", b.pr_id);
            let rec = pr_record(
                pr_env,
                &pr_ref,
                &intent_envs,
                &landed,
                &verdicts,
                zero_ci(),
                queue_input(b),
            )
            .map_err(rollup_error)?;
            // Prefer the projected phase (agrees with `show`); else the declared.
            let phase = phases
                .iter()
                .find(|(id, _)| id == &b.pr_id)
                .map(|(_, p)| *p)
                .or_else(|| parse_phase(&b.phase))
                .ok_or_else(|| {
                    CampaignError::new(
                        "bad_phase",
                        format!("bundle '{}' has unknown phase '{}'", b.pr_id, b.phase),
                        "phase must be one of landed | in_flight | blocked",
                    )
                })?;
            prs.push((rec, phase));
        }

        let rollup = campaign_rollup(campaign_env, &campaign_ref, &prs).map_err(rollup_error)?;
        Ok(Some(rollup))
    }
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

/// Parse a [`Bundle`]'s phase string into a [`PrPhase`].
pub fn parse_phase(s: &str) -> Option<PrPhase> {
    match s {
        "landed" => Some(PrPhase::Landed),
        "in_flight" => Some(PrPhase::InFlight),
        "blocked" => Some(PrPhase::Blocked),
        _ => None,
    }
}

/// The [`PrQueueInput`] for a bundle (queue-seam state the envelope omits).
pub fn queue_input(b: &Bundle) -> PrQueueInput {
    PrQueueInput {
        queue_wait_ms: b.queue_wait_ms,
        human_touches: b.human_touches,
        landed_at: b.landed_at,
    }
}

/// A zero CI cost — the checks-seam economics are out of PC1's scope (the
/// rollup tolerates it: hit/exec counts are `0`, savings honestly `0.0`).
pub fn zero_ci() -> CiCost {
    CiCost {
        cache_hit: 0,
        exec: 0,
        cost_usd: 0.0,
        saved_usd: 0.0,
    }
}

/// Append a record to the log and serialise the world back to disk as a
/// `WorldInput` (events re-emitted un-hashed; envelopes/bundles preserved).
pub fn append_and_persist(
    world: &World,
    path: &Path,
    kind: &str,
    principal_chain: Vec<String>,
    payload: String,
    recorded_at: u64,
) -> Result<(), CampaignError> {
    // Reconstruct the un-hashed event list from the current log, then add ours.
    let mut events: Vec<EventInput> = world
        .log
        .records()
        .iter()
        .map(|r| EventInput {
            kind: r.kind.clone(),
            principal_chain: r.principal_chain.clone(),
            payload: r.payload.clone(),
            recorded_at: r.recorded_at,
        })
        .collect();
    let payload = canonical_json(&payload).unwrap_or(payload);
    events.push(EventInput {
        kind: kind.to_string(),
        principal_chain,
        payload,
        recorded_at,
    });
    let out = WorldInput {
        events,
        envelopes: world.envelopes.clone(),
        bundles: world.bundles.clone(),
        campaign_envelope_ref: world.campaign_envelope_ref.clone(),
    };
    let bytes = serde_json::to_vec_pretty(&out).map_err(|e| {
        CampaignError::new(
            "serialize",
            format!("could not serialise the world: {e}"),
            "this is an internal error — report it",
        )
    })?;
    std::fs::write(path, bytes).map_err(|e| CampaignError::io("write world file", path, &e))
}
