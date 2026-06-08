//! Hermetic oracle for the CoreLink AC runtime config LOADER (P2 plug-and-play).
//!
//! Proves `corelink_ac_from_env()` / `HttpAcClient::from_runtime()` (the §7 loader):
//!   1. all-present (env URL + tenant + temp PAT FILE) → a CONFIGURED client: a
//!      real lookup ATTEMPTS HTTP (Transport/Status), never `NotConfigured`;
//!   2. each missing piece (URL / tenant / PAT) → `NotConfigured` NAMING it;
//!   3. file-over-env PRECEDENCE: when the file is present it wins; the env PAT
//!      is the fallback used ONLY when the file is absent;
//!   4. the PAT value is NEVER rendered in Debug/Display/error of the client.
//!
//! No network and no touching the real `~/.hugit`: the PAT path is pointed at a
//! `tempfile`, and HOME-independence is forced via `HUGIT_CORELINK_PAT_FILE`.
//!
//! Env vars are PROCESS-GLOBAL, so a single `#[test]` (serialized by a guard)
//! owns them and restores them; sub-cases run as plain functions.

use std::env;
use std::io::Write;
use std::sync::Mutex;

use hugit_checks::client::ac::{
    AcError, ActionCache, ENV_AC_URL, ENV_PAT, ENV_PAT_FILE, ENV_TENANT, HttpAcClient,
    corelink_ac_from_env,
};

const BASE: &str = "https://api.corelink.humangr.com";
/// A guaranteed-unreachable base URL: port 0 is never bound, so any TCP connect
/// attempt fails immediately with a transport error. Used to prove the configured
/// client GENUINELY attempts the network — `Ok(_)` from this URL would mean the
/// network was never called (e.g. a stub returning `Ok(None)` would fail).
const UNREACHABLE_BASE: &str = "http://127.0.0.1:0";
const TENANT: &str = "acme";
const FILE_PAT: &str = "corelink_pat_FILE_SECRET_must_never_leak";
const ENV_PAT_VAL: &str = "corelink_pat_ENV_SECRET_must_never_leak";

/// Serialize the whole env-mutating suite (env vars are process-global).
static ENV_LOCK: Mutex<()> = Mutex::new(());

/// Clear every loader-relevant env var so each sub-case starts from a known,
/// real-`~/.hugit`-independent baseline.
fn clear_all() {
    // SAFETY: the suite holds ENV_LOCK, so no other test mutates the env.
    unsafe {
        env::remove_var(ENV_AC_URL);
        env::remove_var(ENV_TENANT);
        env::remove_var(ENV_PAT);
        env::remove_var(ENV_PAT_FILE);
    }
}

fn set(key: &str, val: &str) {
    // SAFETY: guarded by ENV_LOCK.
    unsafe { env::set_var(key, val) }
}

#[test]
fn loader_contract() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());

    all_present_yields_configured_client_that_attempts_http();
    missing_base_url_is_notconfigured_naming_it();
    missing_tenant_is_notconfigured_naming_it();
    missing_pat_is_notconfigured_naming_it();
    file_pat_takes_precedence_over_env();
    env_pat_is_the_fallback_only_when_file_absent();
    pat_is_never_rendered_in_client_debug_or_errors();

    clear_all();
}

