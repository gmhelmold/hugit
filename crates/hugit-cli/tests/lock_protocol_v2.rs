//! HUG-008 native-process checks. Fixtures are isolated; v1 rollout stays disabled.
use hugit_refstore::coordination::{
    CoordinationError, EXPERIMENTAL_DIRECTORY, ExperimentalCoordinator, LockClass, LockRequest,
    MAX_HELD_LOCKS, WriterState,
};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

static NEXT: AtomicU64 = AtomicU64::new(0);
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "hugit-native-lock-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn coordinator(&self) -> ExperimentalCoordinator {
        ExperimentalCoordinator::create(&self.0).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}
fn req(class: LockClass, key: &str) -> LockRequest {
    LockRequest::new(class, key).unwrap()
}
fn global() -> LockRequest {
    req(LockClass::InstallationBudget, "global")
}
fn inventory(path: &Path) -> Vec<String> {
    let mut names: Vec<_> = fs::read_dir(path)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[test]
fn compatible_contenders_are_exclusive_and_release_never_unlinks_or_truncates() {
    let fixture = Fixture::new();
    let c = fixture.coordinator();
    let path = c.lock_path(&global());
    fs::write(&path, b"retained diagnostic bytes").unwrap();
    #[cfg(unix)]
    let identity = {
        use std::os::unix::fs::MetadataExt;
        let m = fs::metadata(&path).unwrap();
        (m.dev(), m.ino())
    };
    let mut first = c.lock_set();
    first.try_acquire(global()).unwrap();
    let mut other = ExperimentalCoordinator::open(&fixture.0)
        .unwrap()
        .lock_set();
    assert!(matches!(
        other.try_acquire(global()),
        Err(CoordinationError::Busy)
    ));
    assert!(other.is_empty());
    assert!(path.exists());
    assert_eq!(first.release_last(), Some(global()));
    other.try_acquire(global()).unwrap();
    drop(other);
    assert_eq!(fs::read(&path).unwrap(), b"retained diagnostic bytes");
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let m = fs::metadata(&path).unwrap();
        assert_eq!(identity, (m.dev(), m.ino()));
    }
}

#[test]
fn reverse_order_schedule_fails_before_open_and_never_deadlocks() {
    let f = Fixture::new();
    let c = f.coordinator();
    let operation = req(LockClass::Operation, "work");
    let mut a = c.lock_set();
    let mut b = c.lock_set();
    a.try_acquire(global()).unwrap();
    b.try_acquire(operation.clone()).unwrap();
    assert!(matches!(
        a.try_acquire(operation.clone()),
        Err(CoordinationError::Busy)
    ));
    let before = inventory(&f.0.join(EXPERIMENTAL_DIRECTORY));
    assert!(matches!(
        b.try_acquire(global()),
        Err(CoordinationError::OutOfOrder)
    ));
    assert_eq!(inventory(&f.0.join(EXPERIMENTAL_DIRECTORY)), before);
    b.release_last();
    a.try_acquire(operation).unwrap();
    assert_eq!(a.len(), 2);
}

#[test]
fn equal_and_reverse_lexical_keys_are_refused_without_new_files() {
    let f = Fixture::new();
    let c = f.coordinator();
    let mut locks = c.lock_set();
    let last = req(LockClass::RepoAdmission, "repo-b");
    locks.try_acquire(last.clone()).unwrap();
    assert!(matches!(
        locks.try_acquire(last),
        Err(CoordinationError::OutOfOrder)
    ));
    let earlier = req(LockClass::RepoAdmission, "repo-a");
    assert!(matches!(
        locks.try_acquire(earlier.clone()),
        Err(CoordinationError::OutOfOrder)
    ));
    assert!(!c.lock_path(&earlier).exists());
    locks
        .try_acquire(req(LockClass::Operation, "next"))
        .unwrap();
}

