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
use crate::pr::filelock::{self, FileLock, LockError};

/// Map a [`LockError`] into a structured [`CampaignError`] (a live holder →
/// `log_busy`, retry-able; an I/O fault → the existing `io` kind).
fn lock_campaign_error(e: LockError) -> CampaignError {
    match e {
        LockError::Busy { path } => CampaignError::new(
            "log_busy",
            format!(
                "the --log file {} is locked by another hugit verb",
                path.display()
            ),
            "another `hugit` process holds the log lock; retry once it releases \
             (a stale lock is auto-reclaimed after a short window)",
        ),
        LockError::Io { .. } => CampaignError::new(
            "io",
            e.to_string(),
            "check the --log path is on a writable directory",
        ),
    }
}

/// The campaign-altitude PR phase — the ledger's [`PrPhase`] (landed / in-flight
/// / blocked) PLUS the **abandoned** terminal the porcelain `pr abandon` writes
/// (`pr.abandoned`).
///
/// The ledger [`PrPhase`] (in `hugit-ledger`, not this crate's to change) has no
/// `Abandoned` variant; modelling it here keeps `pr abandon` HONORED across the
/// campaign projections (WF-2: an abandoned PR must leave the in-flight set so
/// `campaign close` can reach `closed:true`) while preserving the `abandoned`
/// label in `campaign show` (it is NOT in-flight and NOT a `blocked`
/// union-test failure — it is a terminal the owner chose).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CampaignPrPhase {
    /// Settled landed (`pr.landed`).
    Landed,
    /// Proposed/queued, not yet settled (`pr.opened`/`pr.submitted`/`pr.queued`).
    InFlight,
    /// Settled blocked — excluded by a union-test failure (`pr.excluded`).
    Blocked,
    /// Terminally abandoned by the owner (`pr.abandoned`) — settled, never
    /// in-flight, never blocking a close.
    Abandoned,
}

impl CampaignPrPhase {
    /// Map onto the ledger [`PrPhase`] for the F3 rollup, which has no
    /// `Abandoned` variant: an abandoned PR is settled (excluded from the landed
    /// set), so it folds onto [`PrPhase::Blocked`] — the same settled-not-landed
    /// position the rollup already accounts for. `show`/`close` keep the
    /// distinct `abandoned` LABEL via [`CampaignPrPhase`] itself.
    pub fn to_ledger_phase(self) -> PrPhase {
        match self {
            CampaignPrPhase::Landed => PrPhase::Landed,
            CampaignPrPhase::InFlight => PrPhase::InFlight,
            CampaignPrPhase::Blocked | CampaignPrPhase::Abandoned => PrPhase::Blocked,
        }
    }

    /// The stable wire label for this phase (the `phase` field in `show`/`close`).
    pub fn label(self) -> &'static str {
        match self {
            CampaignPrPhase::Landed => "landed",
            CampaignPrPhase::InFlight => "in_flight",
            CampaignPrPhase::Blocked => "blocked",
            CampaignPrPhase::Abandoned => "abandoned",
        }
    }
}

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
    /// Landing-queue phase: `landed` | `in_flight` | `blocked` | `abandoned`.
    pub phase: CampaignPrPhase,
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
/// `kind` for the abandon record — appended by `hugit campaign abandon`.
pub const KIND_CAMPAIGN_ABANDONED: &str = "campaign.abandoned";
/// `kind` carrying a captured PR-altitude envelope (shared with `hugit pr`).
pub const KIND_PR_ENVELOPE: &str = "pr.envelope";
/// `kind` carrying a captured intent-altitude envelope (shared with `hugit pr`).
pub const KIND_INTENT_ENVELOPE: &str = "intent.envelope";

impl World {
    /// Read + parse the canonical `[EventRecord, …]` log and build the real
    /// projections. A missing file is an EMPTY log (so `open` may bootstrap it);
    /// any other read/parse/chain fault fails closed.
    ///
    /// This is the BOOTSTRAP loader — used by the mutating verbs (`open` /
    /// `close` / `abandon`) that may legitimately start from an absent log. The
    /// read-only queries (`show` / `list`) must instead use
    /// [`World::load_existing`], which rejects a missing file (P-CAMPAIGN-EMPTY:
    /// a missing `--log` is `log_not_found`/exit-2, never a silent empty world).
    pub fn load(path: &Path) -> Result<World, CampaignError> {
        let log = load_canonical_log(path)?;
        let ledger = Ledger::from_records(log.records());
        Ok(World { log, ledger })
    }