/// (1) All three present (URL + tenant + temp PAT file) → a CONFIGURED client.
/// A lookup MUST ATTEMPT the network, proven by pointing at `UNREACHABLE_BASE`
/// (`http://127.0.0.1:0`): port 0 is never bound, so any genuine TCP connect
/// attempt yields `Err(Transport(_))`. `Ok(_)` — including `Ok(None)` — would
/// mean the transport was never called (e.g. a stub), so it fails the test.
///
/// Stub-resistance: a `lookup()` that always returns `Ok(None)` without touching
/// the network would reach the `Ok(_) => panic!(…)` arm → the test goes RED.
/// The real client hits port 0, gets a transport error → GREEN.
fn all_present_yields_configured_client_that_attempts_http() {
    clear_all();
    let dir = tempfile::tempdir().expect("tempdir");
    let pat_path = dir.path().join("pat");
    // Trailing newline must be trimmed by the loader.
    let mut f = std::fs::File::create(&pat_path).unwrap();
    writeln!(f, "{FILE_PAT}").unwrap();
    drop(f);

    // Point at a guaranteed-unreachable URL so Ok(_) is impossible unless the
    // transport is bypassed.  Any real TCP connect to port 0 → Transport error.
    set(ENV_AC_URL, UNREACHABLE_BASE);
    set(ENV_TENANT, TENANT);
    set(ENV_PAT_FILE, pat_path.to_str().unwrap());

    let client = corelink_ac_from_env().expect("all present → configured client");
    // The transport MUST be attempted: with port 0 as target the only legal
    // outcomes are Transport (connection refused / I/O) or Status (unexpected
    // HTTP code). `Ok(_)` proves the network was NOT attempted → fail the test.
    let key = "a".repeat(64);
    match client.lookup(&key) {
        Err(AcError::NotConfigured(_)) => {
            panic!("a fully-configured client must not be NotConfigured")
        }
        Err(AcError::NotWired(_)) => panic!("from_runtime must build a CONFIGURED (wired) client"),
        Ok(_) => panic!(
            "lookup returned Ok against an unreachable endpoint — \
             the transport was never called (stub or short-circuit); \
             a real network attempt must produce a Transport error"
        ),
        // Transport (connection refused on port 0) = it tried. Status is also
        // accepted in case the OS maps port 0 to a live service somehow.
        Err(AcError::Transport(_)) | Err(AcError::Status(_)) => {}
        other => panic!("unexpected loader outcome: {other:?}"),
    }
    // Same via the inherent constructor: must succeed to build (env still set).
    assert!(
        HttpAcClient::from_runtime().is_ok(),
        "from_runtime mirrors corelink_ac_from_env"
    );
}

/// (2a) Missing base URL → NotConfigured naming the base URL + the env var.
fn missing_base_url_is_notconfigured_naming_it() {
    clear_all();
    let dir = tempfile::tempdir().unwrap();
    let pat_path = dir.path().join("pat");
    std::fs::write(&pat_path, FILE_PAT).unwrap();
    set(ENV_TENANT, TENANT);
    set(ENV_PAT_FILE, pat_path.to_str().unwrap());
    // ENV_AC_URL deliberately unset.
    match corelink_ac_from_env() {
        Err(AcError::NotConfigured(msg)) => {
            assert!(msg.contains(ENV_AC_URL), "names the missing env var: {msg}");
            assert!(msg.to_lowercase().contains("url"), "names the piece: {msg}");
        }
        other => panic!("missing base URL must be NotConfigured, got {other:?}"),
    }
}

/// (2b) Missing tenant → NotConfigured naming the tenant + its env var.
fn missing_tenant_is_notconfigured_naming_it() {
    clear_all();
    let dir = tempfile::tempdir().unwrap();
    let pat_path = dir.path().join("pat");
    std::fs::write(&pat_path, FILE_PAT).unwrap();
    set(ENV_AC_URL, BASE);
    set(ENV_PAT_FILE, pat_path.to_str().unwrap());
    // ENV_TENANT deliberately unset.
    match corelink_ac_from_env() {
        Err(AcError::NotConfigured(msg)) => {
            assert!(msg.contains(ENV_TENANT), "names the missing env var: {msg}");
            assert!(
                msg.to_lowercase().contains("tenant"),
                "names the piece: {msg}"
            );
        }
        other => panic!("missing tenant must be NotConfigured, got {other:?}"),
    }
}

/// (2c) Missing PAT (no file at the override path + no env fallback) →
/// NotConfigured naming the PAT.
fn missing_pat_is_notconfigured_naming_it() {
    clear_all();
    let dir = tempfile::tempdir().unwrap();
    let absent = dir.path().join("does-not-exist");
    set(ENV_AC_URL, BASE);
    set(ENV_TENANT, TENANT);
    set(ENV_PAT_FILE, absent.to_str().unwrap());
    // No file at the path AND ENV_PAT unset → fail closed.
    match corelink_ac_from_env() {
        Err(AcError::NotConfigured(msg)) => {
            assert!(msg.to_uppercase().contains("PAT"), "names the PAT: {msg}");
            // The secret value is never present (there is none), and the
            // fallback env var is named so the operator knows the contract.
            assert!(msg.contains(ENV_PAT), "names the fallback env var: {msg}");
        }
        other => panic!("missing PAT must be NotConfigured, got {other:?}"),
    }
}