#[test]
fn held_handle_budget_is_checked_before_creating_another_file() {
    let f = Fixture::new();
    let c = f.coordinator();
    let mut locks = c.lock_set();
    for n in 0..MAX_HELD_LOCKS {
        locks
            .try_acquire(req(LockClass::Operation, &format!("op-{n:03}")))
            .unwrap();
    }
    let overflow = req(LockClass::Operation, "op-999");
    assert!(matches!(
        locks.try_acquire(overflow.clone()),
        Err(CoordinationError::TooManyLocks)
    ));
    assert!(!c.lock_path(&overflow).exists());
    assert_eq!(locks.len(), MAX_HELD_LOCKS);
    drop(locks);
    c.lock_set()
        .try_acquire(req(LockClass::Operation, "op-000"))
        .unwrap();
}

#[test]
fn namespace_initialization_never_adopts_existing_or_legacy_data() {
    let f = Fixture::new();
    fs::write(f.0.join("log.json"), b"legacy sentinel").unwrap();
    assert!(ExperimentalCoordinator::open(&f.0).is_err());
    let c = f.coordinator();
    assert!(ExperimentalCoordinator::create(&f.0).is_err());
    c.lock_set().try_acquire(global()).unwrap();
    assert_eq!(fs::read(f.0.join("log.json")).unwrap(), b"legacy sentinel");
    fs::write(
        f.0.join(EXPERIMENTAL_DIRECTORY).join("protocol"),
        b"other protocol",
    )
    .unwrap();
    assert!(ExperimentalCoordinator::open(&f.0).is_err());
    assert_eq!(fs::read(f.0.join("log.json")).unwrap(), b"legacy sentinel");
}

#[test]
fn invalid_keys_and_special_lockfiles_are_refused() {
    for key in ["", "..", "a/b", "a\\b", "CAPS", "\n", "a.b", "é"] {
        assert!(matches!(
            LockRequest::new(LockClass::Operation, key),
            Err(CoordinationError::InvalidKey)
        ));
    }
    assert!(LockRequest::new(LockClass::Operation, &"a".repeat(65)).is_err());
    let f = Fixture::new();
    let c = f.coordinator();
    let path = c.lock_path(&global());
    fs::create_dir(&path).unwrap();
    assert!(matches!(
        c.lock_set().try_acquire(global()),
        Err(CoordinationError::InvalidLockFile)
    ));
    assert!(path.is_dir());
}

#[cfg(unix)]
#[test]
fn symlink_and_hardlink_lockfiles_are_refused_without_changing_the_target() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new();
    let c = f.coordinator();
    let path = c.lock_path(&global());
    let target = f.0.join("other");
    fs::write(&target, b"do not modify").unwrap();
    symlink(&target, &path).unwrap();
    assert!(matches!(
        c.lock_set().try_acquire(global()),
        Err(CoordinationError::InvalidLockFile)
    ));
    fs::remove_file(&path).unwrap();
    fs::hard_link(&target, &path).unwrap();
    assert!(matches!(
        c.lock_set().try_acquire(global()),
        Err(CoordinationError::InvalidLockFile)
    ));
    assert_eq!(fs::read(&target).unwrap(), b"do not modify");
}

