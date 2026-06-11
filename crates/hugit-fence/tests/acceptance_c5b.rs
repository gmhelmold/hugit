//! WP-C5b acceptance oracle — secrets broker + escape red-team harness.
//!
//! Owned items (one `#[test] item_<n>_<slug>` each):
//!   ② `item_2_zero_secret_in_job_env_proc_disk` — a job that uses the broker
//!      has **zero secret material** in its env, process args, or disk.
//!   ③ `item_3_broker_calls_audited_principal_chain` — every broker call records
//!      the full principal chain from the [`RunnerLease`]; the audit is
//!      secret-free.
//!   ④ `item_4_broker_down_fail_closed` — when the secret store is unreachable
//!      the operation **fails CLOSED**; there is no credential-on-runner
//!      fallback.
//!   ⑤ — TRANSFERRED (WP-R4, runner-transfer campaign): the active escape
//!      red-team (`RedTeamHarness`, six vectors incl. the load-bearing
//!      fence-materialized-escape) moved to corelink-runners WITH the fence
//!      enforcement half (`materialize`/`enforce`) it drives, so the
//!      assertion keeps red-teaming the REAL classifier in its new home —
//!      relocated, never weakened (the X4 disposition, R0 freeze).
//!   ⑥ `item_6_positive_path_via_broker_credential_absent` — a job completes a
//!      credential-needing operation **via the broker** successfully, and the
//!      raw credential is **provably absent** during AND after (env/proc/disk
//!      scan clean).
//!
//! **Box-dependent**: item ⑥ drives the live runner box pinned by
//! `HUGIT_RUNNER_HOST` (the suite exports `91.99.11.196`). When the box is
//! unreachable it **FAILS** (not skip) — per contract. It skips only when
//! `HUGIT_RUNNER_HOST` is unset (the bare cargo gate lane). Items ②③④ are
//! deterministic and never touch the box. The box lane's transport is a
//! test-local transcription of the transferred runner's `SshBox` (the live
//! seam impl is the runner product across the wire — disclosed; this oracle
//! carries its own copy so the lane stays runnable, gated exactly as before).
//!
//! Box-sharing: WP-C5a / C2b run on the same box. Everything here is namespaced
//! with the prefix `hugit-c5b-`; spawn/probe/teardown touch only that prefix.
//! The raw credential is **never printed** anywhere.

use std::collections::BTreeMap;

use hugit_contracts::{RunnerLease, RunnerState};
use hugit_fence::broker::{
    AuditOutcome, Broker, BrokerOp, BrokerRequest, InMemoryStore, SecretRef, fail_closed_audit,
    scan_credential_absent,
};
use hugit_fence::seam::{BoxExec, CmdOutput, RunningContainer};

const IMAGE: &str = "alpine:3.20";
/// All box artifacts for this WP carry this prefix (box-sharing isolation).
const PREFIX: &str = "hugit-c5b-";
/// In-container workspace root the broker delivers results into.
const WORKSPACE_ROOT: &str = "/hugit-c5b-job-ws";

/// Whether the box-dependent acceptance lane is active.
fn box_lane_active() -> bool {
    std::env::var("HUGIT_RUNNER_HOST")
        .ok()
        .is_some_and(|h| !h.trim().is_empty())
}

/// The box-gated lane's transport: a test-local implementation of the fence
/// wire seam over `ssh`, transcribed verbatim from the transferred runner's
/// `SshBox::run` (hugit-runner `src/lease.rs` @ the WP-R4 transfer commit;
/// now the runner product in corelink-runners). The PRODUCTION live impl of
/// the seam is the runner product across the wire (disclosed); this copy
/// exists only so the env-gated acceptance lane stays runnable with the
/// exact same transport semantics (BatchMode, pin-on-first-use known_hosts,
/// ConnectTimeout, unconditional single-quoting).
struct SshSeam {
    /// `user@host` target for ssh.
    target: String,
    /// Optional identity file path.
    identity: Option<String>,
}

impl SshSeam {
    /// Construct from `HUGIT_RUNNER_HOST`, defaulting the user to `root` and
    /// the identity to `~/.ssh/hugit-runner-01` when that file exists.
    fn from_env() -> Option<Self> {
        let host = std::env::var("HUGIT_RUNNER_HOST")
            .ok()
            .filter(|h| !h.trim().is_empty())?;
        let identity = std::env::var("HOME").ok().and_then(|home| {
            let p = format!("{home}/.ssh/hugit-runner-01");
            std::path::Path::new(&p).exists().then_some(p)
        });
        Some(Self {
            target: format!("root@{host}"),
            identity,
        })
    }

