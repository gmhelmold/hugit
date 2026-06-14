//! `GET /v1/repos/{repo}/commits` → [`CommitsVm`]. FROZEN signature; body filled
//! by the fleet per master-plan §5 (REAL: commit rows via `project_machine`,
//! branches via `replay`; PRESENTATION: age/avatar_class; honest: `checks_ok`).

use std::collections::BTreeMap;

use hugit_http_contracts::{CommitDayVm, CommitRowVm, CommitsVm};
use hugit_refstore::{
    EventLog,
    intent::projection::{ProjectionRow, project_machine},
    replay,
};

use crate::fmt::{COMMITS_CAP, classify_avatar, humanize_age, scrub, sha_prefix};
use hugit_cli::checks::CHECK_RECORDED_KIND;

/// Sha prefix length echoed into a `CommitRowVm` (structural, not scrubbed).
const SHA_PREFIX_LEN: usize = 6;

/// Build the commits view-model from a verified event log.
///
/// The log is ALREADY verified — we do not re-load or re-verify from disk.
/// `replay` re-verifies the in-memory chain (cheap, always consistent with
/// the already-verified state), then folds the ref state.
pub fn build_commits(log: &EventLog, repo: &str) -> CommitsVm {
    // ── branches via replay ──────────────────────────────────────────────────
    // replay() re-verifies the in-memory chain then folds the ref state.
    // On an empty log this yields an empty RefState — both branches lists are [].
    let ref_state = replay(log).unwrap_or_default();

    // HEAD branch: find refs/heads/* entries; for now pick the first plain
    // (non-intent) branch as HEAD, preferring "main" or "master" if present,
    // otherwise the lexicographically first plain branch.
    let all_head_refs: Vec<String> = ref_state
        .iter()
        .filter(|(name, _)| name.starts_with("refs/heads/"))
        .map(|(name, _)| strip_heads_prefix(name).to_string())
        .collect();

    // Partition into plain branches and generated (intent/*) branches.
    let generated_branches: Vec<String> = all_head_refs
        .iter()
        .filter(|b| b.starts_with("intent/"))
        .cloned()
        .collect();

    let plain_branches: Vec<String> = all_head_refs
        .iter()
        .filter(|b| !b.starts_with("intent/"))
        .cloned()
        .collect();

    // Pick HEAD: prefer "main", then "master", then first lexicographic plain.
    let branch = pick_head(&plain_branches);

    // other_branches = plain branches minus current HEAD.
    let other_branches: Vec<String> = plain_branches
        .iter()
        .filter(|b| b.as_str() != branch)
        .cloned()
        .collect();

    // ── checks_ok index: which commit shas have a successful check.recorded ──
    // A commit's checks_ok is true iff at least one check.recorded event on the
    // log for that commit's sha (matched via the payload's "target" field or by
    // the sha prefix) has exit == 0.  Per the field map: honest default = false.
    let checks_ok_shas = build_checks_ok_set(log);

    // ── machine projection ───────────────────────────────────────────────────
    // project_machine folds the same in-memory records; ignores the chain
    // (projection does not re-verify). On an empty log this yields no rows.
    let machine = project_machine(log).unwrap_or_default();

    // ── group rows by calendar day (UTC), most-recent-first ─────────────────
    // Key = (year, month, day) as a sortable tuple; value = vec of CommitRowVm.
    let mut day_map: BTreeMap<(i32, u32, u32), Vec<CommitRowVm>> = BTreeMap::new();

    for row in machine.rows() {
        let seq = row.seq();
        // Per the master-plan: timestamp is log.records()[seq].recorded_at (unix ms).
        let recorded_at_ms = log
            .records()
            .get(seq as usize)
            .map(|r| r.recorded_at)
            .unwrap_or(0);

        let (commit_row, day_key) = match row {
            ProjectionRow::Intent(c) => {
                // Author: first element of the principal_chain of the originating record
                // that matches "agent:" or "orchestrator:" prefix; else "".
                let author = extract_author(log, seq);
                // avatar_class is a structural classification of the author, not
                // an echo of free text — derive it from the RAW author so the
                // class is stable, then scrub the displayed author below.
                let avatar_class = classify_avatar(&author);
                let sha = sha_prefix(&c.target, SHA_PREFIX_LEN);
                let age = humanize_age(recorded_at_ms);
                let checks_ok = checks_ok_shas.contains(c.target.as_str())
                    || checks_ok_shas.contains(sha.as_str());
                let cr = CommitRowVm {
                    // P0: message + author are free text echoed from the log →
                    // MUST pass through the redaction seam. sha/intent_id/age are
                    // structural (content-addresses), so they are NOT scrubbed.
                    message: scrub(&c.message),
                    author: scrub(&author),
                    avatar_class,
                    age,
                    intent_id: Some(c.intent_id.clone()),
                    sha,
                    checks_ok,
                };
                (cr, day_key_from_ms(recorded_at_ms))
            }
            ProjectionRow::ExternalChange { kind, target, .. } => {
                // ExternalChange: no intent, no author from principal, kind as fallback.
                let sha = target
                    .as_deref()
                    .map(|t| sha_prefix(t, SHA_PREFIX_LEN))
                    .unwrap_or_default();
                let author = extract_author(log, seq);
                let avatar_class = classify_avatar(&author);
                let age = humanize_age(recorded_at_ms);
                let checks_ok = if sha.is_empty() {
                    false
                } else {
                    checks_ok_shas.contains(sha.as_str())
                };
                let cr = CommitRowVm {
                    // P0: `kind` is the displayed free-text message for an external
                    // change and `author` is free text — both pass through scrub.
                    message: scrub(kind),
                    author: scrub(&author),
                    avatar_class,
                    age,
                    intent_id: None,
                    sha,
                    checks_ok,
                };
                (cr, day_key_from_ms(recorded_at_ms))
            }
        };

        day_map.entry(day_key).or_default().push(commit_row);
    }

    // Sort days most-recent-first (BTreeMap is ascending; we collect in reverse).
    // P3: within each day, rows were pushed in ascending log order, so reverse to
    // make the within-day order most-recent-first too — consistent with the
    // most-recent-first day order (the VM reads newest-at-top, both axes).
    let mut days: Vec<CommitDayVm> = day_map
        .into_iter()
        .rev()
        .map(|((year, month, day), mut commits)| {
            commits.reverse();
            CommitDayVm {
                label: format_day_label(year, month, day),
                commits,
            }
        })
        .collect();

    // P1: cap the projected commits at COMMITS_CAP, most-recent-first — the VM IS
    // the page. The cap is applied to the TOTAL row count across days (after
    // grouping + sorting), keeping the 100 most recent and dropping older days /
    // the older tail of the boundary day; empty trailing days are pruned.
    cap_commits(&mut days, COMMITS_CAP);

    CommitsVm {
        repo: repo.to_string(),
        branch,
        other_branches,
        generated_branches,
        days,
    }
}

