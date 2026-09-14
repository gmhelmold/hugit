//! `hugit init` — bootstrap shared Git-common-dir hugit runtime state.
//!
//! Git-proximate: just as `git init` creates `.git/`, `hugit init` creates
//! `.git/hugit/event-log.json` outside tracked worktree state. Idempotent: an
//! existing log is never clobbered — re-running `init` leaves bytes untouched.
//!
//! Output is the same stable-JSON-on-stdout / one-exit-code law as every other
//! verb (`0` success, `2` structured domain error).

use std::path::PathBuf;
use std::process::ExitCode;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::porcelain::PorcelainError;

/// Marker identifying a hook wholly owned by hugit.
pub const HUGIT_HOOK_MARKER: &str = "# hugit-hook (managed by hugit init)";
const HUGIT_DISPATCHER_MARKER: &str = "# hugit-managed-dispatcher v1";

/// Git hook names currently installed by hugit.
pub const HOOK_KINDS: [&str; 6] = [
    "post-commit",
    "post-checkout",
    "pre-push",
    "post-merge",
    "post-rewrite",
    "reference-transaction",
];

/// True if `root` already contains a git repository (`.git` dir or worktree).
fn is_git_repo(root: &std::path::Path) -> bool {
    let dotgit = root.join(".git");
    dotgit.is_dir() || dotgit.is_file() // worktree: .git is a file pointing at the gitdir
}

/// Run `git init` in `root` (git-proximate ceremony), leaving errors as a
/// structured `PorcelainError`.
fn git_init(root: &std::path::Path) -> Result<(), PorcelainError> {
    let output = std::process::Command::new("git")
        .arg("init")
        .arg(root)
        .output()
        .map_err(|e| PorcelainError::io("run git init", root, &e))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(PorcelainError::new(
            "git_init_failed",
            format!("`git init` in {:?} failed: {}", root, stderr.trim()),
            "install git and add it to PATH, then re-run `hugit init`",
        ));
    }
    Ok(())
}

/// Arguments for `hugit init`.
#[derive(clap::Args, Debug)]
pub struct InitArgs {
    /// Where to create the hugit directory (defaults to the current directory).
    #[arg(long)]
    pub dir: Option<PathBuf>,
}

/// Arguments for `hugit attach`.
#[derive(clap::Args, Debug)]
pub struct AttachArgs {
    /// Existing repository to attach (defaults to the current directory).
    #[arg(long)]
    pub dir: Option<PathBuf>,
    /// Inspect current attachment without mutating repository state.
    #[arg(long)]
    pub status: bool,
    /// Show exact hook ownership and planned changes without writing anything.
    #[arg(long)]
    pub preview: bool,
    /// Replace foreign hooks only with a token emitted by a read-only preview.
    #[arg(long)]
    pub adopt_managed_dispatcher: bool,
    /// Hash-bound token from `hugit attach --preview`.
    #[arg(long, requires = "adopt_managed_dispatcher")]
    pub adoption_token: Option<String>,
}

/// Run `hugit init` — create runtime state and an empty canonical event log.
pub fn run(args: InitArgs) -> ExitCode {
    match do_run(&args) {
        Ok(value) => {
            println!("{value}");
            ExitCode::SUCCESS
        }
        Err(err) => {
            println!("{}", err.to_json());
            err.exit_code()
        }
    }
}

/// Attach hugit to an existing Git repository without creating a repository.
pub fn attach_run(args: AttachArgs) -> ExitCode {
    if args.status {
        return crate::health::run(crate::health::HealthArgs { dir: args.dir });
    }
    let requested_root = args.dir.clone().unwrap_or_else(|| PathBuf::from("."));
    let root = match git_top_level(&requested_root) {
        Ok(root) => root,
        Err(_) => {
            let err = PorcelainError::new(
                "not_a_git_repo",
                format!("{} is not a Git repository", requested_root.display()),
                "run `git init` first, then `hugit attach`",
            );
            println!("{}", err.to_json());
            return err.exit_code();
        }
    };
    if args.preview {
        return print_attach_result(attach_preview(&root));
    }
    match configured_hooks_path(&root) {
        Ok(Some(path)) if !args.adopt_managed_dispatcher => {
            let err = PorcelainError::new(
                "hooks_path_unsupported",
                format!("effective core.hooksPath is {path:?}"),
                "remove core.hooksPath or configure hugit through that hook manager before attach",
            );
            println!("{}", err.to_json());
            return err.exit_code();
        }
        Ok(_) => {}
        Err(err) => {
            println!("{}", err.to_json());
            return err.exit_code();
        }
    }
    if args.adopt_managed_dispatcher {
        let Some(token) = args.adoption_token.as_deref() else {
            return print_attach_result(Err(PorcelainError::new(
                "adoption_token_required",
                "managed-dispatcher adoption requires a preview token",
                "run `hugit attach --preview`, then pass its adoption_token",
            )));
        };
        let runtime = match crate::runtime_store::for_repo(&root) {
            Ok(runtime) => runtime,
            Err(error) => return print_attach_result(Err(error)),
        };
        // Prepared manifest binds retry to this token after partial replacement.
        // Fresh adoption still rejects stale previews before init can write state.
        if !runtime.hook_manifest().exists()
            && let Err(error) = validate_adoption_token(&root, token)
        {
            return print_attach_result(Err(error));
        }
    }
    let log_root = match legacy_log_root(&root) {
        Ok(root) => root,
        Err(err) => {
            println!("{}", err.to_json());
            return err.exit_code();
        }
    };
    let result = do_run(&InitArgs {
        dir: Some(log_root),
    })
    .and_then(|mut value| {
        if args.adopt_managed_dispatcher {
            adopt_managed_dispatchers(&root, args.adoption_token.as_deref().expect("checked"))?;
            value["adopted_managed_dispatcher"] = Value::Bool(true);
        }
        Ok(value)
    });
    print_attach_result(result)
}

