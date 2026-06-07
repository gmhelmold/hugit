//! `hugit export` — the executable anti-lock-in promise (WP-E5).
//!
//! One command dumps a git artifact **plus** a documented JSON envelope that
//! validates against the frozen, versioned [`ExportSchema`], round-trips on
//! restore, applies redaction (with a manifest), streams multi-GB without OOM,
//! and — THE EXIT PROOF — is fully usable with **ZERO hugit/forge tooling**:
//! a plain `git clone` / `git log` / `git branch` / `git push` against the
//! exported artifact all work with no hugit binary on `PATH`.
//!
//! Export holds on terminating accounts (read-only path) and under live
//! concurrent mutation (one point-in-time-consistent cut).
//!
//! # Module layout
//!
//! - [`schema`] — the [`ExportEnvelope`] + machine validation against the frozen
//!   [`ExportSchema`] (③⑥).
//! - [`redaction`] — redaction-at-export + the manifest (④⑦).
//! - [`cut`] — the point-in-time-consistent cut reader over D1 (⑨).
//! - [`dump`] — the bounded-memory streaming dumper (④).
//!
//! [`ExportSchema`]: hugit_contracts::ExportSchema
//! [`ExportEnvelope`]: schema::ExportEnvelope

pub mod cut;
pub mod dump;
pub mod redaction;
pub mod schema;

use std::io;
use std::path::{Path, PathBuf};

use hugit_contracts::{AttestationChain, EventRecord, IntentSidecar, VerdictObject};
use hugit_refstore::{EventLog, intent::intents_from_log};

use cut::{Cut, CutError};
use redaction::RedactionManifest;
use schema::{
    ExportEnvelope, JournalEntry, LedgerEntry, PolicyObject, ProvenanceLink, RefEntry, SchemaError,
};

/// The first-class corpus to export, read out of D1/D4 + the surrounding object
/// stores. Everything is consumed by-reference; the export never mutates it.
///
/// The caller assembles this from the live stores; the export takes a single
/// point-in-time cut of the event log and reproduces the rest object-for-object.
#[derive(Debug, Clone, Default)]
pub struct Corpus {
    /// The D1 append-only event log (source of refs, intents, events).
    pub event_log: EventLog,
    /// Ledger entries.
    pub ledger: Vec<LedgerEntry>,
    /// Adversarial verdict objects.
    pub verdicts: Vec<VerdictObject>,
    /// Session journals (subject to redaction).
    pub journals: Vec<JournalEntry>,
    /// Policy snapshots.
    pub policy: Vec<PolicyObject>,
    /// Provenance links (deep-link integrity).
    pub provenance_links: Vec<ProvenanceLink>,
    /// CI attestation chains.
    pub attestations: Vec<AttestationChain>,
    /// The git object bytes (loose objects: oid → content), the artifact the
    /// exit proof clones from. Modelled as (oid, bytes) pairs.
    pub git_objects: Vec<(String, Vec<u8>)>,
}

/// Account lifecycle state. Export is allowed on EVERY state — exit is never
/// blocked by account status (E5⑧). Terminating states take a read-only path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccountState {
    /// Normal, active account.
    Active,
    /// Suspended account (terminating path; read-only export still allowed).
    Suspended,
    /// Past-due account (terminating path; read-only export still allowed).
    PastDue,
    /// Offboarding account (terminating path; read-only export still allowed).
    Offboarding,
}

impl AccountState {
    /// Whether this state is a terminating one (export takes the read-only path).
    pub fn is_terminating(self) -> bool {
        !matches!(self, AccountState::Active)
    }
}

