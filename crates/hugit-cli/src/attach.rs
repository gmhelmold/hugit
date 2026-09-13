//! Safe coexistence for capture hooks already owned by another tool.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::init::{HOOK_KINDS, HUGIT_HOOK_MARKER, hook_script};
use crate::porcelain::PorcelainError;

const MANIFEST_VERSION: u32 = 1;
const DISPATCHER_MARKER: &str = "# hugit-hook-dispatcher v1";

#[derive(clap::Args, Debug)]
pub struct AttachArgs {
    /// Repository to inspect (defaults to current directory).
    #[arg(long)]
    pub repo: Option<PathBuf>,
    /// Report exact changes and adoption tokens. Never writes.
    #[arg(long, conflicts_with_all = ["adopt_managed_dispatcher", "detach"])]
    pub preview: bool,
    /// Adopt only dispatcher whose preview token exactly matches current hook bytes.
    #[arg(long, value_name = "PREVIEW_TOKEN", conflicts_with = "detach")]
    pub adopt_managed_dispatcher: Option<String>,
    /// Restore only byte-matching hooks recorded in manifest.
    #[arg(long)]
    pub detach: bool,
}

#[derive(Debug, Serialize, Deserialize)]
struct Manifest {
    version: u32,
    hooks_path: String,
    hooks: Vec<HookRecord>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct HookRecord {
    kind: String,
    dispatcher_sha256: String,
    foreign_backup: Option<String>,
    foreign_sha256: Option<String>,
    foreign_executable: bool,
}

pub fn run(args: AttachArgs) -> ExitCode {
    match do_run(&args) {
        Ok(v) => {
            println!("{v}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            println!("{}", e.to_json());
            e.exit_code()
        }
    }
}

fn do_run(args: &AttachArgs) -> Result<Value, PorcelainError> {
    let root = args.repo.clone().unwrap_or_else(|| PathBuf::from("."));
    let hooks = hooks_dir(&root)?;
    let runtime = common_dir(&root)?.join("hugit-runtime").join("hooks-v1");
    let manifest_path = runtime.join("manifest.json");
    if args.detach {
        return detach(&hooks, &manifest_path);
    }

    let existing = load_manifest(&manifest_path, &hooks)?;
    let mut preview = Vec::new();
    let mut adopt = Vec::new();
    for kind in HOOK_KINDS {
        let path = hooks.join(kind);
        let bytes = read_regular_or_absent(&path)?;
        let Some(bytes) = bytes else {
            preview.push(json!({"kind": kind, "action": "install"}));
            adopt.push((kind, None, None));
            continue;
        };
        let previous = existing.as_ref().and_then(|m| {
            m.hooks
                .iter()
                .find(|r| r.kind == kind && r.dispatcher_sha256 == digest(&bytes))
                .cloned()
        });
        if previous.is_some() {
            preview.push(json!({"kind": kind, "action": "upgrade_hugit_owned"}));
            adopt.push((kind, None, previous));
        } else {
            let token = token(kind, &bytes);
            let action = if bytes
                .windows(HUGIT_HOOK_MARKER.len())
                .any(|line| line == HUGIT_HOOK_MARKER.as_bytes())
            {
                "legacy_marker_requires_adoption"
            } else {
                "adopt_foreign_dispatcher"
            };
            preview.push(json!({"kind": kind, "action": action, "preview_token": token}));
            adopt.push((kind, Some((bytes, token, is_executable(&path)?)), None));
        }
    }
    if args.preview {
        return Ok(json!({"preview": true, "hooks_path": hooks, "changes": preview}));
    }
    let requested = args.adopt_managed_dispatcher.as_deref();
    if requested.is_none() && adopt.iter().any(|(_, foreign, _)| foreign.is_some()) {
        return Err(PorcelainError::new(
            "adoption_required",
            "foreign hooks found; attach did not modify them",
            "run `hugit attach --preview`, then pass every shown --adopt-managed-dispatcher token",
        ));
    }
    fs::create_dir_all(runtime.join("foreign"))
        .map_err(|e| PorcelainError::io("create hook runtime", &runtime, &e))?;
    fs::create_dir_all(&hooks).map_err(|e| PorcelainError::io("create hooks dir", &hooks, &e))?;
    let mut records = Vec::new();
    for (kind, foreign, previous) in adopt {
        let backup = if let Some(previous) = previous {
            previous.foreign_backup.map(|backup| {
                (
                    PathBuf::from(backup),
                    previous.foreign_sha256.unwrap_or_default(),
                    previous.foreign_executable,
                )
            })
        } else if let Some((bytes, expected, executable)) = foreign {
            if requested != Some(expected.as_str()) {
                return Err(PorcelainError::new(
                    "adoption_token_mismatch",
                    format!("{kind} changed or token is not its preview token"),
                    "rerun `hugit attach --preview`; adoption is hash-bound and one hook per invocation",
                ));
            }
            let digest = digest(&bytes);
            let backup = runtime.join("foreign").join(format!("{kind}-{digest}"));
            // Immutable content-addressed backup: never replace an existing path.
            if !backup.exists() {
                let mut f = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&backup)
                    .map_err(|e| PorcelainError::io("create foreign hook backup", &backup, &e))?;
                f.write_all(&bytes)
                    .map_err(|e| PorcelainError::io("write foreign hook backup", &backup, &e))?;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    fs::set_permissions(
                        &backup,
                        fs::Permissions::from_mode(if executable { 0o555 } else { 0o444 }),
                    )
                    .map_err(|e| PorcelainError::io("protect foreign hook backup", &backup, &e))?;
                }
            }
            Some((backup, digest, executable))
        } else {
            None
        };
        let script = dispatcher(
            kind,
            backup
                .as_ref()
                .and_then(|(p, _, executable)| executable.then_some(p.as_path())),
        );
        atomic_hook(&hooks.join(kind), script.as_bytes())?;
        records.push(HookRecord {
            kind: kind.to_string(),
            dispatcher_sha256: digest(script.as_bytes()),
            foreign_backup: backup.as_ref().map(|(p, _, _)| p.display().to_string()),
            foreign_sha256: backup.as_ref().map(|(_, d, _)| d.clone()),
            foreign_executable: backup.is_some_and(|(_, _, executable)| executable),
        });
    }
    atomic_json(
        &manifest_path,
        &Manifest {
            version: MANIFEST_VERSION,
            hooks_path: hooks.display().to_string(),
            hooks: records,
        },
    )?;
    Ok(json!({"attached": true, "hooks_path": hooks, "manifest": manifest_path}))
}

