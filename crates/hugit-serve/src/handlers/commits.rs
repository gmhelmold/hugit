//! `GET /v1/repos/{repo}/commits` → [`CommitsVm`]. FROZEN signature; body filled
//! by the fleet per master-plan §5 (REAL: commit rows via `project_machine`,
//! branches via `replay`; PRESENTATION: age/avatar_class; honest: `checks_ok`).

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::sync::Arc;
use std::time::Instant;

use gix_hash::ObjectId;
use hugit_http_contracts::{CommitDayVm, CommitRowVm, CommitsVm};
use hugit_refstore::{
    EventLog,
    intent::projection::{ProjectionRow, project_machine},
    replay,
};

use crate::budgeted_source::{BudgetedSource, WALK_BUDGET};
use crate::fmt::{COMMITS_CAP, classify_avatar, humanize_age, scrub, sha_prefix};
use crate::handlers::commit_meta::{CommitMeta, commit_meta_from_cas};
use hugit_cli::checks::CHECK_RECORDED_KIND;

/// Sha prefix length echoed into a `CommitRowVm` (structural, not scrubbed).
const SHA_PREFIX_LEN: usize = 6;

/// Build the commits view-model from a verified event log, UNIONed with the REAL
/// git DAG read straight from the CoreLink CAS (WP1/WP2).
///
/// The log is ALREADY verified — we do not re-load or re-verify from disk. `replay`
/// re-verifies the in-memory chain (cheap) then folds the ref state.
///
/// ## Why the CAS union (the whole point)
/// `git-ingest` (the bulk backfill) and `git push` write the git object closure to
/// CAS but NO event-log records, so a git-pushed-but-never-landed repo (the flagship
/// `githugr`) has an EMPTY log → `project_machine(log)` yields 0 rows → `/commits`
/// used to render EMPTY despite a full git history in CAS. We fix that by projecting
/// the machine altitude from the ACTUAL git commits: a bounded first-parent walk from
/// `head_commit` over `git_source`, decoded via [`commit_meta_from_cas`] (WP0).
///
/// ## Union rule (WP0), honesty-preserving
/// Rows are oid-keyed. A real `project_machine` intent/external row for an oid WINS
/// over a git-only row for the same oid (so its intent chip / checks survive); an oid
/// with NO log row becomes a git-only row (`intent_id: None`). Every field is REAL
/// git data (author / message subject / commit-time / sha) or honestly absent — we
/// write NOTHING to the log and synthesize NO landing / cost / check.
///
/// `git = None` (a repo with no content seam) → `git_source`/`head_commit` `None`,
/// `refs` empty → the ORIGINAL log-only path, unchanged (no regression).
pub fn build_commits(
    log: &EventLog,
    repo: &str,
    git_source: Option<&Arc<dyn hugit_proto::ObjectSource + Send + Sync>>,
    head_commit: Option<ObjectId>,
    refs: &BTreeMap<String, String>,
) -> CommitsVm {
    // ── branches: UNION the replay(log) refs with the LIVE git refs snapshot ──
    // A git-pushed-but-never-landed repo has NO `ref.update` on the log, so replay
    // yields an empty ref set; the branch selector must still show the REAL pushed
    // branches from the git `refs` map. `git = None` → `refs` empty → replay-only
    // (today's behavior). Branch names are attacker-controllable free text → scrubbed
    // at the read boundary (a branch named `feature/ghp_…` must redact).
    let ref_state = replay(log).unwrap_or_default();
    let mut head_ref_names: BTreeSet<String> = BTreeSet::new();
    for (name, _) in ref_state.iter() {
        if let Some(short) = name.strip_prefix("refs/heads/") {
            head_ref_names.insert(short.to_string());
        }
    }
    for name in refs.keys() {
        if let Some(short) = name.strip_prefix("refs/heads/") {
            head_ref_names.insert(short.to_string());
        }
    }

    let generated_branches: Vec<String> = head_ref_names
        .iter()
        .filter(|b| b.starts_with("intent/"))
        .map(|b| scrub(b))
        .collect();
    let plain_branches: Vec<String> = head_ref_names
        .iter()
        .filter(|b| !b.starts_with("intent/"))
        .cloned()
        .collect();
    // Pick HEAD: prefer "main", then "master", then first lexicographic plain.
    let branch = scrub(&pick_head(&plain_branches));
    let other_branches: Vec<String> = plain_branches
        .iter()
        .filter(|b| scrub(b.as_str()) != branch)
        .map(|b| scrub(b))
        .collect();

    // ── checks_ok index: which commit shas have a successful check.recorded ──
    // Honest default = false; a git-only commit with no `check.recorded` stays false.
    let checks_ok_shas = build_checks_ok_set(log);

    // ── the unioned, timestamp-keyed rows ─────────────────────────────────────
    // Each entry pairs the REAL timestamp (log `recorded_at` for a log row; the git
    // commit-time for a git-only row) with its VM. `covered` records oids a log row
    // already represents, so the git walk suppresses them (the union rule).
    let mut entries: Vec<(u64, CommitRowVm)> = Vec::new();
    let mut covered: HashSet<String> = HashSet::new();

    // project_machine folds the same in-memory records (no chain re-verify). On an
    // empty log this yields no rows (the git-pushed-never-landed case).
    let machine = project_machine(log).unwrap_or_default();
    for row in machine.rows() {
        let seq = row.seq();
        // Per the master-plan: a LOG row's timestamp is log.records()[seq].recorded_at.
        let recorded_at_ms = log
            .records()
            .get(seq as usize)
            .map(|r| r.recorded_at)
            .unwrap_or(0);
        let (commit_row, target) = machine_row_vm(log, row, recorded_at_ms, &checks_ok_shas);
        if !target.is_empty() {
            covered.insert(target.to_ascii_lowercase());
        }
        entries.push((recorded_at_ms, commit_row));
    }

    // ── git-only rows: the bounded CAS first-parent walk (WP2) ────────────────
    // Each `get` is a synchronous CAS/R2 fetch on the single-threaded engine, so the
    // walk is WALL-CLOCK bounded ([`WALK_BUDGET`]) AND count-bounded ([`COMMITS_CAP`])
    // — mirrors `blob_history` / `build_home`. A missing/garbled commit STOPS the walk
    // (fail-closed-honest: the rows collected so far, never an error, never a fake).
    if let (Some(src), Some(head)) = (git_source, head_commit) {
        let deadline = Instant::now() + WALK_BUDGET;
        let budgeted = BudgetedSource::new(src.as_ref(), deadline);
        let mut current = Some(head);
        let mut walked = 0usize;
        while let Some(oid) = current {
            if walked >= COMMITS_CAP || Instant::now() >= deadline {
                break;
            }
            walked += 1;
            let Some(meta) = commit_meta_from_cas(&budgeted, &oid) else {
                break;
            };
            let first_parent = meta.parents.first().copied();
            let full = oid.to_hex().to_string();
            // Union rule: a git-only row ONLY where no log row already owns the oid.
            if !covered.contains(&full.to_ascii_lowercase()) {
                entries.push((
                    meta.commit_time_ms,
                    git_row_vm(&meta, &full, &checks_ok_shas),
                ));
            }
            current = first_parent;
        }
    }

    // Newest-first by the REAL timestamp (both axes read newest-at-top). Stable so a
    // log row and a git row sharing a timestamp keep their insertion order.
    entries.sort_by_key(|e| std::cmp::Reverse(e.0));

    // Group consecutive same-day rows: sorted-descending means a calendar day is a
    // contiguous run, so a single linear pass yields the day groups newest-first.
    let mut days: Vec<CommitDayVm> = Vec::new();
    let mut cur_key: Option<(i32, u32, u32)> = None;
    for (ts, row) in entries {
        let key = day_key_from_ms(ts);
        if cur_key != Some(key) {
            days.push(CommitDayVm {
                label: format_day_label(key.0, key.1, key.2),
                commits: Vec::new(),
            });
            cur_key = Some(key);
        }
        days.last_mut()
            .expect("a day was just pushed")
            .commits
            .push(row);
    }

    // P1: cap the projected commits at COMMITS_CAP total (most-recent-first — the VM
    // IS the page); empty trailing days are pruned.
    cap_commits(&mut days, COMMITS_CAP);

    CommitsVm {
        repo: repo.to_string(),
        branch,
        other_branches,
        generated_branches,
        days,
    }
}

