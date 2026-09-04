//! The live GitHub mirror push lane (WP-E1a item ①) — driven by the REAL `git`
//! binary via shell-out.
//!
//! [`LiveGitHubTarget`] implements [`MirrorPushTarget`] with two `git`
//! invocations per push: `git push --no-verify` to move the ref, then
//! `git ls-remote` to re-read the oid the mirror holds for that ref. The
//! returned oid is the OBSERVED mirror state — the caller (the outbound
//! writer) performs the content-hash byte-compare. This target never claims
//! verification; it only reports what the mirror actually holds.
//!
//! # Hermeticity
//!
//! Every `git` invocation pins `GIT_TERMINAL_PROMPT=0` so a missing credential
//! can never hang the lane waiting on a password prompt. Tests additionally
//! null out both global and system git config so no ambient host config can
//! leak into the hermetic fixtures.
//!
//! # Secret hygiene
//!
//! The authenticated remote URL carries the installation token as the
//! `x-access-token` username. That URL is internal to this target and its
//! caller; it is NEVER copied into a [`PushError`] reason — failure detail is
//! stripped stderr with the token scrubbed verbatim.

use std::path::PathBuf;
use std::process::Command;

use super::writer::{MirrorPushTarget, PushError};
use crate::verify::ContentHash;

/// The real-`git` mirror push target.
///
/// Pushes refs from `source_repo` to `remote_url` (an authenticated GitHub
/// `https://` URL in production; a local path in hermetic tests), then
/// re-reads the observed tip via `ls-remote`. The observed oid is returned
/// AS-IS; the caller verifies byte-identity.
#[derive(Debug, Clone)]
pub struct LiveGitHubTarget {
    /// The local repo the refs are pushed FROM.
    source_repo: PathBuf,
    /// The mirror remote (authenticated HTTPS URL in production; local path in tests).
    remote_url: String,
    /// The `git` executable (overridable for hermetic alternate-binary tests).
    git_bin: PathBuf,
}

/// The default `git` executable name.
const GIT_BIN: &str = "git";

/// Upper bound on a [`PushError`] detail, in chars — a noisy `git` error must
/// never flood the divergence log.
const MAX_DETAIL_CHARS: usize = 240;

impl LiveGitHubTarget {
    /// Create a target pushing refs from `source_repo` to `remote_url`.
    pub fn new(source_repo: impl Into<PathBuf>, remote_url: impl Into<String>) -> Self {
        Self {
            source_repo: source_repo.into(),
            remote_url: remote_url.into(),
            git_bin: PathBuf::from(GIT_BIN),
        }
    }

    /// Override the `git` executable (hermetic alternate-binary tests).
    pub fn with_git_bin(mut self, bin: impl Into<PathBuf>) -> Self {
        self.git_bin = bin.into();
        self
    }

    /// A `git` command pinned for this lane, run in `current_dir`.
    ///
    /// Terminal prompts are always disabled (a credential-gated push fails
    /// closed instead of hanging). In tests the global/system config are also
    /// nulled so ambient host config cannot leak into fixtures.
    fn git_command(&self, current_dir: &std::path::Path) -> Command {
        let mut cmd = Command::new(&self.git_bin);
        cmd.current_dir(current_dir).env("GIT_TERMINAL_PROMPT", "0");
        #[cfg(test)]
        {
            cmd.env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_CONFIG_SYSTEM", "/dev/null");
        }
        cmd
    }

    /// Collapse + bound a failure detail, SCRUBBING the installation token if it
    /// appears (the token rides the remote URL's username; git error text must
    /// never echo it). `git` outputs are stripped of trailing whitespace.
    fn sanitize_detail(&self, text: &str) -> String {
        let token = self
            .remote_url
            .split_once("x-access-token:")
            .and_then(|(_, rest)| rest.split_once('@').map(|(tok, _)| tok))
            .unwrap_or("");
        let scrubbed = if token.is_empty() {
            text.to_string()
        } else {
            text.replace(token, "<redacted>")
        };
        let joined = scrubbed.split_whitespace().collect::<Vec<_>>().join(" ");
        let mut out = String::with_capacity(joined.len().min(MAX_DETAIL_CHARS));
        for ch in joined.chars().take(MAX_DETAIL_CHARS) {
            out.push(ch);
        }
        if joined.chars().count() > MAX_DETAIL_CHARS {
            out.push('…');
        }
        out
    }

