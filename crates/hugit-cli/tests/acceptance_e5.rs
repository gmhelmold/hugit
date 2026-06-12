//! Acceptance suite for WP-E5 — export + exit proofs (the anti-lock-in guarantee).
//!
//! Contract: docs/plan/wp-contracts/WP-E5.md
//! One `#[test] item_<n>_<slug>` per owned acceptance item (①–⑨).
//!
//! ① one-command dump: git artifact + documented JSON envelope
//! ② restore round-trip reproduces refs + intents + events object-for-object
//! ③ export validates against the versioned ExportSchema (machine check)
//! ④ redaction policy applied + multi-GB-shaped export streams without OOM
//! ⑤ THE EXIT PROOF: exported git artifact usable with ZERO hugit tooling on PATH
//! ⑥ completeness: all first-class object classes reproduced; out-of-scope enumerated
//! ⑦ redaction red-team: seeded secrets appear NOWHERE; redacted artifact still
//!    passes the exit proof; removals manifested
//! ⑧ exit under exit conditions: export succeeds on suspended/past-due/offboarding
//! ⑨ "any moment" consistency: one point-in-time-consistent cut; restore self-consistent

use std::path::{Path, PathBuf};
use std::process::Command;

use hugit_cli::export::cut::Cut;
use hugit_cli::export::redaction::{self, RedactionManifest};
use hugit_cli::export::schema::{
    ExportEnvelope, FIRST_CLASS_OBJECT_CLASSES, JournalEntry, OUT_OF_SCOPE_CLASSES, ProvenanceLink,
    RefEntry,
};
use hugit_cli::export::{AccountState, Corpus, export, restore, restore_from_bytes};
use hugit_refstore::EventLog;

// ── fixtures ──────────────────────────────────────────────────────────────────

/// A unique scratch dir under the OS temp, per test, cleaned at the start.
fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("hugit-e5-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Build a small, realistic corpus over the D1 event log:
/// - two landed intents (project to refs + native intents),
/// - one raw push (external change — never an intent),
/// - a journal carrying a seeded secret (to exercise redaction),
/// - one provenance link resolving inside the cut.
fn fixture_corpus() -> Corpus {
    let mut log = EventLog::new();
    // intent.landed #1
    log.append_for_test(
        "intent.landed",
        vec!["alice".into()],
        serde_json::json!({
            "intent_id": "I-1",
            "ref": "refs/heads/main",
            "target": "aaaa1111",
            "charter": "land feature A"
        })
        .to_string(),
        1000,
    );
    // intent.landed #2
    log.append_for_test(
        "intent.landed",
        vec!["bob".into()],
        serde_json::json!({
            "intent_id": "I-2",
            "ref": "refs/heads/dev",
            "target": "bbbb2222",
            "charter": "land feature B"
        })
        .to_string(),
        2000,
    );
    // raw push (external change, not an intent)
    log.append_for_test(
        "ref.update",
        vec!["ci".into()],
        serde_json::json!({ "ref": "refs/heads/ci", "target": "cccc3333" }).to_string(),
        3000,
    );

    Corpus {
        event_log: log,
        ledger: vec![hugit_cli::export::schema::LedgerEntry {
            entry_id: "L-1".into(),
            subject: "I-1".into(),
            amount_cents: 0,
        }],
        verdicts: vec![],
        journals: vec![JournalEntry {
            journal_id: "J-1".into(),
            intent_id: "I-1".into(),
            // Seeded secret material — MUST be redacted at export.
            body: "rationale: ok. token=ghp_DEADBEEFcafef00dSECRET trailing".into(),
        }],
        policy: vec![hugit_cli::export::schema::PolicyObject {
            policy_id: "P-1".into(),
            gates: "dco,changelog,secrets".into(),
        }],
        provenance_links: vec![ProvenanceLink {
            object_id: "I-1".into(),
            event_seq: 0,
        }],
        attestations: vec![],
        git_objects: vec![
            ("obj-readme".into(), b"# project\nclean content\n".to_vec()),
            (
                // git object carrying a seeded secret too.
                "obj-config".into(),
                b"api_key=AKIAIOSFODNN7EXAMPLE more".to_vec(),
            ),
        ],
    }
}

/// Run a plain `git` command in `cwd` with an EMPTY PATH containing ONLY the
/// directory of the real git binary — i.e. NO hugit tooling reachable. This is
/// the local realisation of the "zero hugit dependency" exit proof.
fn git_no_hugit(cwd: &Path, args: &[&str]) -> std::process::Output {
    // Resolve the real git binary, then expose only its directory on PATH.
    let git_path = which_git();
    let git_dir = git_path.parent().unwrap().to_path_buf();
    Command::new(&git_path)
        .args(args)
        .current_dir(cwd)
        .env_clear()
        .env("PATH", &git_dir)
        .env("HOME", cwd) // git wants a HOME for config
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .unwrap_or_else(|e| panic!("git {args:?}: {e}"))
}

/// Resolve the absolute path of the real `git` binary by scanning the inherited
/// PATH directly (no shell — the exit proof must not depend on a shell being
/// reachable on the constrained PATH).
fn which_git() -> PathBuf {
    let path = std::env::var_os("PATH").expect("PATH set");
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join("git");
        if candidate.is_file() {
            // Confirm it actually runs.
            if let Ok(out) = Command::new(&candidate).arg("--version").output()
                && out.status.success()
            {
                return candidate;
            }
        }
    }
    panic!("git must be installed for the exit proof");
}