    /// Like [`World::load`], but a MISSING `--log` file is an explicit
    /// `log_not_found` error (exit-2), never a silent empty world.
    ///
    /// The read-only campaign queries (`show` / `list`) call this so they cannot
    /// report an empty/exit-0 result for a path that does not exist
    /// (P-CAMPAIGN-EMPTY — the regression `checks`/`queue` already get right).
    /// `open`'s bootstrap-on-absent behavior is preserved by keeping it on
    /// [`World::load`]; the distinction is at the CALL SITE, not the shared
    /// canonical loader.
    pub fn load_existing(path: &Path) -> Result<World, CampaignError> {
        if !path.exists() {
            return Err(CampaignError::log_not_found(path));
        }
        World::load(path)
    }

    /// Whether a `campaign.opened` record already names this key (idempotency).
    pub fn campaign_opened(&self, key: &str) -> bool {
        self.has_campaign_record(KIND_CAMPAIGN_OPENED, key)
    }

    /// Whether a `campaign.closed` record already names this key.
    pub fn campaign_closed(&self, key: &str) -> bool {
        self.has_campaign_record(KIND_CAMPAIGN_CLOSED, key)
    }

    /// Whether a `campaign.abandoned` record already names this key.
    pub fn campaign_abandoned(&self, key: &str) -> bool {
        self.has_campaign_record(KIND_CAMPAIGN_ABANDONED, key)
    }

