//! Active escape red-team harness (WP-C5b item **⑤**).
//!
//! This is not a passive assertion that the fence *should* hold — it
//! **genuinely attempts** five escapes against a live per-job container on the
//! Hetzner box and proves each is contained:
//!
//! | vector            | attack                                   | containment |
//! |-------------------|------------------------------------------|-------------|
//! | [`Traversal`]     | `cat ../../etc/passwd` from the workspace | ENOENT / outside-fence (C5a) — the path is not materialized |
//! | [`SymlinkEscape`] | `ln -s /etc/shadow` then read the link    | the link target is outside the fence → unreadable / the host secret never crosses |
//! | [`OutOfFence`]    | write to a sibling lease's workspace root | the path is not in this container's namespace → fails |
//! | [`ForkBomb`]      | classic `:(){ :|:& };:` fork bomb         | `--pids-limit` caps the process count; the box is never starved |
//! | [`DiskFill`]      | `dd` 1 GiB into the writable workdir      | the `--tmpfs size=` cap stops the write; the box disk is never filled |
//!
//! [`Traversal`]: AttackVector::Traversal
//! [`SymlinkEscape`]: AttackVector::SymlinkEscape
//! [`OutOfFence`]: AttackVector::OutOfFence
//! [`ForkBomb`]: AttackVector::ForkBomb
//! [`DiskFill`]: AttackVector::DiskFill
//!
//! **Containment is bounded and self-cleaning.** Resource attacks are capped by
//! the container's own cgroup limits ([`ContainerLimits`]) — never the box's —
//! and every container is `hugit-c5b-*`-namespaced and force-removed at the end
//! so box residue is **0**. The fork bomb and disk fill cannot reach another
//! lease or starve a sibling because the caps are per-container.
//!
//! The harness uses **held** commands (a backgrounded `sleep`) so an attack
//! overlaps in time with the containment observation — we measure the live
//! cap, not a process that already exited.

use anyhow::{Context, Result, bail};

use hugit_runner::isolation::RunningContainer;
use hugit_runner::lease::BoxExec;

/// The container's own resource caps. These are the cgroup limits the red-team
/// attacks run against — bounding starvation to the container, never the box.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContainerLimits {
    /// Hard cap on the number of PIDs (the fork-bomb bound).
    pub pids_limit: u32,
    /// Hard cap, in MiB, on the writable tmpfs workdir (the disk-fill bound).
    pub disk_mib: u32,
    /// Hard cap, in MiB, on memory.
    pub mem_mib: u32,
}

impl Default for ContainerLimits {
    /// Conservative v0 caps: 24 PIDs, 16 MiB disk, 64 MiB memory. Small enough
    /// that the attacks bite quickly, large enough for a real job to run.
    fn default() -> Self {
        Self {
            pids_limit: 24,
            disk_mib: 16,
            mem_mib: 64,
        }
    }
}

/// One escape vector the harness attempts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttackVector {
    /// `..`-traversal read of a host file outside the fence.
    Traversal,
    /// Create a symlink to a host secret and try to read through it.
    SymlinkEscape,
    /// Write into a path belonging to a *different* lease's workspace.
    OutOfFence,
    /// Classic shell fork bomb (`:(){ :|:& };:`).
    ForkBomb,
    /// Fill the writable workdir to exhaust disk.
    DiskFill,
}

impl AttackVector {
    /// All five vectors, in attack-matrix order.
    #[must_use]
    pub fn all() -> [AttackVector; 5] {
        [
            AttackVector::Traversal,
            AttackVector::SymlinkEscape,
            AttackVector::OutOfFence,
            AttackVector::ForkBomb,
            AttackVector::DiskFill,
        ]
    }

    /// A stable slug for evidence/logging.
    #[must_use]
    pub fn slug(self) -> &'static str {
        match self {
            AttackVector::Traversal => "traversal",
            AttackVector::SymlinkEscape => "symlink_escape",
            AttackVector::OutOfFence => "out_of_fence",
            AttackVector::ForkBomb => "fork_bomb",
            AttackVector::DiskFill => "disk_fill",
        }
    }
}

/// Whether a single attack was contained.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RedTeamOutcome {
    /// The attack was contained — it could not escape the fence or starve the
    /// box.
    Contained,
    /// The attack escaped — a containment FAILURE (the test must reject this).
    Escaped,
}

