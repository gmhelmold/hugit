//! Supply-chain enforcement on the **real** spawn surface: a runner image is
//! acceptable for spawn **only** if it is pinned by content digest
//! (`repo@sha256:<64-lowercase-hex>`), and that pin must be integrity-verified
//! against the digest the runner box actually resolves **before** any container
//! is spawned (fail-closed, verify-before-spawn).
//!
//! # Why this lives in `hugit-runner`
//! `hugit-invariants/x4` carries a `VerifiedSpawn` *wrapper* over the engine,
//! but the live spawn path (`ContainerSpec::from_lease`, `DockerEngine::spawn`,
//! `concurrency::run_one`, `ws::spawn_workspace`) never routed through it — so
//! the supply-chain invariant was cosmetic (brutal review R2/§hugit-runner).
//! `x4` depends on `hugit-runner`, so the runner cannot depend back on it; the
//! enforcement floor therefore has to live here, on the surface every spawn
//! actually crosses. The `x4` wrapper remains a valid higher-level façade; this
//! module is the load-bearing gate.
//!
//! The two checks are deliberately ordered: **(1) parse the pin** (no box, no
//! spawn) then **(2) verify the digest against the box** (still no spawn). Only
//! after both pass is the container run. A tampered or unpinned image fails
//! CLOSED with no container created and no tenant byte processed.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use anyhow::{Result, bail};

use crate::lease::{BoxExec, CmdOutput};

/// Bounded retries for a *transient* pull error (network/daemon hiccup, or a
/// concurrent-pull race on the same digest). A permanent integrity failure
/// (digest unresolvable / mismatch) is NOT retried — it fails CLOSED at once.
const PULL_MAX_ATTEMPTS: u32 = 4;
/// Base backoff between transient-pull retries (grows linearly per attempt).
const PULL_BACKOFF_BASE: Duration = Duration::from_millis(150);

/// Per-digest pull lock registry. Concurrent verifies of the **same** digest
/// serialize through one mutex so the box is asked to pull a given digest once
/// at a time (not 8× simultaneously) — the C2b concurrency-race fix. Distinct
/// digests get distinct locks and never block each other.
fn digest_lock(digest_hex: &str) -> Arc<Mutex<()>> {
    static LOCKS: OnceLock<Mutex<HashMap<String, Arc<Mutex<()>>>>> = OnceLock::new();
    let map = LOCKS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut guard = map
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    Arc::clone(
        guard
            .entry(digest_hex.to_string())
            .or_insert_with(|| Arc::new(Mutex::new(()))),
    )
}

/// Classify a failed `docker pull` as a **permanent** integrity failure (fail
/// CLOSED, no retry) vs a **transient** error (network/daemon/race — retry).
///
/// Permanent signals mean the registry could not produce the *content* for this
/// digest: the manifest/digest is unknown, not found, refused by auth, or the
/// resolved bytes did not match the requested digest. Those are exactly the
/// tamper/unpinned cases the supply-chain floor must refuse. Everything else
/// (timeouts, connection resets, daemon-busy, TLS hiccups, concurrent-pull
/// contention) is treated as transient and retried a bounded number of times.
fn is_permanent_pull_failure(stderr: &str) -> bool {
    let s = stderr.to_ascii_lowercase();
    const PERMANENT: &[&str] = &[
        "manifest unknown",
        "manifest for", // "manifest for X not found"
        "no such manifest",
        "not found",
        "does not match", // digest mismatch
        "unauthorized",
        "denied",
        "forbidden",
        "repository does not exist",
        "invalid reference",
        "unsupported",
    ];
    PERMANENT.iter().any(|needle| s.contains(needle))
}

/// A runner image reference proven to be content-(digest)-pinned. Construction
/// is the proof: you cannot build one from a floating tag or bare name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PinnedImageRef {
    /// Full reference as written, e.g. `alpine@sha256:d9e8…`.
    reference: String,
    /// The 64-char lowercase-hex `sha256` digest (no `sha256:` prefix).
    digest_hex: String,
}