// ── Private helpers ──────────────────────────────────────────────────────────

/// Strip the `refs/heads/` prefix from a fully-qualified ref name.
fn strip_heads_prefix(name: &str) -> &str {
    name.strip_prefix("refs/heads/").unwrap_or(name)
}

/// Pick the HEAD branch from a list of plain (non-intent) branches.
/// Preference: "main" > "master" > first lexicographic.
fn pick_head(plain: &[String]) -> String {
    if plain.iter().any(|b| b == "main") {
        return "main".to_string();
    }
    if plain.iter().any(|b| b == "master") {
        return "master".to_string();
    }
    plain.first().cloned().unwrap_or_default()
}

/// Build the set of sha prefixes (6 chars) for commits that have at least one
/// successful `check.recorded` event.  Honest default: absent = false.
fn build_checks_ok_set(log: &EventLog) -> std::collections::HashSet<String> {
    let mut set = std::collections::HashSet::new();
    for record in log.records() {
        if record.kind != CHECK_RECORDED_KIND {
            continue;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&record.payload) else {
            continue;
        };
        // exit == 0 means the check passed.
        let exit = v.get("exit").and_then(serde_json::Value::as_i64);
        if exit != Some(0) {
            continue;
        }
        // Index by the "target" sha (full or prefix) when present.
        if let Some(target) = v.get("target").and_then(serde_json::Value::as_str) {
            // Store both the full sha and the 6-char prefix so matching works
            // regardless of which form the commit row carries.
            let prefix = crate::fmt::sha_prefix(target, 6);
            if !prefix.is_empty() {
                set.insert(prefix);
            }
            if target.len() > 6 {
                set.insert(target.to_string());
            }
        }
    }
    set
}

/// Extract the "author" from the originating record's principal_chain.
/// Looks for the first entry that contains "agent:" or "orchestrator:"; if none
/// found, returns the first entry if any, else "".
fn extract_author(log: &EventLog, seq: u64) -> String {
    let Some(record) = log.records().get(seq as usize) else {
        return String::new();
    };
    let chain = &record.principal_chain;
    // Prefer an explicit agent:/orchestrator: entry (the intent principal).
    for entry in chain {
        if entry.contains("agent:") || entry.contains("orchestrator:") {
            return entry.clone();
        }
    }
    // Fall back to the first entry in the chain (human handle or service).
    chain.first().cloned().unwrap_or_default()
}