/// Assert: no hugit binary is reachable on the exit-proof PATH (the proof would
/// be void otherwise). The constrained PATH carries ONLY git's own directory;
/// we scan it for a `hugit` binary in Rust (no shell on the restricted PATH).
fn assert_no_hugit_on_path(_probe_dir: &Path) {
    let git_path = which_git();
    let git_dir = git_path.parent().unwrap().to_path_buf();
    let hugit = git_dir.join("hugit");
    assert!(
        !hugit.exists(),
        "exit proof void: a hugit binary is reachable on the constrained PATH: {}",
        hugit.display()
    );
}

// ── ① one-command dump: git + documented JSON ─────────────────────────────────

#[test]
fn item_1_one_command_dump() {
    let out = scratch("dump");
    let corpus = fixture_corpus();

    let artifact = export(&corpus, &out, AccountState::Active).expect("one-command export");

    // The git artifact exists on disk.
    assert!(artifact.git_dir.is_dir(), "git artifact dir present");
    // The documented JSON envelope exists and parses.
    assert!(artifact.json_path.is_file(), "JSON envelope present");
    let bytes = std::fs::read(&artifact.json_path).unwrap();
    let parsed: ExportEnvelope = serde_json::from_slice(&bytes).expect("JSON parses");
    // The envelope declares its versioned schema (documented).
    assert!(!parsed.schema.version.is_empty(), "schema version declared");
    // The dump captured the refs + events + intents from the log.
    assert!(!parsed.events.is_empty(), "events dumped");
    assert!(!parsed.refs.is_empty(), "refs dumped");
    assert!(!parsed.intents.is_empty(), "intents dumped");
}

// ── ② restore round-trip ──────────────────────────────────────────────────────

#[test]
fn item_2_restore_roundtrip() {
    let out = scratch("roundtrip");
    let corpus = fixture_corpus();

    let artifact = export(&corpus, &out, AccountState::Active).unwrap();
    let restored = restore(&artifact.json_path).expect("restore round-trips");

    // refs reproduced object-for-object.
    assert_eq!(
        restored.refs, artifact.envelope.refs,
        "refs reproduced exactly"
    );
    // intents reproduced object-for-object.
    assert_eq!(
        restored.intents, artifact.envelope.intents,
        "intents reproduced exactly"
    );
    // events reproduced object-for-object: the restored event log equals the
    // exported events, and re-chains validly.
    assert_eq!(
        restored.event_log.records(),
        artifact.envelope.events.as_slice(),
        "events reproduced exactly"
    );
    // The intents projected from the exported log match the original two landed
    // intents (not the raw push).
    assert_eq!(restored.intents.len(), 2, "two intents, raw push excluded");
}

// ── ③ machine validation against the versioned ExportSchema ───────────────────

