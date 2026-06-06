//! Content-pinning + ordered verify-before-spawn for runner images (WP-X4 ①③).
//!
//! The invariant: a runner image reference is acceptable for spawn **only** if
//! it is pinned by content digest (`name@sha256:<64-lowercase-hex>`), and the
//! pinned digest must be **integrity-verified** against the digest the runner
//! box actually resolves — *before* the container is ever spawned.
//!
//! The verification is deliberately structured as a guard that runs **before**
//! the consumed [`hugit_runner`] spawn surface is touched, so that a tampered
//! or unpinned image fails CLOSED with no container spawned and no tenant byte
//! processed. The ordering is the load-bearing property (WP-X4 item ③): a
//! post-hoc detection (spawn first, check later) is a contract violation.

use anyhow::{Result, bail};
use hugit_runner::isolation::{Engine, RunningContainer};
use hugit_runner::lease::{BoxExec, ContainerSpec};

/// A runner image reference that has been parsed and proven to be content
/// (digest) pinned. Construction is the proof: you cannot build a `PinnedImage`
/// from a floating tag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PinnedImage {
    /// Full reference as written, e.g. `alpine@sha256:d9e8…`.
    reference: String,
    /// Repository portion, e.g. `alpine` / `docker.io/library/alpine`.
    repository: String,
    /// The 64-char lowercase-hex `sha256` content digest (no `sha256:` prefix).
    digest_hex: String,
}

impl PinnedImage {
    /// Parse and validate a digest-pinned image reference.
    ///
    /// Accepts exactly `…<repository>@sha256:<64-lowercase-hex>`. A tag-only
    /// reference (`alpine:3.20`), a bare name (`alpine`), a non-`sha256`
    /// algorithm, or a malformed digest is **rejected** — that is the
    /// "unpinned image" branch of WP-X4 item ③.
    ///
    /// # Errors
    /// Returns an error describing why the reference is not content-pinned.
    pub fn parse(reference: &str) -> Result<Self> {
        let r = reference.trim();
        if r.is_empty() {
            bail!("image reference is empty; not content-pinned");
        }
        let Some((repository, digest)) = r.split_once('@') else {
            bail!(
                "image {r:?} is not content-pinned (no `@sha256:` digest); a \
                 floating tag/name is rejected — fail CLOSED"
            );
        };
        if repository.is_empty() {
            bail!("image {r:?} has an empty repository before `@`");
        }
        let Some(hex) = digest.strip_prefix("sha256:") else {
            bail!(
                "image {r:?} digest {digest:?} does not use the sha256 \
                 algorithm; not content-pinned"
            );
        };
        if hex.len() != 64 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            bail!(
                "image {r:?} digest {hex:?} is not a 64-char hex sha256; \
                 malformed pin — fail CLOSED"
            );
        }
        if hex.bytes().any(|b| b.is_ascii_uppercase()) {
            bail!("image {r:?} digest must be lowercase hex (canonical form)");
        }
        Ok(Self {
            reference: r.to_string(),
            repository: repository.to_string(),
            digest_hex: hex.to_string(),
        })
    }

    /// `true` iff `reference` is a content-pinned image. Convenience predicate
    /// over [`PinnedImage::parse`].
    #[must_use]
    pub fn is_pinned(reference: &str) -> bool {
        Self::parse(reference).is_ok()
    }

    /// The full pinned reference (`repo@sha256:hex`).
    #[must_use]
    pub fn reference(&self) -> &str {
        &self.reference
    }

    /// The repository portion.
    #[must_use]
    pub fn repository(&self) -> &str {
        &self.repository
    }

    /// The 64-char lowercase-hex digest (no `sha256:` prefix).
    #[must_use]
    pub fn digest_hex(&self) -> &str {
        &self.digest_hex
    }

    /// Integrity-verify this pin against what the runner box actually resolves,
    /// **without** spawning anything.
    ///
    /// Resolves the reference on the box (a content-addressed pull: the daemon
    /// will refuse to materialize a digest the registry cannot serve, so a
    /// *tampered* digest cannot resolve) and confirms the resolved
    /// `RepoDigests` contain this exact `sha256` digest. Returns `Ok(())` only
    /// when the box-resolved content matches the pin.
    ///
    /// # Errors
    /// Fails CLOSED if the pull is refused (tampered/unresolvable digest) or if
    /// the box-resolved digest does not contain this pin.
    pub fn verify_on_box<B: BoxExec>(&self, boxx: &B) -> Result<()> {
        // Content-addressed pull: a tampered digest is not servable by the
        // registry, so this is refused — integrity is enforced by the
        // content-addressed store itself.
        let pull = boxx.run(&["docker", "pull", &self.reference])?;
        if !pull.ok() {
            bail!(
                "image {} failed integrity verification on box (pull refused: \
                 {}); tampered/unresolvable digest — fail CLOSED",
                self.reference,
                pull.stderr.trim()
            );
        }
        // Confirm the resolved image carries our exact content digest.
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
}

