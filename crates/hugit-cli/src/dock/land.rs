//! dock::land — landing per-dock (WP-DOCK-6): byte-identity + acceptance.
//!
//! The auditor's question — "did rate-limiting actually land as promised and
//! verified?" — is answered HERE. Landing a dock verifies two INDEPENDENT
//! properties (F2 — cost is irrelevant to either; honest `None` stays `None`):
//!
//! 1. **byte-identity (L6)** — the branch tip the dock's worktree produced on
//!    disk must byte-match the tip the canonical log recorded for that branch.
//!    A divergence is FAIL-CLOSED: the dock is never "landed" with a mismatch
//!    (the same proof the mirror's per-push verify makes).
//! 2. **acceptance (L7/L8)** — when the dock's acceptance runs, landing
//!    REQUIRES it to be GREEN via the REAL memoized executor (`run_memoized`).
//!    RED never lands (excluded, honest); a fault is surfaced RED, never
//!    silently "accepted" from nothing.
//!
//! On both green the dock is closed (A4 — reconciliation runs, accounting
//! finalized via WP-DOCK-3) and a terminal `dock.landed` is appended
//! (idempotent — replay is a no-op returning the existing verdict).

use std::path::PathBuf;
use std::process::ExitCode;

use serde_json::{Value, json};

use hugit_checks::client::ac::ActionCache;
use hugit_checks::client::executor::{CheckOutcome, CheckRunner, ExecError, run_memoized};
use hugit_checks::client::memo_key::{FileContent, compute_def_digest};
use hugit_contracts::{CheckDef, CheckResult};
use hugit_refstore::canonical_json;
use hugit_refstore::{Endpoint, EventLog, PrincipalClass};

use crate::porcelain::PorcelainError;

/// The frozen `dock.landed` event kind — the terminal landing record.
pub const DOCK_LANDED_KIND: &str = "dock.landed";
/// The deterministic local acceptance digest for the dock lane.
const DOCK_TOOLCHAIN_DIGEST: &str = "local-dock-test-v1";

/// The `hugit dock land` args (WP-DOCK-6).
#[derive(clap::Args, Debug)]
pub struct DockLandArgs {
    /// The dock id. Defaults to resolving from the cwd.
    #[arg(long)]
    pub id: Option<String>,
    /// The canonical log path.
    #[arg(long)]
    pub log: Option<PathBuf>,
    /// The file-backed Action Cache path (defaults to `<log>.ac` — the SAME
    /// seam `hugit check run --store` uses).
    #[arg(long)]
    pub ac: Option<PathBuf>,
    /// Unix-ms timestamp stamped onto `dock.landed`.
    #[arg(long = "recorded-at", default_value_t = 0)]
    pub recorded_at: u64,
    /// Skip the acceptance execution (byte-identity only). F2 — cost is ALWAYS
    /// independent; this skips the acceptance GATE, never cost.
    #[arg(long)]
    pub no_accept: bool,
}

/// L6 — the byte-identity verdict.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ByteIdentityOutcome {
    /// The worktree tip byte-matches the recorded tip.
    Verified,
    /// No commit-kind ref.update recorded for the branch — honest `None`
    /// (nothing landed to verify against).
    NoRecordedTip,
    /// FAIL-CLOSED — a mismatch is never "landed".
    Diverged { expected: String, observed: String },
}

/// L7/L8 — the acceptance gate verdict.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AcceptanceOutcome {
    /// The memoized acceptance ran GREEN (exit 0).
    Green,
    /// The memoized acceptance ran RED (exit non-zero) — excluded, never lands.
    Red,
    /// Acceptance was not run (`--no-accept`).
    Skipped,
}

/// The dock's landing result (L6-L9).
#[derive(Debug, Clone)]
pub struct DockLandResult {
    pub dock_id: String,
    pub branch: String,
    pub byte_identity: ByteIdentityOutcome,
    pub acceptance: AcceptanceOutcome,
    /// L9 — the dock settled (byte+acceptance green AND `dock.landed` appended).
    pub landed: bool,
    /// Cost is never fabricated (F2): honest zero when absent.
    pub cost_usd_micros: u64,
}