    /// Pinned `known_hosts` path (trust-on-first-use, pin thereafter).
    fn known_hosts_path() -> String {
        if let Ok(p) = std::env::var("HUGIT_RUNNER_KNOWN_HOSTS")
            && !p.trim().is_empty()
        {
            return p;
        }
        match std::env::var("HOME") {
            Ok(home) if !home.trim().is_empty() => format!("{home}/.hugit/known_hosts"),
            _ => ".hugit/known_hosts".to_string(),
        }
    }

    /// POSIX single-quote a command vector into one remote shell string —
    /// every argument unconditionally quoted (no "looks-safe" passthrough).
    fn shell_join(argv: &[&str]) -> String {
        argv.iter()
            .map(|a| {
                if a.is_empty() {
                    "''".to_string()
                } else {
                    format!("'{}'", a.replace('\'', r"'\''"))
                }
            })
            .collect::<Vec<_>>()
            .join(" ")
    }
}

impl BoxExec for SshSeam {
    fn run(&self, argv: &[&str]) -> anyhow::Result<CmdOutput> {
        use anyhow::Context;
        let remote = Self::shell_join(argv);
        let known_hosts = Self::known_hosts_path();
        let mut cmd = std::process::Command::new("ssh");
        if let Some(id) = &self.identity {
            cmd.arg("-i").arg(id);
        }
        // pin-on-first-use: accept-new records the host key on first contact
        // and verifies against the pinned UserKnownHostsFile thereafter.
        cmd.arg("-o")
            .arg("BatchMode=yes")
            .arg("-o")
            .arg("StrictHostKeyChecking=accept-new")
            .arg("-o")
            .arg(format!("UserKnownHostsFile={known_hosts}"))
            .arg("-o")
            .arg("ConnectTimeout=15")
            .arg(&self.target)
            .arg(&remote);
        let out = cmd
            .output()
            .with_context(|| format!("failed to spawn ssh to {}", self.target))?;
        Ok(CmdOutput {
            code: out.status.code(),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        })
    }
}

/// Connect to the live box; FAIL (panic) if it is unreachable, per contract.
fn live_box() -> SshSeam {
    let boxx = SshSeam::from_env().expect("HUGIT_RUNNER_HOST must be set inside the box lane");
    let ping = boxx
        .run(&["docker", "version", "--format", "{{.Server.Version}}"])
        .expect("ssh to runner box failed to spawn");
    assert!(
        ping.ok() && !ping.stdout.trim().is_empty(),
        "runner box {} unreachable or docker down (code={:?} stderr={:?}); \
         box-dependent acceptance must FAIL, not skip",
        boxx.target,
        ping.code,
        ping.stderr.trim(),
    );
    boxx
}

/// Ensure the job image is present on the box.
fn ensure_image(boxx: &SshSeam) {
    let pull = boxx
        .run(&["docker", "pull", IMAGE])
        .expect("docker pull failed to spawn");
    assert!(
        pull.ok(),
        "docker pull {IMAGE} failed: {}",
        pull.stderr.trim()
    );
}

/// A unique, prefix-namespaced container name for this run.
fn fresh_name(slug: &str) -> String {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    format!("{PREFIX}{slug}-{nonce}")
}

/// Spawn a `hugit-c5b-`-prefixed, network-isolated idle job container.
fn spawn_job(boxx: &SshSeam, name: &str) -> RunningContainer {
    let out = boxx
        .run(&[
            "docker",
            "run",
            "-d",
            "--rm",
            "--name",
            name,
            "--network",
            "none",
            "--label",
            "hugit.wp=c5b",
            IMAGE,
            "sleep",
            "300",
        ])
        .expect("docker run failed to spawn");
    assert!(out.ok(), "docker run {name} failed: {}", out.stderr.trim());
    RunningContainer {
        name: name.to_string(),
    }
}

/// Force-remove a `hugit-c5b-`-prefixed container (idempotent, prefix-scoped).
fn teardown_job(boxx: &SshSeam, name: &str) {
    assert!(
        name.starts_with(PREFIX),
        "refusing to tear down non-c5b container {name}"
    );
    let _ = boxx.run(&["docker", "rm", "-f", name]);
}

