//! `GET /v1/repos/{repo}/home` → [`RepoHomeVm`]. FROZEN signature; body filled by
//! the fleet per master-plan §5 (REAL: branches/last_commit/commit_count via
//! refstore `replay`/`project_machine`; STUB: file tree, readme, about-mirror).

use std::collections::BTreeSet;

use hugit_http_contracts::{AboutVm, LastCommitVm, RepoHomeVm, SynergyVm};
use hugit_refstore::EventLog;
use hugit_refstore::intent::intents_from_log;
use hugit_refstore::intent::projection::{ProjectionRow, project_machine};
use hugit_refstore::replay::replay;

use crate::fmt::{CONTRIBUTORS_CAP, humanize_age, scrub};

/// Extract a display name from a principal-chain entry.
///
/// Strips well-known prefixes (`agent:`, `orchestrator:`) so the raw wire
/// value yields a human-readable contributor name.
fn principal_display_name(entry: &str) -> &str {
    for prefix in &["agent:", "orchestrator:"] {
        if let Some(rest) = entry.strip_prefix(prefix) {
            return rest;
        }
    }
    entry
}

/// Build the repo-home view-model from a verified event log.
///
/// The `log` is ALREADY chain-verified by the caller — do NOT re-load or
/// re-verify. Fields sourced from real engine data are tagged REAL below;
/// fields with no local source are honest defaults (STUB) per master-plan §0/§5.
pub fn build_home(log: &EventLog, repo: &str) -> RepoHomeVm {
    // ── REAL: replay → branch / branch_count / tag_count / branches ────────
    let ref_state = replay(log).unwrap_or_default();

    // Primary branch: the first `refs/heads/*` in sorted ref order. replay never
    // inserts a symbolic `refs/HEAD` pointer (a `refs/HEAD` lookup here was dead
    // code — dropped), so this sorted-first heads entry IS the real path. Empty
    // when the log has advanced no branch ref.
    // Ref short-name is attacker-controllable (a pushed branch name is free text), so
    // scrub it at the read boundary — a secret-shaped branch must never echo.
    let branch: String = ref_state
        .iter()
        .find_map(|(name, _)| name.strip_prefix("refs/heads/"))
        .map(scrub)
        .unwrap_or_default();

    let mut branch_count: usize = 0;
    let mut tag_count: u32 = 0;
    let mut branches: Vec<String> = Vec::new();

    for (name, _target) in ref_state.iter() {
        if let Some(short) = name.strip_prefix("refs/heads/") {
            branch_count += 1;
            branches.push(scrub(short)); // attacker-controllable ref name — scrub
        } else if name.starts_with("refs/tags/") {
            tag_count += 1;
        }
    }

    // ── REAL: project_machine → commit_count / last_commit ─────────────────
    let machine = project_machine(log).unwrap_or_default();
    let commit_count = machine.rows().len().to_string();

    let last_commit: LastCommitVm = machine
        .rows()
        .last()
        .map(|row| match row {
            ProjectionRow::Intent(gc) => {
                // The originating `intent.landed` record. By construction the log
                // is gap-free and 0-based, so `seq == index`: ONE indexed lookup
                // (was two O(n) `.iter().find(|r| r.seq == gc.seq)` scans).
                let record = log.records().get(gc.seq as usize);

                // Author: first entry of the originating record's principal_chain,
                // prefix-stripped to a display name. SCRUBBED — a principal entry
                // is free text echoed from the log; a secret-shaped value never
                // reaches the browser (P0 read-path redaction).
                let author = record
                    .and_then(|r| r.principal_chain.first())
                    .map(|p| scrub(principal_display_name(p)))
                    .unwrap_or_default();

                let recorded_at = record.map(|r| r.recorded_at).unwrap_or(0);

                // safe 6-char slice of the target SHA (structural — NOT scrubbed)
                let short_sha = gc.target.get(..6).unwrap_or(gc.target.as_str()).to_string();

                // Strip "Intent-Id: …" trailer (show charter only), then SCRUB —
                // the charter is free text echoed from the log (P0 redaction). The
                // intent_id / short_sha are content-addresses, left structural.
                let message = scrub(
                    gc.message
                        .lines()
                        .take_while(|l| !l.starts_with("Intent-Id:"))
                        .collect::<Vec<_>>()
                        .join("\n")
                        .trim(),
                );

                LastCommitVm {
                    author,
                    intent_id: gc.intent_id.clone(),
                    message,
                    short_sha,
                    age: humanize_age(recorded_at),
                }
            }
            ProjectionRow::ExternalChange { seq, target, .. } => {
                let recorded_at = log
                    .records()
                    .iter()
                    .find(|r| r.seq == *seq)
                    .map(|r| r.recorded_at)
                    .unwrap_or(0);
                let short_sha = target
                    .as_deref()
                    .and_then(|t| t.get(..6))
                    .unwrap_or("")
                    .to_string();
                LastCommitVm {
                    author: String::new(),
                    intent_id: String::new(),
                    message: String::new(),
                    short_sha,
                    age: humanize_age(recorded_at),
                }
            }
        })
        .unwrap_or_default();

    // ── REAL: intents_from_log → contributors (dedup principal-chain names) ─
    // Each principal entry is free text echoed from the log → SCRUBBED (P0). A
    // `BTreeSet<String>` dedups in O(n log n) (was `Vec::contains`, O(n²)) and
    // also gives a stable sorted order. Capped at `CONTRIBUTORS_CAP`.
    let intent_log = intents_from_log(log).unwrap_or_default();
    let mut contributor_set: BTreeSet<String> = BTreeSet::new();
    for intent in intent_log.intents() {
        for entry in &intent.principal_chain {
            contributor_set.insert(scrub(principal_display_name(entry)));
        }
    }
    let contributors: Vec<String> = contributor_set.into_iter().take(CONTRIBUTORS_CAP).collect();

    // ── PRESENTATION: about.updated_ago from last record's recorded_at ──────
    let updated_ago = log
        .records()
        .last()
        .map(|r| humanize_age(r.recorded_at))
        .unwrap_or_default();

    // ── STUB: all fields with no local engine source ─────────────────────────
    // files: no git-tree API → []
    // readme_html: no rendered README → ""
    // about.description/stars/forks/license: GitHub-mirror P2 → ""
    // about.topics/languages: GitHub-mirror P2 → []
    // about.releases_count: P2 → 0
    // about.release: P2 → None
    // about.contributors_suffix: "" (honest; N already in contributors vec)
    // synergy.lines: no live AC seam → []
    RepoHomeVm {
        repo: repo.to_string(),
        branch,
        branch_count,
        tag_count,
        commit_count,
        last_commit,
        branches,
        files: vec![],              // STUB — no git-tree API
        readme_html: String::new(), // STUB — no local rendered README
        about: AboutVm {
            description: String::new(),         // STUB — GitHub-mirror P2
            topics: vec![],                     // STUB — GitHub-mirror P2
            release: None,                      // STUB — P2 release tag
            contributors,                       // REAL — from principal chains
            contributors_suffix: String::new(), // STUB
            stars: String::new(),               // STUB — GitHub-mirror P2
            forks: String::new(),               // STUB — GitHub-mirror P2
            updated_ago,                        // PRESENTATION
            license: String::new(),             // STUB — GitHub-mirror P2
            releases_count: 0,                  // STUB — P2
            languages: vec![],                  // STUB — GitHub-mirror P2
        },
        synergy: SynergyVm {
            lines: vec![], // STUB — no live AC seam in this wave
        },
    }
}
