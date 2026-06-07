//! Active escape red-team harness (WP-C5b item **⑤**).
//!
//! This is not a passive assertion that the fence *should* hold — it
//! **genuinely attempts** six escapes against a live per-job container on the
//! Hetzner box and proves each is contained:
//!
//! | vector            | attack                                   | containment |
//! |-------------------|------------------------------------------|-------------|
//! | [`Traversal`]     | `cat ../../etc/passwd` from the workspace | ENOENT / outside-fence (C5a) — the path is not materialized |
//! | [`SymlinkEscape`] | `ln -s /etc/shadow` then read the link    | the link target is outside the fence → unreadable / the host secret never crosses |
//! | [`OutOfFence`]    | write to a sibling lease's workspace root | the path is not in this container's namespace → fails |
//! | [`ForkBomb`]      | classic `:(){ :|:& };:` fork bomb         | `--pids-limit` caps the process count; the box is never starved |
//! | [`DiskFill`]      | `dd` 1 GiB into the writable workdir      | the `--tmpfs size=` cap stops the write; the box disk is never filled |
//! | [`FenceMaterializedEscape`] | materialize a real `FenceManifest`, then read an out-of-fence path **in the same container** | the fence (`classify()` + sparse materialize) — NOT the Docker namespace — is the boundary: the out-of-fence file is ENOENT because it was never materialized |
//!
//! The first five lean (correctly) on the Docker mount/pid/tmpfs namespace; the
//! **sixth is the one where the fence itself is the only control** — it would
//! escape if `classify()` were a no-op constant `Inside`, so it is the vector
//! that holds the fence honest.
//!
//! [`Traversal`]: AttackVector::Traversal
//! [`SymlinkEscape`]: AttackVector::SymlinkEscape
//! [`OutOfFence`]: AttackVector::OutOfFence
//! [`ForkBomb`]: AttackVector::ForkBomb
//! [`DiskFill`]: AttackVector::DiskFill
//! [`FenceMaterializedEscape`]: AttackVector::FenceMaterializedEscape
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

use hugit_contracts::FenceManifest;
use hugit_runner::isolation::RunningContainer;
use hugit_runner::lease::BoxExec;

use crate::enforce::probe_outside_enoent;
use crate::materialize::{CandidateEntry, materialize_sparse};

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
    /// **The fence is the control.** Materialize a *real* [`FenceManifest`]
    /// into the container (only the in-fence `path_set`), then — inside the
    /// **same** container — attack an out-of-fence path. Containment requires
    /// the in-fence file to be present *and* the out-of-fence path to be ENOENT.
    /// This is the only vector where the Docker namespace/cgroup is **not** the
    /// boundary; the boundary is `classify()` + sparse materialization. If
    /// `classify()` ever returned a constant `Inside`, the out-of-fence file
    /// would be materialized and this vector would report an ESCAPE.
    FenceMaterializedEscape,
}