struct ChildGuard(Child);
impl ChildGuard {
    fn start(f: &Fixture, role: &str) -> Self {
        let child = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "subprocess_fixture", "--ignored", "--nocapture"])
            .env("HUGIT_NATIVE_LOCK_FIXTURE", &f.0)
            .env("HUGIT_NATIVE_LOCK_ROLE", role)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let mut guard = Self(child);
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            if fs::read(f.0.join("ready")).ok().as_deref() == Some(b"ready") {
                break;
            }
            assert!(
                guard.0.try_wait().unwrap().is_none(),
                "child exited before ready"
            );
            assert!(Instant::now() < deadline, "child readiness deadline");
            std::thread::sleep(Duration::from_millis(5));
        }
        guard
    }
    fn kill_and_reap(&mut self) {
        assert!(self.0.try_wait().unwrap().is_none());
        self.0.kill().unwrap();
        let status = self.0.wait().unwrap();
        assert!(!status.success());
        #[cfg(unix)]
        {
            use std::os::unix::process::ExitStatusExt;
            assert_eq!(
                status.signal(),
                Some(9),
                "must be a real SIGKILL, not a helper panic"
            );
        }
    }
    fn finish(&mut self) {
        self.0.stdin.as_mut().unwrap().write_all(b"x").unwrap();
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            if let Some(status) = self.0.try_wait().unwrap() {
                assert!(status.success());
                break;
            }
            assert!(Instant::now() < deadline, "child exit deadline");
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}
impl Drop for ChildGuard {
    fn drop(&mut self) {
        if matches!(self.0.try_wait(), Ok(None)) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}

#[test]
#[ignore = "helper executed only as an owned subprocess by the parent tests"]
fn subprocess_fixture() {
    let parent =
        PathBuf::from(std::env::var_os("HUGIT_NATIVE_LOCK_FIXTURE").expect("parent fixture"));
    let coordinator = ExperimentalCoordinator::open(&parent).unwrap();
    let mut held = coordinator.lock_set();
    if std::env::var("HUGIT_NATIVE_LOCK_ROLE").unwrap() == "holder" {
        held.try_acquire(global()).unwrap();
    }
    fs::write(parent.join("ready"), b"ready").unwrap();
    if let Err(error) = std::io::stdin().read_exact(&mut [0u8; 1]) {
        // Closing the parent's stdin is fixture shutdown. Death tests separately
        // require a SIGKILL exit status, so an EOF exit cannot fake that proof.
        assert_eq!(error.kind(), std::io::ErrorKind::UnexpectedEof);
    }
    drop(held);
}

#[test]
fn process_death_releases_native_lease_without_deleting_its_file() {
    let f = Fixture::new();
    let c = f.coordinator();
    let mut child = ChildGuard::start(&f, "holder");
    assert!(matches!(
        c.lock_set().try_acquire(global()),
        Err(CoordinationError::Busy)
    ));
    child.kill_and_reap();
    c.lock_set().try_acquire(global()).unwrap();
    assert!(c.lock_path(&global()).is_file());
}

#[test]
fn exec_child_does_not_inherit_parent_lock_ownership() {
    let f = Fixture::new();
    let c = f.coordinator();
    let mut held = c.lock_set();
    held.try_acquire(global()).unwrap();
    let mut child = ChildGuard::start(&f, "observer");
    drop(held);
    // The exec child stays alive while a different handle must acquire immediately.
    assert!(child.0.try_wait().unwrap().is_none());
    c.lock_set().try_acquire(global()).unwrap();
    child.finish();
}

#[cfg(unix)]
#[test]
fn paused_owner_130_seconds_is_not_stolen_and_death_releases_the_lease() {
    let f = Fixture::new();
    let c = f.coordinator();
    let mut child = ChildGuard::start(&f, "holder");
    let pid = child.0.id().to_string();
    assert!(
        Command::new("/bin/kill")
            .args(["-STOP", &pid])
            .status()
            .unwrap()
            .success()
    );
    let state = Command::new("ps")
        .args(["-o", "stat=", "-p", &pid])
        .output()
        .unwrap();
    assert!(state.status.success() && String::from_utf8_lossy(&state.stdout).contains('T'));
    assert!(matches!(
        c.lock_set().try_acquire(global()),
        Err(CoordinationError::Busy)
    ));
    let start = Instant::now();
    std::thread::sleep(Duration::from_secs(130));
    assert!(start.elapsed() >= Duration::from_secs(130));
    assert!(matches!(
        c.lock_set().try_acquire(global()),
        Err(CoordinationError::Busy)
    ));
    assert!(child.0.try_wait().unwrap().is_none());
    child.kill_and_reap();
    c.lock_set().try_acquire(global()).unwrap();
    println!(
        "owner_paused_ms={} stable_file_preserved=true",
        start.elapsed().as_millis()
    );
}

// Lifecycle revision: stopping new compatible writers never steals active leases.
fn state_path(f: &Fixture) -> PathBuf {
    f.0.join(EXPERIMENTAL_DIRECTORY).join("writer-state")
}

#[test]
fn disable_requires_all_holders_to_release_and_preserves_files() {
    let f = Fixture::new();
    let c = f.coordinator();
    let mut a = c.lock_set();
    let mut b = c.lock_set();
    a.try_acquire(global()).unwrap();
    b.try_acquire(req(LockClass::Operation, "other")).unwrap();
    let root = f.0.join(EXPERIMENTAL_DIRECTORY);
    let before = inventory(&root);
    assert_eq!(
        ExperimentalCoordinator::inspect(&f.0).unwrap(),
        WriterState::Active
    );
    assert!(matches!(c.try_disable(), Err(CoordinationError::Busy)));
    assert_eq!(fs::read(state_path(&f)).unwrap(), b"active\n");
    a.release_last();
    assert!(matches!(c.try_disable(), Err(CoordinationError::Busy)));
    drop(b);
    c.try_disable().unwrap();
    assert_eq!(
        ExperimentalCoordinator::inspect(&f.0).unwrap(),
        WriterState::Disabled
    );
    assert_eq!(inventory(&root), before);
    assert!(c.lock_path(&global()).is_file());
    assert!(a.is_empty());
}

#[test]
fn disabled_namespace_refuses_old_handles_and_reopen_without_new_files() {
    let f = Fixture::new();
    let c = f.coordinator();
    let copy = c.clone();
    let mut waiting = c.lock_set();
    c.try_disable().unwrap();
    let before = inventory(&f.0.join(EXPERIMENTAL_DIRECTORY));
    let opened = ExperimentalCoordinator::open(&f.0).unwrap();
    for result in [
        waiting.try_acquire(global()),
        copy.lock_set().try_acquire(global()),
        opened.lock_set().try_acquire(global()),
    ] {
        assert!(matches!(result, Err(CoordinationError::WritersDisabled)));
    }
    // Disable is idempotent, not reactivation or a data migration.
    opened.try_disable().unwrap();
    assert_eq!(inventory(&f.0.join(EXPERIMENTAL_DIRECTORY)), before);
    assert!(waiting.is_empty());
    assert!(!c.lock_path(&global()).exists());
}

#[test]
fn failed_first_acquisition_does_not_leak_admission() {
    let f = Fixture::new();
    let c = f.coordinator();
    let mut owner = c.lock_set();
    owner.try_acquire(global()).unwrap();
    let mut contender = c.lock_set();
    assert!(matches!(
        contender.try_acquire(global()),
        Err(CoordinationError::Busy)
    ));
    assert!(contender.is_empty());
    owner.release_last();
    // Contender is still alive: leaked shared admission would make this Busy.
    c.try_disable().unwrap();
    assert!(matches!(
        contender.try_acquire(global()),
        Err(CoordinationError::WritersDisabled)
    ));
}

#[test]
fn malformed_or_missing_state_never_resets_to_active() {
    let f = Fixture::new();
    let c = f.coordinator();
    for bytes in [
        b"".as_slice(),
        b"dctive\n",
        b"disabled\ntrailing",
        b"unknown\n",
    ] {
        fs::write(state_path(&f), bytes).unwrap();
        assert!(matches!(
            ExperimentalCoordinator::inspect(&f.0),
            Err(CoordinationError::InvalidWriterState)
        ));
        assert!(matches!(
            c.try_disable(),
            Err(CoordinationError::InvalidWriterState)
        ));
        assert!(matches!(
            c.lock_set().try_acquire(global()),
            Err(CoordinationError::InvalidWriterState)
        ));
        assert_eq!(fs::read(state_path(&f)).unwrap(), bytes);
        assert!(!c.lock_path(&global()).exists());
    }
    fs::remove_file(state_path(&f)).unwrap();
    assert!(ExperimentalCoordinator::inspect(&f.0).is_err());
    assert!(c.lock_set().try_acquire(global()).is_err());
    assert!(c.try_disable().is_err());
    assert!(!state_path(&f).exists());
    assert!(!c.lock_path(&global()).exists());
}

#[test]
fn previous_experimental_protocol_is_not_upgraded_or_adopted() {
    let f = Fixture::new();
    let root = f.0.join(EXPERIMENTAL_DIRECTORY);
    fs::create_dir(&root).unwrap();
    let old = b"hugit.coordination/2-experimental\n";
    fs::write(root.join("protocol"), old).unwrap();
    fs::write(f.0.join("log.json"), b"legacy corpus").unwrap();
    assert!(matches!(
        ExperimentalCoordinator::open(&f.0),
        Err(CoordinationError::InvalidNamespace)
    ));
    assert!(ExperimentalCoordinator::create(&f.0).is_err());
    assert!(!state_path(&f).exists());
    assert_eq!(fs::read(root.join("protocol")).unwrap(), old);
    assert_eq!(fs::read(f.0.join("log.json")).unwrap(), b"legacy corpus");
}

#[test]
fn protocol_changes_invalidate_previously_opened_coordinators() {
    let f = Fixture::new();
    let c = f.coordinator();
    fs::write(
        f.0.join(EXPERIMENTAL_DIRECTORY).join("protocol"),
        b"other revision",
    )
    .unwrap();
    assert!(matches!(
        c.lock_set().try_acquire(global()),
        Err(CoordinationError::InvalidNamespace)
    ));
    assert!(matches!(
        c.try_disable(),
        Err(CoordinationError::InvalidNamespace)
    ));
    assert_eq!(fs::read(state_path(&f)).unwrap(), b"active\n");
    assert!(!c.lock_path(&global()).exists());
}

#[test]
fn process_death_releases_admission_and_disable_stays_persistent() {
    let f = Fixture::new();
    let c = f.coordinator();
    let mut child = ChildGuard::start(&f, "holder");
    assert!(matches!(c.try_disable(), Err(CoordinationError::Busy)));
    child.kill_and_reap();
    c.try_disable().unwrap();
    assert!(c.lock_path(&global()).is_file());
    drop(c);
    let c = ExperimentalCoordinator::open(&f.0).unwrap();
    assert!(matches!(
        c.lock_set().try_acquire(global()),
        Err(CoordinationError::WritersDisabled)
    ));
    assert_eq!(
        ExperimentalCoordinator::inspect(&f.0).unwrap(),
        WriterState::Disabled
    );
}

#[test]
fn exec_child_cannot_keep_admission_alive_after_parent_release() {
    let f = Fixture::new();
    let c = f.coordinator();
    let mut held = c.lock_set();
    held.try_acquire(global()).unwrap();
    let mut child = ChildGuard::start(&f, "observer");
    drop(held);
    assert!(child.0.try_wait().unwrap().is_none());
    c.try_disable().unwrap();
    assert_eq!(
        ExperimentalCoordinator::inspect(&f.0).unwrap(),
        WriterState::Disabled
    );
    child.finish();
}

#[cfg(unix)]
#[test]
fn disabled_namespace_remains_inspectable_without_write_access() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let f = Fixture::new();
    let c = f.coordinator();
    let path = state_path(&f);
    let before = fs::metadata(&path).unwrap();
    c.try_disable().unwrap();
    let after = fs::metadata(&path).unwrap();
    assert_eq!((before.dev(), before.ino()), (after.dev(), after.ino()));
    fs::set_permissions(&path, fs::Permissions::from_mode(0o400)).unwrap();
    let root = f.0.join(EXPERIMENTAL_DIRECTORY);
    fs::set_permissions(&root, fs::Permissions::from_mode(0o500)).unwrap();
    let result = ExperimentalCoordinator::inspect(&f.0);
    // Restore fixture permissions even if the assertion below fails.
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
    assert_eq!(result.unwrap(), WriterState::Disabled);
    assert_eq!(fs::read(&path).unwrap(), b"disabled\n");
}