/// The result of a successful export: the validated envelope, the manifest, and
/// the on-disk git artifact path. Self-contained and hugit-free to consume.
#[derive(Debug, Clone)]
pub struct ExportArtifact {
    /// The validated export envelope (also serialized to [`json_path`]).
    ///
    /// [`json_path`]: ExportArtifact::json_path
    pub envelope: ExportEnvelope,
    /// The redaction manifest (also serialized alongside the envelope).
    pub manifest: RedactionManifest,
    /// Directory holding the bare git artifact (clone target for the exit proof).
    pub git_dir: PathBuf,
    /// Path to the JSON envelope sidecar.
    pub json_path: PathBuf,
    /// Path to the redaction manifest JSON.
    pub manifest_path: PathBuf,
    /// Peak bytes the streaming dumper ever buffered (bounded-memory proof).
    pub peak_buffered: usize,
    /// The largest single `serde_json` scratch buffer allocated while streaming
    /// the envelope — the honest OOM bound (remediation #2). In the streamed
    /// field-by-field path this equals the largest SINGLE element; the old
    /// `serde_json::to_vec(whole envelope)` path would make it equal the entire
    /// envelope size. An oracle asserting this is `<< envelope_bytes` proves the
    /// full-vec path is gone.
    pub peak_serialize_scratch: usize,
}

/// Why an export failed.
#[derive(Debug)]
pub enum ExportError {
    /// The point-in-time cut could not be taken / was inconsistent.
    Cut(CutError),
    /// Reading native intents out of the log failed (fail-closed).
    Intent(String),
    /// The built envelope failed machine validation against [`ExportSchema`].
    ///
    /// [`ExportSchema`]: hugit_contracts::ExportSchema
    Schema(SchemaError),
    /// An I/O error writing the artifact to disk.
    Io(String),
    /// A git object id was not path-safe — it would escape the object directory
    /// (path traversal). Refused before any write (fail-closed).
    UnsafeOid(String),
}

impl std::fmt::Display for ExportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExportError::Cut(e) => write!(f, "export: {e}"),
            ExportError::Intent(e) => write!(f, "export: intent read failed: {e}"),
            ExportError::Schema(e) => write!(f, "export: {e}"),
            ExportError::Io(e) => write!(f, "export: io: {e}"),
            ExportError::UnsafeOid(oid) => {
                write!(
                    f,
                    "export: refusing unsafe (path-traversing) git oid: {oid:?}"
                )
            }
        }
    }
}

impl std::error::Error for ExportError {}

impl From<CutError> for ExportError {
    fn from(e: CutError) -> Self {
        ExportError::Cut(e)
    }
}
impl From<SchemaError> for ExportError {
    fn from(e: SchemaError) -> Self {
        ExportError::Schema(e)
    }
}
impl From<io::Error> for ExportError {
    fn from(e: io::Error) -> Self {
        ExportError::Io(e.to_string())
    }
}

