//! `GET /v1/repos/{repo}/releases` → [`ReleasesVm`].
//!
//! The `log` is ALREADY chain-verified by the caller — do NOT re-load or
//! re-verify. There is no git-tag / semver / release-artifact / attestation seam
//! on the local event log, so a "release" here is the closest REAL analogue: a
//! `pr.landed` record (each landed PR is a release-like entry). Nothing is faked.
//!
//! REAL backbone:
//! - `repo` — the router path param.
//! - `releases` — one [`ReleaseVm`] per `pr.landed` record, in chain (seq) order,
//!   capped at [`PR_CARDS_CAP`].
//! - `total_count` — the number of releases actually emitted (`releases.len()`).
//! - `ReleaseVm.age` — PRESENTATION: [`humanize_age`] of the landed record's
//!   `recorded_at` (exact mirror of issues.rs/review.rs/landing.rs).
//! - `ReleaseVm.latest` — true ONLY for the `pr.landed` with the max `.seq` (the
//!   most-recently-landed PR renders the "Latest" pill); false for all others.
//! - `ReleaseVm.title` — DERIVED LABEL from the matching `pr.opened` (via
//!   [`find_pr_opened`]): mirrors review.rs. The campaign is free-text from the
//!   payload → MUST be scrubbed. Honest-default `""` when no `pr.opened` resolves.
//!
//! HONEST-DEFAULT (no local seam — never invented):
//! - `ReleaseVm.version` = `""` — no git-tag / semver seam exists; a fabricated
//!   `vN.N.N` would be a lie.
//! - `stable_count` = 0 — the log has NO stable/prerelease distinction.
//! - `year` = `""` — no epoch→calendar helper in `crate::fmt` (only relative
//!   `humanize_age`), and no chrono dep here; a hardcoded "2026" would be a lie.
//! - `ReleaseVm.notes` = `[]` — no per-release notes seam (`pr.landed` has no body).
//! - `ReleaseVm.attest_line` / `attest_hash` = `""` — the disclosed git/attestation
//!   (SLSA / tree-hash) P2 seam; security.rs ships empty for the same reason.
//! - `ReleaseVm.assets` = `[]` — build-artifact / asset metadata needs the git+CI
//!   release-artifact seam that does not exist locally; [`ReleaseAssetVm`] is
//!   therefore never constructed.
//! - `attestation_note` — a STATIC product-copy constant (the issues.rs `doctrine`
//!   / security.rs `attestation_note` pattern), not log-derived.

use crate::fmt::{PR_CARDS_CAP, humanize_age, scrub};
use hugit_cli::pr::{PR_LANDED_KIND, find_pr_opened};
use hugit_http_contracts::releases::{ReleaseVm, ReleasesVm};
use hugit_refstore::EventLog;
use serde_json::Value;

/// Static product copy (honest constant, not log-derived) — mirrors the
/// `doctrine` / `attestation_note` pattern in issues.rs / security.rs.
const ATTESTATION_NOTE: &str = "release sem atestação não existe aqui.";

/// A `pr.landed` record projected off the log: the `pr_id` it names plus the
/// record's presentation/order fields.
struct Landed {
    pr_id: String,
    recorded_at: u64,
    seq: u64,
}

/// Project every `pr.landed` record off the log, in chain (seq) order. Records
/// already iterate in seq order, so the returned vec preserves it. A landed
/// record with no parseable `pr_id` is skipped (NEVER unwrapped).
fn landed_releases(log: &EventLog) -> Vec<Landed> {
    log.records()
        .iter()
        .filter(|r| r.kind == PR_LANDED_KIND)
        .filter_map(|r| {
            let Ok(v) = serde_json::from_str::<Value>(&r.payload) else {
                return None;
            };
            let pr_id = v.get("pr_id").and_then(Value::as_str)?.to_string();
            Some(Landed {
                pr_id,
                recorded_at: r.recorded_at,
                seq: r.seq,
            })
        })
        .collect()
}