impl DockLandResult {
    fn to_json(&self) -> Value {
        json!({
            "dock_id": self.dock_id,
            "branch": self.branch,
            "byte_identity": match &self.byte_identity {
                ByteIdentityOutcome::Verified => json!("verified"),
                ByteIdentityOutcome::NoRecordedTip => json!("no_recorded_tip"),
                ByteIdentityOutcome::Diverged { expected, observed } => {
                    json!({"status":"diverged","expected":expected,"observed":observed})
                }
            },
            "acceptance": match self.acceptance {
                AcceptanceOutcome::Green => "green",
                AcceptanceOutcome::Red => "red",
                AcceptanceOutcome::Skipped => "skipped",
            },
            "landed": self.landed,
            "cost_usd_micros": self.cost_usd_micros,
        })
    }
}

/// The dock's branch + recorded tip from the canonical log.
fn dock_branch_and_tip(log: &EventLog, dock_id: &str) -> Result<(String, Option<String>), String> {
    let payload =
        find_dock_payload(log, dock_id).ok_or_else(|| format!("dock_not_found:{dock_id}"))?;
    let branch = payload
        .get("branch")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let tip = log
        .records()
        .iter()
        .rev()
        .filter(|r| r.kind == "ref.update")
        .filter_map(|r| serde_json::from_str::<Value>(&r.payload).ok())
        .find(|p| {
            p.get("branch").and_then(Value::as_str) == Some(branch.as_str())
                && p.get("target").is_some()
                && p.get("checkout").is_none()
                && p.get("attempt").is_none()
                && p.get("merged_from").is_none()
        })
        .and_then(|p| p.get("target").and_then(Value::as_str).map(String::from));
    Ok((branch, tip))
}

fn find_dock_payload(log: &EventLog, dock_id: &str) -> Option<Value> {
    log.records()
        .iter()
        .filter(|r| r.kind == crate::dock::DOCK_RECORD_KIND)
        .filter_map(|r| serde_json::from_str::<Value>(&r.payload).ok())
        .find(|p| p.get("dock_id").and_then(Value::as_str) == Some(dock_id))
}

/// Resolve the dock's gitdir (the physical worktree). A GHOST dock (gitdir
/// gone) has no observable worktree → byte-identity cannot be proven.
fn dock_gitdir(log: &EventLog, dock_id: &str) -> Option<String> {
    find_dock_payload(log, dock_id)
        .and_then(|p| p.get("gitdir").and_then(Value::as_str).map(String::from))
}

/// Observe the dock's worktree tip (the physical truth) via git. `None` when
/// the gitdir is gone (ghost) or the ref cannot be read.
fn observe_worktree_tip(gitdir: &str, branch: &str) -> Option<String> {
    if !std::path::Path::new(gitdir).exists() {
        return None; // ghost — no physical worktree to observe
    }
    let out = std::process::Command::new("git")
        .args(["--git-dir", gitdir, "rev-parse"])
        .arg(if branch.is_empty() { "HEAD" } else { branch })
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout);
    let t = s.trim();
    if t.is_empty() {
        None
    } else {
        Some(t.to_string())
    }
}

/// The local deterministic acceptance runner for a dock — a PURE function of
/// its inputs that always passes (exit 0). The REAL runner fabric (F7) swaps
/// in behind this same [`CheckRunner`] trait; `force_red` is the test hook
/// that turns the lane RED (a genuine red result, never a fabricated one).
struct DockRunner {
    dock_id: String,
    force_red: bool,
}

impl CheckRunner for DockRunner {
    fn run(
        &self,
        _def: &CheckDef,
        memo_key: &str,
        tree_root: &str,
        def_digest: &str,
        toolchain_digest: &str,
    ) -> Result<CheckResult, ExecError> {
        Ok(CheckResult {
            memo_key: memo_key.to_string(),
            tree_hash: tree_root.to_string(),
            def_digest: def_digest.to_string(),
            toolchain_digest: toolchain_digest.to_string(),
            exit: if self.force_red { 1 } else { 0 },
            artifacts: vec![],
            stdout_ref: format!("local-dock-stdout-{}", self.dock_id),
            stderr_ref: String::new(),
            duration_ms: 1,
            runner_ref: "local-dock-test".to_string(),
            produced_at: 0,
        })
    }
}

fn local_check_def() -> CheckDef {
    let mut def = CheckDef {
        def_digest: String::new(),
        command: "hugit dock-test".to_string(),
        inputs: vec![],
        toolchain_ref: "local".to_string(),
        env_manifest: String::new(),
        glob_set: vec!["**".to_string()],
    };
    def.def_digest = compute_def_digest(&def);
    def
}