fn print_attach_result(result: Result<Value, PorcelainError>) -> ExitCode {
    match result {
        Ok(value) => {
            println!("{value}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            println!("{}", error.to_json());
            error.exit_code()
        }
    }
}

/// The shell body for each hook, generated deterministically. Every hook:
/// - resolves the hugit binary ($HUGIT_BIN then `hugit`),
/// - resolves the repo root via git itself (worktree-safe),
/// - detaches the capture child (`nohup ... &` + re-direct) then exits 0,
///   so a hugit failure can NEVER fail/block the git operation.
#[allow(dead_code)]
pub(crate) fn legacy_hook_script(kind: &str) -> String {
    // The capture invocation for each kind (post-commit takes immutable commit
    // facts only; capture itself discovers paths from that commit with Git;
    // post-checkout passes from/to/branch when flag==1; pre-push reads
    // refspecs/shas from stdin into the child; post-merge passes the merged tip).
    match kind {
    "post-commit" => r#"#!/bin/sh
# hugit-hook (managed by hugit init)
# Silent capture: the LLM used `git commit`; hugit records ref.update async.
HUGIT_BIN="${HUGIT_BIN:-hugit}"
ROOT=$(git rev-parse --show-toplevel 2>/dev/null) || exit 0
# The canonical log lives at the MAIN repo's .hugit (shared across linked
# worktrees). In a worktree, `--show-toplevel` is the WORKTREE root, so resolve
# the shared log from the common git dir (the main .git) instead.
COMMON=$(git rev-parse --path-format=absolute --git-common-dir 2>/dev/null) || exit 0
LOG="$COMMON/../.hugit/log.json"
HL="$COMMON/../.hugit/hooks.log"
[ -f "$LOG" ] || {
  # LAZY BOOT: no hugit log yet — create one (a template-dir repo that never
  # ran `hugit init` still becomes active on the FIRST git op). The log is an
  # empty JSON array; hooks must never block git, so failures are best-effort.
  mkdir -p "$(dirname "$LOG")" 2>/dev/null
  printf '[]\n' > "$LOG" 2>/dev/null
}
OID="$(git rev-parse HEAD 2>/dev/null)" || exit 0
BRANCH="$(git branch --show-current 2>/dev/null)"
RECORDED_AT="$(git log -1 --format=%ct 2>/dev/null)"
(
  "$HUGIT_BIN" capture --kind commit --top-level "$ROOT" --log "$LOG" --hook-log "$HL"     --oid "$OID"     --branch "$BRANCH"     --recorded-at "$RECORDED_AT"
) >>"$HL" 2>&1 &
exit 0
"#.to_string(),
    "post-checkout" => r#"#!/bin/sh
# hugit-hook (managed by hugit init)
# Silent capture: branch checkout (flag=1); records ref.update {checkout:true}.
HUGIT_BIN="${HUGIT_BIN:-hugit}"
ROOT=$(git rev-parse --show-toplevel 2>/dev/null) || exit 0
# The canonical log lives at the MAIN repo's .hugit (shared across linked
# worktrees). In a worktree, `--show-toplevel` is the WORKTREE root, so resolve
# the shared log from the common git dir (the main .git) instead.
COMMON=$(git rev-parse --path-format=absolute --git-common-dir 2>/dev/null) || exit 0
LOG="$COMMON/../.hugit/log.json"
HL="$COMMON/../.hugit/hooks.log"
[ -f "$LOG" ] || {
  # LAZY BOOT: no hugit log yet — create one (a template-dir repo that never
  # ran `hugit init` still becomes active on the FIRST git op). The log is an
  # empty JSON array; hooks must never block git, so failures are best-effort.
  mkdir -p "$(dirname "$LOG")" 2>/dev/null
  printf '[]\n' > "$LOG" 2>/dev/null
}
[ "$3" = "1" ] || exit 0   # only branch checkouts (flag=1), not file checkouts
GITDIR=$(git rev-parse --absolute-git-dir 2>/dev/null) || exit 0
(
  "$HUGIT_BIN" capture --kind checkout --top-level "$ROOT" --log "$LOG" --hook-log "$HL"     --from "$1" --oid "$2" --branch "$(git branch --show-current 2>/dev/null)"
) >>"$HL" 2>&1 &
# Worktree-dock (ADR-0005, WP-DOCK-1): coin the physical binding at checkout
# time (idempotent — marker present ⇒ no-op; never blocks git).
(
  "$HUGIT_BIN" dock coin --top-level "$ROOT" --gitdir "$GITDIR" --log "$LOG" --hook-log "$HL" --branch "$(git branch --show-current 2>/dev/null)"
) >>"$HL" 2>&1 &
exit 0
"#.to_string(),
    "pre-push" => r#"#!/bin/sh
# hugit-hook (managed by hugit init)
# Silent capture: a push is attempted; records ref.update {attempt:true} with
# the LOCAL shas being pushed (the 2nd field of each refspec stdin line), so a
# `git push` is a captured-commit proof too (pr open --commit <pushed-sha>).
# ALWAYS exits 0 — this is a PRE hook; a non-zero exit would BLOCK the push.
HUGIT_BIN="${HUGIT_BIN:-hugit}"
ROOT=$(git rev-parse --show-toplevel 2>/dev/null) || exit 0
# The canonical log lives at the MAIN repo's .hugit (shared across linked
# worktrees). In a worktree, `--show-toplevel` is the WORKTREE root, so resolve
# the shared log from the common git dir (the main .git) instead.
COMMON=$(git rev-parse --path-format=absolute --git-common-dir 2>/dev/null) || exit 0
LOG="$COMMON/../.hugit/log.json"
HL="$COMMON/../.hugit/hooks.log"
[ -f "$LOG" ] || {
  # LAZY BOOT: no hugit log yet — create one (a template-dir repo that never
  # ran `hugit init` still becomes active on the FIRST git op). The log is an
  # empty JSON array; hooks must never block git, so failures are best-effort.
  mkdir -p "$(dirname "$LOG")" 2>/dev/null
  printf '[]\n' > "$LOG" 2>/dev/null
}
STDIN_REFS="$(if [ -n "$HUGIT_PRE_PUSH_FILE" ]; then cat "$HUGIT_PRE_PUSH_FILE"; else cat; fi)"   # dispatcher may preserve stdin for an adopted foreign hook
# Extract the LOCAL sha (2nd field) from each refspec line that has 4 fields.
SHAS="$(echo "$STDIN_REFS" | awk 'NF>=4 {print $2}')"
(
  "$HUGIT_BIN" capture --kind push-attempt --top-level "$ROOT" --log "$LOG" --hook-log "$HL"     --refspecs "$STDIN_REFS" --shas "$SHAS"
) >>"$HL" 2>&1 &
exit 0
"#.to_string(),
    "post-merge" => r#"#!/bin/sh
# hugit-hook (managed by hugit init)
# Silent capture: a local merge landed; records ref.update {merged_from}.
HUGIT_BIN="${HUGIT_BIN:-hugit}"
ROOT=$(git rev-parse --show-toplevel 2>/dev/null) || exit 0
# The canonical log lives at the MAIN repo's .hugit (shared across linked
# worktrees). In a worktree, `--show-toplevel` is the WORKTREE root, so resolve
# the shared log from the common git dir (the main .git) instead.
COMMON=$(git rev-parse --path-format=absolute --git-common-dir 2>/dev/null) || exit 0
LOG="$COMMON/../.hugit/log.json"
HL="$COMMON/../.hugit/hooks.log"
[ -f "$LOG" ] || {
  # LAZY BOOT: no hugit log yet — create one (a template-dir repo that never
  # ran `hugit init` still becomes active on the FIRST git op). The log is an
  # empty JSON array; hooks must never block git, so failures are best-effort.
  mkdir -p "$(dirname "$LOG")" 2>/dev/null
  printf '[]\n' > "$LOG" 2>/dev/null
}
(
  "$HUGIT_BIN" capture --kind merge --top-level "$ROOT" --log "$LOG" --hook-log "$HL"     --from "$(git rev-parse HEAD~1 2>/dev/null)"     --oid "$(git rev-parse HEAD 2>/dev/null)"     --recorded-at "$(git log -1 --format=%ct 2>/dev/null)"
) >>"$HL" 2>&1 &
exit 0
"#.to_string(),
    _ => unreachable!("known hook kind"),
}
}

#[allow(dead_code)]
pub(crate) const LEGACY_HUGIT_HOOK_MARKER: &str = "# hugit-hook (managed by hugit init)";
#[allow(dead_code)]
pub(crate) const LEGACY_HOOK_KINDS: [&str; 4] =
    ["post-commit", "post-checkout", "pre-push", "post-merge"];

#[allow(dead_code)]
pub(crate) struct LegacyHookInstallResult {
    pub installed: Vec<String>,
    pub noop: Vec<String>,
    pub conflict: Vec<String>,
}

/// Read-only coexistence report. Foreign hooks are never auto-chained.
fn attach_preview(root: &std::path::Path) -> Result<Value, PorcelainError> {
    let hooks_path = configured_hooks_path(root)?;
    let hooks_dir = resolve_hooks_dir(root)?;
    let mut hooks = Vec::new();
    for kind in HOOK_KINDS {
        let path = hooks_dir.join(kind);
        let before = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(error) => return Err(PorcelainError::io("read hook", &path, &error)),
        };
        let owned = std::str::from_utf8(&before).ok().is_some_and(|contents| {
            is_managed_hook(kind, contents) || contents.contains(HUGIT_DISPATCHER_MARKER)
        });
        hooks.push(json!({
            "kind": kind, "path": path.display().to_string(),
            "ownership": if before.is_empty() { "missing" } else if owned { "hugit" } else { "foreign" },
            "before_sha256": sha256_hex(&before), "after_sha256": sha256_hex(hook_script(kind).as_bytes()),
            "action": if owned { "unchanged" } else if before.is_empty() { "install" } else { "conflict_requires_explicit_adoption" },
        }));
    }
    Ok(json!({
        "preview": true, "writes": false, "repo": root.display().to_string(),
        "hooks_path": hooks_path.map(|path| json!({"state":"external","path":path})).unwrap_or_else(|| json!({"state":"default"})),
        "hooks": hooks, "adoption_token": adoption_token(root)?,
        "next": "foreign hooks are never chained automatically; explicit managed-dispatcher adoption is required",
    }))
}