/// Derive the title for a landed PR from its `pr.opened` (campaign + intent
/// count) — mirrors review.rs. The campaign is free-text from the payload, so it
/// MUST be scrubbed. Honest-default `""` when no `pr.opened` resolves (numeric
/// `pr_id` is not free-text, so it needs no scrub).
fn release_title(log: &EventLog, pr_id: &str) -> String {
    let Some(opened) = find_pr_opened(log, pr_id) else {
        return String::new();
    };
    // `pr_id` is payload-derived free text here (NOT a router-validated u32 as in
    // review.rs), so the WHOLE composed title is scrubbed at the read boundary —
    // a secret-shaped pr_id embedded in the title would otherwise echo verbatim.
    let raw = if opened.campaign.is_empty() {
        format!("PR #{} — {} intents", opened.pr_id, opened.intent_ids.len())
    } else {
        format!(
            "PR #{} ({}) — {} intents",
            opened.pr_id,
            opened.campaign,
            opened.intent_ids.len()
        )
    };
    scrub(&raw)
}

/// Build the releases view-model (router calls `build_releases(log, repo)`).
///
/// The `log` is ALREADY chain-verified by the caller. REAL fields are projected
/// off `pr.landed` (+ the matching `pr.opened` for the derived title); every
/// no-seam field carries its honest default. Nothing is faked.
pub fn build_releases(log: &EventLog, repo: &str) -> ReleasesVm {
    let landed = landed_releases(log);

    // REAL: the most-recently-landed PR (max seq) renders the "Latest" pill.
    let latest_seq = landed.iter().map(|l| l.seq).max();

    // P1 list cap: bound the releases projected (the VM IS the page). `landed` is
    // ascending-seq (oldest→newest); take the most-recent PR_CARDS_CAP newest-FIRST
    // so the latest release is always present (renders the "Latest" pill) and the
    // page shows recent releases, not the oldest. total_count == releases.len().
    let releases: Vec<ReleaseVm> = landed
        .iter()
        .rev()
        .take(PR_CARDS_CAP)
        .map(|l| ReleaseVm {
            version: String::new(),              // HONEST — no git-tag / semver seam
            age: humanize_age(l.recorded_at),    // PRESENTATION
            title: release_title(log, &l.pr_id), // DERIVED (scrubbed campaign)
            latest: Some(l.seq) == latest_seq,   // REAL (max seq)
            notes: vec![],                       // HONEST — no per-release notes seam
            attest_line: String::new(),          // HONEST — P2 attestation seam
            attest_hash: String::new(),          // HONEST — P2 attestation seam
            assets: vec![],                      // HONEST — no release-artifact seam
        })
        .collect();

    ReleasesVm {
        repo: repo.to_string(),                         // REAL
        total_count: releases.len(),                    // REAL (== emitted releases)
        stable_count: 0,                                // HONEST — no stable/prerelease distinction
        year: String::new(),                            // HONEST — no epoch→calendar helper
        releases,                                       // REAL
        attestation_note: ATTESTATION_NOTE.to_string(), // static product copy
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hugit_refstore::{Endpoint, PrincipalClass};

    fn append(log: &mut EventLog, kind: &str, payload: serde_json::Value, at: u64) {
        let body = hugit_refstore::canonical_json(&payload.to_string())
            .unwrap_or_else(|| payload.to_string());
        log.append_authorized(
            PrincipalClass::Orchestrator,
            Endpoint::Land,
            kind,
            vec!["o".into()],
            body,
            at,
        )
        .expect("append");
    }

    fn open_pr(log: &mut EventLog, pr_id: &str, campaign: &str, intents: &[&str], at: u64) {
        append(
            log,
            "pr.opened",
            serde_json::json!({
                "author_kind": "orchestrator",
                "campaign": campaign,
                "intent_ids": intents,
                "pr_id": pr_id,
            }),
            at,
        );
    }

    fn land_pr(log: &mut EventLog, pr_id: &str, campaign: &str, at: u64) {
        append(
            log,
            PR_LANDED_KIND,
            serde_json::json!({ "campaign": campaign, "pr_id": pr_id }),
            at,
        );
    }

    #[test]
    fn empty_log_honest_defaults() {
        let vm = build_releases(&EventLog::new(), "humangr/hugit");
        assert_eq!(vm.repo, "humangr/hugit");
        assert_eq!(vm.total_count, 0);
        assert_eq!(vm.stable_count, 0);
        assert_eq!(vm.year, "");
        assert!(vm.releases.is_empty());
        assert_eq!(vm.attestation_note, ATTESTATION_NOTE);
    }

    #[test]
    fn real_projection_from_landed_records() {
        let mut log = EventLog::new();
        open_pr(&mut log, "1", "wave-l", &["i1", "i2"], 1000);
        land_pr(&mut log, "1", "wave-l", 1100);
        open_pr(&mut log, "2", "", &["i3"], 2000);
        land_pr(&mut log, "2", "", 2100);

        let vm = build_releases(&log, "r");
        assert_eq!(vm.total_count, 2);
        assert_eq!(vm.releases.len(), 2);
        // Newest-first order: PR 2 landed last, so it heads the page.
        assert_eq!(vm.releases[0].title, "PR #2 — 1 intents");
        assert_eq!(vm.releases[1].title, "PR #1 (wave-l) — 2 intents");
        // latest == the max-seq landed record (PR 2), now at index 0.
        assert!(vm.releases[0].latest);
        assert!(!vm.releases[1].latest);
        // HONEST defaults on every release.
        assert!(vm.releases.iter().all(|r| r.version.is_empty()));
        assert!(vm.releases.iter().all(|r| r.notes.is_empty()));
        assert!(vm.releases.iter().all(|r| r.assets.is_empty()));
        assert!(vm.releases.iter().all(|r| r.attest_line.is_empty()));
        // age is the humanized landed recorded_at (non-empty).
        assert!(vm.releases.iter().all(|r| !r.age.is_empty()));
    }

    #[test]
    fn landed_without_opened_has_honest_empty_title() {
        let mut log = EventLog::new();
        land_pr(&mut log, "99", "orphan", 5000);
        let vm = build_releases(&log, "r");
        assert_eq!(vm.releases.len(), 1);
        assert_eq!(vm.releases[0].title, "");
        assert!(vm.releases[0].latest);
    }

    #[test]
    fn secret_campaign_redacted_in_json() {
        let mut log = EventLog::new();
        let pat = "ghp_AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
        // A secret-shaped campaign in BOTH the opened (title source) and the
        // landed payload must never echo verbatim to the browser.
        open_pr(&mut log, "1", pat, &["i1"], 1000);
        land_pr(&mut log, "1", pat, 1100);
        let vm = build_releases(&log, "r");
        let j = serde_json::to_string(&vm).unwrap();
        assert!(!j.contains(pat), "PAT must not appear in the serialized VM");
        assert!(j.contains("[REDACTED]"));
    }

    #[test]
    fn secret_pr_id_redacted_in_title() {
        // pr_id is payload free text (not a router-validated u32) — a secret-shaped
        // pr_id embedded in the composed title must be scrubbed.
        let mut log = EventLog::new();
        let pat = "ghp_AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
        open_pr(&mut log, pat, "wave-l", &["i1"], 1000);
        land_pr(&mut log, pat, "wave-l", 1100);
        let j = serde_json::to_string(&build_releases(&log, "r")).unwrap();
        assert!(
            !j.contains(pat),
            "secret-shaped pr_id must not echo in the title"
        );
    }

    #[test]
    fn latest_survives_the_cap() {
        let mut log = EventLog::new();
        for i in 0..(PR_CARDS_CAP as u64 + 5) {
            land_pr(&mut log, &i.to_string(), "", 1000 + i);
        }
        let vm = build_releases(&log, "r");
        assert_eq!(vm.releases.len(), PR_CARDS_CAP);
        assert!(
            vm.releases[0].latest,
            "the newest landed release must be present and flagged"
        );
    }

    #[test]
    fn vm_round_trips() {
        let mut log = EventLog::new();
        open_pr(&mut log, "7", "wave-q", &["a", "b", "c"], 1000);
        land_pr(&mut log, "7", "wave-q", 1100);
        let vm = build_releases(&log, "humangr/hugit");
        let j = serde_json::to_string(&vm).unwrap();
        assert_eq!(vm, serde_json::from_str::<ReleasesVm>(&j).unwrap());
    }
}