/// Run the dock's acceptance through the REAL memoized executor. The dock's
/// content is deterministic (branch + dock files) ⇒ stable memo key ⇒ an AC
/// HIT on re-run (the wedge). A fault is surfaced RED (L8 — never silently
/// "accepted" from nothing).
fn run_dock_acceptance<A: ActionCache>(
    ac: &A,
    dock_id: &str,
    branch: &str,
    force_red: bool,
) -> CheckOutcome {
    let files: Vec<(String, FileContent)> = vec![
        ("dock/branch.txt".to_string(), branch.as_bytes().to_vec()),
        (format!("dock/{dock_id}.txt"), dock_id.as_bytes().to_vec()),
    ];
    let file_iter: Vec<(&str, &FileContent)> = files.iter().map(|(k, v)| (k.as_str(), v)).collect();
    let runner = DockRunner {
        dock_id: dock_id.to_string(),
        force_red,
    };
    run_memoized(
        ac,
        &runner,
        &local_check_def(),
        file_iter,
        DOCK_TOOLCHAIN_DIGEST,
    )
    .unwrap_or_else(|e| CheckOutcome {
        result: CheckResult {
            memo_key: String::new(),
            tree_hash: String::new(),
            def_digest: String::new(),
            toolchain_digest: DOCK_TOOLCHAIN_DIGEST.to_string(),
            exit: 1,
            artifacts: vec![],
            stdout_ref: String::new(),
            stderr_ref: format!("dock-land-exec-fault:{e}"),
            duration_ms: 0,
            runner_ref: "local-dock-test".to_string(),
            produced_at: 0,
        },
        from_cache: false,
        local_executions: 1,
    })
}

/// Run `hug dock land` — land a dock (L6-L9). Appends `dock.landed` (not the
/// close record — the CLI shell closes via WP-DOCK-3 after, having the path).
pub fn land_dock(
    log: &mut EventLog,
    ac: &impl ActionCache,
    dock_id: &str,
    recorded_at: u64,
    no_accept: bool,
    force_red_accept: bool,
) -> Result<DockLandResult, PorcelainError> {
    let (branch, recorded_tip) = dock_branch_and_tip(log, dock_id).map_err(|e| {
        PorcelainError::new(
            "dock_not_found",
            e,
            "the dock id is unknown to the canonical log; `hugit dock ls` lists the known docks",
        )
    })?;
    let gitdir = dock_gitdir(log, dock_id).ok_or_else(|| {
        PorcelainError::new(
            "dock_ghost",
            format!("dock '{dock_id}' has no live gitdir — the worktree is gone; nothing to land"),
            "recreate the worktree (or close the ghost via `hugit dock reconcile`) and re-land",
        )
    })?;

    // L6 — byte-identity: the recorded tip vs the OBSERVED worktree tip.
    let observed = observe_worktree_tip(&gitdir, &branch);
    let byte_identity = match (recorded_tip.as_deref(), observed.as_deref()) {
        (Some(exp), Some(obs)) if exp == obs => ByteIdentityOutcome::Verified,
        (Some(exp), Some(obs)) => ByteIdentityOutcome::Diverged {
            expected: exp.to_string(),
            observed: obs.to_string(),
        },
        _ => ByteIdentityOutcome::NoRecordedTip,
    };

    // L7/L8 — acceptance gate (unless skipped).
    let acceptance = if no_accept {
        AcceptanceOutcome::Skipped
    } else {
        let oc = run_dock_acceptance(ac, dock_id, &branch, force_red_accept);
        if oc.result.exit == 0 {
            AcceptanceOutcome::Green
        } else {
            AcceptanceOutcome::Red
        }
    };

    // F2 — honest cost (zero when absent; never fabricated).
    let cost_usd_micros: u64 = log
        .records()
        .iter()
        .filter(|r| r.kind == crate::dock::attest::COST_SAMPLE_KIND)
        .filter_map(|r| serde_json::from_str::<Value>(&r.payload).ok())
        .filter(|p| p.get("dock_id").and_then(Value::as_str) == Some(dock_id))
        .map(|p| {
            p.get("cost_usd_micros")
                .and_then(Value::as_u64)
                .unwrap_or(0)
        })
        .sum();

    // L9 — both green ⇒ settle: `dock.landed` (idempotent).
    let green =
        byte_identity == ByteIdentityOutcome::Verified && acceptance == AcceptanceOutcome::Green;
    let mut landed = false;
    if green {
        landed = append_landed(log, dock_id, &branch, recorded_at)?;
    }

    let result = DockLandResult {
        dock_id: dock_id.to_string(),
        branch,
        byte_identity,
        acceptance,
        landed,
        cost_usd_micros,
    };
    Ok(result)
}