fn adoption_token(root: &std::path::Path) -> Result<String, PorcelainError> {
    let mut binding = Vec::new();
    for kind in HOOK_KINDS {
        let path = resolve_hooks_dir(root)?.join(kind);
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(PorcelainError::io("read hook", &path, &error)),
        };
        let contents = std::str::from_utf8(&bytes).unwrap_or("");
        if !is_managed_hook(kind, contents) && !contents.contains(HUGIT_DISPATCHER_MARKER) {
            binding.extend_from_slice(kind.as_bytes());
            binding.push(0);
            binding.extend_from_slice(&Sha256::digest(&bytes));
            binding.extend_from_slice(&Sha256::digest(hook_script(kind).as_bytes()));
        }
    }
    Ok(sha256_hex(&binding))
}

fn validate_adoption_token(root: &std::path::Path, token: &str) -> Result<(), PorcelainError> {
    (token == adoption_token(root)?)
        .then_some(())
        .ok_or_else(|| {
            PorcelainError::new(
                "adoption_token_stale",
                "hook bytes changed since preview",
                "run `hugit attach --preview` again and use its new adoption_token",
            )
        })
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn shell_quote(path: &std::path::Path) -> String {
    format!("'{}'", path.display().to_string().replace('\'', "'\"'\"'"))
}

fn dispatcher_script(kind: &str, backup: &std::path::Path) -> String {
    let body = hook_script(kind)
        .lines()
        .skip(3)
        .collect::<Vec<_>>()
        .join("\n");
    let backup = shell_quote(backup);
    if kind == "pre-push" {
        format!(
            "#!/bin/sh\n{HUGIT_DISPATCHER_MARKER}\nTMP=$(mktemp \"${{TMPDIR:-/tmp}}/hugit-pre-push.XXXXXX\") || {{ {backup} \"$@\"; exit $?; }}\ntrap 'rm -f \"$TMP\"' 0 HUP INT TERM\ndd bs=1 count=8193 <&0 > \"$TMP\" 2>/dev/null\n{{ dd if=\"$TMP\" bs=1 2>/dev/null; dd bs=8192 2>/dev/null; }} | {backup} \"$@\"\nSTATUS=$?\nTUPLES=$(dd if=\"$TMP\" bs=1 count=8193 2>/dev/null; printf .)\nTUPLES=${{TUPLES%.}}\n[ \"$STATUS\" -eq 0 ] || exit \"$STATUS\"\nHUGIT_PRE_PUSH_TUPLES=\"$TUPLES\"; export HUGIT_PRE_PUSH_TUPLES\n{body}\n"
        )
    } else {
        format!(
            "#!/bin/sh\n{HUGIT_DISPATCHER_MARKER}\n{backup} \"$@\"\nSTATUS=$?\n[ \"$STATUS\" -eq 0 ] || exit \"$STATUS\"\n{body}\n"
        )
    }
}

/// Exact dispatcher renderer is ownership proof. Marker alone is not enough.
pub(crate) fn is_managed_dispatcher(kind: &str, contents: &str) -> bool {
    contents.contains(HUGIT_DISPATCHER_MARKER)
        && contents.contains("HUGIT_BIN=\"${HUGIT_BIN:-hugit}\"")
        && HOOK_KINDS.contains(&kind)
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct AdoptionEntry {
    kind: String,
    backup: String,
    before_sha256: String,
    dispatcher_sha256: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct AdoptionManifest {
    version: u8,
    state: String,
    adoption_token: String,
    entries: Vec<AdoptionEntry>,
}

fn backup_name(kind: &str, before_sha256: &str) -> String {
    format!("{kind}-{before_sha256}.hook")
}

/// Resolve a backup from manifest fields, never trusting a manifest path.
fn manifest_backup(
    runtime: &crate::runtime_store::RuntimeStore,
    entry: &AdoptionEntry,
) -> Result<PathBuf, PorcelainError> {
    if !HOOK_KINDS.contains(&entry.kind.as_str())
        || entry.backup != backup_name(&entry.kind, &entry.before_sha256)
        || entry.before_sha256.len() != 64
        || !entry
            .before_sha256
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(PorcelainError::new(
            "hook_manifest_invalid",
            "hook manifest backup is invalid",
            "preserve evidence and repair hook-manifest.json before detach",
        ));
    }
    let backups = std::fs::canonicalize(runtime.hook_backups())
        .map_err(|e| PorcelainError::io("resolve hook backup dir", &runtime.hook_backups(), &e))?;
    let backup = runtime.hook_backups().join(&entry.backup);
    let resolved = std::fs::canonicalize(&backup)
        .map_err(|e| PorcelainError::io("resolve hook backup", &backup, &e))?;
    if !resolved.starts_with(&backups) {
        return Err(PorcelainError::new(
            "hook_manifest_invalid",
            "hook backup escapes common-dir runtime",
            "preserve evidence and repair hook-manifest.json before detach",
        ));
    }
    Ok(resolved)
}

fn read_adoption_manifest(
    runtime: &crate::runtime_store::RuntimeStore,
) -> Result<Option<AdoptionManifest>, PorcelainError> {
    if !runtime.hook_manifest().exists() {
        return Ok(None);
    }
    serde_json::from_slice(
        &std::fs::read(runtime.hook_manifest())
            .map_err(|e| PorcelainError::io("read hook manifest", &runtime.hook_manifest(), &e))?,
    )
    .map(Some)
    .map_err(|e| {
        PorcelainError::new(
            "hook_manifest_invalid",
            e.to_string(),
            "preserve evidence and repair hook-manifest.json before retrying",
        )
    })
}

fn write_adoption_manifest(
    runtime: &crate::runtime_store::RuntimeStore,
    manifest: &AdoptionManifest,
) -> Result<(), PorcelainError> {
    crate::pr::filelock::atomic_write_unprepared(
        &runtime.hook_manifest(),
        &serde_json::to_vec_pretty(manifest).expect("json"),
    )
    .map_err(|e| {
        PorcelainError::new(
            "hook_manifest_write_failed",
            e.to_string(),
            "retry `hugit attach`",
        )
    })
}

fn rollback_adoption(
    runtime: &crate::runtime_store::RuntimeStore,
    hooks_dir: &std::path::Path,
    entries: &[AdoptionEntry],
) -> Result<(), PorcelainError> {
    for entry in entries {
        let backup = manifest_backup(runtime, entry)?;
        let path = hooks_dir.join(&entry.kind);
        if std::fs::read(&path).ok().as_deref()
            == Some(dispatcher_script(&entry.kind, &backup).as_bytes())
        {
            let before = std::fs::read(&backup)
                .map_err(|e| PorcelainError::io("read hook backup", &backup, &e))?;
            crate::pr::filelock::atomic_write_unprepared(&path, &before).map_err(|e| {
                PorcelainError::new("hook_restore_failed", e.to_string(), "retry `hugit attach`")
            })?;
            set_hook_executable(&path)?;
        }
    }
    Ok(())
}

/// Backup + manifest complete before any atomic dispatcher replacement.
fn adopt_managed_dispatchers(root: &std::path::Path, token: &str) -> Result<(), PorcelainError> {
    let runtime = crate::runtime_store::for_repo(root)?;
    let hooks_dir = resolve_hooks_dir(root)?;
    std::fs::create_dir_all(runtime.hook_backups())
        .map_err(|e| PorcelainError::io("create hook backup dir", &runtime.hook_backups(), &e))?;
    let mut manifest = match read_adoption_manifest(&runtime)? {
        Some(manifest) => {
            if manifest.version != 1
                || !matches!(manifest.state.as_str(), "prepared" | "installed")
                || manifest.adoption_token != token
            {
                return Err(PorcelainError::new(
                    "adoption_token_stale",
                    "adoption state differs from preview",
                    "run `hugit attach --preview` again and use its new adoption_token",
                ));
            }
            manifest
        }
        None => {
            validate_adoption_token(root, token)?;
            let mut entries = Vec::new();
            for kind in HOOK_KINDS {
                let path = hooks_dir.join(kind);
                let before = match std::fs::read(&path) {
                    Ok(bytes) => bytes,
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                    Err(e) => return Err(PorcelainError::io("read hook", &path, &e)),
                };
                let contents = std::str::from_utf8(&before).unwrap_or("");
                if is_managed_hook(kind, contents) || contents.contains(HUGIT_DISPATCHER_MARKER) {
                    continue;
                }
                let digest = sha256_hex(&before);
                let backup = runtime.hook_backups().join(backup_name(kind, &digest));
                if !backup.exists() {
                    crate::pr::filelock::atomic_write_unprepared(&backup, &before).map_err(
                        |e| {
                            PorcelainError::new(
                                "hook_backup_write_failed",
                                e.to_string(),
                                "retry `hugit attach`",
                            )
                        },
                    )?;
                }
                let script = dispatcher_script(kind, &backup);
                entries.push(AdoptionEntry {
                    kind: kind.into(),
                    backup: backup
                        .file_name()
                        .expect("backup name")
                        .to_string_lossy()
                        .into_owned(),
                    before_sha256: digest,
                    dispatcher_sha256: sha256_hex(script.as_bytes()),
                });
            }
            let manifest = AdoptionManifest {
                version: 1,
                state: "prepared".into(),
                adoption_token: token.into(),
                entries,
            };
            write_adoption_manifest(&runtime, &manifest)?;
            manifest
        }
    };
    for entry in &manifest.entries {
        let backup = manifest_backup(&runtime, entry)?;
        let before = std::fs::read(&backup)
            .map_err(|e| PorcelainError::io("read hook backup", &backup, &e))?;
        if sha256_hex(&before) != entry.before_sha256 {
            return Err(PorcelainError::new(
                "hook_manifest_invalid",
                "hook backup digest differs from manifest",
                "preserve evidence and repair hook-manifest.json before retrying",
            ));
        }
        let path = hooks_dir.join(&entry.kind);
        let current = std::fs::read(&path).unwrap_or_default();
        let dispatcher = dispatcher_script(&entry.kind, &backup);
        if current == dispatcher.as_bytes() {
            continue;
        }
        if current != before {
            return Err(PorcelainError::new(
                "hook_adoption_interrupted",
                format!("{} changed during adoption", entry.kind),
                "preserve hook bytes and run `hugit attach --preview`",
            ));
        }
        if let Err(error) =
            crate::pr::filelock::atomic_write_unprepared(&path, dispatcher.as_bytes())
        {
            rollback_adoption(&runtime, &hooks_dir, &manifest.entries)?;
            return Err(PorcelainError::new(
                "hook_dispatcher_write_failed",
                error.to_string(),
                "retry `hugit attach`",
            ));
        }
        if let Err(error) = set_hook_executable(&path) {
            rollback_adoption(&runtime, &hooks_dir, &manifest.entries)?;
            return Err(error);
        }
    }
    manifest.state = "installed".into();
    write_adoption_manifest(&runtime, &manifest)?;
    Ok(())
}

fn set_hook_executable(path: &std::path::Path) -> Result<(), PorcelainError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = std::fs::metadata(path)
            .map_err(|e| PorcelainError::io("stat hook", path, &e))?
            .permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(path, permissions)
            .map_err(|e| PorcelainError::io("chmod hook", path, &e))?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

/// Remove only hooks wholly owned by hugit, retaining all captured evidence.
pub fn detach_run(args: InitArgs) -> ExitCode {
    let requested_root = args.dir.unwrap_or_else(|| PathBuf::from("."));
    let root = match git_top_level(&requested_root) {
        Ok(root) => root,
        Err(_) => {
            let err = PorcelainError::new(
                "not_a_git_repo",
                format!("{} is not a Git repository", requested_root.display()),
                "run `hugit detach` inside an existing Git repository",
            );
            println!("{}", err.to_json());
            return err.exit_code();
        }
    };
    let result = (|| {
        let hooks_dir = resolve_hooks_dir(&root)?;
        let runtime = crate::runtime_store::for_repo(&root)?;
        let mut removed = Vec::new();
        let mut restored = Vec::new();
        let mut preserved = Vec::new();
        if let Some(manifest) = read_adoption_manifest(&runtime)? {
            if manifest.version != 1 || manifest.state != "installed" {
                return Err(PorcelainError::new(
                    "hook_manifest_invalid",
                    "hook manifest is not a completed adoption",
                    "preserve evidence and repair hook-manifest.json before detach",
                ));
            }
            for entry in manifest.entries {
                let backup = manifest_backup(&runtime, &entry)?;
                let path = hooks_dir.join(&entry.kind);
                let current = std::fs::read(&path).unwrap_or_default();
                let backup_bytes = std::fs::read(&backup)
                    .map_err(|e| PorcelainError::io("read hook backup", &backup, &e))?;
                if entry.before_sha256 != sha256_hex(&backup_bytes)
                    || entry.dispatcher_sha256 != sha256_hex(&current)
                {
                    preserved.push(entry.kind);
                    continue;
                }
                crate::pr::filelock::atomic_write_unprepared(&path, &backup_bytes).map_err(
                    |e| {
                        PorcelainError::new(
                            "hook_restore_failed",
                            e.to_string(),
                            "retry `hugit detach`",
                        )
                    },
                )?;
                restored.push(entry.kind);
            }
        }
        for kind in HOOK_KINDS {
            let path = hooks_dir.join(kind);
            match std::fs::read_to_string(&path) {
                Ok(contents) if is_managed_hook(kind, &contents) => {
                    std::fs::remove_file(&path)
                        .map_err(|e| PorcelainError::io("remove hugit hook", &path, &e))?;
                    removed.push(kind.to_string());
                }
                Ok(_) if !restored.iter().any(|restored_kind| restored_kind == kind) => {
                    preserved.push(kind.to_string())
                }
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(PorcelainError::io("read hook", &path, &error)),
            }
        }
        Ok(serde_json::json!({
            "detached": true,
            "removed": removed,
            "restored": restored,
            "preserved": preserved,
            "evidence_retained": true,
        }))
    })();
    match result {
        Ok(value) => {
            println!("{value}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            println!("{}", error.to_json());
            error.exit_code()
        }
    }
}

/// Resolve effective hook path through Git, including worktree-aware paths.
pub fn resolve_hooks_dir(root: &std::path::Path) -> Result<std::path::PathBuf, PorcelainError> {
    let out = std::process::Command::new("git")
        .args([
            "-C",
            root.to_str().unwrap_or("."),
            "rev-parse",
            "--git-path",
            "hooks",
        ])
        .output()
        .map_err(|e| PorcelainError::io("resolve hooks dir", root, &e))?;
    if !out.status.success() {
        return Err(PorcelainError::new(
            "git_hooks_dir_failed",
            format!("`git rev-parse --git-path hooks` in {:?} failed", root),
            "is `git` on PATH?",
        ));
    }
    let path = String::from_utf8_lossy(&out.stdout);
    let rel = std::path::PathBuf::from(path.trim());
    Ok(if rel.is_absolute() {
        rel
    } else {
        root.join(rel)
    })
}

/// Return effective `core.hooksPath` when Git configuration overrides default hooks.
pub fn configured_hooks_path(root: &std::path::Path) -> Result<Option<String>, PorcelainError> {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["config", "--get", "core.hooksPath"])
        .output()
        .map_err(|e| PorcelainError::io("read core.hooksPath", root, &e))?;
    if !output.status.success() {
        return Ok(None);
    }
    let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Ok((!path.is_empty()).then_some(path))
}

pub(crate) struct HookInstallResult {
    pub installed: Vec<String>,
    pub noop: Vec<String>,
    pub conflict: Vec<String>,
}

/// Install managed hooks only into Git's active default hook directory.
pub(crate) fn install_hooks(root: &std::path::Path) -> Result<HookInstallResult, PorcelainError> {
    if configured_hooks_path(root)?.is_some() {
        return Err(PorcelainError::new(
            "custom_hooks_path",
            "refusing to install hooks while core.hooksPath is configured",
            "unset core.hooksPath or install hugit hooks in that path yourself",
        ));
    }
    let hooks_dir = resolve_hooks_dir(root)?;
    std::fs::create_dir_all(&hooks_dir)
        .map_err(|e| PorcelainError::io("create hooks dir", &hooks_dir, &e))?;
    let mut result = HookInstallResult {
        installed: Vec::new(),
        noop: Vec::new(),
        conflict: Vec::new(),
    };
    for kind in HOOK_KINDS {
        let path = hooks_dir.join(kind);
        match std::fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
                return Err(PorcelainError::new(
                    "unsafe_hook_path",
                    format!("refusing non-regular hook path {path:?}"),
                    "replace the hook path with a regular file, then rerun setup",
                ));
            }
            Ok(_) => {
                let contents = std::fs::read_to_string(&path)
                    .map_err(|e| PorcelainError::io("read hook", &path, &e))?;
                if is_managed_hook(kind, &contents) {
                    result.noop.push(kind.to_string());
                } else if contents.contains(HUGIT_HOOK_MARKER)
                    && contents
                        .lines()
                        .any(|line| line.starts_with("# hugit-hook-version: "))
                {
                    crate::pr::filelock::atomic_write_unprepared(
                        &path,
                        hook_script(kind).as_bytes(),
                    )
                    .map_err(|e| {
                        PorcelainError::new(
                            "hook_upgrade_failed",
                            e.to_string(),
                            "retry `hugit attach`",
                        )
                    })?;
                    set_hook_executable(&path)?;
                    result.installed.push(format!("{kind}:upgraded"));
                } else {
                    result.conflict.push(kind.to_string());
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                crate::pr::filelock::atomic_write_unprepared(&path, hook_script(kind).as_bytes())
                    .map_err(|e| {
                    PorcelainError::new("hook_write_failed", e.to_string(), "retry `hugit setup`")
                })?;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let mut permissions = std::fs::metadata(&path)
                        .map_err(|e| PorcelainError::io("stat hook", &path, &e))?
                        .permissions();
                    permissions.set_mode(0o755);
                    std::fs::set_permissions(&path, permissions)
                        .map_err(|e| PorcelainError::io("chmod hook", &path, &e))?;
                }
                result.installed.push(kind.to_string());
            }
            Err(error) => return Err(PorcelainError::io("stat hook", &path, &error)),
        }
    }
    Ok(result)
}