fn detach(hooks: &Path, manifest_path: &Path) -> Result<Value, PorcelainError> {
    let bytes = fs::read(manifest_path)
        .map_err(|e| PorcelainError::io("read hook manifest", manifest_path, &e))?;
    let manifest: Manifest = serde_json::from_slice(&bytes).map_err(|e| {
        PorcelainError::new(
            "invalid_hook_manifest",
            format!("invalid manifest {}: {e}", manifest_path.display()),
            "do not detach; repair or remove only after manual review",
        )
    })?;
    if manifest.version != MANIFEST_VERSION || manifest.hooks_path != hooks.display().to_string() {
        return Err(PorcelainError::new(
            "invalid_hook_manifest",
            "manifest does not belong to this active hooks path",
            "do not detach from a different repository or hooks path",
        ));
    }
    let mut restored = Vec::new();
    let mut preserved = Vec::new();
    for record in manifest.hooks {
        let path = hooks.join(&record.kind);
        let current = read_regular_or_absent(&path)?;
        if current.as_deref().map(digest) != Some(record.dispatcher_sha256) {
            preserved.push(record.kind);
            continue;
        }
        if let Some(backup) = record.foreign_backup {
            let b = fs::read(&backup).map_err(|e| {
                PorcelainError::io("read foreign hook backup", Path::new(&backup), &e)
            })?;
            if Some(digest(&b)) != record.foreign_sha256 {
                return Err(PorcelainError::new(
                    "foreign_backup_changed",
                    format!("foreign backup changed: {backup}"),
                    "detach refuses altered backup; restore it manually after review",
                ));
            }
            atomic_hook(&path, &b)?;
            #[cfg(unix)]
            if !record.foreign_executable {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(&path, fs::Permissions::from_mode(0o644))
                    .map_err(|e| PorcelainError::io("restore hook mode", &path, &e))?;
            }
        } else {
            fs::remove_file(&path)
                .map_err(|e| PorcelainError::io("remove hugit hook", &path, &e))?;
        }
        restored.push(record.kind);
    }
    Ok(json!({"detached": true, "restored": restored, "preserved": preserved}))
}

fn load_manifest(path: &Path, hooks: &Path) -> Result<Option<Manifest>, PorcelainError> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(PorcelainError::io("read hook manifest", path, &e)),
    };
    let manifest: Manifest = serde_json::from_slice(&bytes).map_err(|e| {
        PorcelainError::new(
            "invalid_hook_manifest",
            format!("invalid manifest {}: {e}", path.display()),
            "do not upgrade hooks until manifest is repaired or reviewed",
        )
    })?;
    if manifest.version != MANIFEST_VERSION || manifest.hooks_path != hooks.display().to_string() {
        return Err(PorcelainError::new(
            "invalid_hook_manifest",
            "manifest does not belong to this active hooks path",
            "do not upgrade hooks from a different repository or hooks path",
        ));
    }
    Ok(Some(manifest))
}