#[test]
fn partial_release_and_failed_extension_retain_admission() {
    let f = Fixture::new();
    let c = f.coordinator();
    let mut held = c.lock_set();
    held.try_acquire(global()).unwrap();
    held.try_acquire(req(LockClass::RepoAdmission, "repo"))
        .unwrap();
    let bad = req(LockClass::Operation, "bad");
    fs::create_dir(c.lock_path(&bad)).unwrap();
    assert!(matches!(
        held.try_acquire(bad),
        Err(CoordinationError::InvalidLockFile)
    ));
    assert_eq!(held.len(), 2);
    held.release_last();
    assert!(matches!(c.try_disable(), Err(CoordinationError::Busy)));
    drop(held);
    c.try_disable().unwrap();
    assert_eq!(
        ExperimentalCoordinator::inspect(&f.0).unwrap(),
        WriterState::Disabled
    );
}

#[test]
fn concurrent_admission_and_disable_have_a_single_safe_winner() {
    use std::sync::{Arc, Barrier, mpsc};
    for _ in 0..32 {
        let f = Fixture::new();
        let c = f.coordinator();
        let other = c.clone();
        let barrier = Arc::new(Barrier::new(2));
        let ready = barrier.clone();
        let (tx, rx) = mpsc::channel();
        let (release, wait) = mpsc::channel::<()>();
        let worker = std::thread::spawn(move || {
            let mut held = other.lock_set();
            ready.wait();
            let result = held.try_acquire(global());
            tx.send(result).unwrap();
            // Keep a successful lease until the controller has observed disable.
            let _ = wait.recv_timeout(Duration::from_secs(10));
            drop(held);
        });
        barrier.wait();
        let disabled = c.try_disable();
        let admitted = rx.recv_timeout(Duration::from_secs(10)).unwrap();
        let safe = matches!(
            (&disabled, &admitted),
            (
                Ok(()),
                Err(CoordinationError::WritersDisabled | CoordinationError::Busy)
            ) | (Err(CoordinationError::Busy), Ok(()))
        );
        drop(release);
        worker.join().unwrap();
        assert!(safe, "disable={disabled:?}, admission={admitted:?}");
        c.try_disable().unwrap();
        assert!(matches!(
            c.lock_set().try_acquire(global()),
            Err(CoordinationError::WritersDisabled)
        ));
    }
}