/// Resolve repository root through Git, including nested directories and worktrees.
pub fn git_top_level(path: &std::path::Path) -> Result<PathBuf, PorcelainError> {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(path)
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .map_err(|e| PorcelainError::io("resolve repository root", path, &e))?;
    if !output.status.success() {
        return Err(PorcelainError::new(
            "not_a_git_repo",
            format!("{} is not a Git repository", path.display()),
            "run this command inside a Git repository",
        ));
    }
    let root = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if root.is_empty() {
        return Err(PorcelainError::new(
            "not_a_git_repo",
            format!("{} has no Git top-level", path.display()),
            "run this command inside a Git repository",
        ));
    }
    Ok(PathBuf::from(root))
}

/// A hook is hugit-owned only when it exactly matches deterministic renderer output.
pub fn is_managed_hook(kind: &str, contents: &str) -> bool {
    contents == hook_script(kind)
}

/// Legacy hook storage is shared by linked worktrees through their main worktree.
pub fn legacy_log_root(path: &std::path::Path) -> Result<PathBuf, PorcelainError> {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(path)
        .args(["worktree", "list", "--porcelain"])
        .output()
        .map_err(|e| PorcelainError::io("resolve main worktree", path, &e))?;
    if !output.status.success() {
        return Err(PorcelainError::new(
            "git_worktree_list_failed",
            format!("`git worktree list --porcelain` in {:?} failed", path),
            "use a non-bare Git working tree",
        ));
    }
    let main = String::from_utf8_lossy(&output.stdout)
        .lines()
        .find_map(|line| line.strip_prefix("worktree "))
        .map(PathBuf::from);
    main.ok_or_else(|| {
        PorcelainError::new(
            "git_worktree_list_invalid",
            "Git returned no main working tree",
            "use a non-bare Git working tree",
        )
    })
}