fn lease(principals: &[&str]) -> RunnerLease {
    RunnerLease {
        lease_id: "lease/c5b-acceptance".to_string(),
        principal_chain: principals.iter().map(|s| s.to_string()).collect(),
        path_set: vec!["src/".to_string()],
        expiry: 0,
        net_policy: "none".to_string(),
        tmp_root: "/work/tmp".to_string(),
        state: RunnerState::Held,
    }
}

fn store_with(name: &str, secret: &[u8]) -> InMemoryStore {
    let mut m = BTreeMap::new();
    m.insert(name.to_string(), secret.to_vec());
    InMemoryStore::new(m)
}

// ── ② zero secret material in the job (deterministic core) ───────────────────
#[test]
fn item_2_zero_secret_in_job_env_proc_disk() {
    // The broker's result carries only the public output; the credential is
    // consumed entirely inside `execute` and never appears in the response,
    // the audit, or anything destined for the runner.
    let secret = b"NEVER-PRINT-THIS-deploy-key-42";
    let broker = Broker::new(store_with("deploy-key", secret));
    let l = lease(&["user:owner", "agent:9"]);
    let req = BrokerRequest {
        lease: &l,
        op: BrokerOp::Sign {
            secret: SecretRef::new("deploy-key"),
            message: b"artifact-digest-abc".to_vec(),
        },
    };
    let resp = broker.execute(&req).expect("broker op should complete");

    // The output (a hex signature) must not be or contain the credential.
    let secret_str = String::from_utf8_lossy(secret);
    assert!(
        !resp.output.contains(secret_str.as_ref()),
        "broker output must not contain the credential"
    );
    // The audit record (which IS allowed to leave the broker) is secret-free.
    let line = resp.audit.to_log_line();
    assert!(
        !line.contains(secret_str.as_ref()),
        "audit line must never carry secret material"
    );
    // The whole response, debug-formatted, must be credential-free.
    assert!(
        !format!("{resp:?}").contains(secret_str.as_ref()),
        "no secret material may appear anywhere in the broker response"
    );
}

// ── ③ broker calls audited with the principal chain ──────────────────────────
#[test]
fn item_3_broker_calls_audited_principal_chain() {
    let broker = Broker::new(store_with("k", b"s3cr3t-material"));
    let l = lease(&["user:alice", "team:platform", "agent:42"]);
    let req = BrokerRequest {
        lease: &l,
        op: BrokerOp::Sign {
            secret: SecretRef::new("k"),
            message: b"m".to_vec(),
        },
    };
    let resp = broker.execute(&req).expect("op should complete");
    // The audit copies the FULL ordered principal chain from the lease.
    assert_eq!(resp.audit.principal_chain, l.principal_chain);
    assert_eq!(resp.audit.lease_id, l.lease_id);
    assert_eq!(resp.audit.outcome, AuditOutcome::Completed);
    assert_eq!(resp.audit.op, "sign");
    assert_eq!(resp.audit.secret_name, "k");
    // The principal chain is rendered in order, secret-free.
    let line = resp.audit.to_log_line();
    assert!(line.contains("user:alice>team:platform>agent:42"));
    assert!(!line.contains("s3cr3t-material"));
}

// ── ④ broker down → fail CLOSED (no credential-on-runner fallback) ───────────
#[test]
fn item_4_broker_down_fail_closed() {
    let broker = Broker::new(InMemoryStore::down());
    let l = lease(&["user:owner"]);
    let req = BrokerRequest {
        lease: &l,
        op: BrokerOp::Sign {
            secret: SecretRef::new("k"),
            message: b"m".to_vec(),
        },
    };
    let err = broker
        .execute(&req)
        .expect_err("must fail when store is down");
    assert!(
        err.is_broker_down(),
        "broker-down must fail CLOSED (no fallback); got {err:?}"
    );
    // Even on failure, the call is audited as FailedClosed with the chain.
    let audit = fail_closed_audit(&req);
    assert_eq!(audit.outcome, AuditOutcome::FailedClosed);
    assert_eq!(audit.principal_chain, l.principal_chain);
    // The error message itself is secret-free.
    assert!(!format!("{err}").contains("k=") || !format!("{err}").contains("material"));
}

// ── ⑤ TRANSFERRED (WP-R4): the escape red-team harness + its acceptance ──────
// moved to corelink-runners with the fence enforcement half it drives
// (materialize/enforce + RedTeamHarness, all six vectors incl. the
// load-bearing fence-materialized-escape and the hermetic FakeFsBox oracle).
// The assertions run unmodified against the same production code in the new
// home — relocated, never weakened.

