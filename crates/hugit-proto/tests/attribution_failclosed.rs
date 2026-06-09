//! FP-3 oracle suite — attribution fail-closed + consolidated JSON escaping.
//!
//! Three pre-decided oracles (audit-remediation FP-3):
//!   (a) `SerializedWriter::push` with an empty `principal_chain` returns a typed
//!       `Rejected(MissingAttribution)` outcome and does NOT panic (was: panic via
//!       `.expect()` on the external-change recorder).
//!   (b) `receive_pack` with an empty `principal_chain` returns `Err` and appends
//!       NO event (was: silently recorded an unattributed raw-push event).
//!   (c) a ref name carrying a control char is escaped identically by the single
//!       consolidated `json_str` on both the external and the store path.

use hugit_proto::write::receive::{ReceiveRequest, RecvLimits, RefUpdate as RecvRefUpdate};
use hugit_proto::write::store::InMemoryCas;
use hugit_proto::{FlagGate, PushOutcome, PushReject, RefUpdate, SerializedWriter};

use hugit_refstore::EventLog;

/// A real, valid one-commit packfile + the head oid it delivers, built with the
/// system git binary. Kept local (not the shared fixture module) so this oracle
/// binary has no unused-fixture dead code.
struct RealPack {
    pack: Vec<u8>,
    head_oid: String,
}

fn real_pack(seed: &str) -> RealPack {
    use std::io::Write;
    use std::process::{Command, Stdio};

    let dir = std::env::temp_dir().join(format!(
        "hugit-attrib-fx-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).expect("create scratch dir");

    let git = |args: &[&str]| -> String {
        let out = Command::new("git")
            .arg("-C")
            .arg(&dir)
            .args(args)
            .output()
            .unwrap_or_else(|e| panic!("spawn git {args:?}: {e}"));
        assert!(
            out.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).into_owned()
    };

    git(&["init", "-q", "-b", "main", "."]);
    git(&["config", "user.email", "test@hugit.dev"]);
    git(&["config", "user.name", "hugit-test"]);
    git(&["config", "commit.gpgsign", "false"]);
    std::fs::write(dir.join("f"), format!("content {seed}\n")).expect("write fixture file");
    git(&["add", "f"]);
    {
        let out = Command::new("git")
            .arg("-C")
            .arg(&dir)
            .args(["commit", "-q", "-m", "fixture commit"])
            .env("GIT_AUTHOR_DATE", "2026-06-05T00:00:00 +0000")
            .env("GIT_COMMITTER_DATE", "2026-06-05T00:00:00 +0000")
            .output()
            .expect("spawn git commit");
        assert!(out.status.success(), "git commit failed");
    }
    let head_oid = git(&["rev-parse", "HEAD"]).trim().to_string();

    // pack the reachable closure to stdout (what a real push delivers).
    let mut child = Command::new("git")
        .arg("-C")
        .arg(&dir)
        .args(["pack-objects", "--revs", "--stdout"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn git pack-objects");
    child
        .stdin
        .take()
        .expect("git stdin")
        .write_all(head_oid.as_bytes())
        .expect("write git stdin");
    let out = child.wait_with_output().expect("git pack-objects wait");
    assert!(out.status.success(), "git pack-objects failed");

    let _ = std::fs::remove_dir_all(&dir);
    RealPack {
        pack: out.stdout,
        head_oid,
    }
}

// ─── (a) order path: empty chain ⇒ Rejected, never panic ─────────────────────

/// A serialized push carrying NO principal is rejected with a typed
/// `MissingAttribution` outcome — it must not panic, and it must append nothing.
#[test]
fn push_with_empty_principal_chain_is_rejected_not_panic() {
    let writer = SerializedWriter::new();
    let outcome = writer.push(RefUpdate {
        ref_name: "refs/heads/main".into(),
        expected: None,
        target: "fe8f5f1e013d57d0629ff3999a71986ffc2b05fb".into(),
        principal_chain: vec![],
        recorded_at: 1_717_000_000_000,
    });

    assert_eq!(
        outcome,
        PushOutcome::Rejected(PushReject::MissingAttribution),
        "an unattributed serialized push is rejected, never panics"
    );
    assert!(
        writer.is_empty(),
        "the rejected push appended NOTHING to the log"
    );
}

// ─── (b) receive path: empty chain ⇒ Err, writes no event ────────────────────

/// `receive_pack` with an empty `principal_chain` is refused with a typed error
/// BEFORE any event is recorded — symmetry with the order path / the
/// external-change recorder. (Was: silently recorded an unattributed event.)
#[test]
fn receive_pack_with_empty_principal_chain_errs_and_records_nothing() {
    use hugit_proto::write::receive::receive_pack;

    let gate = FlagGate::self_hosted_alpha();
    let mut cas = InMemoryCas::new();
    let mut log = EventLog::new();
    let before = log.len();

    // A REAL, valid pack: the ONLY thing that may refuse this push is the
    // empty-chain attribution gate. (With an empty/short pack the push would be
    // refused as malformed regardless, which would not exercise the oracle — the
    // bug was a *silent unattributed record* on an otherwise-valid push.)
    let rp = real_pack("attrib-fail");
    let req = ReceiveRequest {
        pack: rp.pack,
        update: RecvRefUpdate {
            ref_name: "refs/heads/main".into(),
            expected: None,
            new_oid: rp.head_oid,
        },
        principal_chain: vec![],
        recorded_at: 1_717_000_000_000,
    };

    let result = receive_pack(&gate, &req, &mut cas, &mut log, RecvLimits::default());
    assert!(
        result.is_err(),
        "an unattributed receive-pack is refused (typed error)"
    );
    assert_eq!(
        log.len(),
        before,
        "the refused receive-pack appended NOTHING to the log"
    );
}

// ─── (c) one consolidated json_str escapes identically on both paths ─────────

/// A ref name carrying a control char (U+0001) is escaped identically by the one
/// consolidated `json_str` on both the external payload path and the store path.
#[test]
fn control_char_is_escaped_identically_on_both_paths() {
    let ctrl_ref = "refs/heads/\u{01}evil";

    // External path: a recorded raw-push update carries the escaped ref in its
    // payload.
    let mut log = EventLog::new();
    let rec = hugit_proto::record_external_change(
        &mut log,
        &hugit_proto::RawPush::Update {
            ref_name: ctrl_ref.into(),
            target: "deadbeef".into(),
        },
        vec!["user:gustavo".into()],
        1_717_000_000_000,
    )
    .expect("attributed push records")
    .0;

    // Store path: the canonical raw-push payload for the same ref/target.
    let store_payload = hugit_proto::write::store::raw_push_payload(ctrl_ref, "deadbeef");

    assert_eq!(
        rec.payload, store_payload,
        "both paths escape the control char identically via one json_str"
    );
    // And the control char IS escaped (full escaping, not the old lossy copy).
    assert!(
        store_payload.contains("\\u0001"),
        "the control char is escaped as \\u0001, got {store_payload:?}"
    );
    assert!(
        !store_payload.contains('\u{01}'),
        "the raw control char does not survive into the payload"
    );
}
