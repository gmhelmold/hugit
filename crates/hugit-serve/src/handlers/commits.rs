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

use hugit_cli::checks::CHECK_RECORDED_KIND;

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
                let avatar_class = classify_avatar(&author);
                let sha = safe_sha_prefix(&c.target);
                let age = humanize_age(recorded_at_ms);
                let checks_ok = checks_ok_shas.contains(c.target.as_str())
                    || checks_ok_shas.contains(sha.as_str());
                let cr = CommitRowVm {
                    message: c.message.clone(),
                    author,
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
                let sha = target.as_deref().map(safe_sha_prefix).unwrap_or_default();
                let author = extract_author(log, seq);
                let avatar_class = classify_avatar(&author);
                let age = humanize_age(recorded_at_ms);
                let checks_ok = if sha.is_empty() {
                    false
                } else {
                    checks_ok_shas.contains(sha.as_str())
                };
                let cr = CommitRowVm {
                    message: kind.clone(),
                    author,
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
    let days: Vec<CommitDayVm> = day_map
        .into_iter()
        .rev()
        .map(|((year, month, day), commits)| CommitDayVm {
            label: format_day_label(year, month, day),
            commits,
        })
        .collect();

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
            let prefix = safe_sha_prefix(target);
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

/// Classify an author string into an avatar_class.
/// - contains "opus" → "opus"
/// - contains "sonnet" → "sonnet"
/// - else → first token (split on ':' then '-' then ' ') or the full string
fn classify_avatar(author: &str) -> String {
    let lower = author.to_lowercase();
    if lower.contains("opus") {
        return "opus".to_string();
    }
    if lower.contains("sonnet") {
        return "sonnet".to_string();
    }
    // First "token": split on ':' first (agent:foo → "agent"), then on '-' and ' '.
    // For "agent:opus-4.8" → lower = "agent:opus-4.8" → contains "opus" already.
    // For a bare human handle like "ana" the first token is "ana".
    let token = author
        .split(':')
        .next()
        .unwrap_or(author)
        .split(['-', ' ', '/'])
        .next()
        .unwrap_or(author);
    if token.is_empty() {
        author.to_string()
    } else {
        token.to_string()
    }
}

/// Return a 6-char sha prefix from a full sha string.  If the string is shorter
/// than 6 chars, return the whole string (safe — never panics on short input).
fn safe_sha_prefix(sha: &str) -> String {
    sha.chars().take(6).collect()
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

/// Humanize a Unix-ms age as a pt-BR relative string.
/// Now is wall-clock UTC (system time).  Uses coarse buckets.
///
/// Note: the current time is read from `std::time::SystemTime` so tests that
/// pass `0` as a timestamp will render as a very old age — that is honest.
fn humanize_age(unix_ms: u64) -> String {
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);

    let diff_ms = now_ms.saturating_sub(unix_ms);
    let diff_secs = diff_ms / 1000;

    if diff_secs < 60 {
        return "há poucos segundos".to_string();
    }
    let diff_min = diff_secs / 60;
    if diff_min < 60 {
        return format!("há {diff_min} min");
    }
    let diff_h = diff_min / 60;
    if diff_h < 24 {
        return format!("há {diff_h} h");
    }
    let diff_d = diff_h / 24;
    if diff_d == 1 {
        return "há 1 dia".to_string();
    }
    if diff_d < 30 {
        return format!("há {diff_d} dias");
    }
    let diff_m = diff_d / 30;
    if diff_m == 1 {
        return "há 1 mês".to_string();
    }
    if diff_m < 12 {
        return format!("há {diff_m} meses");
    }
    let diff_y = diff_m / 12;
    if diff_y == 1 {
        return "há 1 ano".to_string();
    }
    format!("há {diff_y} anos")
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

    #[test]
    fn classify_avatar_opus() {
        assert_eq!(classify_avatar("agent:opus-4.8"), "opus");
    }

    #[test]
    fn classify_avatar_sonnet() {
        assert_eq!(classify_avatar("agent:sonnet-4.6"), "sonnet");
    }

    #[test]
    fn classify_avatar_human() {
        assert_eq!(classify_avatar("ana"), "ana");
    }

    #[test]
    fn safe_sha_prefix_short() {
        assert_eq!(safe_sha_prefix("abc"), "abc");
        assert_eq!(safe_sha_prefix("a31f9cff00"), "a31f9c");
        assert_eq!(safe_sha_prefix(""), "");
    }

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
    fn humanize_age_zero_is_old() {
        // ts=0 is 1970-01-01; should render as "há N anos" (very old).
        let age = humanize_age(0);
        assert!(age.starts_with("há"), "expected pt-BR age, got: {age}");
        assert!(age.contains("ano"), "expected years, got: {age}");
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