/// Project ONE machine-altitude (`project_machine`) row to its `CommitRowVm` plus the
/// full target oid it represents (for the union-rule `covered` set; empty when an
/// external change named no target). REAL log data, scrubbed at the read boundary.
fn machine_row_vm(
    log: &EventLog,
    row: &ProjectionRow,
    recorded_at_ms: u64,
    checks_ok_shas: &HashSet<String>,
) -> (CommitRowVm, String) {
    let seq = row.seq();
    match row {
        ProjectionRow::Intent(c) => {
            // Author: first principal-chain entry matching "agent:"/"orchestrator:".
            let author = extract_author(log, seq);
            // avatar_class is a structural classification derived from the RAW author,
            // then the displayed author is scrubbed below.
            let avatar_class = classify_avatar(&author);
            let sha = sha_prefix(&c.target, SHA_PREFIX_LEN);
            let age = humanize_age(recorded_at_ms);
            let checks_ok =
                checks_ok_shas.contains(c.target.as_str()) || checks_ok_shas.contains(sha.as_str());
            let cr = CommitRowVm {
                // P0: message + author are free text echoed from the log → scrub.
                // sha/intent_id/age are structural (content-addresses) → NOT scrubbed.
                message: scrub(&c.message),
                author: scrub(&author),
                avatar_class,
                age,
                intent_id: Some(c.intent_id.clone()),
                sha,
                checks_ok,
            };
            (cr, c.target.clone())
        }
        ProjectionRow::ExternalChange { kind, target, .. } => {
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
                // P0: `kind` is the displayed free-text message; `author` is free text.
                message: scrub(kind),
                author: scrub(&author),
                avatar_class,
                age,
                intent_id: None,
                sha,
                checks_ok,
            };
            (cr, target.clone().unwrap_or_default())
        }
    }
}