fn dispatcher(kind: &str, backup: Option<&Path>) -> String {
    let foreign = backup.map(shell_quote).unwrap_or_default();
    let hugit = hook_script(kind);
    let body = hugit.lines().skip(1).collect::<Vec<_>>().join("\n");
    if kind == "pre-push" && !foreign.is_empty() {
        let body = body.trim_end().strip_suffix("exit 0").unwrap_or(&body);
        format!(
            "#!/bin/sh\n{DISPATCHER_MARKER}\nRUNTIME=$(dirname {})\nINPUT=$(mktemp \"$RUNTIME/.pre-push.XXXXXX\") || exec {} \"$@\"\ncat >\"$INPUT\"\n{} \"$@\" <\"$INPUT\"\nSTATUS=$?\nHUGIT_PRE_PUSH_FILE=\"$INPUT\"\n{}\nrm -f \"$INPUT\"\nexit $STATUS\n",
            foreign, foreign, foreign, body
        )
    } else if !foreign.is_empty() {
        let body = body.trim_end().strip_suffix("exit 0").unwrap_or(&body);
        format!(
            "#!/bin/sh\n{DISPATCHER_MARKER}\n{} \"$@\"\nSTATUS=$?\n{}\nexit $STATUS\n",
            foreign, body
        )
    } else {
        format!("#!/bin/sh\n{body}\n")
    }
}
fn shell_quote(path: &Path) -> String {
    format!(
        "'{}'",
        path.display().to_string().replace('\'', "'\\\"'\\\"'")
    )
}

fn hooks_dir(root: &Path) -> Result<PathBuf, PorcelainError> {
    git_path(
        root,
        &["rev-parse", "--git-path", "hooks"],
        "resolve hooks dir",
    )
}
fn common_dir(root: &Path) -> Result<PathBuf, PorcelainError> {
    git_path(
        root,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
        "resolve git common dir",
    )
}
fn git_path(root: &Path, args: &[&str], what: &str) -> Result<PathBuf, PorcelainError> {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .map_err(|e| PorcelainError::io(what, root, &e))?;
    if !out.status.success() {
        return Err(PorcelainError::new(
            "git_hooks_dir_failed",
            format!("`git {}` failed", args.join(" ")),
            "run this inside an existing Git repository",
        ));
    }
    let p = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());
    Ok(if p.is_absolute() { p } else { root.join(p) })
}
fn read_regular_or_absent(path: &Path) -> Result<Option<Vec<u8>>, PorcelainError> {
    match fs::symlink_metadata(path) {
        Ok(m) if m.is_file() => fs::read(path)
            .map(Some)
            .map_err(|e| PorcelainError::io("read hook", path, &e)),
        Ok(_) => Err(PorcelainError::new(
            "unsafe_hook_path",
            format!("refusing non-regular hook path {path:?}"),
            "replace hook path with a regular file, then retry",
        )),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(PorcelainError::io("stat hook", path, &e)),
    }
}
fn is_executable(path: &Path) -> Result<bool, PorcelainError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::metadata(path)
            .map(|m| m.permissions().mode() & 0o111 != 0)
            .map_err(|e| PorcelainError::io("stat hook", path, &e))
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Ok(true)
    }
}
fn digest(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}
fn token(kind: &str, bytes: &[u8]) -> String {
    format!("{kind}:{}", digest(bytes))
}
fn atomic_hook(path: &Path, bytes: &[u8]) -> Result<(), PorcelainError> {
    let tmp = path.with_extension(format!("hugit-{}", std::process::id()));
    fs::write(&tmp, bytes).map_err(|e| PorcelainError::io("write hook", &tmp, &e))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&tmp, fs::Permissions::from_mode(0o755))
            .map_err(|e| PorcelainError::io("chmod hook", &tmp, &e))?;
    }
    fs::rename(&tmp, path).map_err(|e| PorcelainError::io("install hook", path, &e))
}
fn atomic_json(path: &Path, value: &Manifest) -> Result<(), PorcelainError> {
    let tmp = path.with_extension(format!("tmp-{}", std::process::id()));
    let body = serde_json::to_vec_pretty(value)
        .map_err(|e| PorcelainError::internal(format!("serialise hook manifest: {e}")))?;
    fs::write(&tmp, body).map_err(|e| PorcelainError::io("write hook manifest", &tmp, &e))?;
    fs::rename(&tmp, path).map_err(|e| PorcelainError::io("install hook manifest", path, &e))
}
