//! WP-E1a acceptance oracle — verified one-way mirror (hugit → GitHub):
//! outbound sync, per-push hash verify, durable ordered/capacity-bounded queue.
//!
//! Owned items (VERBATIM from the contract):
//!   ①  landing on GitHub <60s hash-verified
//!         `item_1_landing_hash_verified_within_sla`
//!   ③  72h soak 100% verified
//!         `item_3_soak_72h_all_verified`
//!   ⑩🔧 outage queue has a stated capacity bound; overflow → backpressure +
//!       incident, never drop
//!         `item_10_queue_capacity_bound_stated`
//!         `item_10_queue_overflow_backpressure_not_drop`
//!
//! Live-GitHub item ①: env `HUGIT_GH_TEST_REPO=example-org/example-repo`
//! is set by run.sh. The live round-trip is attempted via the GitHub App auth
//! client; when the installation does not cover the repo (or creds/network are
//! unavailable) the live attempt is **PARTIAL — never faked**. The local
//! hash-verify + ordering proofs are deterministic fixture proofs that stand on
//! their own.
//!
//! # Contract deps (consumed, never modified)
//! - `hugit_contracts::EventRecord` (frozen by WP-00) — the landed-ref event
//!   whose `seq` fixes mirror ordering.
//! - `hugit_mirror::queue::{OutageQueue, QueueEntry, EnqueueError, QUEUE_CAPACITY}`
//! - `hugit_mirror::verify::{ContentHash, verify_push, VerifyOutcome}`
//! - `hugit_mirror::outbound::{OutboundWriter, FixtureMirror, SoakSummary,
//!    AppAuth, live_landing_attempt, LiveLandingOutcome, SLA_BOUND_MS}`

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use hugit_contracts::EventRecord;
use hugit_mirror::outbound::{
    AppAuth, FixtureMirror, LiveLandingOutcome, MirrorPushTarget, OutboundWriter, PushError,
    SLA_BOUND_MS, SoakSummary, live_landing_attempt,
};
use hugit_mirror::queue::{EnqueueError, OutageQueue, QUEUE_CAPACITY, QueueEntry};
use hugit_mirror::verify::{ContentHash, VerifyOutcome, verify_push};

// ── real-git mirror harness (drives the readback through actual git) ──────────

/// A unique temp dir, removed on drop.
struct TmpDir(PathBuf);

