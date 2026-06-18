//! `git-ingest` — the git-objects → CoreLink CAS ingest tool (the git analog of
//! `hugit-snapshot`).
//!
//! Enumerates a local repo's full object closure (via `git rev-list` plus
//! `git cat-file`), frames each object as a git loose object
//! ([`hugit_serve::cas::encode_loose`]), keys it by BLAKE3-256 (the confirmed
//! CoreLink CAS key), and PUTs it to `{CAS_URL}/v1/cas/{tenant}/{blake3}`
//! (`cas:rw`). It then publishes the mutable manifests to hugit's R2:
//! `refs.json` (refs from `for-each-ref`, head from `symbolic-ref HEAD`) and
//! `oid-index.json` (the git-sha1-to-blake3 index the read-path loader
//! rehydrates from), both under the `<tenant>/<repo>/` prefix.
//!
//! Config: `HUGIT_SERVE_CAS_URL` / `HUGIT_SERVE_CAS_TENANT_ID` / the CAS PAT
//! (file `~/.hugit/secrets/corelink/pat` or `HUGIT_SERVE_CAS_PAT`), plus the
//! `HUGIT_SERVE_R2_*` cred for the manifest writes (a one-shot READ+WRITE grant —
//! the standing engine cred is read-only and PUTs 403). The CAS PUT needs a
//! `cas:rw`-scoped PAT.
//!
//! Usage: `git-ingest <git-dir> <repo-slug>`
//!   e.g. `git-ingest /path/to/hugit.git hugit`

use std::collections::BTreeMap;
use std::process::{Command, ExitCode};

use hugit_proto::ObjectKind;
use hugit_serve::cas::{CasClient, IngestObject, ingest_repo};
use hugit_serve::state::{R2Config, is_safe_repo_slug};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 3 {
        eprintln!("usage: git-ingest <git-dir> <repo-slug>");
        return ExitCode::from(2);
    }
    let (git_dir, repo) = (&args[1], &args[2]);

    if !is_safe_repo_slug(repo) {
        eprintln!("git-ingest: refusing unsafe repo slug {repo:?}");
        return ExitCode::from(2);
    }

    let tenant = match std::env::var("HUGIT_SERVE_CAS_TENANT_ID") {
        Ok(t) if !t.trim().is_empty() => t,
        _ => {
            eprintln!("git-ingest: HUGIT_SERVE_CAS_TENANT_ID is not set");
            return ExitCode::from(2);
        }
    };

    // 1. Enumerate the repo (the only step that touches `git`).
    let (head, refs, objects) = match enumerate_repo(git_dir) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("git-ingest: {e}");
            return ExitCode::from(2);
        }
    };

    // 2. Build the CAS + R2 clients (CAS PUT needs cas:rw; R2 needs the RW grant).
    let cas = match CasClient::from_env() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("git-ingest: cannot configure CAS client — {e}");
            return ExitCode::from(2);
        }
    };
    let r2 = match R2Config::from_env() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("git-ingest: cannot configure R2 (manifest store) — {e}");
            return ExitCode::from(2);
        }
    };

    // 3. Ingest: PUT every object under its blake3, then publish the manifests.
    match ingest_repo(&cas, &r2, &tenant, repo, &head, &refs, &objects) {
        Ok(n) => {
            println!(
                "git-ingest: ingested {n} objects → CAS ({tenant}/{repo}), \
                 published refs.json + oid-index.json"
            );
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("git-ingest: ingest failed — {e}");
            ExitCode::FAILURE
        }
    }
}

/// The enumerated repo state: `(head_refname, refs, object closure)`.
type RepoEnumeration = (String, BTreeMap<String, String>, Vec<IngestObject>);

/// Enumerate a local git dir's HEAD ref, ref map, and full object closure via the
/// local `git` binary (the same `cat-file`/`rev-list` plumbing `load_git_dir`
/// uses). Returns `(head_refname, refs, objects)`. Fail-closed on any `git` error.
fn enumerate_repo(git_dir: &str) -> Result<RepoEnumeration, String> {
    let git = |args: &[&str]| -> Result<Vec<u8>, String> {
        let out = Command::new("git")
            .arg("-C")
            .arg(git_dir)
            .args(args)
            .output()
            .map_err(|e| format!("failed to spawn `git`: {e}"))?;
        if !out.status.success() {
            return Err(format!(
                "`git {}` failed: {}",
                args.join(" "),
                String::from_utf8_lossy(&out.stderr).trim()
            ));
        }
        Ok(out.stdout)
    };

    // HEAD's symbolic ref (e.g. `refs/heads/main`).
    let head = String::from_utf8(git(&["symbolic-ref", "HEAD"])?)
        .map_err(|e| format!("symbolic-ref HEAD is not UTF-8: {e}"))?
        .trim()
        .to_string();

    // The full ref map (`<refname> <objectname>` per line).
    let refs_listing =
        String::from_utf8(git(&["for-each-ref", "--format=%(refname) %(objectname)"])?)
            .map_err(|e| format!("for-each-ref output is not UTF-8: {e}"))?;
    let mut refs = BTreeMap::new();
    for line in refs_listing.lines() {
        let mut it = line.split_whitespace();
        if let (Some(name), Some(oid)) = (it.next(), it.next()) {
            refs.insert(name.to_string(), oid.to_string());
        }
    }

    // The full object closure reachable from ANY ref (`--all`).
    let listing = String::from_utf8(git(&["rev-list", "--objects", "--all"])?)
        .map_err(|e| format!("rev-list output is not UTF-8: {e}"))?;
    let mut objects = Vec::new();
    for line in listing.lines() {
        let oid_hex = line.split_whitespace().next().unwrap_or("");
        if oid_hex.is_empty() {
            continue;
        }
        let kind_raw = git(&["cat-file", "-t", oid_hex])?;
        let kind_str = String::from_utf8_lossy(&kind_raw);
        let kind = match kind_str.trim() {
            "blob" => ObjectKind::Blob,
            "tree" => ObjectKind::Tree,
            "commit" => ObjectKind::Commit,
            "tag" => ObjectKind::Tag,
            _ => continue,
        };
        let body = git(&["cat-file", kind_str.trim(), oid_hex])?;
        objects.push(IngestObject {
            git_oid: oid_hex.to_string(),
            kind,
            body,
        });
    }

    Ok((head, refs, objects))
}