/// The shell body for each hook, generated deterministically. Every hook:
/// - resolves the hugit binary ($HUGIT_BIN then `hugit`),
/// - resolves the repo root via git itself (worktree-safe),
/// - snapshots immutable facts before detach, then detaches capture with stdin
///   closed (`... </dev/null &`), then exits 0,
///   so a hugit failure can NEVER fail/block the git operation.
pub(crate) fn hook_script(kind: &str) -> String {
    // Shell snapshots hook args plus cheap immutable Git facts before detach.
    // Rust resolves commit paths from the captured OID; detached children never
    // query moving HEAD, branch, or stdin.
    match kind {
    "post-commit" => r#"#!/bin/sh
# hugit-hook (managed by hugit init)
# hugit-hook-version: 1
# Silent capture: the LLM used `git commit`; hugit records ref.update async.
HUGIT_BIN="${HUGIT_BIN:-hugit}"
ROOT=$(git rev-parse --show-toplevel 2>/dev/null) || exit 0
# Runtime state lives in Git common dir, shared across all linked worktrees.
COMMON=$(git rev-parse --path-format=absolute --git-common-dir 2>/dev/null) || exit 0
RUNTIME="$COMMON/hugit"
LOG="$RUNTIME/event-log.json"
HL="$RUNTIME/hooks.log"
OID=$(git rev-parse HEAD 2>/dev/null) || exit 0
BRANCH=$(git symbolic-ref --quiet --short HEAD 2>/dev/null || true)
RECORDED_AT=$(git show -s --format=%ct "$OID" 2>/dev/null) || exit 0
( "$HUGIT_BIN" capture --kind commit --top-level "$ROOT" --log "$LOG" --hook-log "$HL" --oid "$OID" --branch "$BRANCH" --recorded-at "$RECORDED_AT" </dev/null ) >/dev/null 2>&1 &
exit 0
"#.to_string(),
    "post-checkout" => r#"#!/bin/sh
# hugit-hook (managed by hugit init)
# hugit-hook-version: 1
# Silent capture: branch checkout (flag=1); records ref.update {checkout:true}.
HUGIT_BIN="${HUGIT_BIN:-hugit}"
ROOT=$(git rev-parse --show-toplevel 2>/dev/null) || exit 0
# Runtime state lives in Git common dir, shared across all linked worktrees.
COMMON=$(git rev-parse --path-format=absolute --git-common-dir 2>/dev/null) || exit 0
RUNTIME="$COMMON/hugit"
LOG="$RUNTIME/event-log.json"
HL="$RUNTIME/hooks.log"
[ "$3" = "1" ] || exit 0   # only branch checkouts (flag=1), not file checkouts
GITDIR=$(git rev-parse --absolute-git-dir 2>/dev/null) || exit 0
BRANCH=$(git symbolic-ref --quiet --short HEAD 2>/dev/null || true)
( "$HUGIT_BIN" capture --kind checkout --top-level "$ROOT" --log "$LOG" --hook-log "$HL" --from "$1" --oid "$2" --branch "$BRANCH" </dev/null ) >/dev/null 2>&1 &
# Worktree-dock (ADR-0005, WP-DOCK-1): coin the physical binding at checkout
# time (idempotent — marker present ⇒ no-op; never blocks git).
( "$HUGIT_BIN" dock coin --top-level "$ROOT" --gitdir "$GITDIR" --log "$LOG" --hook-log "$HL" --branch "$BRANCH" </dev/null ) >/dev/null 2>&1 &
exit 0
"#.to_string(),
    "pre-push" => r#"#!/bin/sh
# hugit-hook (managed by hugit init)
# hugit-hook-version: 1
# Silent capture: a push is attempted; records ref.update {attempt:true} with
# the LOCAL shas being pushed (the 2nd field of each refspec stdin line), so a
# `git push` is a captured-commit proof too (pr open --commit <pushed-sha>).
# ALWAYS exits 0 — this is a PRE hook; a non-zero exit would BLOCK the push.
HUGIT_BIN="${HUGIT_BIN:-hugit}"
ROOT=$(git rev-parse --show-toplevel 2>/dev/null) || exit 0
# Runtime state lives in Git common dir, shared across all linked worktrees.
COMMON=$(git rev-parse --path-format=absolute --git-common-dir 2>/dev/null) || exit 0
RUNTIME="$COMMON/hugit"
LOG="$RUNTIME/event-log.json"
HL="$RUNTIME/hooks.log"
# Read at most 8193 bytes before parsing. Sentinel preserves trailing newlines
# lost by command substitution; byte 8193 makes Rust persist incomplete/oversize.
if [ "${HUGIT_PRE_PUSH_TUPLES+x}" = x ]; then
  TUPLES=$HUGIT_PRE_PUSH_TUPLES
else
  TUPLES=$(dd bs=1 count=8193 2>/dev/null; printf .)
  TUPLES=${TUPLES%.}
fi
( "$HUGIT_BIN" capture --kind push-attempt --top-level "$ROOT" --log "$LOG" --hook-log "$HL" --push-tuples "$TUPLES" </dev/null ) >/dev/null 2>&1 &
exit 0
"#.to_string(),
    "post-merge" => r#"#!/bin/sh
# hugit-hook (managed by hugit init)
# hugit-hook-version: 1
# Silent capture: a local merge landed; records ref.update {merged_from}.
HUGIT_BIN="${HUGIT_BIN:-hugit}"
ROOT=$(git rev-parse --show-toplevel 2>/dev/null) || exit 0
# Runtime state lives in Git common dir, shared across all linked worktrees.
COMMON=$(git rev-parse --path-format=absolute --git-common-dir 2>/dev/null) || exit 0
RUNTIME="$COMMON/hugit"
LOG="$RUNTIME/event-log.json"
HL="$RUNTIME/hooks.log"
OID=$(git rev-parse HEAD 2>/dev/null) || exit 0
# ORIG_HEAD is only fast-forward fact candidate. Rust verifies it against
# captured commit parent set; it is never represented as merge source.
ORIG=$(git rev-parse ORIG_HEAD 2>/dev/null || true)
RECORDED_AT=$(git show -s --format=%ct "$OID" 2>/dev/null) || exit 0
( "$HUGIT_BIN" capture --kind merge --top-level "$ROOT" --log "$LOG" --hook-log "$HL" --from "$ORIG" --oid "$OID" --recorded-at "$RECORDED_AT" </dev/null ) >/dev/null 2>&1 &
exit 0
"#.to_string(),
    "post-rewrite" => r#"#!/bin/sh
# hugit-hook (managed by hugit init)
# hugit-hook-version: 1
HUGIT_BIN="${HUGIT_BIN:-hugit}"
ROOT=$(git rev-parse --show-toplevel 2>/dev/null) || exit 0
COMMON=$(git rev-parse --path-format=absolute --git-common-dir 2>/dev/null) || exit 0
RUNTIME="$COMMON/hugit"; LOG="$RUNTIME/event-log.json"; HL="$RUNTIME/hooks.log"
TUPLES=$(dd bs=1 count=8193 2>/dev/null; printf .); TUPLES=${TUPLES%.}
case "$1" in amend|rebase) ;; *) exit 0 ;; esac
( "$HUGIT_BIN" capture --kind rewrite --top-level "$ROOT" --log "$LOG" --hook-log "$HL" --rewrite-type "$1" --rewrite-tuples "$TUPLES" </dev/null ) >/dev/null 2>&1 &
exit 0
"#.to_string(),
    "reference-transaction" => r#"#!/bin/sh