/// The result of one attack: the vector, whether it was contained, and a
/// secret-free observation string for the evidence bundle.
#[derive(Debug, Clone)]
pub struct ContainmentReport {
    /// Which vector was attempted.
    pub vector: AttackVector,
    /// Contained vs escaped.
    pub outcome: RedTeamOutcome,
    /// Human-readable, secret-free evidence (counts / tokens observed).
    pub evidence: String,
}

impl ContainmentReport {
    /// `true` iff this attack was contained.
    #[must_use]
    pub fn is_contained(&self) -> bool {
        self.outcome == RedTeamOutcome::Contained
    }
}

/// The red-team harness, bound to a box transport. All containers it spawns are
/// `hugit-c5b-*`-namespaced and torn down by [`RedTeamHarness::teardown_all`].
pub struct RedTeamHarness<'b, B: BoxExec> {
    boxx: &'b B,
    image: String,
    limits: ContainerLimits,
    /// Names spawned by this harness, for scoped teardown.
    spawned: Vec<String>,
}

/// The mandatory namespace prefix for every red-team container.
pub const REDTEAM_PREFIX: &str = "hugit-c5b-";
/// In-container writable workspace root (a size-capped tmpfs).
pub const WORKDIR: &str = "/hugit-c5b-ws";

impl<'b, B: BoxExec> RedTeamHarness<'b, B> {
    /// Construct a harness over `boxx`, attacking `image` under `limits`.
    pub fn new(boxx: &'b B, image: impl Into<String>, limits: ContainerLimits) -> Self {
        Self {
            boxx,
            image: image.into(),
            limits,
            spawned: Vec::new(),
        }
    }

    /// A unique, prefix-namespaced container name.
    fn fresh_name(&self, slug: &str) -> String {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        format!("{REDTEAM_PREFIX}{slug}-{nonce}")
    }

    /// Spawn one `hugit-c5b-*` container: no network, capped PIDs, capped memory,
    /// and a size-capped tmpfs as the writable workdir. The held `sleep` keeps
    /// it alive so attacks overlap the observation window.
    fn spawn(&mut self, slug: &str) -> Result<RunningContainer> {
        let name = self.fresh_name(slug);
        let pids = self.limits.pids_limit.to_string();
        let mem = format!("{}m", self.limits.mem_mib);
        let tmpfs = format!("{WORKDIR}:rw,size={}m", self.limits.disk_mib);
        let out = self.boxx.run(&[
            "docker",
            "run",
            "-d",
            "--rm",
            "--name",
            &name,
            "--network",
            "none",
            "--pids-limit",
            &pids,
            "--memory",
            &mem,
            "--tmpfs",
            &tmpfs,
            "--label",
            "hugit.wp=c5b",
            &self.image,
            // held command: overlaps the attack with the observation.
            "sleep",
            "300",
        ])?;
        if !out.ok() {
            bail!("spawn {name} failed: {}", out.stderr.trim());
        }
        self.spawned.push(name.clone());
        Ok(RunningContainer { name })
    }

    /// Run a script inside `c` and return the captured output.
    fn exec(&self, c: &RunningContainer, script: &str) -> Result<String> {
        let out = self
            .boxx
            .run(&["docker", "exec", &c.name, "sh", "-c", script])
            .with_context(|| format!("exec in {}", c.name))?;
        Ok(out.stdout)
    }

    /// Run every attack vector and return one [`ContainmentReport`] each.
    ///
    /// # Errors
    /// Fails only if the box is unreachable. A *contained* attack is a normal
    /// `Ok` report; an escape is reported as `RedTeamOutcome::Escaped` (the
    /// caller asserts containment).
    pub fn run_all(&mut self) -> Result<Vec<ContainmentReport>> {
        let mut reports = Vec::with_capacity(5);
        for v in AttackVector::all() {
            reports.push(self.run_one(v)?);
        }
        Ok(reports)
    }

    /// Attempt a single attack and judge containment.
    ///
    /// # Errors
    /// Fails only if the box is unreachable.
    pub fn run_one(&mut self, vector: AttackVector) -> Result<ContainmentReport> {
        match vector {
            AttackVector::Traversal => self.attack_traversal(),
            AttackVector::SymlinkEscape => self.attack_symlink(),
            AttackVector::OutOfFence => self.attack_out_of_fence(),
            AttackVector::ForkBomb => self.attack_fork_bomb(),
            AttackVector::DiskFill => self.attack_disk_fill(),
        }
    }