impl PinnedImageRef {
    /// Parse and validate a digest-pinned image reference.
    ///
    /// Accepts exactly `<repository>@sha256:<64-lowercase-hex>`. A tag-only
    /// reference (`alpine:3.20`), a bare name (`alpine`), a non-`sha256`
    /// algorithm, an uppercase/short/non-hex digest, or anything containing
    /// shell metacharacters is **rejected** — this is the "unpinned image"
    /// fail-closed branch.
    ///
    /// # Errors
    /// Returns an error describing why the reference is not content-pinned.
    pub fn parse(reference: &str) -> Result<Self> {
        let r = reference.trim();
        if r.is_empty() {
            bail!("image reference is empty; not content-pinned — fail CLOSED");
        }
        let Some((repository, digest)) = r.split_once('@') else {
            bail!(
                "image {r:?} is not content-pinned (no `@sha256:` digest); a \
                 floating tag/name is rejected — fail CLOSED"
            );
        };
        if repository.is_empty() {
            bail!("image {r:?} has an empty repository before `@` — fail CLOSED");
        }
        // The repository portion is interpolated into `docker pull`/`docker run`
        // argv; keep it to a conservative registry-safe charset so it can never
        // smuggle a shell metacharacter even if the seam ever shell-joins.
        if !repository
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-' | b'/' | b':'))
        {
            bail!("image {r:?} repository has illegal characters — fail CLOSED");
        }
        let Some(hex) = digest.strip_prefix("sha256:") else {
            bail!(
                "image {r:?} digest {digest:?} does not use the sha256 \
                 algorithm; not content-pinned — fail CLOSED"
            );
        };
        if hex.len() != 64 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            bail!(
                "image {r:?} digest {hex:?} is not a 64-char hex sha256; \
                 malformed pin — fail CLOSED"
            );
        }
        if hex.bytes().any(|b| b.is_ascii_uppercase()) {
            bail!("image {r:?} digest must be lowercase hex (canonical) — fail CLOSED");
        }
        Ok(Self {
            reference: r.to_string(),
            digest_hex: hex.to_string(),
        })
    }

    /// `true` iff `reference` is a content-pinned image.
    #[must_use]
    pub fn is_pinned(reference: &str) -> bool {
        Self::parse(reference).is_ok()
    }

    /// The full pinned reference (`repo@sha256:hex`).
    #[must_use]
    pub fn reference(&self) -> &str {
        &self.reference
    }

    /// The 64-char lowercase-hex digest (no `sha256:` prefix).
    #[must_use]
    pub fn digest_hex(&self) -> &str {
        &self.digest_hex
    }

    /// Integrity-verify this pin against what the runner box actually resolves,
    /// **without** spawning anything.
    ///
    /// A content-addressed `docker pull` of a tampered digest is refused by the
    /// registry (the CAS itself enforces integrity); a successful pull is then
    /// cross-checked against the box-resolved `RepoDigests` to confirm the exact
    /// digest is present. Returns `Ok(())` only when the box-resolved content
    /// matches the pin.
    ///
    /// # Concurrency safety (WP-C2b regression fix)
    /// Under ≥8 jobs that share one pinned digest (the common fleet case), the
    /// naive "pull on every spawn" raced the Docker daemon: the box, hit with 8
    /// simultaneous pulls of the SAME digest, returned a *transient* error to
    /// some of them, which the old code misread as an integrity failure and
    /// failed CLOSED — killing a genuinely-pinned job. The fix is two-fold and
    /// does **not** weaken tamper rejection:
    ///   1. **Per-digest serialization** — concurrent verifies of the same
    ///      digest take one lock so the box pulls that digest once at a time, not
    ///      8× at once (distinct digests never block each other).
    ///   2. **Transient-vs-permanent classification with bounded retry** — a
    ///      pull error that signals the digest is unresolvable/mismatched
    ///      (manifest unknown, not found, digest mismatch, denied…) fails CLOSED
    ///      immediately; a network/daemon/race hiccup is retried a bounded number
    ///      of times with backoff. A genuinely tampered/unpinned digest produces
    ///      a permanent signal and is still refused before any spawn.
    ///
    /// # Errors
    /// Fails CLOSED if the pull is permanently refused (tampered/unresolvable
    /// digest), if transient retries are exhausted, or if the box-resolved digest
    /// set does not contain this pin.
    pub fn verify_on_box<B: BoxExec>(&self, boxx: &B) -> Result<()> {
        // Serialize same-digest verifies: the box pulls a given digest once at a
        // time, eliminating the 8×-concurrent-pull race. Distinct digests use
        // distinct locks, so independent jobs are never serialized behind us.
        let lock = digest_lock(&self.digest_hex);
        let _guard = lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);

        self.pull_with_retry(boxx)?;

        let inspect = boxx.run(&[
            "docker",
            "image",
            "inspect",
            &self.reference,
            "--format",
            "{{range .RepoDigests}}{{.}}\n{{end}}",
        ])?;
        if !inspect.ok() {
            bail!(
                "image {} could not be inspected after pull: {}",
                self.reference,
                inspect.stderr.trim()
            );
        }
        let needle = format!("sha256:{}", self.digest_hex);
        if !inspect.stdout.contains(&needle) {
            bail!(
                "image {} resolved to a digest set [{}] that does not contain \
                 the pinned {}; integrity mismatch — fail CLOSED",
                self.reference,
                inspect.stdout.replace('\n', " ").trim(),
                needle
            );
        }
        Ok(())
    }

    /// `docker pull` the pinned reference, retrying only **transient** failures.
    ///
    /// A permanent failure (unresolvable / mismatched / denied digest — see
    /// [`is_permanent_pull_failure`]) fails CLOSED on the first attempt with no
    /// retry: a tampered or unpinned image must never be papered over by a retry
    /// loop. A transient failure (network/daemon/concurrent-pull race) is retried
    /// up to [`PULL_MAX_ATTEMPTS`] with a linear backoff; only after the budget is
    /// exhausted does it fail CLOSED (a box that cannot pull a real pin is still
    /// not safe to spawn on).
    fn pull_with_retry<B: BoxExec>(&self, boxx: &B) -> Result<()> {
        let mut last: Option<CmdOutput> = None;
        for attempt in 1..=PULL_MAX_ATTEMPTS {
            let pull = boxx.run(&["docker", "pull", &self.reference])?;
            if pull.ok() {
                return Ok(());
            }
            let stderr = pull.stderr.trim().to_string();
            if is_permanent_pull_failure(&stderr) {
                bail!(
                    "image {} failed integrity verification on box (pull refused: \
                     {}); tampered/unresolvable digest — fail CLOSED",
                    self.reference,
                    stderr
                );
            }
            last = Some(pull);
            if attempt < PULL_MAX_ATTEMPTS {
                std::thread::sleep(PULL_BACKOFF_BASE * attempt);
            }
        }
        bail!(
            "image {} could not be pulled after {} transient attempts (last error: \
             {}); box not in a safe state to spawn — fail CLOSED",
            self.reference,
            PULL_MAX_ATTEMPTS,
            last.map(|o| o.stderr.trim().to_string())
                .unwrap_or_default()
        );
    }
}