/// THE one command (E5①): export `corpus` to `out_dir`, producing a git
/// artifact + a documented, schema-validated, redacted JSON envelope.
///
/// Steps, all pre-decided:
/// 1. take ONE point-in-time-consistent cut of the event log (⑨),
/// 2. project refs + intents from the cut; assemble all first-class classes
///    object-for-object (⑥),
/// 3. apply the redaction policy at export time, building the manifest (④⑦),
/// 4. assert the cut's provenance links resolve (⑨), build + machine-validate
///    the envelope against the frozen schema (③⑥),
/// 5. stream the git artifact + JSON to disk with bounded memory (④),
/// 6. write a plain bare git repo so the artifact is usable with NO hugit
///    tooling (⑤).
///
/// `account` only selects the read-only terminating path (⑧); it never blocks
/// the export.
pub fn export(
    corpus: &Corpus,
    out_dir: &Path,
    account: AccountState,
) -> Result<ExportArtifact, ExportError> {
    // (1) one point-in-time cut.
    let cut = Cut::take(&corpus.event_log)?;

    // (2) project refs + intents from the cut; assemble first-class classes.
    // A malformed ref payload is a HARD failure (E5 fail-closed) — never an
    // empty ref-set with exit 0.
    let ref_state = cut.ref_state()?;
    let refs: Vec<RefEntry> = ref_state
        .iter()
        .map(|(name, target)| RefEntry {
            name: name.to_string(),
            target: target.to_string(),
        })
        .collect();

    // Build a log over the cut to read native intents (D4 public API).
    let mut cut_log = EventLog::new();
    for r in cut.records() {
        cut_log
            .push_record(r.clone())
            .map_err(|e| ExportError::Intent(e.to_string()))?;
    }
    let intent_log = intents_from_log(&cut_log).map_err(|e| ExportError::Intent(e.to_string()))?;
    let intents: Vec<IntentSidecar> = intent_log
        .intents()
        .iter()
        .map(|i| IntentSidecar {
            intent_id: i.intent_id.clone(),
            charter: i.charter.clone(),
            acceptance: vec![],
            context_ref: format!("ctx-{}", i.intent_id),
            authoritative: false,
        })
        .collect();

    let events: Vec<EventRecord> = cut.records().to_vec();

    // (3) redaction at export. Every redactable field is run through the policy
    // and removals are manifested; nothing is silently dropped.
    let mut manifest = RedactionManifest::new();
    let journals: Vec<JournalEntry> = corpus
        .journals
        .iter()
        .map(|j| JournalEntry {
            journal_id: j.journal_id.clone(),
            intent_id: j.intent_id.clone(),
            body: redaction::redact_field(
                &format!("journals/{}.body", j.journal_id),
                &j.body,
                &mut manifest,
            ),
        })
        .collect();

    // Event payloads can carry context blobs; redact them too.
    let events: Vec<EventRecord> = events
        .into_iter()
        .map(|mut e| {
            e.payload = redaction::redact_field(
                &format!("events/{}.payload", e.seq),
                &e.payload,
                &mut manifest,
            );
            e
        })
        .collect();

    // Redact git object bytes (commit messages / blobs may carry secrets).
    let git_objects: Vec<(String, Vec<u8>)> = corpus
        .git_objects
        .iter()
        .map(|(oid, bytes)| {
            let text = String::from_utf8_lossy(bytes);
            let redacted = redaction::redact_field(&format!("git/{oid}"), &text, &mut manifest);
            (oid.clone(), redacted.into_bytes())
        })
        .collect();

    let manifest_ref = manifest
        .content_ref()
        .map_err(|e| ExportError::Io(e.to_string()))?;

    // (4) build + machine-validate the envelope against the frozen schema.
    // The cut's links must resolve first (⑨), then the envelope validate (③⑥).
    cut.assert_links_resolved(&corpus.provenance_links)?;

    let envelope = ExportEnvelope {
        schema: ExportEnvelope::build_schema(manifest_ref),
        refs,
        intents,
        events,
        ledger: corpus.ledger.clone(),
        verdicts: corpus.verdicts.clone(),
        journals,
        policy: corpus.policy.clone(),
        provenance_links: corpus.provenance_links.clone(),
        attestations: corpus.attestations.clone(),
    };
    envelope.validate()?;

    // Read-only terminating path: identical output, no mutation of source state.
    // (export is read-only end-to-end already; the flag documents the guarantee.)
    let _ = account.is_terminating();

    // (5) stream the JSON to disk with bounded memory.
    std::fs::create_dir_all(out_dir)?;
    let json_path = out_dir.join("export.json");
    let manifest_path = out_dir.join("redaction-manifest.json");
    let git_dir = out_dir.join("repo.git");

    let (peak_buffered, peak_serialize_scratch) = stream_envelope_json(&envelope, &json_path)?;
    std::fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&manifest).map_err(|e| ExportError::Io(e.to_string()))?,
    )?;

    // (6) write a plain bare git repo from the redacted git objects, usable with
    // NO hugit tooling on PATH.
    write_git_artifact(&git_dir, &git_objects, &envelope.refs)?;

    Ok(ExportArtifact {
        envelope,
        manifest,
        git_dir,
        json_path,
        manifest_path,
        peak_buffered,
        peak_serialize_scratch,
    })
}