/// Cap the total commit rows across all days at `cap`, keeping the most-recent
/// `cap` rows. `days` is already most-recent-first (both across days and within
/// a day), so we walk from the front and stop once `cap` rows are kept; the
/// boundary day is truncated and any fully-dropped trailing days are removed.
fn cap_commits(days: &mut Vec<CommitDayVm>, cap: usize) {
    let mut remaining = cap;
    let mut keep_days = 0usize;
    for day in days.iter_mut() {
        if remaining == 0 {
            break;
        }
        if day.commits.len() > remaining {
            day.commits.truncate(remaining);
        }
        remaining -= day.commits.len();
        keep_days += 1;
    }
    days.truncate(keep_days);
}

/// Convert a Unix epoch ms timestamp to a (year, month, day) UTC triple.
/// Uses a purely arithmetic UTC date conversion (no external crate required).
fn day_key_from_ms(unix_ms: u64) -> (i32, u32, u32) {
    // Convert ms → seconds, then compute UTC date via civil-calendar arithmetic.
    let secs = (unix_ms / 1000) as i64;
    // Days since Unix epoch (1970-01-01).
    let days = secs.div_euclid(86400) as i32;
    civil_date(days)
}

/// Convert days-since-Unix-epoch (1970-01-01) to (year, month, day) UTC.
/// Uses the Proleptic Gregorian civil calendar algorithm (Luca Vigano, public domain).
fn civil_date(z: i32) -> (i32, u32, u32) {
    let z = z + 719468;
    let era: i32 = z.div_euclid(146097);
    let doe = z.rem_euclid(146097) as u32; // day of era [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365; // [0, 399]
    let y = yoe as i32 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    let y = y + if m <= 2 { 1 } else { 0 };
    (y, m, d)
}

/// Format a day label in pt-BR style: "Commits em <d> de <mês> de <ano>".
/// Month names are abbreviated (pt-BR convention).
fn format_day_label(year: i32, month: u32, day: u32) -> String {
    let month_name = pt_br_month(month);
    format!("Commits em {day} de {month_name} de {year}")
}

/// Abbreviated pt-BR month name (lowercase, as used in the canonical fixture).
fn pt_br_month(month: u32) -> &'static str {
    match month {
        1 => "jan",
        2 => "fev",
        3 => "mar",
        4 => "abr",
        5 => "mai",
        6 => "jun",
        7 => "jul",
        8 => "ago",
        9 => "set",
        10 => "out",
        11 => "nov",
        12 => "dez",
        _ => "???",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_log_yields_empty_vm() {
        let log = EventLog::new();
        let vm = build_commits(&log, "hugit");
        assert_eq!(vm.repo, "hugit");
        assert_eq!(vm.branch, "");
        assert!(vm.other_branches.is_empty());
        assert!(vm.generated_branches.is_empty());
        assert!(vm.days.is_empty());
    }

    // NOTE: classify_avatar / sha_prefix / humanize_age now live in `crate::fmt`
    // (the shared DRY seam) and are unit-tested there; the local forks were
    // deleted, so their tests moved with them.

    #[test]
    fn civil_date_unix_epoch() {
        // 1970-01-01 = day 0
        assert_eq!(civil_date(0), (1970, 1, 1));
    }

    #[test]
    fn civil_date_known() {
        // 2026-06-09: days since epoch
        // Quick known value: 2026-01-01 = 20454 days from epoch; +159 days = 20613
        // Let's compute: 2026-06-09 unix: 1749427200 / 86400 = 20249 days
        // Actually let's just verify round-trip for a specific known date.
        // 2024-01-01 = 19723 days since epoch.
        assert_eq!(civil_date(19723), (2024, 1, 1));
    }

    #[test]
    fn format_day_label_canonical() {
        // The canonical fixture uses "Commits em 9 de jun de 2026".
        assert_eq!(format_day_label(2026, 6, 9), "Commits em 9 de jun de 2026");
    }

    #[test]
    fn pick_head_prefers_main() {
        let branches = vec![
            "feat/x".to_string(),
            "master".to_string(),
            "main".to_string(),
        ];
        assert_eq!(pick_head(&branches), "main");
    }

    #[test]
    fn pick_head_empty_is_empty_string() {
        assert_eq!(pick_head(&[]), "");
    }
}
