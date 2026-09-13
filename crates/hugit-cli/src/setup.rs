//! `hugit setup` — one-time global install of the git hooks via
//! `init.templateDir` (the "boot ceremony", no per-repo `hugit init`).
//!
//! Git has NO `post-init` hook, but it DOES copy the contents of
//! `init.templateDir` into every fresh `.git/` on `git init`. `setup` points
//! that global config at a hugit-owned template containing all capture hooks —
//! so from then on,
//! EVERY `git init` on this machine ships the hugit hooks automatically.
//!
//! Hooks require `hugit attach` or `hugit init` to provision verified runtime
//! state. They never create tracked working-tree state.
//!
//! `setup --repo` installs into an existing repository without changing Git
//! global configuration. Otherwise it writes ONE global config key + a template
//! directory the user owns. Idempotent: re-running rewrites same target.

use std::path::PathBuf;
use std::process::ExitCode;

use serde_json::{Value, json};

use super::init;

/// `hugit setup` args.
#[derive(clap::Args, Debug)]
pub struct SetupArgs {
    /// Optional explicit template dir to install instead of the default
    /// `~/.config/hugit/template` (for tests / custom layouts).
    #[arg(long, conflicts_with = "repo")]
    pub dir: Option<PathBuf>,
    /// Install hooks into this existing Git repository. Does not run `git init`.
    #[arg(long, conflicts_with_all = ["dir", "status", "replace_global_template"])]
    pub repo: Option<PathBuf>,
    /// Inspect global setup without changing Git configuration.
    #[arg(long, conflicts_with = "repo")]
    pub status: bool,
    /// Replace an existing global init.templateDir owned by another tool.
    #[arg(long, conflicts_with = "repo")]
    pub replace_global_template: bool,
}