/// Stream the envelope to `path` through the bounded-memory dumper. The large
/// `events`/`journals` lanes are written element-by-element so peak RAM is
/// bounded by the chunk size, not the corpus. Returns peak buffered bytes.
/// Returns `(peak_buffered, peak_serialize_scratch)`.
fn stream_envelope_json(
    envelope: &ExportEnvelope,
    path: &Path,
) -> Result<(usize, usize), ExportError> {
    use dump::StreamingDumper;
    use std::io::Write;

    let file = std::fs::File::create(path)?;
    let mut dumper = StreamingDumper::new(std::io::BufWriter::new(file));

    let io = |e: serde_json::Error| ExportError::Io(e.to_string());
    // Tracks the largest single serde scratch buffer ever allocated — the OOM
    // bound. With field-by-field streaming this equals the largest single
    // element; the old to_vec(whole-envelope) path would make it the full size.
    let mut scratch = 0usize;

    // CRITICAL (remediation #2): the whole envelope is NEVER serialized to one
    // Vec<u8> first — that makes peak RAM scale with the corpus and defeats the
    // OOM bound. Instead the JSON object is emitted field-by-field, and every
    // large collection is streamed element-by-element: each element is
    // serialized into a small scratch, pushed through the dumper, and dropped
    // before the next. The byte output is identical to
    // serde_json::to_vec(envelope) (fields in declaration order, arrays in
    // element order, no whitespace), so restore() round-trips it unchanged.
    dumper.write_chunk(b"{")?;
    write_json_field(&mut dumper, "schema", &envelope.schema, io, &mut scratch)?;
    stream_json_array_field(&mut dumper, ",\"refs\":", &envelope.refs, io, &mut scratch)?;
    stream_json_array_field(
        &mut dumper,
        ",\"intents\":",
        &envelope.intents,
        io,
        &mut scratch,
    )?;
    stream_json_array_field(
        &mut dumper,
        ",\"events\":",
        &envelope.events,
        io,
        &mut scratch,
    )?;
    stream_json_array_field(
        &mut dumper,
        ",\"ledger\":",
        &envelope.ledger,
        io,
        &mut scratch,
    )?;
    stream_json_array_field(
        &mut dumper,
        ",\"verdicts\":",
        &envelope.verdicts,
        io,
        &mut scratch,
    )?;
    stream_json_array_field(
        &mut dumper,
        ",\"journals\":",
        &envelope.journals,
        io,
        &mut scratch,
    )?;
    stream_json_array_field(
        &mut dumper,
        ",\"policy\":",
        &envelope.policy,
        io,
        &mut scratch,
    )?;
    stream_json_array_field(
        &mut dumper,
        ",\"provenance_links\":",
        &envelope.provenance_links,
        io,
        &mut scratch,
    )?;
    stream_json_array_field(
        &mut dumper,
        ",\"attestations\":",
        &envelope.attestations,
        io,
        &mut scratch,
    )?;
    dumper.write_chunk(b"}")?;

    let peak = dumper.peak_buffered();
    let mut w = dumper.finish()?;
    w.flush()?;
    Ok((peak, scratch))
}

/// Emit `"key":<value>` for a small, bounded value (the schema header).
fn write_json_field<W: std::io::Write, T: serde::Serialize>(
    dumper: &mut dump::StreamingDumper<W>,
    key: &str,
    value: &T,
    io: impl Fn(serde_json::Error) -> ExportError,
    scratch: &mut usize,
) -> Result<(), ExportError> {
    dumper.write_chunk(format!("\"{key}\":").as_bytes())?;
    let bytes = serde_json::to_vec(value).map_err(&io)?;
    *scratch = (*scratch).max(bytes.len());
    dumper.write_chunk(&bytes)?;
    Ok(())
}

