//! Repo metadata — `hugit meta set` (the `owner_tenant` producer).
//!
//! Records a **`repo.meta`** event (visibility + owning tenant) onto the canonical
//! `--log`. This is the PRODUCER half of the engine's per-tenant authz seam: the
//! engine (`hugit-serve::authz::project_repo_meta`) already CONSUMES the latest
//! `repo.meta` record to decide reads (visibility) and writes (ownership); until
//! now nothing wrote it, so every repo defaulted fail-safe PRIVATE / no-owner
//! (operator-only). `meta set` closes that gap through the SAME D14-guarded,
//! scrub-on-append, atomic-persist chokepoint the campaign verbs use — the record
//! is chain-valid by construction, never hand-assembled JSON.
//!
//! The CLI verb is `meta` (not `repo`): git 2.54 added a `git repo` builtin, and
//! the WP-X5 namespace law forbids any hugit verb shadowing a git command — so
//! the verb yields the name to git. The on-the-wire event kind stays `repo.meta`
//! (a frozen authz contract the engine consumes); only the CLI surface renamed.
//!
//! Hermetic file seam: like the campaign/pr porcelain, it operates on local state
//! via `--log <path>` (a JSON `[EventRecord, …]` array). Live R2 binding is the P2
//! disclosed seam. Latest-wins: each `set` appends a new authoritative record.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::Subcommand;

mod set;

/// The engine-consumed event kind (must match `authz::REPO_META_KIND`).
pub const KIND_REPO_META: &str = "repo.meta";

/// `hugit meta <subcommand>` — repo authz metadata (visibility + owner_tenant).
#[derive(clap::Args, Debug)]
pub struct MetaArgs {
    #[command(subcommand)]
    pub command: MetaCommand,
}

#[derive(Subcommand, Debug)]
pub enum MetaCommand {
    /// Set visibility + owner_tenant → a chain-valid `repo.meta` record.
    Set(SetMetaArgs),
}

/// `hugit meta set` — record the repo's authz metadata.
#[derive(clap::Args, Debug)]
pub struct SetMetaArgs {
    /// Path to the JSON world file (the local event log). Read, then rewritten
    /// with the appended `repo.meta` record. `--log` doubles as the output path.
    #[arg(long)]
    pub log: PathBuf,
    /// Visibility: `public` (readable by anyone) or `private` (owner-only reads).
    /// Visibility governs READS; writes are always owner/operator-only.
    #[arg(long)]
    pub visibility: String,
    /// The owning tenant (the engine's `clerk:{org}` org segment). Empty ⇒
    /// unassigned (operator-only reads+writes until set). Without this, the
    /// owner's per-session token cannot read/write a private repo.
    #[arg(long, default_value = "")]
    pub owner_tenant: String,
    /// The owning **human** principal recording the change (D14).
    #[arg(long, default_value = "humangr")]
    pub by: String,
    /// Optional explicit record timestamp (ms). Omitted ⇒ 0 (the porcelain
    /// default used by the snapshot builder, which supplies real PR timestamps).
    #[arg(long)]
    pub recorded_at: Option<u64>,
}

pub fn run(args: MetaArgs) -> ExitCode {
    let result = match args.command {
        MetaCommand::Set(a) => set::run(a),
    };
    match result {
        Ok(json) => {
            println!("{json}");
            ExitCode::SUCCESS
        }
        Err(err) => {
            println!("{}", err.to_json());
            err.exit_code()
        }
    }
}