#[test]
fn item_3_schema_validates() {
    let out = scratch("schema");
    let corpus = fixture_corpus();
    let artifact = export(&corpus, &out, AccountState::Active).unwrap();

    // The exported envelope passes machine validation.
    artifact
        .envelope
        .validate()
        .expect("exported envelope is schema-valid");

    // Machine check is real, not a doc-presence heuristic: corrupt the schema
    // version and validation MUST fail.
    let mut bad = artifact.envelope.clone();
    bad.schema.version = String::new();
    assert!(bad.validate().is_err(), "empty version rejected");

    // Dropping a first-class object class from the declared set MUST fail.
    let mut bad2 = artifact.envelope.clone();
    bad2.schema.object_classes.retain(|c| c != "events");
    assert!(
        bad2.validate().is_err(),
        "silently dropping a class rejected"
    );

    // Empty redaction manifest ref MUST fail (a manifest is always mandatory).
    let mut bad3 = artifact.envelope.clone();
    bad3.schema.redaction_manifest = String::new();
    assert!(bad3.validate().is_err(), "missing manifest ref rejected");
}

// ── ④ redaction applied + streaming without OOM ───────────────────────────────

#[test]
fn item_4_redaction_no_oom() {
    let out = scratch("noom");
    let corpus = fixture_corpus();
    let artifact = export(&corpus, &out, AccountState::Active).unwrap();

    // Redaction applied: the seeded secrets are gone from the envelope.
    let json = serde_json::to_string(&artifact.envelope).unwrap();
    assert!(!json.contains("ghp_DEADBEEF"), "journal secret redacted");
    assert!(
        !json.contains("AKIAIOSFODNN7EXAMPLE"),
        "git secret redacted"
    );

    // Streaming without OOM: the dumper's peak buffered bytes are bounded by the
    // fixed chunk size, NOT by the (large) corpus.
    use hugit_cli::export::dump::{CHUNK_BYTES, StreamingDumper};
    let mut dumper = StreamingDumper::new(Vec::new());
    // Stream ~16 MB through a 64 KB-bounded dumper, one 4 KB record at a time.
    let record = vec![b'x'; 4096];
    let total: u64 = 16 * 1024 * 1024;
    let mut written = 0u64;
    while written < total {
        dumper.write_chunk(&record).unwrap();
        written += record.len() as u64;
    }
    let peak = dumper.peak_buffered();
    let sink = dumper.finish().unwrap();
    assert_eq!(sink.len() as u64, total, "all bytes streamed to sink");
    // Peak resident is bounded by ~one chunk plus one record — never the corpus.
    assert!(
        (peak as u64) < (CHUNK_BYTES as u64) + record.len() as u64 + 1,
        "peak buffered {peak} bounded (corpus was {total} bytes)"
    );
}

// ── ⑤ THE EXIT PROOF: zero hugit dependency ───────────────────────────────────

#[test]
fn item_5_exit_proof_zero_hugit() {
    let out = scratch("exit");
    let corpus = fixture_corpus();
    let artifact = export(&corpus, &out, AccountState::Active).unwrap();

    // Guard: the constrained PATH must NOT reach any hugit tooling.
    assert_no_hugit_on_path(&out);

    let clone_dst = out.join("cloned");
    // clone — with NO hugit on PATH.
    let c = git_no_hugit(
        &out,
        &[
            "clone",
            "--quiet",
            artifact.git_dir.to_str().unwrap(),
            clone_dst.to_str().unwrap(),
        ],
    );
    assert!(
        c.status.success(),
        "git clone of exported artifact failed: {}",
        String::from_utf8_lossy(&c.stderr)
    );

    // log — history is present.
    let l = git_no_hugit(&clone_dst, &["log", "--oneline"]);
    assert!(l.status.success(), "git log failed");
    assert!(
        !String::from_utf8_lossy(&l.stdout).trim().is_empty(),
        "clone has commit history"
    );

    // branch — branching works.
    let b = git_no_hugit(&clone_dst, &["branch", "exit-test"]);
    assert!(
        b.status.success(),
        "git branch failed: {}",
        String::from_utf8_lossy(&b.stderr)
    );

    // push-elsewhere — push to a fresh bare remote (still no hugit).
    let other = out.join("elsewhere.git");
    let init = git_no_hugit(
        &out,
        &["init", "--quiet", "--bare", other.to_str().unwrap()],
    );
    assert!(init.status.success(), "bare init failed");
    let p = git_no_hugit(
        &clone_dst,
        &[
            "push",
            "--quiet",
            other.to_str().unwrap(),
            "HEAD:refs/heads/imported",
        ],
    );
    assert!(
        p.status.success(),
        "git push-elsewhere failed: {}",
        String::from_utf8_lossy(&p.stderr)
    );
}