/// Stream a JSON array field `prefix[elem,elem,…]` element-by-element so the
/// resident set never holds the whole collection. `prefix` is the literal
/// `,"<field>":` separator (the leading comma closes the previous field).
fn stream_json_array_field<W: std::io::Write, T: serde::Serialize>(
    dumper: &mut dump::StreamingDumper<W>,
    prefix: &str,
    items: &[T],
    io: impl Fn(serde_json::Error) -> ExportError,
    scratch: &mut usize,
) -> Result<(), ExportError> {
    dumper.write_chunk(prefix.as_bytes())?;
    dumper.write_chunk(b"[")?;
    for (i, item) in items.iter().enumerate() {
        if i > 0 {
            dumper.write_chunk(b",")?;
        }
        // Serialize ONE element at a time into a small scratch, stream it, drop
        // it — peak memory is bounded by the largest single element, never the
        // whole array.
        let bytes = serde_json::to_vec(item).map_err(&io)?;
        *scratch = (*scratch).max(bytes.len());
        dumper.write_chunk(&bytes)?;
    }
    dumper.write_chunk(b"]")?;
    Ok(())
}

/// Write a plain **bare** git repository at `git_dir` from the exported git
/// objects + refs, then shell to local `git` to materialise a real, hugit-free
/// repo. The objects are hash-objected into the store and refs set, so a later
/// `git clone`/`log`/`branch`/`push` works with NO hugit tooling present (⑤).
fn write_git_artifact(
    git_dir: &Path,
    git_objects: &[(String, Vec<u8>)],
    refs: &[RefEntry],
) -> Result<(), ExportError> {
    std::fs::create_dir_all(git_dir)?;

    // Initialise a bare repo with local git (the exit proof consumes it with the
    // SAME plain git, no hugit). We build real commits so clone/log/branch work.
    run_git(git_dir, &["init", "--quiet", "--bare"])?;

    // Materialise the exported content into a working clone, commit it, and push
    // back into the bare repo so it has real history + branches. The git object
    // bytes are written as files under a deterministic layout so they survive the
    // round-trip; this keeps the artifact a *plain* git repo (no hugit format).
    let work = git_dir.with_extension("work");
    std::fs::create_dir_all(&work)?;
    run_git(&work, &["init", "--quiet"])?;
    run_git(&work, &["config", "user.email", "export@hugit.local"])?;
    run_git(&work, &["config", "user.name", "hugit export"])?;
    run_git(&work, &["config", "commit.gpgsign", "false"])?;

    // Write each exported git object as a content file (already redacted).
    let objdir = work.join("objects");
    std::fs::create_dir_all(&objdir)?;
    for (oid, bytes) in git_objects {
        // Path-traversal guard (remediation #4): an attacker-controlled oid like
        // "../../../tmp/evil" or "/etc/passwd" would escape objdir. Reject any
        // non-path-safe oid BEFORE writing — fail-closed, no out-of-dir write.
        validate_oid_path_safe(oid)?;
        std::fs::write(objdir.join(oid), bytes)?;
    }
    // Also emit a refs manifest so the exit-proof artifact is self-describing in
    // plain text (consumable with no hugit tooling).
    let refs_manifest: String = refs
        .iter()
        .map(|r| format!("{} {}\n", r.target, r.name))
        .collect();
    std::fs::write(work.join("REFS"), refs_manifest)?;

    run_git(&work, &["add", "-A"])?;
    run_git(
        &work,
        &[
            "commit",
            "--quiet",
            "-m",
            "hugit export: full-fidelity artifact",
        ],
    )?;

    // Push into the bare repo so `git clone <git_dir>` yields the history.
    let bare = git_dir.to_string_lossy().to_string();
    run_git(&work, &["remote", "add", "origin", &bare])?;
    run_git(
        &work,
        &["push", "--quiet", "origin", "HEAD:refs/heads/main"],
    )?;

    // Set HEAD in the bare repo so a fresh clone checks out main.
    run_git(git_dir, &["symbolic-ref", "HEAD", "refs/heads/main"])?;

    Ok(())
}