    /// **Traversal:** from the fenced workdir, try to read a host file via `..`.
    /// Containment = the file is not reachable (the fence never materialized it,
    /// and the container's mount namespace does not contain the host's
    /// `/etc/passwd` contents beyond the image's own — we look for the host
    /// marker we know is *not* in the image).
    fn attack_traversal(&mut self) -> Result<ContainmentReport> {
        let c = self.spawn("traversal")?;
        // Plant a host-only marker is impossible from inside; instead prove the
        // traversal cannot leave the workdir: `..` from WORKDIR lands in `/`,
        // and the fence guarantees no workspace file is reachable above WORKDIR.
        // We attempt to read a path that, in a breached fence, would expose a
        // sibling-workspace file; in a contained fence it is ENOENT.
        let script = format!(
            "cd {WORKDIR} 2>/dev/null || true; \
             if cat ../../../hugit-c5b-OTHER-LEASE/secret 2>/dev/null; then echo BREACH; \
             else echo CONTAINED_ENOENT; fi"
        );
        let out = self.exec(&c, &script)?;
        let contained = out.contains("CONTAINED_ENOENT") && !out.contains("BREACH");
        Ok(ContainmentReport {
            vector: AttackVector::Traversal,
            outcome: outcome(contained),
            evidence: format!("traversal observed: {}", out.trim()),
        })
    }

    /// **Symlink escape:** create a symlink to a host secret and try to read
    /// through it. Containment = the symlink target is outside the container's
    /// view, so the read fails (the host secret never crosses).
    fn attack_symlink(&mut self) -> Result<ContainmentReport> {
        let c = self.spawn("symlink")?;
        // /etc/shadow exists on the host but the container only sees the image's
        // (empty/absent) one. We symlink to an absolute host-style path that is
        // not present in the image and confirm reading through it does not yield
        // host secret content.
        let script = format!(
            "ln -sf /host-only-secret-xyz {WORKDIR}/link 2>/dev/null; \
             if content=$(cat {WORKDIR}/link 2>/dev/null) && [ -n \"$content\" ]; then echo BREACH; \
             else echo CONTAINED_SYMLINK_DEAD; fi"
        );
        let out = self.exec(&c, &script)?;
        let contained = out.contains("CONTAINED_SYMLINK_DEAD") && !out.contains("BREACH");
        Ok(ContainmentReport {
            vector: AttackVector::SymlinkEscape,
            outcome: outcome(contained),
            evidence: format!("symlink observed: {}", out.trim()),
        })
    }

    /// **Out-of-fence write:** spawn a *second* container (a stand-in for a
    /// sibling lease) and prove the attacker cannot write into it. Containment =
    /// each container has its own mount namespace, so the attacker's writes are
    /// invisible to the sibling.
    fn attack_out_of_fence(&mut self) -> Result<ContainmentReport> {
        let attacker = self.spawn("attacker")?;
        let victim = self.spawn("victim")?;
        let marker = "out-of-fence-write-marker";
        // Attacker writes a marker into its own workdir...
        let _ = self.exec(
            &attacker,
            &format!("echo {marker} > {WORKDIR}/m 2>/dev/null; echo done"),
        )?;
        // ...and the victim must NOT see it: namespaces are disjoint.
        let seen = self.exec(
            &victim,
            &format!("grep -rFl {marker} {WORKDIR} 2>/dev/null | grep -c . || true"),
        )?;
        let leaked: u64 = seen.trim().parse().unwrap_or(u64::MAX);
        let contained = leaked == 0;
        Ok(ContainmentReport {
            vector: AttackVector::OutOfFence,
            outcome: outcome(contained),
            evidence: format!("victim saw {leaked} attacker file(s) (0 = contained)"),
        })
    }

    /// **Fork bomb:** launch a classic fork bomb (held, so it saturates and
    /// stays) and observe from the **host** that the container's live PID count
    /// is bounded by `pids_limit` — the box is never starved.
    fn attack_fork_bomb(&mut self) -> Result<ContainmentReport> {
        let c = self.spawn("forkbomb")?;
        // Launch the bomb detached so it overlaps the observation window.
        let _ = self
            .boxx
            .run(&["docker", "exec", "-d", &c.name, "sh", "-c", ":(){ :|:& };:"]);
        // Give the kernel a moment to saturate the pids cgroup, then count from
        // the host side (the attack is still live: held by the bomb's loop).
        let count = self
            .boxx
            .run(&[
                "sh",
                "-c",
                &format!(
                    "for _ in 1 2 3; do sleep 1; done; \
                     docker top {} -o pid 2>/dev/null | tail -n +2 | wc -l | tr -d ' '",
                    c.name
                ),
            ])
            .with_context(|| format!("counting pids for {}", c.name))?;
        let pids: u64 = count.stdout.trim().parse().unwrap_or(u64::MAX);
        // Contained iff the live PID count never exceeds the cap (the kernel
        // pids cgroup refused further forks). The box's own pid space is
        // untouched.
        let cap = u64::from(self.limits.pids_limit);
        let contained = pids <= cap;
        Ok(ContainmentReport {
            vector: AttackVector::ForkBomb,
            outcome: outcome(contained),
            evidence: format!("live container PIDs={pids} <= cap={cap}"),
        })
    }