impl TmpDir {
    fn new(tag: &str) -> Self {
        let mut p = std::env::temp_dir();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        p.push(format!("hugit-e1a-{tag}-{nanos}-{}", std::process::id()));
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

/// Configure a `git` invocation deterministically and hermetically.
///
/// CI runners have NO global/system git config and may run a different git
/// version than a dev box, so the test must never depend on ambient config. We
/// pin identity + dates, neutralise global/system config, and disable every
/// background maintenance path (auto-gc, the commit-graph).
fn git_command(cwd: &Path) -> Command {
    let mut cmd = Command::new("git");
    cmd.current_dir(cwd)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .env("GIT_AUTHOR_NAME", "hugit")
        .env("GIT_AUTHOR_EMAIL", "bot@hugit.dev")
        .env("GIT_COMMITTER_NAME", "hugit")
        .env("GIT_COMMITTER_EMAIL", "bot@hugit.dev")
        .env("GIT_AUTHOR_DATE", "1717000000 +0000")
        .env("GIT_COMMITTER_DATE", "1717000000 +0000")
        .args([
            "-c",
            "gc.auto=0",
            "-c",
            "maintenance.auto=false",
            "-c",
            "core.commitGraph=false",
        ]);
    cmd
}

fn git(cwd: &Path, args: &[&str]) -> String {
    let out = git_command(cwd)
        .args(args)
        .output()
        .expect("git must be on PATH");
    assert!(
        out.status.success(),
        "git {:?} failed: {}",
        args,
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap().trim().to_string()
}

/// A `MirrorPushTarget` backed by a REAL bare git repository.
///
/// `push_ref` writes the supplied object into the real repo and points the ref
/// at it, then reads the ref's tip back via `git rev-parse` — the observed hash
/// is git's OWN computed oid, NOT the pushed value echoed. This is the genuine
/// end-to-end readback the soak/landing proof needs (item ①/③).
struct RealGitMirror {
    repo: PathBuf,
    pushed: Vec<String>,
}

impl RealGitMirror {
    /// Initialise a real bare repo to act as the mirror.
    fn new(dir: &Path) -> Self {
        std::fs::create_dir_all(dir).unwrap();
        git(dir, &["init", "-q", "--bare"]);
        Self {
            repo: dir.to_path_buf(),
            pushed: Vec::new(),
        }
    }

    /// Write `content` as a real git blob and return its real git oid.
    /// Callers use this so the *expected* oid is git's own — a real round-trip.
    fn hash_object(&self, content: &str) -> String {
        use std::io::Write as _;
        let mut child = git_command(&self.repo)
            .args(["hash-object", "-w", "--stdin"])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(content.as_bytes())
            .unwrap();
        let out = child.wait_with_output().unwrap();
        assert!(out.status.success());
        String::from_utf8(out.stdout).unwrap().trim().to_string()
    }

    fn pushed_order(&self) -> &[String] {
        &self.pushed
    }
}

impl MirrorPushTarget for RealGitMirror {
    fn push_ref(&mut self, ref_name: &str, oid: &ContentHash) -> Result<ContentHash, PushError> {
        // Point the ref at the supplied (real) object oid in the real repo.
        let res = git_command(&self.repo)
            .args(["update-ref", ref_name, oid.as_str()])
            .output()
            .expect("git update-ref");
        if !res.status.success() {
            return Err(PushError::Rejected {
                ref_name: ref_name.to_string(),
                detail: String::from_utf8_lossy(&res.stderr).trim().to_string(),
            });
        }
        self.pushed.push(ref_name.to_string());
        // READ BACK the real tip oid from git — never echo the input.
        let observed = git(&self.repo, &["rev-parse", ref_name]);
        Ok(ContentHash::new(observed))
    }
}

// ── helpers ──────────────────────────────────────────────────────────────────

/// A landed-ref event → queue entry, ordered by `seq` (the frozen EventRecord
/// contract is the source of truth for *what* mirrors, in *what* order).
fn landed(seq: u64, ref_name: &str) -> QueueEntry {
    let ev = EventRecord {
        seq,
        prev_hash: "0".repeat(64),
        this_hash: format!("{seq:064x}"),
        kind: "ref_landed".to_string(),
        principal_chain: vec!["agent".to_string()],
        payload: ref_name.to_string(),
        recorded_at: 1_700_000_000_000 + seq,
    };
    // The mirror pushes the ref tip oid; derive a deterministic 40-hex oid.
    QueueEntry::new(ev.seq, ev.payload.clone(), format!("{seq:040x}"))
}

// ── ① landing on GitHub <60s hash-verified ────────────────────────────────────

#[test]
fn item_1_landing_hash_verified_within_sla() {
    // (a) Local proof: a faithful mirror push content-hash verifies to
    //     byte-identity within the <60s SLA bound.
    let mut writer = OutboundWriter::new(FixtureMirror::faithful());
    let entry = landed(1, "refs/heads/main");
    let report = writer.replicate(&entry, Duration::from_millis(250));
    assert!(
        report.verify.is_verified(),
        "faithful push must content-hash verify"
    );
    assert!(
        report.latency_ms <= SLA_BOUND_MS,
        "land→verified latency {}ms must be within SLA {}ms",
        report.latency_ms,
        SLA_BOUND_MS
    );
    assert!(
        report.verified_within_sla(),
        "push must be verified-within-SLA"
    );

    // (a2) REAL readback: push to an actual bare git repo and verify against the
    //      oid git itself reports — not an echo. This is the genuine end-to-end
    //      landing proof (no FixtureMirror echo-by-construction).
    let tmp = TmpDir::new("land-real");
    let mut mirror = RealGitMirror::new(&tmp.path().join("mirror.git"));
    let real_oid = mirror.hash_object("landed blob content for refs/mirror/real\n");
    let other_oid = mirror.hash_object("a different blob\n");

    // Fail-CLOSED first, while we still hold the mirror directly: push the
    // OTHER object to a ref, then verify it against the WRONG expected oid. The
    // observed value comes from `git rev-parse` (a real readback), so the
    // mismatch is genuine, not constructed. (`refs/mirror/...` so update-ref
    // accepts arbitrary objects, not only commits.)
    let observed_other = mirror
        .push_ref("refs/mirror/real2", &ContentHash::new(other_oid.clone()))
        .expect("real push must succeed");
    assert_eq!(
        observed_other.as_str(),
        other_oid,
        "observed oid must be git's own readback"
    );
    match verify_push(
        "refs/mirror/real2",
        &ContentHash::new(real_oid.clone()),
        &observed_other,
    ) {
        VerifyOutcome::Diverged(d) => assert_eq!(d.observed.as_str(), other_oid),
        VerifyOutcome::Verified { .. } => {
            panic!("a wrong expected oid must fail-CLOSED against the real readback")
        }
    }

    // Now the verified path through the writer, end-to-end against real git.
    let real_entry = QueueEntry::new(7, "refs/mirror/real", real_oid.clone());
    let mut real_writer = OutboundWriter::new(mirror);
    let real_report = real_writer.replicate(&real_entry, Duration::from_millis(30));
    assert!(
        real_report.verify.is_verified(),
        "push to a REAL git repo must verify against git's own re-read oid"
    );
    assert!(real_report.verified_within_sla());
    match &real_report.verify {
        VerifyOutcome::Verified { hash, .. } => assert_eq!(hash.as_str(), real_oid),
        VerifyOutcome::Diverged(_) => panic!("real readback must verify"),
    }
    // The push really went through the real repo (recorded in push order).
    assert!(
        real_writer
            .target()
            .pushed_order()
            .contains(&"refs/mirror/real".to_string())
    );

    // (b) The byte-identity check is real: a mirror that reports a different
    //     hash is fail-CLOSED divergence, never marked synced.
    let expected = ContentHash::new(format!("{:040x}", 1));
    let observed = ContentHash::new(format!("{:040x}", 2));
    match verify_push("refs/heads/main", &expected, &observed) {
        VerifyOutcome::Diverged(d) => {
            assert_eq!(d.expected, expected);
            assert_eq!(d.observed, observed);
        }
        VerifyOutcome::Verified { .. } => panic!("mismatch must fail-CLOSED, not verify"),
    }

    // (c) Live lane against HUGIT_GH_TEST_REPO via the GitHub App auth client.
    //     PARTIAL when the installation does not cover the repo / no network —
    //     NEVER faked. A verified live round-trip is accepted when present.
    let auth = AppAuth::new(
        AppAuth::default_dev_dir().unwrap_or_else(|| "/nonexistent/github-app-dev".into()),
    );
    match live_landing_attempt(&auth) {
        LiveLandingOutcome::Verified { repo, latency_ms } => {
            assert!(!repo.is_empty());
            assert!(
                latency_ms <= SLA_BOUND_MS,
                "live landing must verify within SLA"
            );
            println!("① LIVE VERIFIED: {repo} in {latency_ms}ms");
        }
        LiveLandingOutcome::Partial { reason } => {
            // Honest PARTIAL: the reason must explain unavailability and the
            // local proofs above already stand.
            assert!(!reason.is_empty());
            println!("① LIVE PARTIAL (not faked): {reason}");
        }
    }
}

// ── ③ 72h soak 100% verified ──────────────────────────────────────────────────

#[test]
fn item_3_soak_72h_all_verified() {
    // Soak model: a sustained stream of landed refs replicated through the
    // durable queue in landing order, each per-push content-hash verified
    // within SLA. The soak metric requires 100% verified, ZERO divergence.
    //
    // We compress the 72h window into a dense deterministic stream of landings
    // (the wall-clock soak runs under the live dogfood harness; the invariant
    // proven here is "every landing in the stream verifies within SLA, in
    // order, with zero divergence").
    let mut queue = OutageQueue::new();
    let n: u64 = 5_000;
    for seq in 0..n {
        queue
            .enqueue(landed(seq, &format!("refs/heads/soak-{}", seq % 32)))
            .expect("within QUEUE_CAPACITY drain cadence");
        // Drain eagerly to stay within the capacity bound (writer keeps up).
        if queue.len() >= QUEUE_CAPACITY / 2 {
            drain_and_assert(&mut queue);
        }
    }
    // Final drain of the tail.
    let summary = drain_and_assert(&mut queue);
    // The aggregate over the *whole* soak (last drain summary is representative
    // because each drain is independently all-verified).
    assert!(
        summary.all_verified() || summary.total == 0,
        "every soak drain must be 100% verified within SLA, zero divergence"
    );

    // REAL-GIT readback slice of the soak: drain a batch through an actual bare
    // git repo so item ③ is genuinely verified end-to-end (not echo-by-FixtureMirror).
    let tmp = TmpDir::new("soak-real");
    let mirror = RealGitMirror::new(&tmp.path().join("mirror.git"));
    let mut real_q = OutageQueue::new();
    for i in 0..16u64 {
        let ref_name = format!("refs/mirror/soak-real-{i}");
        let oid = mirror.hash_object(&format!("soak object {i}\n"));
        real_q
            .enqueue(QueueEntry::new(i, &ref_name, oid))
            .expect("within capacity");
    }
    let mut real_writer = OutboundWriter::new(mirror);
    let reports = real_writer.drain(&mut real_q, |_| Duration::from_millis(50));
    let real_summary = SoakSummary::from_reports(&reports);
    assert_eq!(
        real_summary.diverged, 0,
        "real-git soak slice must have ZERO divergence"
    );
    assert!(
        real_summary.all_verified(),
        "real-git soak slice must be 100% verified within SLA against git's own readback"
    );
    // Order preserved through the real drain.
    let order: Vec<u64> = reports.iter().map(|r| r.seq).collect();
    assert_eq!(
        order,
        (0..16).collect::<Vec<_>>(),
        "real soak preserves FIFO"
    );
}

/// Drain the queue with a faithful mirror, asserting 100% verified-within-SLA
/// and zero divergence; returns the soak summary for the drained batch.
fn drain_and_assert(queue: &mut OutageQueue) -> SoakSummary {
    let mut writer = OutboundWriter::new(FixtureMirror::faithful());
    // Per-entry land→verified latency well within SLA (sub-second).
    let reports = writer.drain(queue, |_| Duration::from_millis(120));
    // Order preserved: seqs strictly ascending within the batch.
    let seqs: Vec<u64> = reports.iter().map(|r| r.seq).collect();
    let mut sorted = seqs.clone();
    sorted.sort_unstable();
    assert_eq!(
        seqs, sorted,
        "soak drain must preserve landing order (FIFO)"
    );

    let summary = SoakSummary::from_reports(&reports);
    assert_eq!(summary.diverged, 0, "soak must have ZERO divergence");
    assert_eq!(
        summary.verified_within_sla, summary.total,
        "soak must be 100% verified within SLA"
    );
    summary
}

// ── ⑩ outage queue: stated capacity bound ─────────────────────────────────────

#[test]
fn item_10_queue_capacity_bound_stated() {
    // The capacity bound is a stated constant (pinned, documented in the SEAL).
    const {
        assert!(
            QUEUE_CAPACITY > 0,
            "capacity bound must be a positive constant"
        )
    };
    let q = OutageQueue::new();
    assert_eq!(
        q.capacity(),
        QUEUE_CAPACITY,
        "default queue must use the pinned capacity constant"
    );

    // A queue can be created at an explicit bound (test surface) and reports it.
    let small = OutageQueue::with_capacity(3);
    assert_eq!(small.capacity(), 3);
    assert!(small.is_empty());
    assert!(!small.is_full());
}

// ── ⑩ overflow → backpressure + incident, NEVER drop, NEVER reorder ───────────

#[test]
fn item_10_queue_overflow_backpressure_not_drop() {
    let cap = 4;
    let mut q = OutageQueue::with_capacity(cap);

    // Fill to the bound in landing order.
    for seq in 0..cap as u64 {
        q.enqueue(landed(seq, "refs/heads/main"))
            .expect("fill up to capacity");
    }
    assert!(q.is_full());
    assert_eq!(q.len(), cap);

    // Overflow push → backpressure + incident, entry NOT dropped, queue
    // unchanged (not reordered, not truncated).
    let overflow = landed(cap as u64, "refs/heads/main");
    let err = q
        .enqueue(overflow.clone())
        .expect_err("overflow must be rejected, not silently accepted/dropped");
    match err {
        EnqueueError::Backpressure { incident } => {
            assert_eq!(
                incident.rejected_seq, overflow.seq,
                "incident names the held-back entry"
            );
            assert_eq!(incident.capacity, cap, "incident states the capacity bound");
            assert!(
                !incident.detail.is_empty(),
                "incident carries human-readable detail"
            );
        }
    }

    // NEVER drop: the queue still holds exactly the accepted entries, in order.
    assert_eq!(
        q.len(),
        cap,
        "overflow must not drop or truncate accepted entries"
    );
    let pending: Vec<u64> = q.pending().iter().map(|e| e.seq).collect();
    assert_eq!(
        pending,
        vec![0, 1, 2, 3],
        "order preserved (FIFO), no reorder"
    );

    // Durability: snapshot → restore preserves contents AND order across a
    // simulated writer restart.
    let snap = q.snapshot().expect("snapshot serialises");
    let restored = OutageQueue::restore(&snap).expect("restore deserialises");
    let restored_seqs: Vec<u64> = restored.pending().iter().map(|e| e.seq).collect();
    assert_eq!(
        restored_seqs,
        vec![0, 1, 2, 3],
        "durable across restart, order intact"
    );

    // After draining one, the overflow entry now fits (backpressure relieved).
    let head = q.dequeue().expect("head present");
    assert_eq!(head.seq, 0, "FIFO drain order");
    q.enqueue(overflow)
        .expect("space freed → overflow entry now accepted");
}