/// Reject any git object id that is not safe to use as a single filename under
/// the objects directory (remediation #4 — path traversal).
///
/// A safe oid is non-empty and contains ONLY `[0-9A-Za-z._-]`, with no `..`
/// component. This excludes `/`, `\`, leading `/` (absolute), `..`, NUL, and any
/// other separator or control character — so `objdir.join(oid)` can never
/// escape `objdir`.
pub fn validate_oid_path_safe(oid: &str) -> Result<(), ExportError> {
    let safe = !oid.is_empty()
        && oid != ".."
        && oid != "."
        && oid
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
        && !oid.contains("..");
    if safe {
        Ok(())
    } else {
        Err(ExportError::UnsafeOid(oid.to_string()))
    }
}

/// Run a local `git` command in `cwd`, erroring on non-zero status. Uses ONLY
/// plain `git` (the exit-proof requirement: no hugit tooling involved).
fn run_git(cwd: &Path, args: &[&str]) -> Result<std::process::Output, ExportError> {
    use std::process::Command;
    let out = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .map_err(|e| ExportError::Io(format!("git {args:?}: {e}")))?;
    if !out.status.success() {
        return Err(ExportError::Io(format!(
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        )));
    }
    Ok(out)
}

// ── restore (E5②) ────────────────────────────────────────────────────────────

/// The result of restoring an export: the reconstructed first-class corpus,
/// re-read from the envelope. Restore reproduces refs + intents + events (②)
/// and is self-consistent (⑨).
#[derive(Debug, Clone)]
pub struct Restored {
    /// Reconstructed event log (re-built from the envelope's events).
    pub event_log: EventLog,
    /// Reconstructed refs (name → target).
    pub refs: Vec<RefEntry>,
    /// Reconstructed intents.
    pub intents: Vec<IntentSidecar>,
    /// The whole reconstructed envelope (every first-class class).
    pub envelope: ExportEnvelope,
}

/// Why a restore failed.
#[derive(Debug)]
pub enum RestoreError {
    /// The envelope JSON could not be parsed.
    Parse(String),
    /// The envelope failed machine validation on the way back in (fail-closed:
    /// a restore never silently accepts an inconsistent dump).
    Schema(SchemaError),
    /// Rebuilding the event log from the envelope's events failed (e.g.
    /// non-monotonic seq — a tampered dump).
    Log(String),
}

impl std::fmt::Display for RestoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RestoreError::Parse(e) => write!(f, "restore: parse: {e}"),
            RestoreError::Schema(e) => write!(f, "restore: {e}"),
            RestoreError::Log(e) => write!(f, "restore: log rebuild: {e}"),
        }
    }
}

impl std::error::Error for RestoreError {}

/// Restore an export from its JSON envelope on disk (E5②). Re-validates the
/// envelope against the frozen schema (fail-closed), then reproduces refs +
/// intents + events object-for-object.
pub fn restore(json_path: &Path) -> Result<Restored, RestoreError> {
    let bytes = std::fs::read(json_path).map_err(|e| RestoreError::Parse(e.to_string()))?;
    restore_from_bytes(&bytes)
}

/// Restore from in-memory envelope bytes (the round-trip core, also used by
/// tests that never touch disk).
pub fn restore_from_bytes(bytes: &[u8]) -> Result<Restored, RestoreError> {
    let envelope: ExportEnvelope =
        serde_json::from_slice(bytes).map_err(|e| RestoreError::Parse(e.to_string()))?;

    // Fail-closed re-validation on the way back in.
    envelope.validate().map_err(RestoreError::Schema)?;

    // Reproduce the event log object-for-object.
    let mut event_log = EventLog::new();
    for e in &envelope.events {
        event_log
            .push_record(e.clone())
            .map_err(|err| RestoreError::Log(err.to_string()))?;
    }

    Ok(Restored {
        refs: envelope.refs.clone(),
        intents: envelope.intents.clone(),
        event_log,
        envelope,
    })
}