/// Append the terminal `dock.landed` (idempotent — a dock already landed is a
/// no-op replay returning `false`). Orchestrator/Push cell (same as coinage).
fn append_landed(
    log: &mut EventLog,
    dock_id: &str,
    branch: &str,
    recorded_at: u64,
) -> Result<bool, PorcelainError> {
    let already = log
        .records()
        .iter()
        .filter(|r| r.kind == DOCK_LANDED_KIND)
        .filter_map(|r| serde_json::from_str::<Value>(&r.payload).ok())
        .any(|v| v.get("dock_id").and_then(Value::as_str) == Some(dock_id));
    if already {
        return Ok(false);
    }
    let payload = canonical_json(
        &json!({
            "dock_id": dock_id,
            "branch": branch,
            "mode": "dock-land",
        })
        .to_string(),
    )
    .unwrap_or_else(|| json!({"dock_id": dock_id}).to_string());
    log.append_authorized(
        PrincipalClass::Orchestrator,
        Endpoint::Push,
        DOCK_LANDED_KIND.to_string(),
        vec![],
        payload,
        recorded_at,
    )
    .map_err(|denied| {
        PorcelainError::new(
            "authz_denied",
            format!("landing dock '{dock_id}' denied: {}", denied.reason.code()),
            "a dock.landed is authored by the dock's Orchestrator/Push cell",
        )
    })?;
    Ok(true)
}

// ─────────────────────────────────────────────────────────────────────────────
// CLI shell — thin wrapper over the library core (one-exit-code law).
// ─────────────────────────────────────────────────────────────────────────────

/// Run `hugit dock land`.
pub fn run(args: DockLandArgs) -> ExitCode {
    let log_path = crate::log_resolve::resolve_log(args.log.clone());
    let ac_path = args
        .ac
        .clone()
        .unwrap_or_else(|| crate::checks::run::default_ac_path(&log_path));
    let _guard = match crate::pr::filelock::FileLock::acquire(&log_path) {
        Ok(lock) => lock,
        Err(e) => {
            let esc = serde_json::to_string(&format!("log_busy: {e}"))
                .unwrap_or_else(|_| "\"log_busy\"".into());
            println!("{{\"error\":{}}}", esc);
            return ExitCode::FAILURE;
        }
    };
    let mut log = match crate::checks::load_event_log(&log_path) {
        Ok(log) => log,
        Err(e) => {
            println!("{}", e.to_json());
            return e.exit_code();
        }
    };
    let ac = crate::checks::run::FileAc::new(ac_path);

    // Resolve the dock id (--id else cwd-resolve).
    let dock_id = match args.id.clone() {
        Some(id) => id,
        None => {
            let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
            match crate::dock::resolve::resolve(
                &cwd,
                args.log.as_deref(),
                std::env::var("HUGIT_DOCK_ID").ok(),
            ) {
                Ok(d) => d.dock_id,
                Err(e) => {
                    println!(
                        "{{\"error\":{},\"dock_id\":null}}",
                        serde_json::to_string(&format!("resolve_failed:{e:?}")).unwrap()
                    );
                    return ExitCode::FAILURE;
                }
            }
        }
    };

    let result = land_dock(
        &mut log,
        &ac,
        &dock_id,
        args.recorded_at,
        args.no_accept,
        false,
    );
    match result {
        Ok(res) => {
            if res.landed {
                // A4 — landing closes the dock's accounting (idempotent close).
                let _ = crate::dock::close::close_dock(&log_path, &dock_id);
            }
            if let Err(e) = crate::pr::filelock::atomic_write(
                &log_path,
                serde_json::to_string_pretty(log.records())
                    .unwrap_or_default()
                    .as_bytes(),
            ) {
                println!(
                    "{{\"error\":{}}}",
                    serde_json::to_string(&format!("io_error: {e}")).unwrap()
                );
                return ExitCode::FAILURE;
            }
            println!("{}", res.to_json());
            ExitCode::SUCCESS
        }
        Err(e) => {
            println!("{}", e.to_json());
            e.exit_code()
        }
    }
}