/// Outcome of a spawn attempt routed through the verify-before-spawn guard.
///
/// The discriminant records **whether any tenant work could have begun**: a
/// `RejectedBeforeSpawn` proves the path failed CLOSED with no container
/// spawned (WP-X4 item ③ ordering).
#[derive(Debug)]
pub enum GuardedSpawn {
    /// The image was content-pinned AND its digest verified against the box;
    /// only then was the container spawned. Carries the live container.
    Spawned(RunningContainer),
    /// The image was unpinned or failed integrity verification; the guard
    /// refused **before** the spawn surface was touched. No container exists,
    /// no tenant byte was processed.
    RejectedBeforeSpawn(String),
}

impl GuardedSpawn {
    /// `true` iff a container was actually spawned.
    #[must_use]
    pub fn spawned(&self) -> bool {
        matches!(self, GuardedSpawn::Spawned(_))
    }

    /// `true` iff the spawn was rejected before any tenant work.
    #[must_use]
    pub fn rejected_before_spawn(&self) -> bool {
        matches!(self, GuardedSpawn::RejectedBeforeSpawn(_))
    }
}

/// A guard over a consumed [`Engine`] that enforces content-pinning +
/// integrity verification **before** delegating to the engine's spawn.
///
/// This is the WP-X4 supply-chain gate: it never modifies the spawn surface,
/// it wraps it. The verification runs first; the engine's `spawn` is reached
/// only on success. The consumed engine and box are borrowed read-only.
pub struct VerifiedSpawn<'a, E: Engine, B: BoxExec> {
    engine: &'a E,
    boxx: &'a B,
}

impl<'a, E: Engine, B: BoxExec> VerifiedSpawn<'a, E, B> {
    /// Wrap a consumed engine + box.
    pub fn new(engine: &'a E, boxx: &'a B) -> Self {
        Self { engine, boxx }
    }

    /// Spawn `spec`'s container **only if** its image is content-pinned and the
    /// pin integrity-verifies against the box — checked, in that order, before
    /// the engine's spawn surface is touched.
    ///
    /// A tampered or unpinned image yields [`GuardedSpawn::RejectedBeforeSpawn`]
    /// with **no** call to [`Engine::spawn`], so no container exists and no
    /// tenant byte is processed (WP-X4 item ③).
    ///
    /// # Errors
    /// Propagates a transport error from the engine's own `spawn` only after
    /// verification has already passed.
    pub fn spawn_verified(&self, spec: &ContainerSpec) -> Result<GuardedSpawn> {
        // STEP 1 — pin check (no box, no spawn).
        let pinned = match PinnedImage::parse(&spec.image) {
            Ok(p) => p,
            Err(e) => return Ok(GuardedSpawn::RejectedBeforeSpawn(e.to_string())),
        };
        // STEP 2 — integrity verification against the box (still no spawn).
        if let Err(e) = pinned.verify_on_box(self.boxx) {
            return Ok(GuardedSpawn::RejectedBeforeSpawn(e.to_string()));
        }
        // STEP 3 — only now is the consumed spawn surface reached.
        let container = self.engine.spawn(spec)?;
        Ok(GuardedSpawn::Spawned(container))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_valid_digest_pin() {
        let r = "alpine@sha256:d9e853e87e55526f6b2917df91a2115c36dd7c696a35be12163d44e6e2a4b6bc";
        let p = PinnedImage::parse(r).expect("valid pin");
        assert_eq!(p.repository(), "alpine");
        assert_eq!(
            p.digest_hex(),
            "d9e853e87e55526f6b2917df91a2115c36dd7c696a35be12163d44e6e2a4b6bc"
        );
        assert!(PinnedImage::is_pinned(r));
    }

    #[test]
    fn rejects_floating_tag() {
        assert!(PinnedImage::parse("alpine:3.20").is_err());
        assert!(!PinnedImage::is_pinned("alpine:3.20"));
    }

    #[test]
    fn rejects_bare_name() {
        assert!(PinnedImage::parse("alpine").is_err());
    }

    #[test]
    fn rejects_non_sha256_algorithm() {
        assert!(PinnedImage::parse("alpine@md5:abc").is_err());
    }

    #[test]
    fn rejects_short_or_nonhex_digest() {
        assert!(PinnedImage::parse("alpine@sha256:deadbeef").is_err());
        assert!(
            PinnedImage::parse(
                "alpine@sha256:zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz"
            )
            .is_err()
        );
    }

    #[test]
    fn rejects_uppercase_digest() {
        // Same value, uppercased — non-canonical, rejected.
        let up = "alpine@sha256:D9E853E87E55526F6B2917DF91A2115C36DD7C696A35BE12163D44E6E2A4B6BC";
        assert!(PinnedImage::parse(up).is_err());
    }

    #[test]
    fn rejects_empty() {
        assert!(PinnedImage::parse("").is_err());
        assert!(PinnedImage::parse("   ").is_err());
    }
}