/// Run `hugit setup`.
pub fn run(args: SetupArgs) -> ExitCode {
    let result = if args.status {
        do_status(&args)
    } else {
        do_setup(&args)
    };
    match result {
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

fn do_status(args: &SetupArgs) -> Result<Value, crate::porcelain::PorcelainError> {
    let template_dir = args.dir.clone().unwrap_or_else(default_template_dir);
    let configured = std::process::Command::new("git")
        .args(["config", "--global", "--get", "init.templateDir"])
        .output()
        .map_err(|e| crate::porcelain::PorcelainError::io("read git config", &template_dir, &e))?;
    let configured_path = if configured.status.success() {
        String::from_utf8_lossy(&configured.stdout)
            .trim()
            .to_string()
    } else {
        String::new()
    };
    let mut hooks = serde_json::Map::new();
    for kind in init::HOOK_KINDS {
        let path = template_dir.join("hooks").join(kind);
        hooks.insert(
            kind.replace('-', "_"),
            json!({"path": path.display().to_string(), "exists": path.exists(), "executable": hook_is_executable(&path)}),
        );
    }
    Ok(json!({
        "template_dir": template_dir.display().to_string(),
        "owned": template_dir.join("OWNED-BY-HUGIT").is_file(),
        "global_init_template_dir": configured_path,
        "active": configured_path == template_dir.display().to_string()
            && init::HOOK_KINDS.iter().all(|kind| hook_is_executable(&template_dir.join("hooks").join(kind))),
        "hooks": hooks,
    }))
}

fn do_setup(args: &SetupArgs) -> Result<Value, crate::porcelain::PorcelainError> {
    if let Some(repo) = &args.repo {
        let hooks = init::install_hooks(repo)?;
        return Ok(json!({
            "repo": repo.display().to_string(),
            "git_created": false,
            "hooks_installed": hooks.installed,
            "hooks_noop": hooks.noop,
            "hooks_conflict": hooks.conflict,
            "next": "git operations now capture through installed hugit hooks",
        }));
    }
    let template_dir = args.dir.clone().unwrap_or_else(default_template_dir);
    let previous_template = global_template_dir(&template_dir)?;
    let owns_template = owns_template(&template_dir);
    if template_dir.exists() && !owns_template {
        return Err(crate::porcelain::PorcelainError::new(
            "template_dir_conflict",
            format!(
                "template directory {} is not hugit-owned",
                template_dir.display()
            ),
            "choose an empty template directory; hugit never overwrites another tool's hooks",
        ));
    }
    let needs_replace = previous_template
        .as_deref()
        .is_some_and(|previous| previous != template_dir.display().to_string());
    if needs_replace && !args.replace_global_template {
        return Err(crate::porcelain::PorcelainError::new(
            "global_template_conflict",
            format!(
                "template directory is not hugit-owned or global init.templateDir points elsewhere: {previous_template:?}"
            ),
            "re-run with --replace-global-template only after reviewing that template",
        ));
    }
    let hooks_dir = template_dir.join("hooks");
    std::fs::create_dir_all(&hooks_dir)
        .map_err(|e| crate::porcelain::PorcelainError::io("create template dir", &hooks_dir, &e))?;

    for kind in init::HOOK_KINDS {
        let script = init::hook_script(kind);
        let dest = hooks_dir.join(kind);
        std::fs::write(&dest, script)
            .map_err(|e| crate::porcelain::PorcelainError::io("write template hook", &dest, &e))?;
        // Hooks must be executable for git to run them from a template.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o755)).map_err(
                |e| crate::porcelain::PorcelainError::io("chmod template hook", &dest, &e),
            )?;
        }
    }
    // Mark ownership so an operator can tell the template apart and, if ever
    // needed, uninstall it cleanly.
    std::fs::write(
        template_dir.join("OWNED-BY-HUGIT"),
        b"Generated by `hugit setup`; safe to delete. Points git's global init.templateDir at this dir.\n",
    )
    .map_err(|e| {
        crate::porcelain::PorcelainError::io("write template marker", &template_dir, &e)
    })?;

    // Point git's GLOBAL init.templateDir at it (idempotent overwrite).
    let git_out = std::process::Command::new("git")
        .args([
            "config",
            "--global",
            "init.templateDir",
            template_dir.to_str().unwrap_or(""),
        ])
        .output()
        .map_err(|e| crate::porcelain::PorcelainError::io("run git config", &template_dir, &e))?;
    if !git_out.status.success() {
        return Err(crate::porcelain::PorcelainError::new(
            "git_config_failed",
            "`git config --global init.templateDir` failed",
            "is git on PATH and is a repo not required for global config?",
        ));
    }

    Ok(json!({
        "template_dir": template_dir.display().to_string(),
        "hooks": init::HOOK_KINDS,
        "global_init_template_dir": true,
        "previous_global_init_template_dir": previous_template,
        "next": "any future `git init` in this machine already ships the hugit hooks; existing repos use `hugit init <dir>` or a first git op lazy-boots them.",
    }))
}

fn global_template_dir(
    template_dir: &std::path::Path,
) -> Result<Option<String>, crate::porcelain::PorcelainError> {
    let configured = std::process::Command::new("git")
        .args(["config", "--global", "--get", "init.templateDir"])
        .output()
        .map_err(|e| crate::porcelain::PorcelainError::io("read git config", template_dir, &e))?;
    if !configured.status.success() {
        return Ok(None);
    }
    let path = String::from_utf8_lossy(&configured.stdout)
        .trim()
        .to_string();
    Ok((!path.is_empty()).then_some(path))
}

fn owns_template(template_dir: &std::path::Path) -> bool {
    if !template_dir.join("OWNED-BY-HUGIT").is_file() {
        return false;
    }
    init::HOOK_KINDS.iter().all(|kind| {
        std::fs::read_to_string(template_dir.join("hooks").join(kind))
            .is_ok_and(|contents| init::is_managed_hook(kind, &contents))
    })
}

#[cfg(unix)]
fn hook_is_executable(path: &std::path::Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .map(|metadata| metadata.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn hook_is_executable(path: &std::path::Path) -> bool {
    path.is_file()
}

/// Default template dir under the user's config root.
fn default_template_dir() -> PathBuf {
    dirs_home_config("hugit").join("template")
}

/// `$XDG_CONFIG_HOME/hugit` else `~/.config/hugit`.
fn dirs_home_config(leaf: &str) -> PathBuf {
    if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME") {
        PathBuf::from(xdg).join(leaf)
    } else {
        let home = std::env::var_os("HOME").unwrap_or_else(|| "/tmp".into());
        PathBuf::from(home).join(".config").join(leaf)
    }
}