// ── ⑥ completeness: all first-class classes object-for-object ─────────────────

#[test]
fn item_6_completeness_all_classes() {
    let out = scratch("complete");
    let corpus = fixture_corpus();
    let artifact = export(&corpus, &out, AccountState::Active).unwrap();
    let env = &artifact.envelope;

    // Every first-class class is declared in the schema.
    for class in FIRST_CLASS_OBJECT_CLASSES {
        assert!(
            env.schema.object_classes.iter().any(|c| c == class),
            "class '{class}' declared in ExportSchema"
        );
    }

    // Out-of-scope classes are explicitly enumerated (today: none — asserted
    // empty, not assumed).
    let declared_oos: Vec<&String> = env
        .schema
        .object_classes
        .iter()
        .filter(|c| c.starts_with("!out-of-scope:"))
        .collect();
    assert_eq!(
        declared_oos.len(),
        OUT_OF_SCOPE_CLASSES.len(),
        "out-of-scope set matches the enumerated set exactly"
    );

    // Object-for-object reproduction across a restore: every populated class
    // survives byte-for-byte.
    let restored = restore(&artifact.json_path).unwrap();
    let r = &restored.envelope;
    assert_eq!(r.refs, env.refs, "refs object-for-object");
    assert_eq!(r.intents, env.intents, "intents object-for-object");
    assert_eq!(r.events, env.events, "events object-for-object");
    assert_eq!(r.ledger, env.ledger, "ledger object-for-object");
    assert_eq!(r.verdicts, env.verdicts, "verdicts object-for-object");
    assert_eq!(r.journals, env.journals, "journals object-for-object");
    assert_eq!(r.policy, env.policy, "policy object-for-object");
    assert_eq!(
        r.provenance_links, env.provenance_links,
        "provenance links object-for-object"
    );
    assert_eq!(
        r.attestations, env.attestations,
        "attestations object-for-object"
    );
}

// ── ⑦ redaction red-team ──────────────────────────────────────────────────────

#[test]
fn item_7_redaction_redteam() {
    let out = scratch("redteam");
    // A corpus seeded with secrets in EVERY redactable surface.
    let mut corpus = fixture_corpus();
    corpus.journals.push(JournalEntry {
        journal_id: "J-2".into(),
        intent_id: "I-2".into(),
        body: "-----BEGIN RSA PRIVATE KEY-----\nMIIE...\n leak".into(),
    });
    corpus.git_objects.push((
        "obj-secret".into(),
        b"Bearer sk-supersecrettoken-xyz body".to_vec(),
    ));

    let artifact = export(&corpus, &out, AccountState::Active).unwrap();

    // The seeded secrets appear NOWHERE in the JSON envelope.
    let json = serde_json::to_string(&artifact.envelope).unwrap();
    for needle in [
        "ghp_DEADBEEF",
        "AKIAIOSFODNN7EXAMPLE",
        "BEGIN RSA PRIVATE KEY",
        "sk-supersecrettoken",
    ] {
        assert!(!json.contains(needle), "secret '{needle}' absent from JSON");
    }
    // A scan confirms no residual secret signature anywhere in the envelope.
    assert!(
        !redaction::contains_secret(&json),
        "no residual secret signature in JSON"
    );

    // The seeded secrets appear NOWHERE in the exported git artifact bytes
    // (walk the whole artifact dir).
    fn scan_dir_for_secret(dir: &Path, needle: &str) -> bool {
        let mut found = false;
        if let Ok(rd) = std::fs::read_dir(dir) {
            for e in rd.flatten() {
                let p = e.path();
                if p.is_dir() {
                    found |= scan_dir_for_secret(&p, needle);
                } else if let Ok(bytes) = std::fs::read(&p)
                    && String::from_utf8_lossy(&bytes).contains(needle)
                {
                    found = true;
                }
            }
        }
        found
    }
    for needle in [
        "AKIAIOSFODNN7EXAMPLE",
        "sk-supersecrettoken",
        "ghp_DEADBEEF",
    ] {
        assert!(
            !scan_dir_for_secret(&artifact.git_dir, needle),
            "secret '{needle}' absent from git artifact"
        );
    }

    // Removals are MANIFESTED (recorded, not silent): the manifest is non-empty
    // and its content ref is the one the schema points at.
    assert!(
        !artifact.manifest.is_empty(),
        "redaction manifest records the removals"
    );
    assert_eq!(
        artifact.envelope.schema.redaction_manifest,
        artifact
            .manifest
            .content_ref()
            .expect("manifest serializes"),
        "schema's redaction_manifest ref binds the actual manifest"
    );
    // The manifest carries no plaintext — only digests.
    let mfst_json = serde_json::to_string(&artifact.manifest).unwrap();
    assert!(
        !redaction::contains_secret(&mfst_json),
        "manifest itself carries no plaintext secret"
    );

    // The redacted artifact STILL passes the exit proof.
    assert_no_hugit_on_path(&out);
    let clone_dst = out.join("redacted-clone");
    let c = git_no_hugit(
        &out,
        &[
            "clone",
            "--quiet",
            artifact.git_dir.to_str().unwrap(),
            clone_dst.to_str().unwrap(),
        ],
    );
    assert!(
        c.status.success(),
        "redacted artifact still clones: {}",
        String::from_utf8_lossy(&c.stderr)
    );
    let l = git_no_hugit(&clone_dst, &["log", "--oneline"]);
    assert!(l.status.success(), "redacted clone log works");
}