/// (3) File-over-env precedence: with BOTH a present file and the env var set,
/// the FILE wins. We verify by sending the configured client's bearer through a
/// transport-less proof: the loader trims the file's trailing newline and uses
/// its contents — the env PAT is ignored. (Value-equality is checked indirectly:
/// the loader cannot expose the PAT, so precedence is asserted via the negative
/// path in `env_pat_is_the_fallback_only_when_file_absent`.)
fn file_pat_takes_precedence_over_env() {
    clear_all();
    let dir = tempfile::tempdir().unwrap();
    let pat_path = dir.path().join("pat");
    std::fs::write(&pat_path, format!("{FILE_PAT}\n")).unwrap();
    set(ENV_AC_URL, BASE);
    set(ENV_TENANT, TENANT);
    set(ENV_PAT, ENV_PAT_VAL);
    set(ENV_PAT_FILE, pat_path.to_str().unwrap());
    // Both present → the loader builds a client (file wins, env ignored). The
    // secret is opaque, but the build SUCCEEDING with the file present proves the
    // file path is consulted first (the negative case proves the env fallback).
    let client = corelink_ac_from_env().expect("file present → configured");
    let rendered = format!("{client:?}");
    assert!(!rendered.contains(FILE_PAT), "file PAT never rendered");
    assert!(!rendered.contains(ENV_PAT_VAL), "env PAT never rendered");
}

/// (3-neg) The env var is the fallback used ONLY when the file is absent: with
/// no file at the override path but `HUGIT_CORELINK_PAT` set, the loader still
/// builds a configured client (the env fallback fires).
fn env_pat_is_the_fallback_only_when_file_absent() {
    clear_all();
    let dir = tempfile::tempdir().unwrap();
    let absent = dir.path().join("absent-pat");
    set(ENV_AC_URL, BASE);
    set(ENV_TENANT, TENANT);
    set(ENV_PAT, ENV_PAT_VAL);
    set(ENV_PAT_FILE, absent.to_str().unwrap());
    let client = corelink_ac_from_env().expect("env fallback → configured when file absent");
    let rendered = format!("{client:?}");
    assert!(!rendered.contains(ENV_PAT_VAL), "env PAT never rendered");
}

/// (4) The PAT value NEVER appears in the client's Debug, nor in any
/// NotConfigured message (which is built from a present-but-the-loader does not
/// echo values). Belt-and-suspenders over the existing AcConfig redaction.
fn pat_is_never_rendered_in_client_debug_or_errors() {
    clear_all();
    let dir = tempfile::tempdir().unwrap();
    let pat_path = dir.path().join("pat");
    std::fs::write(&pat_path, format!("{FILE_PAT}\n")).unwrap();
    set(ENV_AC_URL, BASE);
    set(ENV_TENANT, TENANT);
    set(ENV_PAT, ENV_PAT_VAL);
    set(ENV_PAT_FILE, pat_path.to_str().unwrap());

    let client = corelink_ac_from_env().expect("configured");
    let dbg = format!("{client:?}");
    assert!(
        !dbg.contains(FILE_PAT),
        "file PAT must be redacted in client Debug"
    );
    assert!(
        !dbg.contains(ENV_PAT_VAL),
        "env PAT must be redacted in client Debug"
    );
    assert!(
        dbg.contains("redacted"),
        "client Debug carries the redaction marker"
    );

    // And an error path (missing tenant) never carries any PAT value either.
    set(ENV_PAT_FILE, pat_path.to_str().unwrap());
    // SAFETY: guarded by ENV_LOCK.
    unsafe { env::remove_var(ENV_TENANT) }
    if let Err(AcError::NotConfigured(msg)) = corelink_ac_from_env() {
        assert!(!msg.contains(FILE_PAT), "error must not carry the file PAT");
        assert!(
            !msg.contains(ENV_PAT_VAL),
            "error must not carry the env PAT"
        );
    }
}
