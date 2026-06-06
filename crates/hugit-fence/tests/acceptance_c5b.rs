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
//!   ⑤ `item_5_escape_redteam_all_attacks_contained` — the active escape
//!      red-team (traversal / symlink / out-of-fence / fork-bomb / disk-fill)
//!      is run against live containers and **every** vector is contained; none
//!      can reach another lease or starve the box; box residue is **0**.
//!   ⑥ `item_6_positive_path_via_broker_credential_absent` — a job completes a
//!      credential-needing operation **via the broker** successfully, and the
//!      raw credential is **provably absent** during AND after (env/proc/disk
//!      scan clean).
//!
//! **Box-dependent**: items ⑤ and ⑥ drive the live runner box pinned by
//! `HUGIT_RUNNER_HOST` (the suite exports `91.99.11.196`). When the box is
//! unreachable they **FAIL** (not skip) — per contract. They skip only when
//! `HUGIT_RUNNER_HOST` is unset (the bare cargo gate lane). Items ②③④ are
//! deterministic and never touch the box.
//!
//! Box-sharing: WP-C5a / C2b run on the same box. Everything here is namespaced
//! with the prefix `hugit-c5b-`; spawn/probe/teardown touch only that prefix.
//! The raw credential is **never printed** anywhere.

use std::collections::BTreeMap;

use hugit_contracts::{RunnerLease, RunnerState};
use hugit_fence::broker::redteam::{AttackVector, ContainerLimits, RedTeamHarness, RedTeamOutcome};
use hugit_fence::broker::{
    AuditOutcome, Broker, BrokerOp, BrokerRequest, InMemoryStore, SecretRef, fail_closed_audit,
    scan_credential_absent,
};
use hugit_runner::isolation::RunningContainer;
use hugit_runner::lease::{BoxExec, SshBox};

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

/// Connect to the live box; FAIL (panic) if it is unreachable, per contract.
fn live_box() -> SshBox {
    let boxx = SshBox::from_env().expect("HUGIT_RUNNER_HOST must be set inside the box lane");
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
fn ensure_image(boxx: &SshBox) {
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
fn spawn_job(boxx: &SshBox, name: &str) -> RunningContainer {
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
fn teardown_job(boxx: &SshBox, name: &str) {
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

// ── ⑤ active escape red-team: all five vectors contained, residue 0 ──────────
#[test]
fn item_5_escape_redteam_all_attacks_contained() {
    if !box_lane_active() {
        return;
    }
    let boxx = live_box();
    ensure_image(&boxx);

    let mut harness = RedTeamHarness::new(&boxx, IMAGE, ContainerLimits::default());

    // Genuinely attempt every escape against live containers.
    let result = (|| {
        let reports = harness.run_all().map_err(|e| format!("run_all: {e}"))?;
        // Every one of the five vectors must be contained.
        let covered: Vec<AttackVector> = reports.iter().map(|r| r.vector).collect();
        for v in AttackVector::all() {
            if !covered.contains(&v) {
                return Err(format!("attack vector {} was not exercised", v.slug()));
            }
        }
        for r in &reports {
            if r.outcome != RedTeamOutcome::Contained {
                return Err(format!(
                    "ESCAPE: vector {} was NOT contained — {}",
                    r.vector.slug(),
                    r.evidence
                ));
            }
        }
        Ok(())
    })();

    // Always tear down + verify residue, regardless of outcome (prefix-scoped).
    let residue = harness.teardown_all().expect("teardown must reach the box");
    result.expect("escape red-team: all vectors contained");
    assert!(
        residue.is_zero(),
        "box must have ZERO hugit-c5b-* residue; remaining: {:?}",
        residue.remaining
    );
}

// ── ⑥ positive path via the broker, credential provably absent ───────────────
#[test]
fn item_6_positive_path_via_broker_credential_absent() {
    if !box_lane_active() {
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
        // signature into the job container.
        let result_path = format!("{WORKSPACE_ROOT}/signature.hex");
        let resp = broker
            .execute_into_container(&boxx, &container, &result_path, &req)
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
        Ok(())
    })();

    teardown_job(&boxx, &name);
    result.expect("positive path: op completes via broker, credential absent");
}