// ── ⑧ exit under exit conditions (terminating accounts) ───────────────────────

#[test]
fn item_8_terminating_account() {
    let corpus = fixture_corpus();
    // Export must succeed — complete and valid — on EVERY terminating state.
    for (tag, state) in [
        ("suspended", AccountState::Suspended),
        ("pastdue", AccountState::PastDue),
        ("offboarding", AccountState::Offboarding),
    ] {
        assert!(state.is_terminating(), "{tag} is a terminating state");
        let out = scratch(&format!("term-{tag}"));
        let artifact = export(&corpus, &out, state)
            .unwrap_or_else(|e| panic!("export blocked on {tag} account: {e}"));
        // Complete + valid: schema validates and refs/intents/events present.
        artifact.envelope.validate().expect("valid on terminating");
        assert!(!artifact.envelope.events.is_empty(), "complete on {tag}");
        // The result is identical to the active-account export (read-only path
        // yields the same artifact, modulo non-deterministic git timestamps).
        let active_out = scratch(&format!("term-{tag}-active"));
        let active = export(&corpus, &active_out, AccountState::Active).unwrap();
        assert_eq!(
            artifact.envelope.refs, active.envelope.refs,
            "terminating export == active export refs"
        );
        assert_eq!(
            artifact.envelope.events, active.envelope.events,
            "terminating export == active export events"
        );
    }
}

// ── ⑨ "any moment" consistency: one point-in-time-consistent cut ──────────────