/// Assert an image reference is content-pinned, returning the parsed pin.
///
/// This is the single chokepoint every spawn-spec derivation calls so an
/// unpinned image is rejected at spec-build time — before the box is touched.
///
/// # Errors
/// Fails CLOSED if `image` is not a `repo@sha256:<64-lowercase-hex>` reference.
pub fn require_pinned(image: &str) -> Result<PinnedImageRef> {
    PinnedImageRef::parse(image)
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID: &str =
        "alpine@sha256:d9e853e87e55526f6b2917df91a2115c36dd7c696a35be12163d44e6e2a4b6bc";

    #[test]
    fn parses_valid_digest_pin() {
        let p = PinnedImageRef::parse(VALID).expect("valid pin");
        assert_eq!(
            p.digest_hex(),
            "d9e853e87e55526f6b2917df91a2115c36dd7c696a35be12163d44e6e2a4b6bc"
        );
        assert!(PinnedImageRef::is_pinned(VALID));
    }

    #[test]
    fn rejects_floating_tag_and_bare_name() {
        assert!(PinnedImageRef::parse("alpine:3.20").is_err());
        assert!(PinnedImageRef::parse("alpine").is_err());
        assert!(!PinnedImageRef::is_pinned("alpine:3.20"));
    }

    #[test]
    fn rejects_bad_digests() {
        assert!(PinnedImageRef::parse("alpine@md5:abc").is_err());
        assert!(PinnedImageRef::parse("alpine@sha256:deadbeef").is_err());
        assert!(
            PinnedImageRef::parse(
                "alpine@sha256:zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz"
            )
            .is_err()
        );
        // uppercase non-canonical
        assert!(
            PinnedImageRef::parse(
                "alpine@sha256:D9E853E87E55526F6B2917DF91A2115C36DD7C696A35BE12163D44E6E2A4B6BC"
            )
            .is_err()
        );
    }

    #[test]
    fn rejects_shell_metachars_in_repository() {
        let evil = "al pine; rm -rf /@sha256:\
                    d9e853e87e55526f6b2917df91a2115c36dd7c696a35be12163d44e6e2a4b6bc";
        assert!(PinnedImageRef::parse(evil).is_err());
    }

    #[test]
    fn rejects_empty() {
        assert!(PinnedImageRef::parse("").is_err());
        assert!(PinnedImageRef::parse("   ").is_err());
    }

    // ── concurrency-safe pull path (WP-C2b item_4 regression) ─────────────────
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A scriptable box for verify_on_box: the first `transient_failures` pulls
    /// of any reference return a *transient* (retryable) error, then succeed;
    /// inspect always echoes the reference's RepoDigest. Counts total pulls so a
    /// test can prove how often the box was actually hit.
    struct ScriptedBox {
        transient_failures: usize,
        permanent: bool,
        pulls: AtomicUsize,
    }

    impl ScriptedBox {
        fn transient(n: usize) -> Self {
            Self {
                transient_failures: n,
                permanent: false,
                pulls: AtomicUsize::new(0),
            }
        }
        fn permanent() -> Self {
            Self {
                transient_failures: 0,
                permanent: true,
                pulls: AtomicUsize::new(0),
            }
        }
    }

    impl BoxExec for ScriptedBox {
        fn run(&self, argv: &[&str]) -> Result<CmdOutput> {
            if argv.first() == Some(&"docker") && argv.get(1) == Some(&"pull") {
                let n = self.pulls.fetch_add(1, Ordering::SeqCst);
                if self.permanent {
                    return Ok(CmdOutput {
                        code: Some(1),
                        stdout: String::new(),
                        stderr: "manifest unknown".to_string(),
                    });
                }
                if n < self.transient_failures {
                    // A network/daemon hiccup — must be retried, NOT fail-closed.
                    return Ok(CmdOutput {
                        code: Some(1),
                        stdout: String::new(),
                        stderr: "net/http: TLS handshake timeout".to_string(),
                    });
                }
                return Ok(CmdOutput {
                    code: Some(0),
                    stdout: String::new(),
                    stderr: String::new(),
                });
            }
            if argv.first() == Some(&"docker")
                && argv.get(1) == Some(&"image")
                && argv.get(2) == Some(&"inspect")
            {
                let reference = argv.get(3).copied().unwrap_or("");
                return Ok(CmdOutput {
                    code: Some(0),
                    stdout: format!("{reference}\n"),
                    stderr: String::new(),
                });
            }
            Ok(CmdOutput {
                code: Some(0),
                stdout: String::new(),
                stderr: String::new(),
            })
        }
    }

    #[test]
    fn transient_pull_error_is_retried_not_fail_closed() {
        // 2 transient hiccups then success: the genuinely-pinned image must
        // verify (this is the exact C2b item_4 spurious-fail class).
        let pin = PinnedImageRef::parse(VALID).unwrap();
        let boxx = ScriptedBox::transient(2);
        pin.verify_on_box(&boxx)
            .expect("a transient pull error must be retried, not failed closed");
        assert_eq!(
            boxx.pulls.load(Ordering::SeqCst),
            3,
            "2 retries then success"
        );
    }

    #[test]
    fn permanent_pull_error_fails_closed_without_retry() {
        // Tamper-rejection oracle: an unresolvable digest fails CLOSED on the
        // FIRST attempt — no retry loop papers over a tampered image.
        let pin = PinnedImageRef::parse(VALID).unwrap();
        let boxx = ScriptedBox::permanent();
        assert!(
            pin.verify_on_box(&boxx).is_err(),
            "a permanent (manifest unknown) failure must fail CLOSED"
        );
        assert_eq!(
            boxx.pulls.load(Ordering::SeqCst),
            1,
            "a permanent failure must NOT be retried"
        );
    }

    #[test]
    fn transient_budget_exhaustion_fails_closed() {
        // A box that can never pull a real pin is still unsafe to spawn on:
        // after the retry budget is spent, fail CLOSED.
        let pin = PinnedImageRef::parse(VALID).unwrap();
        let boxx = ScriptedBox::transient(usize::MAX);
        assert!(
            pin.verify_on_box(&boxx).is_err(),
            "exhausting the transient retry budget must fail CLOSED"
        );
        assert_eq!(
            boxx.pulls.load(Ordering::SeqCst),
            PULL_MAX_ATTEMPTS as usize,
            "all attempts are spent before failing closed"
        );
    }

    #[test]
    fn classifier_separates_transient_from_permanent() {
        // Permanent (fail CLOSED, the tamper/unpinned signals).
        for s in [
            "manifest unknown",
            "manifest for alpine@sha256:x not found",
            "digest does not match",
            "pull access denied",
            "unauthorized: authentication required",
        ] {
            assert!(is_permanent_pull_failure(s), "should be permanent: {s:?}");
        }
        // Transient (retry, the concurrent-pull/network race).
        for s in [
            "net/http: TLS handshake timeout",
            "connection reset by peer",
            "Error response from daemon: i/o timeout",
            "context deadline exceeded",
            "received unexpected HTTP status: 503",
        ] {
            assert!(!is_permanent_pull_failure(s), "should be transient: {s:?}");
        }
    }

    #[test]
    fn permanent_pull_failure_is_case_insensitive() {
        // The classifier lowercases the stderr, so a permanent signal is caught
        // regardless of case — and a transient signal stays transient even when
        // shouted in all-caps.
        assert!(is_permanent_pull_failure("Manifest Unknown"));
        assert!(!is_permanent_pull_failure("TOOMANYREQUESTS"));
    }

    #[test]
    fn concurrent_same_digest_verifies_all_succeed() {
        use std::sync::Arc;
        use std::thread;
        // 8 threads verify the SAME digest at once against a box whose first few
        // pulls hiccup transiently. The per-digest lock + retry must let every
        // one of them verify (no spurious fail-closed) — the item_4 invariant.
        let boxx = Arc::new(ScriptedBox::transient(3));
        let mut handles = Vec::new();
        for _ in 0..8 {
            let boxx = Arc::clone(&boxx);
            handles.push(thread::spawn(move || {
                let pin = PinnedImageRef::parse(VALID).unwrap();
                pin.verify_on_box(boxx.as_ref())
            }));
        }
        for h in handles {
            h.join()
                .unwrap()
                .expect("every concurrent same-digest verify must succeed");
        }
    }
}
