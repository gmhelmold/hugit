//! `GET /v1/repos/{repo}/home` → [`RepoHomeVm`]. FROZEN signature; body filled by
//! the fleet per master-plan §5 (REAL: branches/last_commit/commit_count via
//! refstore `replay`/`project_machine`; STUB: file tree, readme, about-mirror).

use hugit_http_contracts::{AboutVm, LastCommitVm, RepoHomeVm, SynergyVm};
use hugit_refstore::EventLog;
use hugit_refstore::intent::intents_from_log;
use hugit_refstore::intent::projection::{ProjectionRow, project_machine};
use hugit_refstore::replay::replay;

/// Humanize a Unix-epoch-millisecond timestamp into a pt-BR age string.
///
/// Returns a string like "há 3 min", "há 2h", "há 5d". The wall clock is read
/// via `std::time::SystemTime`; this is presentation-only (not in any hash
/// chain), so the non-hermetic clock read is acceptable here.
fn humanize_age(unix_ms: u64) -> String {
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;

    let delta_ms = now_ms.saturating_sub(unix_ms);
    let secs = delta_ms / 1_000;

    if secs < 60 {
        "há menos de 1 min".to_string()
    } else if secs < 3_600 {
        let mins = secs / 60;
        format!("há {mins} min")
    } else if secs < 86_400 {
        let hours = secs / 3_600;
        format!("há {hours}h")
    } else {
        let days = secs / 86_400;
        format!("há {days}d")
    }
}

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

    // HEAD branch: resolve refs/HEAD → a symbolic target, then strip prefix.
    // hugit stores the head branch as refs/heads/<name>. We look for the
    // symbolic HEAD pointer (stored as "refs/HEAD" → "refs/heads/<branch>")
    // and fall back to any refs/heads entry.
    let branch: String = ref_state
        .get("refs/HEAD")
        .and_then(|target| target.strip_prefix("refs/heads/"))
        .map(str::to_string)
        .unwrap_or_else(|| {
            // Fallback: first refs/heads/* in sorted order.
            ref_state
                .iter()
                .find_map(|(name, _)| name.strip_prefix("refs/heads/"))
                .map(str::to_string)
                .unwrap_or_default()
        });

    let mut branch_count: usize = 0;
    let mut tag_count: u32 = 0;
    let mut branches: Vec<String> = Vec::new();

    for (name, _target) in ref_state.iter() {
        if let Some(short) = name.strip_prefix("refs/heads/") {
            branch_count += 1;
            branches.push(short.to_string());
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
                // Author: first entry in the originating intent's principal_chain
                // that carries agent:/orchestrator: prefix. We recover the intent
                // to get the principal_chain via intents_from_log.
                let author = log
                    .records()
                    .iter()
                    .find(|r| r.seq == gc.seq)
                    .and_then(|r| r.principal_chain.first())
                    .map(|p| principal_display_name(p).to_string())
                    .unwrap_or_default();

                let recorded_at = log
                    .records()
                    .iter()
                    .find(|r| r.seq == gc.seq)
                    .map(|r| r.recorded_at)
                    .unwrap_or(0);

                // safe 6-char slice of the target SHA
                let short_sha = gc.target.get(..6).unwrap_or(gc.target.as_str()).to_string();

                // Strip "Intent-Id: …" trailer from display message (show charter only).
                let message = gc
                    .message
                    .lines()
                    .take_while(|l| !l.starts_with("Intent-Id:"))
                    .collect::<Vec<_>>()
                    .join("\n")
                    .trim()
                    .to_string();

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
    let intent_log = intents_from_log(log).unwrap_or_default();
    let mut contributor_set: Vec<String> = Vec::new();
    for intent in intent_log.intents() {
        for entry in &intent.principal_chain {
            let name = principal_display_name(entry).to_string();
            if !contributor_set.contains(&name) {
                contributor_set.push(name);
            }
        }
    }

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
            contributors: contributor_set,      // REAL — from principal chains
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