#[test]
fn item_9_live_consistency_cut() {
    let corpus = fixture_corpus();

    // Model concurrent mutation: take a cut, THEN append more events to the live
    // log. The cut must be unaffected — ONE point-in-time-consistent snapshot.
    let cut = Cut::take(&corpus.event_log).unwrap();
    let cut_len = cut.len();

    let mut live = corpus.event_log.clone();
    // Concurrent landings + mirror sync + event append land AFTER the cut.
    live.append_for_test(
        "intent.landed",
        vec!["carol".into()],
        serde_json::json!({"intent_id":"I-3","ref":"refs/heads/late","target":"dddd","charter":"late"}).to_string(),
        9000,
    );
    live.append_for_test("mirror.sync", vec!["sys".into()], "{}".to_string(), 9100);

    // The cut never sees the post-cut events.
    let cut_after = Cut::take_to(&live, cut.seq_bound()).unwrap();
    assert_eq!(cut_after.len(), cut_len, "cut excludes concurrent appends");

    // No dangling provenance link: every link resolves inside the cut.
    cut.assert_links_resolved(&corpus.provenance_links)
        .expect("all provenance links resolve in the cut");

    // A link past the cut is REFUSED (would be dangling).
    let dangling = vec![ProvenanceLink {
        object_id: "I-3".into(),
        event_seq: 99,
    }];
    assert!(
        cut.assert_links_resolved(&dangling).is_err(),
        "dangling provenance link refused"
    );

    // The export over the live log is itself a single consistent cut, and
    // restore is self-consistent (re-validates fail-closed).
    let out = scratch("live");
    let artifact = export(&corpus, &out, AccountState::Active).unwrap();
    let restored = restore_from_bytes(&serde_json::to_vec(&artifact.envelope).unwrap())
        .expect("restore is self-consistent");
    // Every event in the restored log references only objects in the same cut:
    // re-validation passing IS the self-consistency assertion.
    restored
        .envelope
        .validate()
        .expect("restored cut is self-consistent");

    // An export whose envelope has an event referencing an absent object (a
    // dangling link) MUST fail restore (no event referencing an absent object).
    let mut tampered = artifact.envelope.clone();
    tampered.provenance_links.push(ProvenanceLink {
        object_id: "ghost".into(),
        event_seq: 9999,
    });
    let bytes = serde_json::to_vec(&tampered).unwrap();
    assert!(
        restore_from_bytes(&bytes).is_err(),
        "restore refuses a dump with a dangling provenance link"
    );

    // Reference the unused helper path explicitly to keep the cut snapshot live.
    let _ = RefEntry {
        name: "refs/heads/main".into(),
        target: "aaaa1111".into(),
    };
    let _ = RedactionManifest::new();
}

// ── remediation #2: real export streams field-by-field, bounded memory ─────────

/// The REAL `export()` path must stream the JSON envelope without ever holding
/// the whole corpus in RAM. We drive export with a large events corpus and
/// assert `peak_buffered` is bounded by ~one chunk (CHUNK_BYTES) — NOT by the
/// envelope size. Under the old `serde_json::to_vec(whole envelope)` path the
/// peak equals the full serialized envelope, which is far larger than a chunk,
/// turning this RED.
#[test]
fn item_4b_real_export_bounded_memory() {
    use hugit_cli::export::dump::CHUNK_BYTES;

    let out = scratch("bounded");

    // Build a corpus whose events lane alone dwarfs one chunk: many events, each
    // carrying a payload, so the full envelope is >> CHUNK_BYTES.
    let mut log = EventLog::new();
    let big_payload = serde_json::json!({
        "intent_id": "I",
        "ref": "refs/heads/main",
        "target": "deadbeef",
        "charter": "x".repeat(2048)
    })
    .to_string();
    for i in 0..400 {
        log.append_for_test(
            "intent.landed",
            vec![format!("author-{i}")],
            big_payload.clone(),
            1000 + i,
        );
    }

    let corpus = Corpus {
        event_log: log,
        ledger: vec![],
        verdicts: vec![],
        journals: vec![],
        policy: vec![],
        provenance_links: vec![],
        attestations: vec![],
        git_objects: vec![],
    };

    let artifact = export(&corpus, &out, AccountState::Active).expect("large export");

    // The serialized envelope is much larger than one chunk...
    let envelope_bytes = serde_json::to_vec(&artifact.envelope).unwrap().len();
    assert!(
        envelope_bytes > 4 * CHUNK_BYTES,
        "corpus must dwarf one chunk for the bound to be meaningful (got {envelope_bytes} bytes)"
    );

    // ...yet the largest single serde scratch buffer allocated while streaming
    // is bounded by the largest SINGLE element — far below the whole envelope.
    // The OLD `serde_json::to_vec(whole envelope)` path would make this equal
    // `envelope_bytes`; asserting it is a small fraction proves the full-vec
    // path is GONE. This is the RED→GREEN tripwire for #2.
    let scratch = artifact.peak_serialize_scratch;
    assert!(
        scratch * 4 < envelope_bytes,
        "largest serialize scratch {scratch} must be << envelope {envelope_bytes} \
         (bounded by one element) — the whole-envelope to_vec path must be gone"
    );
    // The streaming dumper buffer also stays bounded.
    let peak = artifact.peak_buffered;
    assert!(
        peak < 2 * CHUNK_BYTES,
        "export dumper peak {peak} bounded by ~CHUNK_BYTES ({CHUNK_BYTES})"
    );

    // Round-trips: the streamed bytes are valid, restorable JSON.
    let restored = restore(&artifact.json_path).expect("streamed export restores");
    assert_eq!(
        restored.envelope.events.len(),
        400,
        "every streamed event survives the round-trip"
    );
}

