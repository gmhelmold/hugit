//! Metadata-only receipts for CLI queue attempts, not restartable authority.
//!
//! A start is synced before evaluation. The current pointer is replaced atomically;
//! an interrupted prior attempt is invalidated before a new one starts. No old
//! result is replayed and no user ref is repaired. Missing completion means an
//! uncertain outcome (the corpus write may already have happened).
//!
//! The caller holds the existing porcelain log lock throughout. These receipts
//! do not strengthen that legacy lock against unrelated/noncooperative writers.
//! File sync and Unix directory sync are requested; power-loss qualification and
//! Windows directory durability are NOT inferred from successful process tests.

use super::EvaluationContext;
use crate::porcelain::PorcelainError;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

const SCHEMA: &str = "hugit.queue-attempt/1";
const MAX_RECORD_BYTES: u64 = 8192;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Outcome {
    Applied,
    Held,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Previous {
    operation_id: String,
    /// Terminal observation or explicit invalidation, never a replay instruction.
    disposition: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Started {
    schema: String,
    operation_id: String,
    base_id: String,
    config_id: String,
    scope: String,
    previous: Option<Previous>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Current {
    schema: String,
    operation_id: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Finished {
    schema: String,
    operation_id: String,
    outcome: Outcome,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Invalidated {
    schema: String,
    operation_id: String,
    reason: String,
    prior_effects: String,
}

/// Private CLI persistence helper. Deserialized receipts cannot construct a live
/// EvaluationContext/capability or supply any content to the evaluator.
pub(super) struct AttemptJournal {
    dir: PathBuf,
    operation_id: String,
}

fn error(message: impl Into<String>) -> PorcelainError {
    PorcelainError::new(
        "queue_journal_error",
        message,
        "preserve the attempt receipts and current corpus; resolve the I/O or corruption before retrying; do not replay old results",
    )
}

fn valid_id(id: &str) -> bool {
    id.strip_prefix("queue-").is_some_and(|suffix| {
        suffix.len() == 32
            && suffix
                .bytes()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
    })
}

fn valid_identity(value: &str) -> bool {
    !value.trim().is_empty() && value.len() <= 512 && !value.chars().any(char::is_control)
}

fn record_path(dir: &Path, id: &str, kind: &str) -> Result<PathBuf, PorcelainError> {
    if !valid_id(id) {
        return Err(error("invalid operation ID in queue attempt receipt"));
    }
    Ok(dir.join(format!("{id}.{kind}.json")))
}

fn read_record<T: DeserializeOwned>(path: &Path) -> Result<Option<T>, PorcelainError> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(error(format!("cannot inspect queue receipt: {e}"))),
    };
    if !metadata.is_file() || metadata.len() > MAX_RECORD_BYTES {
        return Err(error("queue receipt is not a bounded regular file"));
    }
    // The directory is cooperative same-user storage, not a hostile filesystem.
    // Recheck the opened file and cap the actual read, not only the metadata.
    let file = File::open(path).map_err(|e| error(format!("cannot read queue receipt: {e}")))?;
    let metadata = file.metadata().map_err(|e| error(e.to_string()))?;
    if !metadata.is_file() || metadata.len() > MAX_RECORD_BYTES {
        return Err(error("opened queue receipt is not a bounded regular file"));
    }
    let mut bytes = Vec::new();
    file.take(MAX_RECORD_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| error(format!("cannot read queue receipt: {e}")))?;
    if bytes.len() as u64 > MAX_RECORD_BYTES {
        return Err(error("queue receipt grew beyond its read limit"));
    }
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|_| error("invalid queue attempt receipt; no new evaluation admitted"))
}

fn sync_dir(path: &Path) -> Result<(), PorcelainError> {
    #[cfg(unix)]
    File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|e| error(format!("cannot sync queue receipt directory: {e}")))?;
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

fn encode<T: Serialize>(value: &T) -> Result<Vec<u8>, PorcelainError> {
    let bytes = serde_json::to_vec(value).map_err(|_| error("cannot encode queue receipt"))?;
    if bytes.len() as u64 > MAX_RECORD_BYTES {
        return Err(error("queue receipt exceeds its write limit"));
    }
    Ok(bytes)
}

/// Immutable receipt. A partial failed write is retained and rejected on read;
/// do not erase evidence of an uncertain write or overwrite another attempt.
fn create_record<T: Serialize>(path: &Path, value: &T) -> Result<(), PorcelainError> {
    let bytes = encode(value)?;
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .map_err(|e| error(format!("cannot create queue receipt: {e}")))?;
    file.write_all(&bytes)
        .and_then(|()| file.sync_all())
        .map_err(|e| error(format!("cannot persist queue receipt: {e}")))?;
    sync_dir(path.parent().unwrap_or_else(|| Path::new(".")))
}

impl AttemptJournal {
    /// O(1) metadata lookups: no scan of old attempts and no replay of their data.
    pub(super) fn begin(
        log_path: &Path,
        context: &EvaluationContext,
    ) -> Result<Self, PorcelainError> {
        if !valid_id(&context.operation_id)
            || context.scope != "simulation_only"
            || !valid_identity(&context.base_id)
            || !valid_identity(&context.config_id)
        {
            return Err(error("invalid live queue attempt context"));
        }
        let mut name = log_path.as_os_str().to_os_string();
        name.push(".queue-attempts");
        let dir = PathBuf::from(name);
        let mut created = false;
        match fs::symlink_metadata(&dir) {
            Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {}
            Ok(_) => return Err(error("queue attempt journal is not a real directory")),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let mut builder = fs::DirBuilder::new();
                #[cfg(unix)]
                {
                    use std::os::unix::fs::DirBuilderExt;
                    builder.mode(0o700);
                }
                builder
                    .create(&dir)
                    .map_err(|e| error(format!("cannot create queue journal: {e}")))?;
                let parent = dir
                    .parent()
                    .filter(|p| !p.as_os_str().is_empty())
                    .unwrap_or_else(|| Path::new("."));
                sync_dir(parent)?;
                created = true;
            }
            Err(e) => return Err(error(format!("cannot inspect queue journal: {e}"))),
        }
        let current_path = dir.join("current.json");
        let previous = if let Some(current) = read_record::<Current>(&current_path)? {
            if current.schema != SCHEMA || current.operation_id == context.operation_id {
                return Err(error("invalid or reused queue attempt identity"));
            }
            let started_path = record_path(&dir, &current.operation_id, "started")?;
            let started: Started = read_record(&started_path)?
                .ok_or_else(|| error("current queue attempt is missing its start receipt"))?;
            if started.schema != SCHEMA
                || started.operation_id != current.operation_id
                || started.scope != "simulation_only"
                || !valid_identity(&started.base_id)
                || !valid_identity(&started.config_id)
            {
                return Err(error(
                    "current queue attempt does not match its start receipt",
                ));
            }
            let finished =
                read_record::<Finished>(&record_path(&dir, &current.operation_id, "finished")?)?;
            let invalidated_path = record_path(&dir, &current.operation_id, "invalidated")?;
            let invalidated = read_record::<Invalidated>(&invalidated_path)?;
            let disposition = if let Some(finished) = finished {
                if finished.schema != SCHEMA
                    || finished.operation_id != current.operation_id
                    || invalidated.is_some()
                {
                    return Err(error("conflicting queue terminal receipts"));
                }
                match finished.outcome {
                    Outcome::Applied => "applied",
                    Outcome::Held => "held",
                    Outcome::Failed => "failed",
                }
            } else {
                let expected = Invalidated {
                    schema: SCHEMA.into(),
                    operation_id: current.operation_id.clone(),
                    reason: "interrupted_attempt_never_replayed".into(),
                    prior_effects: "unknown_preserve_current_corpus".into(),
                };
                if let Some(observed) = invalidated {
                    if encode(&observed)? != encode(&expected)? {
                        return Err(error("conflicting queue invalidation receipt"));
                    }
                } else {
                    create_record(&invalidated_path, &expected)?;
                }
                "invalidated"
            };
            Some(Previous {
                operation_id: current.operation_id,
                disposition: disposition.into(),
            })
        } else {
            if !created {
                return Err(error(
                    "existing queue journal is missing its current pointer",
                ));
            }
            None
        };
        let started = Started {
            schema: SCHEMA.into(),
            operation_id: context.operation_id.clone(),
            base_id: context.base_id.clone(),
            config_id: context.config_id.clone(),
            scope: context.scope.clone(),
            previous,
        };
        create_record(
            &record_path(&dir, &context.operation_id, "started")?,
            &started,
        )?;
        // A failed pointer write leaves an unreferenced start receipt, not a
        // completed or admitted evaluation. Preserve it; do not guess its effect.
        let pointer = encode(&Current {
            schema: SCHEMA.into(),
            operation_id: context.operation_id.clone(),
        })?;
        crate::pr::filelock::atomic_write_unprepared(&current_path, &pointer)
            .map_err(|e| error(format!("cannot publish current queue attempt: {e}")))?;
        Ok(Self {
            dir,
            operation_id: context.operation_id.clone(),
        })
    }

    /// Call Applied only AFTER the corpus write succeeds. If this receipt fails,
    /// report an uncertain completion and never undo an already written corpus.
    pub(super) fn finish(&self, outcome: Outcome) -> Result<(), PorcelainError> {
        create_record(
            &record_path(&self.dir, &self.operation_id, "finished")?,
            &Finished {
                schema: SCHEMA.into(),
                operation_id: self.operation_id.clone(),
                outcome,
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "hugit-attempt-test-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::SeqCst)
            ));
            fs::create_dir(&path).unwrap();
            fs::write(path.join("log.json"), "[]").unwrap();
            Self(path)
        }
        fn log(&self) -> PathBuf {
            self.0.join("log.json")
        }
        fn dir(&self) -> PathBuf {
            self.0.join("log.json.queue-attempts")
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }
    fn context(n: u8) -> EvaluationContext {
        EvaluationContext {
            operation_id: format!("queue-{n:032x}"),
            base_id: "event-log-sha256:input".into(),
            config_id: "simulation-config-sha256:config".into(),
            scope: "simulation_only".into(),
        }
    }

    #[test]
    fn completed_attempt_records_terminal_and_new_identity_without_invalidating_success() {
        let f = Fixture::new();
        AttemptJournal::begin(&f.log(), &context(1))
            .unwrap()
            .finish(Outcome::Applied)
            .unwrap();
        let next = AttemptJournal::begin(&f.log(), &context(2)).unwrap();
        let started: Started =
            read_record(&record_path(&f.dir(), &context(2).operation_id, "started").unwrap())
                .unwrap()
                .unwrap();
        assert_eq!(started.previous.unwrap().disposition, "applied");
        assert!(
            !record_path(&f.dir(), &context(1).operation_id, "invalidated")
                .unwrap()
                .exists()
        );
        next.finish(Outcome::Held).unwrap();
        assert_eq!(fs::read(f.log()).unwrap(), b"[]");
    }

    #[test]
    fn interrupted_attempt_is_persistently_invalidated_and_never_replayed() {
        let f = Fixture::new();
        drop(AttemptJournal::begin(&f.log(), &context(1)).unwrap());
        fs::write(f.log(), "human change stays").unwrap();
        let mut next_context = context(2);
        next_context.base_id = "new-current-corpus".into();
        let next = AttemptJournal::begin(&f.log(), &next_context).unwrap();
        let invalidated: Invalidated =
            read_record(&record_path(&f.dir(), &context(1).operation_id, "invalidated").unwrap())
                .unwrap()
                .unwrap();
        assert_eq!(invalidated.prior_effects, "unknown_preserve_current_corpus");
        let start: Started =
            read_record(&record_path(&f.dir(), &next_context.operation_id, "started").unwrap())
                .unwrap()
                .unwrap();
        assert_eq!(start.base_id, "new-current-corpus");
        assert_eq!(start.previous.unwrap().disposition, "invalidated");
        next.finish(Outcome::Applied).unwrap();
        assert_eq!(fs::read(f.log()).unwrap(), b"human change stays");
    }

    #[test]
    fn corrupt_or_missing_start_refuses_before_another_attempt_is_created() {
        for missing in [false, true] {
            let f = Fixture::new();
            drop(AttemptJournal::begin(&f.log(), &context(1)).unwrap());
            let path = record_path(&f.dir(), &context(1).operation_id, "started").unwrap();
            if missing {
                fs::remove_file(path).unwrap();
            } else {
                fs::write(path, "{").unwrap();
            }
            assert!(AttemptJournal::begin(&f.log(), &context(2)).is_err());
            assert!(
                !record_path(&f.dir(), &context(2).operation_id, "started")
                    .unwrap()
                    .exists()
            );
            assert_eq!(fs::read(f.log()).unwrap(), b"[]");
        }
    }

    #[test]
    fn invalid_pointer_and_oversized_or_special_receipts_are_refused() {
        for mode in 0..4 {
            let f = Fixture::new();
            drop(AttemptJournal::begin(&f.log(), &context(1)).unwrap());
            let path = f.dir().join("current.json");
            match mode {
                0 => fs::write(&path, br#"{"schema":"hugit.queue-attempt/1","operation_id":"../../other"}"#).unwrap(),
                1 => fs::write(&path, vec![b' '; MAX_RECORD_BYTES as usize + 1]).unwrap(),
                2 => { fs::remove_file(&path).unwrap(); fs::create_dir(&path).unwrap(); }
                _ => fs::write(&path, br#"{"schema":"unknown","operation_id":"queue-00000000000000000000000000000001"}"#).unwrap(),
            }
            assert!(AttemptJournal::begin(&f.log(), &context(2)).is_err());
            assert_eq!(fs::read(f.log()).unwrap(), b"[]");
        }
    }

    #[test]
    fn duplicate_identity_and_conflicting_terminal_are_not_overwritten() {
        let f = Fixture::new();
        let first = AttemptJournal::begin(&f.log(), &context(1)).unwrap();
        assert!(AttemptJournal::begin(&f.log(), &context(1)).is_err());
        first.finish(Outcome::Held).unwrap();
        assert!(first.finish(Outcome::Applied).is_err());
        let terminal: Finished =
            read_record(&record_path(&f.dir(), &context(1).operation_id, "finished").unwrap())
                .unwrap()
                .unwrap();
        assert_eq!(terminal.outcome, Outcome::Held);
    }

    #[test]
    fn corrupt_terminal_is_not_reinterpreted_as_interruption() {
        let f = Fixture::new();
        let first = AttemptJournal::begin(&f.log(), &context(1)).unwrap();
        first.finish(Outcome::Applied).unwrap();
        fs::write(
            record_path(&f.dir(), &context(1).operation_id, "finished").unwrap(),
            "{",
        )
        .unwrap();
        assert!(AttemptJournal::begin(&f.log(), &context(2)).is_err());
        assert!(
            !record_path(&f.dir(), &context(1).operation_id, "invalidated")
                .unwrap()
                .exists()
        );
    }

    #[test]
    fn missing_pointer_does_not_silently_reset_attempt_history() {
        let f = Fixture::new();
        drop(AttemptJournal::begin(&f.log(), &context(1)).unwrap());
        fs::remove_file(f.dir().join("current.json")).unwrap();
        assert!(AttemptJournal::begin(&f.log(), &context(2)).is_err());
        assert!(
            !record_path(&f.dir(), &context(2).operation_id, "started")
                .unwrap()
                .exists()
        );
        assert_eq!(fs::read(f.log()).unwrap(), b"[]");
    }

    #[cfg(unix)]
    #[test]
    fn receipt_symlink_is_refused_and_private_permissions_are_set() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let f = Fixture::new();
        drop(AttemptJournal::begin(&f.log(), &context(1)).unwrap());
        assert_eq!(
            fs::metadata(f.dir()).unwrap().permissions().mode() & 0o777,
            0o700
        );
        let started = record_path(&f.dir(), &context(1).operation_id, "started").unwrap();
        assert_eq!(
            fs::metadata(&started).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let pointer = f.dir().join("current.json");
        fs::remove_file(&pointer).unwrap();
        symlink(&started, &pointer).unwrap();
        assert!(AttemptJournal::begin(&f.log(), &context(2)).is_err());
    }
}