    /// Build a rejection for a failed step, with the sanitised detail.
    fn reject(&self, ref_name: &str, fallback: &str) -> PushError {
        let mut detail = self.sanitize_detail(fallback);
        if detail.is_empty() {
            detail = format!("git refused to push {ref_name}");
        }
        PushError::Rejected {
            ref_name: ref_name.to_string(),
            detail,
        }
    }
}

impl MirrorPushTarget for LiveGitHubTarget {
    fn push_ref(&mut self, ref_name: &str, _oid: &ContentHash) -> Result<ContentHash, PushError> {
        // 1. Move the ref on the mirror. The expected oid is NOT passed to git —
        // the source branch resolves its own tip; the caller's expected hash is
        // checked against what we observe below.
        let dst = format!("{ref_name}:{ref_name}");
        let args: [&str; 4] = [
            "push",
            "--no-verify",
            self.remote_url.as_str(),
            dst.as_str(),
        ];
        let out = self.git_command(&self.source_repo).args(args).output();
        let out = match out {
            Ok(o) => o,
            Err(e) => {
                return Err(self.reject(ref_name, &format!("git push could not be started: {e}")));
            }
        };
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr);
            let fallback = if stderr.trim().is_empty() {
                String::from_utf8_lossy(&out.stdout)
            } else {
                stderr
            };
            return Err(self.reject(ref_name, &fallback));
        }

        // 2. Re-read the tip the mirror holds for the ref AFTER the push.
        let args: [&str; 3] = ["ls-remote", self.remote_url.as_str(), ref_name];
        let out = self.git_command(&self.source_repo).args(args).output();
        let out = match out {
            Ok(o) => o,
            Err(e) => {
                return Err(self.reject(
                    ref_name,
                    &format!("git ls-remote could not be started: {e}"),
                ));
            }
        };
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr);
            return Err(self.reject(ref_name, &stderr));
        }

        // 3. The first whitespace token of ls-remote stdout is the observed oid.
        let stdout = String::from_utf8_lossy(&out.stdout);
        let observed = stdout.split_whitespace().next().unwrap_or("");
        if observed.is_empty() {
            // Fail-CLOSED: a push that left no observable ref is a hard
            // rejection — never a silent Ok with an empty hash.
            return Err(PushError::Rejected {
                ref_name: ref_name.to_string(),
                detail: format!(
                    "mirror reports no oid for {ref_name} after the push (ref absent; \
                     fail-closed, not marked synced)"
                ),
            });
        }
        Ok(ContentHash::new(observed))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    /// A unique temp dir, removed on drop.
    struct TmpDir(PathBuf);

    impl TmpDir {
        fn new(tag: &str) -> Self {
            let mut p = std::env::temp_dir();
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            p.push(format!(
                "hugit-mirror-live-{tag}-{nanos}-{}",
                std::process::id()
            ));
            std::fs::create_dir_all(&p).unwrap();
            TmpDir(p)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TmpDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// Deterministic, hermetic `git` — no ambient config, pinned identity.
    fn git_command(cwd: &Path) -> Command {
        let mut cmd = Command::new("git");
        cmd.current_dir(cwd)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_SYSTEM", "/dev/null")
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_AUTHOR_NAME", "hugit")
            .env("GIT_AUTHOR_EMAIL", "bot@hugit.dev")
            .env("GIT_COMMITTER_NAME", "hugit")
            .env("GIT_COMMITTER_EMAIL", "bot@hugit.dev");
        cmd
    }

    /// Run `git` in `cwd`, asserting success; return trimmed stdout.
    fn git(cwd: &Path, args: &[&str]) -> String {
        let out = git_command(cwd)
            .args(args)
            .output()
            .expect("git must be on PATH");
        assert!(
            out.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    /// A hermetic two-sided fixture: a normal source repo + a bare mirror repo.
    struct Harness {
        _tmp: TmpDir,
        source: PathBuf,
        mirror: PathBuf,
    }

    impl Harness {
        fn new(tag: &str) -> Self {
            let tmp = TmpDir::new(tag);
            let source = tmp.path().join("source");
            let mirror = tmp.path().join("mirror.git");
            std::fs::create_dir_all(&source).unwrap();
            git(&source, &["init", "-q", "-b", "main"]);
            std::fs::create_dir_all(&mirror).unwrap();
            git(&mirror, &["init", "-q", "--bare", "-b", "main"]);
            Harness {
                _tmp: tmp,
                source,
                mirror,
            }
        }

        /// Commit `content` onto `main`; return the new real HEAD oid.
        fn commit(&self, content: &str) -> String {
            std::fs::write(self.source.join("f.txt"), content).unwrap();
            git(&self.source, &["add", "f.txt"]);
            git(
                &self.source,
                &["commit", "-q", "-m", &format!("c:{content}")],
            );
            git(&self.source, &["rev-parse", "HEAD"])
        }

        /// Whether the mirror currently holds `ref_name`.
        fn mirror_has(&self, ref_name: &str) -> bool {
            let out = git_command(&self.mirror)
                .args(["show-ref", "--verify", "--quiet", ref_name])
                .output()
                .expect("git show-ref");
            out.status.success()
        }
    }

    #[test]
    fn pushes_a_ref_to_a_real_local_bare_mirror_and_observes_the_tip() {
        let h = Harness::new("tip");
        let head1 = h.commit("one");

        let mut target = LiveGitHubTarget::new(h.source.as_path(), h.mirror.to_string_lossy());
        let observed1 = target
            .push_ref("refs/heads/main", &ContentHash::new(&head1))
            .expect("first push must land");
        assert_eq!(
            observed1,
            ContentHash::new(&head1),
            "the mirror must observe the source HEAD byte-identically"
        );
        assert!(
            h.mirror_has("refs/heads/main"),
            "the ref exists on the mirror"
        );

        // A second commit on top; push again → the mirror observes the new tip.
        let head2 = h.commit("two");
        assert_ne!(head2, head1);
        let observed2 = target
            .push_ref("refs/heads/main", &ContentHash::new(&head2))
            .expect("second push must land");
        assert_eq!(
            observed2,
            ContentHash::new(&head2),
            "the mirror must fast-forward to the new source HEAD"
        );
        assert!(h.mirror_has("refs/heads/main"));
    }

    #[test]
    fn absent_ref_is_hard_rejection() {
        let h = Harness::new("absent");
        let head1 = h.commit("one");

        // Positive control: a first real push lands, so the lane itself works.
        let mut target = LiveGitHubTarget::new(h.source.as_path(), h.mirror.to_string_lossy());
        let observed = target
            .push_ref("refs/heads/main", &ContentHash::new(&head1))
            .expect("first push must land");
        assert_eq!(observed, ContentHash::new(&head1));
        assert!(h.mirror_has("refs/heads/main"));

        // Arm the mirror to LOSE the ref it just accepted: a post-receive hook
        // that deletes every received ref AFTER receive-pack has acknowledged
        // it. `git push` then exits 0 (the server acked) yet the ref is absent
        // — the opaque-loss case the fail-CLOSED observe step must catch.
        // (A delete-then-repush does NOT reproduce this: real git re-creates an
        // absent ref, so the hook is the deterministic construction.)
        let hook = h.mirror.join("hooks").join("post-receive");
        std::fs::create_dir_all(hook.parent().unwrap()).unwrap();
        std::fs::write(
            &hook,
            "#!/bin/sh\nwhile read _old _new ref; do git update-ref -d \"$ref\"; done\nexit 0\n",
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&hook).unwrap().permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&hook, perms).unwrap();
        }

        // A real update push: the hook eats the ref, so it never sticks.
        let head2 = h.commit("two");
        match target.push_ref("refs/heads/main", &ContentHash::new(&head2)) {
            Err(PushError::Rejected { ref_name, detail }) => {
                assert_eq!(ref_name, "refs/heads/main");
                assert!(
                    !detail.is_empty(),
                    "the reject reason must carry the failure detail"
                );
            }
            other => {
                panic!("a push whose ref never sticks must be a hard rejection, got {other:?}")
            }
        }
        assert!(
            !h.mirror_has("refs/heads/main"),
            "the mirror genuinely dropped the ref — the rejection isn't masking a landed ref"
        );
    }
}