// ── remediation #4: unsanitized OID path-traversal is rejected ────────────────

/// A git object whose oid is a path-traversal string MUST be refused before any
/// write — no out-of-dir file is created. Under the old unsanitized
/// `fs::write(objdir.join(oid))` the bytes land at the traversed path.
#[test]
fn item_5b_oid_path_traversal_rejected() {
    use hugit_cli::export::validate_oid_path_safe;

    let out = scratch("traversal");

    // A canary path OUTSIDE the export dir that an attacker oid would target.
    let evil_target = out.join("EVIL_ESCAPED");
    let rel_escape = format!(
        "../../{}/EVIL_ESCAPED",
        out.file_name().unwrap().to_str().unwrap()
    );

    let mut corpus = fixture_corpus();
    corpus
        .git_objects
        .push((rel_escape.clone(), b"pwned".to_vec()));

    let err = export(&corpus, &out, AccountState::Active)
        .expect_err("export must REFUSE a path-traversing oid");
    let msg = err.to_string();
    assert!(
        msg.contains("unsafe") || msg.contains("oid"),
        "error must name the unsafe oid, got: {msg}"
    );

    // Fail-closed: nothing was written to the escaped location.
    assert!(
        !evil_target.exists(),
        "no out-of-dir write may occur for a traversing oid"
    );

    // The unit guard rejects the classic vectors and accepts plain hex/ids.
    for bad in ["../../../tmp/evil", "/etc/passwd", "..", "a/b", "x\0y", ""] {
        assert!(
            validate_oid_path_safe(bad).is_err(),
            "oid {bad:?} must be rejected"
        );
    }
    for good in [
        "obj-readme",
        "deadbeefdeadbeefdeadbeefdeadbeefdeadbeef",
        "a.b_c-1",
    ] {
        assert!(
            validate_oid_path_safe(good).is_ok(),
            "oid {good:?} must be accepted"
        );
    }
}

// ── remediation #5: malformed ref payload → hard export failure ───────────────

/// A `ref.update` event with a malformed (missing-target) payload makes ref
/// replay fail. The export MUST surface that as a hard error, never swallow it
/// into an empty ref-set + exit 0. Under the old `replay().unwrap_or_default()`
/// this exported silently with zero refs.
#[test]
fn item_5c_malformed_ref_payload_fails_export() {
    let out = scratch("malformed");

    let mut log = EventLog::new();
    // A real landed intent first (so a naive empty-refs export would still look
    // "successful").
    log.append_for_test(
        "intent.landed",
        vec!["alice".into()],
        serde_json::json!({"intent_id":"I-1","ref":"refs/heads/main","target":"aaaa","charter":"c"})
            .to_string(),
        1000,
    );
    // A ref.update whose payload is valid JSON but MISSING the `target` field —
    // a malformed ref payload. The hash chain stays valid; replay must reject it.
    log.append_for_test(
        "ref.update",
        vec!["ci".into()],
        serde_json::json!({ "ref": "refs/heads/broken" }).to_string(),
        2000,
    );

    let corpus = Corpus {
        event_log: log,
        ledger: vec![],
        verdicts: vec![],
        journals: vec![],
        policy: vec![],
        provenance_links: vec![],
        attestations: vec![],
        git_objects: vec![],
    };

    let err = export(&corpus, &out, AccountState::Active)
        .expect_err("malformed ref payload must FAIL the export, not silent-empty");
    let msg = err.to_string();
    assert!(
        msg.contains("ref replay") || msg.contains("malformed") || msg.contains("payload"),
        "export error must name the ref-replay failure, got: {msg}"
    );

    // Direct cut-level proof: ref_state() returns Err, never Ok(empty).
    let cut = Cut::take(&corpus.event_log).expect("cut (chain is valid)");
    assert!(
        cut.ref_state().is_err(),
        "ref_state must propagate the malformed payload, not unwrap_or_default to empty"
    );
}