    /// Project the charter and owner of the first `campaign.opened` record for
    /// `key`, if any.
    pub fn campaign_charter_owner(&self, key: &str) -> Option<(String, String)> {
        self.log
            .records()
            .iter()
            .find(|r| {
                r.kind == KIND_CAMPAIGN_OPENED
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
            .and_then(|r| {
                let v: serde_json::Value = serde_json::from_str(&r.payload).ok()?;
                let charter = v.get("charter")?.as_str()?.to_string();
                let owner = v.get("owner")?.as_str()?.to_string();
                Some((charter, owner))
            })
    }

    /// Project the abandon reason for `key`, if abandoned.
    pub fn campaign_abandon_reason(&self, key: &str) -> Option<String> {
        self.log
            .records()
            .iter()
            .filter(|r| r.kind == KIND_CAMPAIGN_ABANDONED)
            .filter_map(|r| serde_json::from_str::<serde_json::Value>(&r.payload).ok())
            .filter(|v| v.get("campaign").and_then(|c| c.as_str()) == Some(key))
            .filter_map(|v| v.get("reason").and_then(|r| r.as_str()).map(str::to_string))
            .next_back()
    }

    /// Project all known campaign keys from the log, in stable (sorted) order.
    ///
    /// A key appears if it has at least one `campaign.opened` record.
    pub fn all_campaign_keys(&self) -> Vec<String> {
        use std::collections::BTreeSet;
        let mut keys: BTreeSet<String> = BTreeSet::new();
        for r in self.log.records() {
            if r.kind == KIND_CAMPAIGN_OPENED
                && let Ok(v) = serde_json::from_str::<serde_json::Value>(&r.payload)
                && let Some(key) = v.get("campaign").and_then(|c| c.as_str())
            {
                keys.insert(key.to_string());
            }
        }
        keys.into_iter().collect()
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
    /// `pr.excluded` settles it **blocked**; **`pr.abandoned` settles it
    /// `abandoned`** (WF-2 — the porcelain `pr abandon` terminal; an abandoned
    /// PR must leave the in-flight set so it no longer blocks `campaign close`,
    /// exactly as `pr show`/`pr list` already project it). `pr.abandoned`
    /// refuses to overwrite a `pr.landed` (a landed PR cannot be abandoned —
    /// the verb itself refuses), so a stray out-of-order record can never
    /// un-settle a landed PR. Only PRs whose payload names this campaign are
    /// counted. Returned sorted by pr_id for deterministic output.
    pub fn pr_phases(&self, key: &str) -> Vec<(String, CampaignPrPhase)> {
        use std::collections::BTreeMap;
        let mut phase: BTreeMap<String, CampaignPrPhase> = BTreeMap::new();
        for r in self.log.records() {
            let (pr_id, campaign) = match pr_payload_fields(&r.payload) {
                Some(f) => f,
                None => continue,
            };
            // WF-2: the porcelain `pr abandon` writes `pr.abandoned` with a
            // `{"pr_id","reason"}` payload that does NOT name the campaign (the
            // pr porcelain has no campaign in scope at abandon time). Match it
            // by pr_id against the PRs THIS campaign already tracks: an
            // abandoned PR leaves the in-flight set so it no longer blocks
            // `campaign close`. Never demote an already-landed PR (abandon
            // refuses a landed PR; a stray record must not un-settle it).
            if r.kind == "pr.abandoned" {
                if let Some(slot) = phase.get_mut(&pr_id)
                    && *slot != CampaignPrPhase::Landed
                {
                    *slot = CampaignPrPhase::Abandoned;
                }
                continue;
            }
            // Every other PR-lifecycle kind is scoped to this campaign by its
            // payload's `campaign` field.
            if campaign.as_deref() != Some(key) {
                continue;
            }
            match r.kind.as_str() {
                // Proposed — by either porcelain; first sight enters in-flight.
                // `pr.queued` (entered the landing queue) is still unsettled.
                "pr.submitted" | "pr.opened" | "pr.queued" => {
                    phase.entry(pr_id).or_insert(CampaignPrPhase::InFlight);
                }
                "pr.landed" => {
                    phase.insert(pr_id, CampaignPrPhase::Landed);
                }
                "pr.excluded" => {
                    phase.insert(pr_id, CampaignPrPhase::Blocked);
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
            .filter(|(_, p)| *p == CampaignPrPhase::InFlight)
            .map(|(id, _)| id)
            .collect()
    }

    /// The PR bundles of this campaign, projected from `pr.opened` records on
    /// the log (the queue's truth: pr_id → intent_ids), paired with each PR's
    /// projected phase. Sorted by pr_id (deterministic).
    pub fn bundles(&self, key: &str) -> Vec<Bundle> {
        use std::collections::BTreeMap;
        let phases: BTreeMap<String, CampaignPrPhase> = self.pr_phases(key).into_iter().collect();
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
                let phase = phases
                    .get(&pr_id)
                    .copied()
                    .unwrap_or(CampaignPrPhase::InFlight);
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
        phases: &[(String, CampaignPrPhase)],
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
            // The F3 rollup consumes the ledger `PrPhase` (no `Abandoned`
            // variant) — an abandoned PR folds onto the settled-not-landed
            // `Blocked` position (it never landed, never blocks).
            let phase = phases
                .iter()
                .find(|(id, _)| id == &b.pr_id)
                .map(|(_, p)| *p)
                .unwrap_or(b.phase)
                .to_ledger_phase();
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

/// Append a campaign-lifecycle record through the **D14 authorization guard**
/// ([`EventLog::append_authorized`]) and persist back to `path`.
///
/// Campaign mutations are human-owned (the owner field carries the human
/// principal: `user:<owner>`). The guard is gated on
/// `(PrincipalClass::Human, Endpoint::Policy)` — the human stakeholder-control
/// cell, which is distinct from the PR porcelain's `(Human, Undo)` gate and
/// matches the "campaign owner decides" semantics. A denial (which can only
/// happen if the asserted class were somehow not human) is mapped to a
/// structured `CampaignError` and the audit record is persisted alongside it.
pub fn append_authorized_and_persist(
    world: &World,
    path: &Path,
    kind: &str,
    owner: &str,
    payload: String,
    recorded_at: u64,
) -> Result<(), CampaignError> {
    use hugit_refstore::{Endpoint, PrincipalClass};
    // Serialize the append+persist behind the advisory exclusive lock (WP-WC1):
    // a campaign mutation racing another verb on the same `--log` gets the
    // structured `log_busy` rather than clobbering. The guard releases when this
    // function returns (or on any early `?`). The atomic write below makes the
    // persist truncation-proof; together they kill the campaign-seam TOCTOU.
    let _lock = FileLock::acquire(path).map_err(lock_campaign_error)?;
    let mut log = world.log.clone();
    let payload = hugit_refstore::canonical_json(&payload).unwrap_or(payload);
    let principal_chain = vec![format!("user:{owner}")];
    log.append_authorized(
        PrincipalClass::Human,
        Endpoint::Policy,
        kind.to_string(),
        principal_chain,
        payload,
        recorded_at,
    )
    .map_err(|denied| {
        CampaignError::new(
            "authz_denied",
            format!(
                "campaign mutation denied by D14 guard: {}",
                denied.reason.code()
            ),
            "campaign operations must be driven by a human principal (user: prefix)",
        )
    })?;
    // The log now carries the appended record; persist it.
    persist_log(path, &log)
}

/// Persist the canonical event log back to `path` as a pretty `[EventRecord, …]`
/// array (the same shape [`World::load`] reads), via the **atomic**
/// temp-file-then-rename write (WP-WC1) — a reader or a crash sees the whole old
/// log or the whole new one, never a truncated file.
///
/// The lock-discipline is the caller's: [`append_authorized_and_persist`] holds
/// the advisory exclusive lock across its append→persist. A direct
/// [`persist_log`] caller relies on the atomic write alone for truncation-safety.
pub fn persist_log(path: &Path, log: &EventLog) -> Result<(), CampaignError> {
    let bytes = serde_json::to_vec_pretty(log.records()).map_err(|e| {
        CampaignError::new(
            "serialize",
            format!("could not serialise the event log: {e}"),
            "this is an internal error — report it",
        )
    })?;
    filelock::atomic_write(path, &bytes).map_err(lock_campaign_error)
}