/// Project a git-only commit (decoded from CAS, WP0) to its `CommitRowVm`. Every
/// field is REAL git data (author / message subject / commit-time / sha) or honestly
/// absent — `intent_id: None` because a git-pushed commit carries NO landing
/// provenance (never fabricate one). Author + subject are free text → scrubbed.
fn git_row_vm(meta: &CommitMeta, full_oid: &str, checks_ok_shas: &HashSet<String>) -> CommitRowVm {
    // The row shows the message SUBJECT (first line), like `git log --oneline`.
    let subject = meta.message.lines().next().unwrap_or("");
    let sha = sha_prefix(full_oid, SHA_PREFIX_LEN);
    let checks_ok = checks_ok_shas.contains(full_oid) || checks_ok_shas.contains(sha.as_str());
    CommitRowVm {
        message: scrub(subject),
        author: scrub(&meta.author),
        avatar_class: classify_avatar(&meta.author),
        // The REAL commit time — NEVER the log `recorded_at` (there is none).
        age: humanize_age(meta.commit_time_ms),
        intent_id: None,
        sha,
        checks_ok,
    }
}

// ── Private helpers ──────────────────────────────────────────────────────────

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

    use hugit_proto::{CasObjectSource, ObjectId, ObjectKind};

    // ── git-object fixture builders (mirror commit_detail.rs / history.rs) ────
    fn blob(src: &mut CasObjectSource, body: &str) -> ObjectId {
        src.insert_raw(ObjectKind::Blob, body.as_bytes().to_vec())
    }
    fn tree(src: &mut CasObjectSource, mut entries: Vec<(&str, &str, ObjectId)>) -> ObjectId {
        entries.sort_by(|a, b| a.1.as_bytes().cmp(b.1.as_bytes()));
        let mut out = Vec::new();
        for (mode, name, oid) in &entries {
            out.extend_from_slice(mode.as_bytes());
            out.push(b' ');
            out.extend_from_slice(name.as_bytes());
            out.push(0);
            out.extend_from_slice(oid.as_bytes());
        }
        src.insert_raw(ObjectKind::Tree, out)
    }
    fn commit(
        src: &mut CasObjectSource,
        tree: ObjectId,
        parent: Option<ObjectId>,
        author: &str,
        time_secs: i64,
        message: &str,
    ) -> ObjectId {
        let mut body = format!("tree {tree}\n");
        if let Some(p) = parent {
            body.push_str(&format!("parent {p}\n"));
        }
        body.push_str(&format!(
            "author {author} {time_secs} +0000\ncommitter {author} {time_secs} +0000\n\n{message}\n"
        ));
        src.insert_raw(ObjectKind::Commit, body.into_bytes())
    }

    fn no_git() -> Option<&'static Arc<dyn hugit_proto::ObjectSource + Send + Sync>> {
        None
    }
    fn no_refs() -> BTreeMap<String, String> {
        BTreeMap::new()
    }

    /// Collect every row across all day groups (order preserved) for assertions.
    fn all_rows(vm: &CommitsVm) -> Vec<&CommitRowVm> {
        vm.days.iter().flat_map(|d| d.commits.iter()).collect()
    }

    #[test]
    fn empty_log_yields_empty_vm() {
        let log = EventLog::new();
        let vm = build_commits(&log, "hugit", no_git(), None, &no_refs());
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

    // ── Branch name scrubbing ────────────────────────────────────────────────

    #[test]
    fn secret_shaped_branch_name_is_scrubbed_in_commits_vm() {
        // Push a ref.update record for a branch whose name is secret-shaped.
        let mut log = EventLog::new();
        log.append_for_test(
            "ref.update",
            vec!["o".into()],
            serde_json::json!({
                "ref": "refs/heads/feature/ghp_16C7e42F292c6912E7710c838347Ae178B4a",
                "target": "aabbccddeeff00112233445566778899aabbccdd"
            })
            .to_string(),
            1000,
        );

        let vm = build_commits(&log, "hugit", no_git(), None, &no_refs());

        // The branch field must not carry the raw secret-shaped name.
        assert_ne!(
            vm.branch, "feature/ghp_16C7e42F292c6912E7710c838347Ae178B4a",
            "secret-shaped branch name must be scrubbed in CommitsVm.branch"
        );
        assert!(
            !vm.branch.contains("ghp_"),
            "raw secret token must not appear in CommitsVm.branch: {}",
            vm.branch
        );
        // other_branches and generated_branches are also scrubbed.
        for b in vm.other_branches.iter().chain(vm.generated_branches.iter()) {
            assert!(
                !b.contains("ghp_"),
                "secret-shaped token must not appear in other_branches/generated_branches: {b}"
            );
        }
    }

    // ── WP2/WP5: the CAS commit-walk projection (git-pushed-but-never-landed) ──

    /// Build a 3-commit first-parent chain in CAS and return `(head, [c1,c2,c3])`.
    /// c1 (root) → c2 → c3 (head), each on a distinct real author/message/time.
    fn three_commit_chain(src: &mut CasObjectSource) -> (ObjectId, [ObjectId; 3]) {
        let b1 = blob(src, "a\n");
        let t1 = tree(src, vec![("100644", "f.txt", b1)]);
        let c1 = commit(src, t1, None, "Ana <a@x>", 1_700_000_000, "add f");

        let b2 = blob(src, "a\nb\n");
        let t2 = tree(src, vec![("100644", "f.txt", b2)]);
        let c2 = commit(src, t2, Some(c1), "Bru <b@x>", 1_700_100_000, "edit f");

        let b3 = blob(src, "a\nb\nc\n");
        let t3 = tree(src, vec![("100644", "f.txt", b3)]);
        let c3 = commit(src, t3, Some(c2), "Cai <c@x>", 1_700_200_000, "extend f");
        (c3, [c1, c2, c3])
    }

    /// A git-pushed-but-never-landed repo (EMPTY event log) renders 3 REAL commit
    /// rows from CAS — real author/message/sha, newest-first, `intent_id: None`,
    /// and the branch selector reflects the real pushed ref. The event log is never
    /// written to.
    #[test]
    fn git_only_chain_yields_real_rows_on_empty_log() {
        let mut src = CasObjectSource::new();
        let (head, [c1, _c2, c3]) = three_commit_chain(&mut src);
        let src: Arc<dyn hugit_proto::ObjectSource + Send + Sync> = Arc::new(src);
        let mut refs = BTreeMap::new();
        refs.insert("refs/heads/main".to_string(), head.to_string());

        let vm = build_commits(&EventLog::new(), "githugr", Some(&src), Some(head), &refs);

        // Branch selector: the REAL pushed branch (from refs, since the log is empty).
        assert_eq!(vm.branch, "main");

        // Exactly 3 REAL rows, newest-first (head c3 first, root c1 last).
        let rows = all_rows(&vm);
        assert_eq!(rows.len(), 3, "3 git commits → 3 rows: {rows:?}");
        assert_eq!(rows[0].sha, sha_prefix(&c3.to_string(), 6));
        assert_eq!(rows[0].message, "extend f");
        assert_eq!(rows[0].author, "Cai <c@x>");
        assert_eq!(rows[2].sha, sha_prefix(&c1.to_string(), 6));
        assert_eq!(rows[2].message, "add f");
        // git-only rows carry NO landing provenance + honest-default checks.
        for r in &rows {
            assert!(r.intent_id.is_none(), "a git-only row has no intent chip");
            assert!(!r.checks_ok, "no check.recorded → honest-false");
            assert!(!r.age.is_empty(), "real commit-time humanizes to an age");
        }
    }

    /// UNION rule (WP0): a repo with ONE real `intent.landed` (whose target IS a git
    /// commit oid) plus the git chain — the intent row WINS for its oid (keeps its
    /// `intent_id` chip), and the remaining git commits appear as git-only rows. No
    /// oid is duplicated.
    #[test]
    fn union_intent_row_wins_over_git_row_for_same_oid() {
        let mut src = CasObjectSource::new();
        let (head, [c1, _c2, _c3]) = three_commit_chain(&mut src);
        let src: Arc<dyn hugit_proto::ObjectSource + Send + Sync> = Arc::new(src);
        let mut refs = BTreeMap::new();
        refs.insert("refs/heads/main".to_string(), head.to_string());

        // A landed intent whose target is the ROOT commit c1 (a real oid on the chain).
        let mut log = EventLog::new();
        log.append_for_test(
            "intent.landed",
            vec!["agent:opus".to_string()],
            serde_json::json!({
                "intent_id": "a31",
                "ref": "refs/heads/main",
                "target": c1.to_string(),
                "charter": "land the base"
            })
            .to_string(),
            1_700_050_000_000, // recorded_at (ms)
        );

        let vm = build_commits(&log, "githugr", Some(&src), Some(head), &refs);

        let rows = all_rows(&vm);
        // 3 distinct oids total (c1 via the intent row; c2/c3 as git-only) — no dupes.
        assert_eq!(rows.len(), 3, "one row per oid, no duplication: {rows:?}");

        // Exactly ONE row carries the intent chip, and it is c1's.
        let intent_rows: Vec<&&CommitRowVm> =
            rows.iter().filter(|r| r.intent_id.is_some()).collect();
        assert_eq!(intent_rows.len(), 1, "the log row wins for its oid");
        assert_eq!(intent_rows[0].intent_id.as_deref(), Some("a31"));
        assert_eq!(intent_rows[0].sha, sha_prefix(&c1.to_string(), 6));
        // c1 does NOT also appear as a git-only (intent_id None) row.
        let c1_prefix = sha_prefix(&c1.to_string(), 6);
        let c1_git_only = rows
            .iter()
            .any(|r| r.sha == c1_prefix && r.intent_id.is_none());
        assert!(!c1_git_only, "c1 must not appear twice (git-only + intent)");
    }

    /// `git = None` (no content seam) is the ORIGINAL log-only path, unchanged: a log
    /// with one raw push still projects exactly one external-change row and no CAS
    /// walk runs.
    #[test]
    fn git_none_is_log_only_unchanged() {
        let mut log = EventLog::new();
        log.append_for_test(
            "ref.update",
            vec!["o".into()],
            serde_json::json!({
                "ref": "refs/heads/main",
                "target": "aabbccddeeff00112233445566778899aabbccdd"
            })
            .to_string(),
            1_700_000_000_000,
        );
        let vm = build_commits(&log, "hugit", no_git(), None, &no_refs());
        let rows = all_rows(&vm);
        assert_eq!(rows.len(), 1, "one raw push → one external-change row");
        assert!(rows[0].intent_id.is_none());
        assert_eq!(vm.branch, "main", "branch from replay(log), no git seam");
    }
}