// ── ⑥ positive path via the broker, credential provably absent ───────────────
#[test]
fn item_6_positive_path_via_broker_credential_absent() {
    if !box_lane_active() {
        // NOTE: cargo test captures stdout by default; pass --nocapture or run
        // with `cargo test -- --nocapture` to see this line in the terminal.
        println!(
            "SKIPPED item_6_positive_path_via_broker_credential_absent: \
             HUGIT_RUNNER_HOST unset — live lane not exercised"
        );
        return;
    }
    let boxx = live_box();
    ensure_image(&boxx);
    let name = fresh_name("positive");
    let container = spawn_job(&boxx, &name);

    // The credential is held ONLY by the broker. The job needs a signature over
    // its artifact digest but must never see the key.
    let secret = b"NEVER-PRINT-credential-positive-path-7";
    let broker = Broker::new(store_with("signing-key", secret));
    let l = lease(&["user:owner", "agent:positive"]);
    let req = BrokerRequest {
        lease: &l,
        op: BrokerOp::Sign {
            secret: SecretRef::new("signing-key"),
            message: b"artifact:release-9.0".to_vec(),
        },
    };

    let result = (|| {
        // BEFORE: the credential is already absent (sanity — nothing planted).
        let before = scan_credential_absent(&boxx, &container, WORKSPACE_ROOT, secret)
            .map_err(|e| format!("pre-scan: {e}"))?;
        if !before.is_clean() {
            return Err(format!("pre-scan unexpectedly dirty: {}", before.report));
        }

        // The broker completes the credential-needing op and delivers ONLY the
        // signature into the job container. The result path is FENCE-RELATIVE;
        // the broker joins it under WORKSPACE_ROOT (and rejects any `..`/absolute
        // result path before resolving the secret — see the traversal guard).
        let result_rel = "signature.hex";
        let result_path = format!("{WORKSPACE_ROOT}/{result_rel}");
        let resp = broker
            .execute_into_container(&boxx, &container, WORKSPACE_ROOT, result_rel, &req)
            .map_err(|e| format!("broker execute_into_container: {e}"))?;

        // The job CAN use the result: the signature file is present & non-empty.
        let read = boxx
            .run(&["docker", "exec", &container.name, "cat", &result_path])
            .map_err(|e| format!("read signature: {e}"))?;
        if !(read.ok() && read.stdout.trim() == resp.output && resp.output.len() == 64) {
            return Err(format!(
                "signature must be delivered to the job (got len {}, ok={})",
                read.stdout.trim().len(),
                read.ok()
            ));
        }

        // DURING/AFTER: scan env + proc + disk — the raw credential is provably
        // ABSENT. (We never print it; the scan reports only counts.)
        let after = scan_credential_absent(&boxx, &container, WORKSPACE_ROOT, secret)
            .map_err(|e| format!("post-scan: {e}"))?;
        if !after.is_clean() {
            return Err(format!(
                "raw credential leaked into the job: {}",
                after.report
            ));
        }
        // And the delivered output is provably NOT the credential.
        let secret_str = String::from_utf8_lossy(secret);
        if resp.output.contains(secret_str.as_ref()) {
            return Err("delivered output contains the credential".to_string());
        }

        // NEGATIVE (box-lane): a `..`/absolute result path that would deliver
        // OUTSIDE the workspace root is refused fail-closed, with NO file written
        // anywhere — the broker rejects before resolving the secret or touching
        // the box. Probe a would-be escape target and confirm it is absent.
        for evil_rel in ["../escaped.hex", "/tmp/escaped.hex"] {
            let err = broker
                .execute_into_container(&boxx, &container, WORKSPACE_ROOT, evil_rel, &req)
                .err()
                .ok_or_else(|| format!("escaping result_rel {evil_rel:?} must be rejected"))?;
            if !format!("{err}").contains("outside the workspace root") {
                return Err(format!("unexpected error for {evil_rel:?}: {err}"));
            }
        }
        // The most dangerous absolute target must not exist on the box.
        let leaked = boxx
            .run(&[
                "docker",
                "exec",
                &container.name,
                "sh",
                "-c",
                "test -e /tmp/escaped.hex && echo LEAKED || echo SAFE",
            ])
            .map_err(|e| format!("probe escape target: {e}"))?;
        if leaked.stdout.trim() != "SAFE" {
            return Err("a rejected escape result path leaked a file".to_string());
        }
        Ok(())
    })();

    teardown_job(&boxx, &name);
    result.expect("positive path: op completes via broker, credential absent");
}