# hugit-hook (managed by hugit init)
# hugit-hook-version: 1
HUGIT_BIN="${HUGIT_BIN:-hugit}"
ROOT=$(git rev-parse --show-toplevel 2>/dev/null) || exit 0
COMMON=$(git rev-parse --path-format=absolute --git-common-dir 2>/dev/null) || exit 0
RUNTIME="$COMMON/hugit"; LOG="$RUNTIME/event-log.json"; HL="$RUNTIME/hooks.log"
case "$1" in prepared|committed|aborted) ;; *) exit 0 ;; esac
TUPLES=$(dd bs=1 count=8193 2>/dev/null; printf .); TUPLES=${TUPLES%.}
( "$HUGIT_BIN" capture --kind reference-transaction --top-level "$ROOT" --log "$LOG" --hook-log "$HL" --transaction-phase "$1" --reference-tuples "$TUPLES" </dev/null ) >/dev/null 2>&1 &
exit 0
"#.to_string(),
    _ => unreachable!("known hook kind"),
}
}

pub(crate) fn do_run(args: &InitArgs) -> Result<serde_json::Value, PorcelainError> {
    let root = args.dir.clone().unwrap_or_else(|| PathBuf::from("."));

    // Git-proximate ceremony: ensure a git repo exists. If `root` is not yet
    // a git repository, shell out to `git init` (same CLI a user would run);
    // if it already is (`.git` dir or worktree file) leave it untouched.
    let git_created = if is_git_repo(&root) {
        false
    } else {
        git_init(&root)?;
        true
    };

    // Validate source before runtime writes or hook installation. Re-entry
    // resumes each atomic file boundary without changing source evidence.
    let runtime = crate::runtime_store::for_repo(&root)?;
    let log_path = runtime.canonical_log();
    let created = !log_path.exists();
    let legacy_root = legacy_log_root(&root)?;
    let migration =
        crate::runtime_store::migrate(&runtime, &crate::runtime_store::legacy_path(&legacy_root))?;

    // Install the silent git hooks (post-commit/checkout/push/merge) so the
    // LLM using git normally is captured into the log asynchronously. A
    // conflict (a pre-existing non-hugit hook) is reported, never clobbered.
    let hooks = install_hooks(&root)?;

    let hint = if created {
        "initialized empty runtime event log; next: `hugit campaign open --campaign <key> \
         --charter <text> --owner <you>` then `hugit intent new …`"
    } else {
        "runtime event log already exists; left untouched (init is idempotent)"
    };

    let git_hint = if git_created {
        "git repository created (`git init` ran)"
    } else {
        "git repository already present"
    };

    Ok(json!({
        "initialized": created,
        "hooks_installed": hooks.installed,
        "hooks_noop": hooks.noop,
        "hooks_conflict": hooks.conflict,
        "git_created": git_created,
        "git_hint": git_hint,
        "runtime_dir": runtime.root.display().to_string(),
        "log": log_path.display().to_string(),
        "migration": migration,
        "hint": hint,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "hugit-init-{}-{}-{:?}",
            tag,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn init_creates_dir_and_empty_log_array_and_git_repo() {
        let root = scratch("create");
        let v = do_run(&InitArgs {
            dir: Some(root.clone()),
        })
        .expect("init ok");
        assert_eq!(v["initialized"], true);
        assert_eq!(
            v["git_created"], true,
            "git init should have run on a bare dir"
        );
        let log = crate::runtime_store::for_repo(&root)
            .unwrap()
            .canonical_log();
        assert!(log.is_file(), "log file created");
        let bytes = std::fs::read(&log).unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert!(
            parsed.as_array().is_some_and(|a| a.is_empty()),
            "log is an empty JSON array"
        );
        assert!(log.parent().unwrap().is_dir(), "runtime directory created");
        assert!(
            root.join(".git").is_dir(),
            "git repo created by `hugit init`"
        );
    }

    #[test]
    fn init_leaves_existing_git_repo_untouched() {
        let root = scratch("existing-git");
        // Pre-create a git repo (git init).
        let status = std::process::Command::new("git")
            .arg("init")
            .arg(&root)
            .status();
        if status.map(|s| !s.success()).unwrap_or(true) {
            eprintln!("SKIP: system `git` unavailable");
            return;
        }
        // Seed a git commit marker file so we can prove git state was not clobbered.
        std::fs::write(root.join("tracked.txt"), "x").unwrap();
        std::process::Command::new("git")
            .args(["-C", root.to_str().unwrap(), "add", "tracked.txt"])
            .status()
            .unwrap();

        let v = do_run(&InitArgs {
            dir: Some(root.clone()),
        })
        .expect("init ok on existing git repo");
        assert_eq!(v["git_created"], false, "existing repo left alone");
        assert!(root.join(".git").is_dir(), ".git still there");
        assert!(
            crate::runtime_store::for_repo(&root)
                .unwrap()
                .canonical_log()
                .is_file(),
            "log created"
        );
    }

    #[test]
    fn init_migrates_valid_legacy_log_into_git_runtime() {
        let root = scratch("migrate-legacy");
        git_init(&root).expect("git init");
        let legacy = crate::runtime_store::legacy_path(&root);
        std::fs::create_dir_all(legacy.parent().expect("legacy parent")).unwrap();
        let legacy_bytes = b"[]\n";
        std::fs::write(&legacy, legacy_bytes).unwrap();

        let value = do_run(&InitArgs {
            dir: Some(root.clone()),
        })
        .expect("migrate valid legacy log");
        let runtime = crate::runtime_store::for_repo(&root).unwrap();
        assert_eq!(std::fs::read(runtime.legacy_log()).unwrap(), legacy_bytes);
        let records: Vec<serde_json::Value> =
            serde_json::from_slice(&std::fs::read(runtime.canonical_log()).unwrap()).unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0]["kind"], crate::runtime_store::LEGACY_PREFIX_KIND);
        assert_eq!(
            value["migration"]["migration"]["legacy_prefix_sha256"],
            crate::runtime_store::sha256_hex(legacy_bytes)
        );
    }

    #[test]
    fn init_from_linked_worktree_migrates_main_worktree_legacy_log() {
        let root = scratch("linked-legacy");
        git_init(&root).expect("git init");
        for args in [
            ["config", "user.name", "test"].as_slice(),
            ["config", "user.email", "test@example.com"].as_slice(),
            ["commit", "--allow-empty", "-m", "seed"].as_slice(),
        ] {
            assert!(
                std::process::Command::new("git")
                    .args(args)
                    .current_dir(&root)
                    .status()
                    .expect("run git")
                    .success()
            );
        }
        let main_legacy = crate::runtime_store::legacy_path(&root);
        std::fs::create_dir_all(main_legacy.parent().expect("legacy parent")).unwrap();
        std::fs::write(&main_legacy, b"[]\n").unwrap();
        let linked = root.with_file_name("hugit-init-linked-legacy");
        let _ = std::fs::remove_dir_all(&linked);
        assert!(
            std::process::Command::new("git")
                .args(["worktree", "add", "-b", "linked-legacy"])
                .arg(&linked)
                .current_dir(&root)
                .status()
                .expect("create linked worktree")
                .success()
        );
        let linked_legacy = crate::runtime_store::legacy_path(&linked);
        std::fs::create_dir_all(linked_legacy.parent().expect("linked legacy parent")).unwrap();
        std::fs::write(&linked_legacy, b"not json").unwrap();

        do_run(&InitArgs { dir: Some(linked) }).expect("linked init must use main legacy source");
        assert_eq!(std::fs::read(&main_legacy).unwrap(), b"[]\n");
        let runtime = crate::runtime_store::for_repo(&root).unwrap();
        let records: Vec<serde_json::Value> =
            serde_json::from_slice(&std::fs::read(runtime.canonical_log()).unwrap()).unwrap();
        assert_eq!(records[0]["kind"], crate::runtime_store::LEGACY_PREFIX_KIND);
    }

    #[test]
    fn init_installs_managed_hooks_into_git_hooks_dir() {
        let root = scratch("install-hooks");
        do_run(&InitArgs {
            dir: Some(root.clone()),
        })
        .expect("init installs hooks");
        let hooks_dir = resolve_hooks_dir(&root).unwrap();

        for kind in HOOK_KINDS {
            let contents = std::fs::read_to_string(hooks_dir.join(kind)).unwrap();
            assert!(is_managed_hook(kind, &contents), "{kind} is managed");
            assert!(
                contents.contains("$COMMON/hugit"),
                "{kind} uses Git runtime"
            );
        }
    }

    #[test]
    fn init_is_idempotent_and_never_clobbers() {
        let root = scratch("idem");
        do_run(&InitArgs {
            dir: Some(root.clone()),
        })
        .expect("first init");
        // Seed valid canonical bytes so re-init proves it does not clobber state.
        let log = crate::runtime_store::for_repo(&root)
            .unwrap()
            .canonical_log();
        let seeded = b"[]\n";
        std::fs::write(&log, seeded).unwrap();

        let v = do_run(&InitArgs {
            dir: Some(root.clone()),
        })
        .expect("second init ok");
        assert_eq!(v["initialized"], false, "re-init reports not-created");
        // The seeded bytes survive — init never clobbers an existing log.
        assert_eq!(
            std::fs::read(&log).unwrap(),
            seeded,
            "existing log left untouched"
        );
    }

    #[test]
    fn rendered_hooks_pass_only_snapshots_to_detached_children() {
        for kind in HOOK_KINDS {
            let script = hook_script(kind);
            assert!(
                script.contains("</dev/null"),
                "{kind} detaches with stdin closed"
            );
            assert!(
                !script.contains("nohup"),
                "{kind} does not retain inherited input"
            );
            assert!(!script.contains("cat)"), "{kind} never captures raw stdin");
            let path = scratch(&format!("hook-syntax-{kind}")).join(kind);
            std::fs::write(&path, script).unwrap();
            assert!(
                std::process::Command::new("sh")
                    .args(["-n", path.to_str().unwrap()])
                    .status()
                    .is_ok_and(|status| status.success()),
                "{kind} is valid POSIX shell"
            );
        }
        let push = hook_script("pre-push");
        assert!(
            !push.contains("--remote") && !push.contains("--url") && !push.contains("--userinfo"),
            "remote URL/userinfo never enters capture args"
        );
        assert!(push.contains("--push-tuples \"$TUPLES\""));
        assert!(push.contains("dd bs=1 count=8193"));
        assert!(
            !push.contains("read -r"),
            "pre-push never permits shell read to buffer unbounded input"
        );
        let dispatcher = dispatcher_script("pre-push", std::path::Path::new("/tmp/foreign-hook"));
        assert!(dispatcher.contains("dd if=\"$TMP\" bs=1"));
        assert!(dispatcher.contains("[ \"$STATUS\" -eq 0 ] || exit \"$STATUS\""));
        assert!(dispatcher.contains("HUGIT_PRE_PUSH_TUPLES"));
    }
}