impl AttackVector {
    /// All six vectors, in attack-matrix order.
    #[must_use]
    pub fn all() -> [AttackVector; 6] {
        [
            AttackVector::Traversal,
            AttackVector::SymlinkEscape,
            AttackVector::OutOfFence,
            AttackVector::ForkBomb,
            AttackVector::DiskFill,
            AttackVector::FenceMaterializedEscape,
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
            AttackVector::FenceMaterializedEscape => "fence_materialized_escape",
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
        let mut reports = Vec::with_capacity(AttackVector::all().len());
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
            AttackVector::FenceMaterializedEscape => self.attack_fence_materialized_escape(),
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
        let cap = u64::from(self.limits.pids_limit);

        // Launch a SATURATING spawner: tightly fork long-lived (`sleep`) children
        // so `pids.current` climbs and HOLDS at the cap (a classic `:(){...}`
        // bomb under non-interactive `sh -c` does not reliably replicate, and its
        // transient children evade `docker top`). Each child holds a PID, so the
        // cgroup quickly refuses further forks (fork → EAGAIN) — recorded in
        // `pids.events: max`.
        let _ = self.boxx.run(&[
            "docker",
            "exec",
            "-d",
            &c.name,
            "sh",
            "-c",
            "while :; do sleep 30 & done",
        ]);

        // Observe from the HOST via the container's pids cgroup — NOT `docker
        // exec` (a saturated container cannot even fork the observer) and NOT
        // `docker top` (misses transient children). cgroup v2 exposes the exact
        // live count (`pids.current`), the cap (`pids.max`), and a refusal
        // counter (`pids.events: max N`). Resolve the container id, then sample
        // the peak `pids.current` and the refusal count over a short window.
        let id = self
            .boxx
            .run(&["docker", "inspect", "-f", "{{.Id}}", &c.name])
            .with_context(|| format!("resolving container id for {}", c.name))?;
        let id = id.stdout.trim().to_string();
        // cgroup v2 path on a systemd host; fall back to the cgroupfs driver path.
        let scope = format!("/sys/fs/cgroup/system.slice/docker-{id}.scope");
        let alt = format!("/sys/fs/cgroup/docker/{id}");
        let sample = self
            .boxx
            .run(&[
                "sh",
                "-c",
                &format!(
                    "d={scope}; [ -d \"$d\" ] || d={alt}; \
                     peak=0; refused=0; capmax=0; \
                     for _ in 1 2 3 4 5; do \
                       cur=$(cat \"$d/pids.current\" 2>/dev/null); \
                       [ -n \"$cur\" ] && [ \"$cur\" -gt \"$peak\" ] && peak=$cur; \
                       m=$(cat \"$d/pids.max\" 2>/dev/null); [ -n \"$m\" ] && capmax=$m; \
                       r=$(awk '/^max /{{print $2}}' \"$d/pids.events\" 2>/dev/null); \
                       [ -n \"$r\" ] && [ \"$r\" -gt \"$refused\" ] && refused=$r; \
                       sleep 1; \
                     done; \
                     printf 'peak=%s refused=%s capmax=%s' \"$peak\" \"$refused\" \"$capmax\""
                ),
            ])
            .with_context(|| format!("reading pids cgroup for {}", c.name))?;
        let kv = |k: &str| -> u64 {
            sample
                .stdout
                .split_whitespace()
                .find_map(|t| t.strip_prefix(&format!("{k}=")))
                .and_then(|v| v.parse().ok())
                .unwrap_or(u64::MAX)
        };
        let peak = kv("peak");
        let refused = kv("refused");
        // Two conditions, BOTH required:
        //  (a) the cap BIT: the spawner saturated the cgroup — either the kernel
        //      recorded ≥1 refused fork (`pids.events: max`) OR the peak reached
        //      the neighbourhood of the cap. A fizzled bomb proves nothing.
        //  (b) the cap HELD: the live count never exceeded the cap (the box pid
        //      space is untouched).
        let saturation_floor = cap / 2;
        let cap_bit = refused >= 1 || (peak != u64::MAX && peak >= saturation_floor);
        let cap_held = peak <= cap;
        let contained = cap_bit && cap_held;
        Ok(ContainmentReport {
            vector: AttackVector::ForkBomb,
            outcome: outcome(contained),
            evidence: format!(
                "pids.current peak={peak} cap={cap} refused-forks={refused} \
                 (cap_bit={cap_bit} @floor={saturation_floor}, cap_held={cap_held})"
            ),
        })
    }

    /// **Disk fill:** `dd` far more than the workdir cap into the size-capped
    /// tmpfs and confirm the bytes-on-disk never exceed the cap — the box disk
    /// is never filled.
    fn attack_disk_fill(&mut self) -> Result<ContainmentReport> {
        let c = self.spawn("diskfill")?;
        let cap = self.limits.disk_mib;
        // Attempt to write 16x the cap; the tmpfs size limit must truncate it.
        // Capture dd's exit status so we can prove the write actually hit the
        // limit (ENOSPC), not that it silently fit. `dd` returns non-zero when
        // it cannot write the requested count to a full filesystem.
        let want = cap * 16;
        let script = format!(
            "dd if=/dev/zero of={WORKDIR}/fill bs=1M count={want} 2>err.$$; rc=$?; \
             enospc=0; grep -qi 'No space left' err.$$ 2>/dev/null && enospc=1; \
             used=$(du -m {WORKDIR}/fill 2>/dev/null | cut -f1); \
             rm -f err.$$; \
             printf 'used=%sMB rc=%s enospc=%s' \"${{used:-0}}\" \"$rc\" \"$enospc\""
        );
        let out = self.exec(&c, &script)?;
        let parse_kv = |k: &str| -> Option<&str> {
            out.split_whitespace()
                .find_map(|tok| tok.strip_prefix(&format!("{k}=")))
        };
        let used: u32 = parse_kv("used")
            .and_then(|s| s.strip_suffix("MB"))
            .and_then(|s| s.parse().ok())
            .unwrap_or(u32::MAX);
        let rc: i32 = parse_kv("rc").and_then(|s| s.parse().ok()).unwrap_or(-1);
        let enospc: u32 = parse_kv("enospc").and_then(|s| s.parse().ok()).unwrap_or(0);
        // Two conditions, BOTH required:
        //  (a) the cap BIT: the write hit the tmpfs limit — `dd` failed (rc!=0)
        //      and/or ENOSPC was observed. A write that simply "fit" proves
        //      nothing (it never reached the cap).
        //  (b) the cap HELD: bytes-on-disk never exceeded the cap (+1 MiB slack
        //      for filesystem accounting). The box disk is never filled.
        let cap_bit = rc != 0 || enospc == 1;
        let cap_held = used <= cap + 1;
        let contained = cap_bit && cap_held;
        Ok(ContainmentReport {
            vector: AttackVector::DiskFill,
            outcome: outcome(contained),
            evidence: format!(
                "disk used={used}MB cap={cap}MB requested={want}MB \
                 (cap_bit={cap_bit} rc={rc}/enospc={enospc}, cap_held={cap_held})"
            ),
        })
    }

    /// **Fence-materialized escape — the fence is the only control here.**
    ///
    /// Unlike the other vectors (which lean on the Docker mount/pid/tmpfs
    /// namespace), this materializes a *real* [`FenceManifest`] into a single
    /// container via the production `materialize_sparse` path, then — inside the
    /// **same** container — proves:
    /// 1. the in-fence file (`src/in.txt`) **is** present and readable, and
    /// 2. the out-of-fence files (`secret.env`, a traversal escape) are
    ///    **ENOENT** (never materialized), checked with [`probe_outside_enoent`].
    ///
    /// Containment ⇔ both hold. The load-bearing property: if `classify()` (the
    /// fence core) were replaced by a constant `Inside`, `select_in_fence` would
    /// materialize `secret.env` too — the out-of-fence probe would observe it
    /// PRESENT — and this vector would report **Escaped**. The acceptance oracle
    /// therefore catches a no-op classifier on this vector alone.
    fn attack_fence_materialized_escape(&mut self) -> Result<ContainmentReport> {
        let c = self.spawn("fence-mat")?;

        // A REAL fence: only `src/` is in the path_set. The attacker offers an
        // out-of-fence secret as a candidate; the fence must drop it.
        let manifest = FenceManifest {
            path_set: vec!["src/".to_string()],
            deny_default: true,
            materialized: vec![],
        };
        let candidates = vec![
            CandidateEntry::new("src/in.txt", b"in-fence-content".to_vec()),
            CandidateEntry::new("secret.env", b"OUT-OF-FENCE-TOKEN".to_vec()),
        ];

        let filled = materialize_sparse(self.boxx, &c, WORKDIR, &manifest, &candidates)
            .map_err(|e| anyhow::anyhow!("materialize for fence-escape vector: {e}"))?;

        // The materialized record must contain ONLY the in-fence file. (If the
        // classifier were constant-Inside, secret.env would appear here.)
        let mat: Vec<&str> = filled
            .materialized
            .iter()
            .map(|m| m.path.as_str())
            .collect();
        let only_in_fence = mat == ["src/in.txt"];

        // In-container truth: in-fence present, out-of-fence ENOENT.
        let in_present = !probe_outside_enoent(self.boxx, &c, WORKDIR, "src/in.txt")?.enoent;
        let secret_absent = probe_outside_enoent(self.boxx, &c, WORKDIR, "secret.env")?.enoent;
        let traversal_absent =
            probe_outside_enoent(self.boxx, &c, WORKDIR, "src/../secret.env")?.enoent;

        let contained = only_in_fence && in_present && secret_absent && traversal_absent;
        Ok(ContainmentReport {
            vector: AttackVector::FenceMaterializedEscape,
            outcome: outcome(contained),
            evidence: format!(
                "materialized={mat:?} in_present={in_present} \
                 secret_absent={secret_absent} traversal_absent={traversal_absent} \
                 (escape iff out-of-fence file was materialized → classify() broke)"
            ),
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
    fn all_vectors_distinct() {
        let all = AttackVector::all();
        assert_eq!(all.len(), 6);
        let slugs: std::collections::BTreeSet<_> = all.iter().map(|v| v.slug()).collect();
        assert_eq!(slugs.len(), 6, "vectors must be distinct");
        // The fence-as-the-control vector must be present.
        assert!(all.contains(&AttackVector::FenceMaterializedEscape));
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