#[test]
fn every_partial_disable_value_is_refused_without_repair() {
    let f = Fixture::new();
    let c = f.coordinator();
    for cut in 1..b"disabled\n".len() {
        let mut partial = b"active\n".to_vec();
        if cut > partial.len() {
            partial.resize(cut, 0);
        }
        partial[..cut].copy_from_slice(&b"disabled\n"[..cut]);
        fs::write(state_path(&f), &partial).unwrap();
        assert!(
            matches!(
                c.lock_set().try_acquire(global()),
                Err(CoordinationError::InvalidWriterState)
            ),
            "cut={cut}"
        );
        assert!(matches!(
            c.try_disable(),
            Err(CoordinationError::InvalidWriterState)
        ));
        assert_eq!(fs::read(state_path(&f)).unwrap(), partial);
    }
    // These are explicit fixture injections, not a claim of power-loss tests.
    assert!(!c.lock_path(&global()).exists());
}

#[cfg(unix)]
#[test]
fn linked_state_is_refused_and_external_bytes_are_preserved() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new();
    let c = f.coordinator();
    let target = f.0.join("unrelated");
    fs::write(&target, b"active\n").unwrap();
    fs::remove_file(state_path(&f)).unwrap();
    symlink(&target, state_path(&f)).unwrap();
    assert!(c.lock_set().try_acquire(global()).is_err());
    assert!(c.try_disable().is_err());
    fs::remove_file(state_path(&f)).unwrap();
    fs::hard_link(&target, state_path(&f)).unwrap();
    assert!(c.lock_set().try_acquire(global()).is_err());
    assert!(c.try_disable().is_err());
    assert_eq!(fs::read(target).unwrap(), b"active\n");
}