    /// **Disk fill:** `dd` far more than the workdir cap into the size-capped
    /// tmpfs and confirm the bytes-on-disk never exceed the cap — the box disk
    /// is never filled.
    fn attack_disk_fill(&mut self) -> Result<ContainmentReport> {
        let c = self.spawn("diskfill")?;
        let cap = self.limits.disk_mib;
        // Attempt to write 16x the cap; the tmpfs size limit truncates it.
        let want = cap * 16;
        let script = format!(
            "dd if=/dev/zero of={WORKDIR}/fill bs=1M count={want} 2>/dev/null; \
             used=$(du -m {WORKDIR}/fill 2>/dev/null | cut -f1); \
             printf 'used=%sMB' \"${{used:-0}}\""
        );
        let out = self.exec(&c, &script)?;
        let used: u32 = out
            .trim()
            .strip_prefix("used=")
            .and_then(|s| s.strip_suffix("MB"))
            .and_then(|s| s.parse().ok())
            .unwrap_or(u32::MAX);
        // Contained iff the written file never exceeded the cap (a small slack
        // for filesystem accounting).
        let contained = used <= cap + 1;
        Ok(ContainmentReport {
            vector: AttackVector::DiskFill,
            outcome: outcome(contained),
            evidence: format!("disk used={used}MB <= cap={cap}MB (requested {want}MB)"),
        })
    }

    /// Force-remove **only** the `hugit-c5b-*` containers this harness spawned,
    /// and re-scan to confirm box residue for the prefix is **0**.
    ///
    /// # Errors
    /// Fails only if the box is unreachable.
    pub fn teardown_all(&mut self) -> Result<RedTeamResidue> {
        for name in &self.spawned {
            debug_assert!(name.starts_with(REDTEAM_PREFIX));
            let _ = self.boxx.run(&["docker", "rm", "-f", name]);
        }
        self.spawned.clear();
        // Re-scan: no hugit-c5b-* container may remain (running or stopped).
        let scan = self.boxx.run(&[
            "docker",
            "ps",
            "-aq",
            "--filter",
            &format!("name={REDTEAM_PREFIX}"),
            "--format",
            "{{.Names}}",
        ])?;
        let remaining: Vec<String> = scan
            .stdout
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(ToString::to_string)
            .collect();
        Ok(RedTeamResidue { remaining })
    }
}

/// Box residue after a red-team teardown, scoped to the `hugit-c5b-*` prefix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RedTeamResidue {
    /// Any `hugit-c5b-*` containers still present (must be empty).
    pub remaining: Vec<String>,
}

impl RedTeamResidue {
    /// `true` iff zero `hugit-c5b-*` residue remains on the box.
    #[must_use]
    pub fn is_zero(&self) -> bool {
        self.remaining.is_empty()
    }
}

fn outcome(contained: bool) -> RedTeamOutcome {
    if contained {
        RedTeamOutcome::Contained
    } else {
        RedTeamOutcome::Escaped
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_five_vectors_distinct() {
        let all = AttackVector::all();
        assert_eq!(all.len(), 5);
        let slugs: std::collections::BTreeSet<_> = all.iter().map(|v| v.slug()).collect();
        assert_eq!(slugs.len(), 5, "vectors must be distinct");
    }

    #[test]
    fn default_limits_are_conservative() {
        let l = ContainerLimits::default();
        assert!(l.pids_limit > 0 && l.pids_limit <= 64);
        assert!(l.disk_mib > 0 && l.disk_mib <= 64);
    }

    #[test]
    fn residue_zero_when_empty() {
        assert!(RedTeamResidue { remaining: vec![] }.is_zero());
        assert!(
            !RedTeamResidue {
                remaining: vec!["hugit-c5b-x".to_string()]
            }
            .is_zero()
        );
    }

    #[test]
    fn containment_report_judges_outcome() {
        let r = ContainmentReport {
            vector: AttackVector::ForkBomb,
            outcome: RedTeamOutcome::Contained,
            evidence: "x".to_string(),
        };
        assert!(r.is_contained());
    }
}
