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

    // The full object closure reachable from ANY ref (`--all`), streamed through
    // ONE `git cat-file --batch` process — NOT two `git` spawns per object, which
    // turns a ~4k-object repo into ~8k process spawns (minutes, and effectively
    // wedged under CPU contention). `--batch` reads oids on stdin and emits, per
    // object, a header `<oid> SP <type> SP <size> LF` then `<size>` raw body bytes
    // and a trailing LF. A writer thread feeds oids while we drain stdout (and a
    // third drains stderr) so the OS pipe buffer can't deadlock on MB-sized bodies.
    // (Same plumbing as `hugit_serve::state::load_git_dir`.)
    let listing = String::from_utf8(git(&["rev-list", "--objects", "--all"])?)
        .map_err(|e| format!("rev-list output is not UTF-8: {e}"))?;
    let oids: Vec<String> = listing
        .lines()
        .filter_map(|l| l.split_whitespace().next())
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect();

    let mut objects = Vec::new();
    if !oids.is_empty() {
        use std::io::{Read, Write};

        let mut child = Command::new("git")
            .arg("-C")
            .arg(git_dir)
            .args(["cat-file", "--batch"])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| format!("failed to spawn `git cat-file --batch`: {e}"))?;

        let oids_owned = oids.clone();
        let mut stdin = child.stdin.take().expect("piped stdin");
        let writer = std::thread::spawn(move || -> std::io::Result<()> {
            for oid in &oids_owned {
                stdin.write_all(oid.as_bytes())?;
                stdin.write_all(b"\n")?;
            }
            Ok(()) // drop(stdin) closes the pipe so cat-file finishes
        });
        let mut stderr_pipe = child.stderr.take().expect("piped stderr");
        let errs = std::thread::spawn(move || {
            let mut s = String::new();
            let _ = stderr_pipe.read_to_string(&mut s);
            s
        });

        let mut out = Vec::new();
        child
            .stdout
            .take()
            .expect("piped stdout")
            .read_to_end(&mut out)
            .map_err(|e| format!("reading `git cat-file --batch`: {e}"))?;

        let writer_res = writer.join();
        let stderr_text = errs.join().unwrap_or_default();
        let status = child
            .wait()
            .map_err(|e| format!("`git cat-file --batch` wait: {e}"))?;
        match writer_res {
            Ok(Ok(())) => {}
            Ok(Err(e)) => return Err(format!("writing oids to `git cat-file --batch`: {e}")),
            Err(_) => return Err("the cat-file writer thread panicked".to_string()),
        }
        if !status.success() {
            return Err(format!(
                "`git cat-file --batch` failed: {}",
                stderr_text.trim()
            ));
        }

        // Parse repeated `<oid> SP <type> SP <size> LF <body> LF`.
        let mut i = 0usize;
        while i < out.len() {
            let nl = match out[i..].iter().position(|&b| b == b'\n') {
                Some(p) => i + p,
                None => break,
            };
            let header = std::str::from_utf8(&out[i..nl])
                .map_err(|_| "non-UTF-8 cat-file header".to_string())?;
            i = nl + 1;
            let mut parts = header.split(' ');
            let oid_hex = parts.next().unwrap_or("").to_string();
            let type_str = parts.next().unwrap_or("");
            if type_str == "missing" {
                continue; // no body follows
            }
            let size: usize = parts
                .next()
                .and_then(|s| s.parse().ok())
                .ok_or_else(|| format!("bad cat-file header {header:?}"))?;
            let kind = match type_str {
                "blob" => ObjectKind::Blob,
                "tree" => ObjectKind::Tree,
                "commit" => ObjectKind::Commit,
                "tag" => ObjectKind::Tag,
                _ => {
                    i += size + 1; // skip body + trailing LF for an unknown type
                    continue;
                }
            };
            if i + size > out.len() {
                return Err("truncated cat-file object body".to_string());
            }
            objects.push(IngestObject {
                git_oid: oid_hex,
                kind,
                body: out[i..i + size].to_vec(),
            });
            i += size;
            if i < out.len() && out[i] == b'\n' {
                i += 1; // trailing LF after the body
            }
        }
    }

    Ok((head, refs, objects))
}
