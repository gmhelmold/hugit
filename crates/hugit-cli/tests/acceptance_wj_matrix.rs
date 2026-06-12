//! WP-WJ-INT acceptance — THE per-verb secret MATRIX, proven over the REAL
//! `hugit` binary (`CARGO_BIN_EXE_hugit`), Wave J integration (adversarial
//! Round-6 residual).
//!
//! This is the verification-gap-made-permanent: the secret-at-rest class kept
//! regressing because each wave proved the scrub boundary on ONE verb and
//! asserted it for all. Round 6 found three live leaks the WJ WPs surfaced
//! across fences (an `ident.rs` door weaker than the engine; the hyphenated
//! `sk-proj-` format; a tournament error echoing a raw `--intent`). This suite
//! drives the binary against a MATRIX so the class can never silently regress:
//!
//! For EVERY identifier-bearing field of EVERY verb —
//!
//!   - campaign: `--campaign`, `--owner`
//!   - intent:   `--id`, `--campaign`
//!   - pr:       `--pr`, `--campaign`, `--run-id`, `--intent`
//!   - verdict:  `--intent`, `--lens`
//!   - check:    `--def`, `--cmd`, `--pr`, `--principal`, `--toolchain`
//!
//! it asserts a two-column invariant:
//!
//!   (a) **A PREFIXED / structural SECRET** (`ghp_`, `xoxb-`, `clp_`,
//!       `Bearer …`, a SHORT `sk-proj-<id>`, a `postgres://u:p@h` conn-string)
//!       is EITHER rejected at the door (exit-2 `secret_in_identifier` /
//!       `invalid_argument`, no record written) OR `[REDACTED]` at rest — ZERO
//!       verbatim bytes in the `--log`, in any `.hugit/*` store, and in the
//!       `<log>.ac` Action-Cache sidecar.
//!
//!   (b) **A legitimate ADDRESS** (a 40-hex git-sha, a ULID, a `feature/login`
//!       slug) SURVIVES verbatim and stays addressable (not collapsed to the
//!       sentinel, not redacted).
//!
//! The check is "door OR rest" because the door (`ident.rs`) validates only a
//! SUBSET of identifier fields (`--pr`/`--campaign`/`--run-id`/`--id`/`--owner`)
//! while the central scrub boundary covers EVERY write path — so a field not
//! door-validated MUST still be redacted at rest. Both halves are load-bearing.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn hugit_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_hugit"))
}

/// A fresh scratch dir per test (process-id + tag namespaced).
fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("hugit-wjmatrix-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn run(args: &[&str]) -> Output {
    Command::new(hugit_bin())
        .args(args)
        .output()
        .expect("hugit binary runs")
}

/// The `[REDACTED]` sentinel a scrubbed value collapses to.
const REDACTED: &str = "[REDACTED]";

// ── The secret specimens (column a) ──────────────────────────────────────────
// Each is a DISTINCT, recognisable substring so an at-rest grep is unambiguous.

/// GitHub classic PAT.
const GHP: &str = "ghp_16C7e42F292c6912E7710c838347Ae178B4a";
/// Slack bot token (the prefix the old `ident.rs` hand-list OMITTED).
const XOXB: &str = "xoxb-2222222222-3333333333-leaklensslack01";
/// CoreLink PAT (another prefix the old hand-list omitted).
const CLP: &str = "clp_live_9f8e7d6c5b4a3210fedcba9876543210";
/// `Bearer ` token (omitted by the old hand-list).
const BEARER: &str = "Bearer abc123def456ghi789jklmnop";
/// SHORT modern OpenAI project key — the hyphenated `sk-proj-<id>` that the
/// length-gated scan missed before WJ-INT.
const SK_PROJ: &str = "sk-proj-leaklens99999";
/// Connection string with an embedded password.
const CONN: &str = "postgres://dbuser:S3cr3tLeakPassw0rd@db.internal:5432/app";

/// Every secret specimen + a label, for matrix iteration.
fn secrets() -> Vec<(&'static str, &'static str)> {
    vec![
        ("ghp_", GHP),
        ("xoxb-", XOXB),
        ("clp_", CLP),
        ("Bearer", BEARER),
        ("sk-proj-", SK_PROJ),
        ("conn-string", CONN),
    ]
}

/// The raw, distinguishing substring of a secret that must NOT appear verbatim
/// at rest (the password / token body, not the prefix that may also live in a
/// structural KEY name).
fn secret_needle(secret: &str) -> &str {
    match secret {
        GHP => "16C7e42F292c6912E7710c838347Ae178B4a",
        XOXB => "leaklensslack01",
        CLP => "9f8e7d6c5b4a3210fedcba9876543210",
        BEARER => "abc123def456ghi789jklmnop",
        SK_PROJ => "leaklens99999",
        CONN => "S3cr3tLeakPassw0rd",
        other => other,
    }
}

// ── The address specimens (column b) ─────────────────────────────────────────

/// A 40-hex git-sha-shaped content address.
const HEX40: &str = "a1b2c3d4e5f60718293a4b5c6d7e8f9012345678";
/// A second distinct 40-hex address (to prove no collapse).
const HEX40_B: &str = "ffeeddccbbaa99887766554433221100ffeeddcc";
/// A ULID (Crockford base32 — the canonical intent-id shape).
const ULID: &str = "01HQXW8ZK4M9P2N7R3T5V6Y8BC";
/// A git-style ref slug.
const SLUG: &str = "feature/login";

// ── At-rest assertions ───────────────────────────────────────────────────────

/// All files written under `dir` whose bytes the matrix must inspect: the
/// `--log`, any `.hugit/*` store, and any `<log>.ac` Action-Cache sidecar.
fn at_rest_bytes(dir: &Path) -> String {
    let mut acc = String::new();
    collect(dir, &mut acc, &|_| true);
    acc
}

/// Bytes at rest EXCLUDING the `<log>.ac` Action-Cache sidecar. The AC store is
/// a SEPARATE write path inside the engine's `run_memoized` (checks logic) that
/// does NOT route through the porcelain scrub boundary — a structural secret in
/// `check --toolchain` therefore lands verbatim in `<log>.ac` today (a KNOWN
/// residual this suite pins in `check_toolchain_leaks_into_ac_known_residual_escalate`
/// and ESCALATES — it is OUTSIDE the WJ-INT fence, which owns the scrub boundary
/// the `--log` + `.hugit/*` stores go through). This view asserts the boundary
/// WJ-INT actually owns is clean.
fn at_rest_bytes_excluding_ac(dir: &Path) -> String {
    let mut acc = String::new();
    collect(dir, &mut acc, &|p| !p.to_string_lossy().ends_with(".ac"));
    acc
}

fn collect(dir: &Path, acc: &mut String, keep: &dyn Fn(&Path) -> bool) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            collect(&p, acc, keep);
        } else if keep(&p)
            && let Ok(s) = std::fs::read_to_string(&p)
        {
            acc.push_str(&s);
            acc.push('\n');
        }
    }
}

/// Assert a secret never reached the WJ-owned bytes at rest (the `--log` +
/// `.hugit/*` stores — the scrub boundary WJ-INT covers), verbatim, under `dir`.
/// Excludes the engine's `<log>.ac` sidecar (a separate, fenced write path — see
/// [`at_rest_bytes_excluding_ac`]).
fn assert_no_secret_at_rest(dir: &Path, secret: &str, ctx: &str) {
    let bytes = at_rest_bytes_excluding_ac(dir);
    let needle = secret_needle(secret);
    assert!(
        !bytes.contains(needle),
        "{ctx}: secret body `{needle}` leaked VERBATIM at rest:\n{bytes}"
    );
    // The full literal (prefix + body) must never appear either.
    assert!(
        !bytes.contains(secret),
        "{ctx}: secret `{secret}` leaked VERBATIM at rest:\n{bytes}"
    );
}

/// Assert an address survived verbatim at rest under `dir`.
fn assert_address_at_rest(dir: &Path, address: &str, ctx: &str) {
    let bytes = at_rest_bytes(dir);
    assert!(
        bytes.contains(address),
        "{ctx}: address `{address}` must SURVIVE verbatim at rest:\n{bytes}"
    );
    assert!(
        !bytes.contains(REDACTED),
        "{ctx}: an address must NOT collapse to the sentinel:\n{bytes}"
    );
}

/// A verb invocation outcome the matrix classifies: door-rejected (exit-2,
/// nothing written) OR accepted (exit-0/2 with the value scrubbed at rest).
fn was_door_rejected(out: &Output) -> bool {
    let s = String::from_utf8_lossy(&out.stdout);
    out.status.code() == Some(2)
        && (s.contains("secret_in_identifier") || s.contains("invalid_argument"))
}

/// THE per-field invariant (column a): the field carrying `secret` is EITHER
/// door-rejected (no write) OR scrubbed at rest (0 verbatim in `dir`).
fn assert_door_or_rest(dir: &Path, out: &Output, secret: &str, ctx: &str) {
    if was_door_rejected(out) {
        // Door path: the secret must not have been persisted at all.
        assert_no_secret_at_rest(dir, secret, &format!("{ctx} [door-rejected, no write]"));
    } else {
        // Accept path: the central scrub boundary must have redacted it at rest.
        assert_no_secret_at_rest(
            dir,
            secret,
            &format!("{ctx} [accepted → must scrub at rest]"),
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// campaign — --campaign, --owner
// ─────────────────────────────────────────────────────────────────────────────

fn open_campaign(log: &str, campaign: &str, owner: &str) -> Output {
    run(&[
        "campaign",
        "open",
        "--log",
        log,
        "--campaign",
        campaign,
        "--charter",
        "ship it",
        "--owner",
        owner,
    ])
}

#[test]
fn campaign_campaign_field_matrix() {
    for (label, secret) in secrets() {
        let dir = scratch(&format!(
            "camp-campaign-{}",
            label.replace(['/', ' ', '-'], "_")
        ));
        let log = dir.join("log.json");
        let out = open_campaign(log.to_str().unwrap(), secret, "alice");
        assert_door_or_rest(&dir, &out, secret, &format!("campaign --campaign={label}"));
    }
}

#[test]
fn campaign_owner_field_matrix() {
    for (label, secret) in secrets() {
        let dir = scratch(&format!(
            "camp-owner-{}",
            label.replace(['/', ' ', '-'], "_")
        ));
        let log = dir.join("log.json");
        // A valid campaign key so only --owner carries the secret.
        let out = open_campaign(log.to_str().unwrap(), "camp-x", secret);
        assert_door_or_rest(&dir, &out, secret, &format!("campaign --owner={label}"));
    }
}

#[test]
fn campaign_addresses_survive() {
    let dir = scratch("camp-addr");
    let log = dir.join("log.json");
    let out = open_campaign(log.to_str().unwrap(), HEX40, "ops-team");
    assert!(out.status.success(), "40-hex campaign must open: {out:?}");
    assert_address_at_rest(&dir, HEX40, "campaign --campaign 40-hex");
}

// ─────────────────────────────────────────────────────────────────────────────
// intent — --id, --campaign
// ─────────────────────────────────────────────────────────────────────────────

fn new_intent(store: &str, campaign: &str, id: &str) -> Output {
    run(&[
        "intent",
        "new",
        "--charter",
        "do a thing",
        "--campaign",
        campaign,
        "--id",
        id,
        "--store",
        store,
    ])
}

#[test]
fn intent_id_field_matrix() {
    for (label, secret) in secrets() {
        let dir = scratch(&format!(
            "intent-id-{}",
            label.replace(['/', ' ', '-'], "_")
        ));
        let store = dir.join("store.json");
        let out = new_intent(store.to_str().unwrap(), "camp-x", secret);
        assert_door_or_rest(&dir, &out, secret, &format!("intent --id={label}"));
    }
}

#[test]
fn intent_campaign_field_matrix() {
    for (label, secret) in secrets() {
        let dir = scratch(&format!(
            "intent-camp-{}",
            label.replace(['/', ' ', '-'], "_")
        ));
        let store = dir.join("store.json");
        let out = new_intent(store.to_str().unwrap(), secret, "intent-ok-id");
        assert_door_or_rest(&dir, &out, secret, &format!("intent --campaign={label}"));
    }
}

#[test]
fn intent_ulid_id_survives() {
    let dir = scratch("intent-ulid");
    let store = dir.join("store.json");
    let out = new_intent(store.to_str().unwrap(), "camp-x", ULID);
    assert!(out.status.success(), "ULID --id must be accepted: {out:?}");
    assert_address_at_rest(&dir, ULID, "intent --id ULID");
    // Distinct ULID survives distinctly (no collapse).
    let dir2 = scratch("intent-ulid2");
    let store2 = dir2.join("store.json");
    let out2 = new_intent(
        store2.to_str().unwrap(),
        "camp-x",
        "01HQXW8ZK4M9P2N7R3T5V6Y8XY",
    );
    assert!(out2.status.success());
    assert_address_at_rest(&dir2, "01HQXW8ZK4M9P2N7R3T5V6Y8XY", "intent --id ULID#2");
}

#[test]
fn intent_slug_id_survives() {
    let dir = scratch("intent-slug");
    let store = dir.join("store.json");
    let out = new_intent(store.to_str().unwrap(), "camp-x", SLUG);
    assert!(out.status.success(), "slug --id must be accepted: {out:?}");
    assert_address_at_rest(&dir, SLUG, "intent --id slug");
}

// ─────────────────────────────────────────────────────────────────────────────
// pr — --pr, --campaign, --run-id, --intent
// ─────────────────────────────────────────────────────────────────────────────

fn open_pr(log: &str, pr: &str, campaign: &str, run_id: &str, intent: &str) -> Output {
    run(&[
        "pr",
        "open",
        "--log",
        log,
        "--pr",
        pr,
        "--campaign",
        campaign,
        "--author-kind",
        "orchestrator",
        "--run-id",
        run_id,
        "--intent",
        intent,
    ])
}

#[test]
fn pr_pr_field_matrix() {
    for (label, secret) in secrets() {
        let dir = scratch(&format!("pr-pr-{}", label.replace(['/', ' ', '-'], "_")));
        let log = dir.join("log.json");
        let out = open_pr(log.to_str().unwrap(), secret, "camp-x", "run-1", "i1");
        assert_door_or_rest(&dir, &out, secret, &format!("pr --pr={label}"));
    }
}

#[test]
fn pr_campaign_field_matrix() {
    for (label, secret) in secrets() {
        let dir = scratch(&format!("pr-camp-{}", label.replace(['/', ' ', '-'], "_")));
        let log = dir.join("log.json");
        let out = open_pr(log.to_str().unwrap(), "pr-1", secret, "run-1", "i1");
        assert_door_or_rest(&dir, &out, secret, &format!("pr --campaign={label}"));
    }
}

#[test]
fn pr_run_id_field_matrix() {
    for (label, secret) in secrets() {
        let dir = scratch(&format!("pr-run-{}", label.replace(['/', ' ', '-'], "_")));
        let log = dir.join("log.json");
        let out = open_pr(log.to_str().unwrap(), "pr-1", "camp-x", secret, "i1");
        assert_door_or_rest(&dir, &out, secret, &format!("pr --run-id={label}"));
    }
}

#[test]
fn pr_intent_field_matrix() {
    for (label, secret) in secrets() {
        let dir = scratch(&format!(
            "pr-intent-{}",
            label.replace(['/', ' ', '-'], "_")
        ));
        let log = dir.join("log.json");
        // --intent is NOT door-validated → it MUST be scrubbed at rest.
        let out = open_pr(log.to_str().unwrap(), "pr-1", "camp-x", "run-1", secret);
        assert_door_or_rest(&dir, &out, secret, &format!("pr --intent={label}"));
    }
}

#[test]
fn pr_distinct_40hex_addresses_survive_and_do_not_collapse() {
    let dir = scratch("pr-addr");
    let log = dir.join("log.json");
    let log_s = log.to_str().unwrap();
    let a = open_pr(log_s, HEX40, "camp-x", "run-1", "i1");
    let b = open_pr(log_s, HEX40_B, "camp-x", "run-1", "i1");
    assert!(a.status.success() && b.status.success(), "both PRs open");
    let bytes = at_rest_bytes(&dir);
    assert!(bytes.contains(HEX40), "PR_A 40-hex survives:\n{bytes}");
    assert!(
        bytes.contains(HEX40_B),
        "PR_B 40-hex survives (distinct):\n{bytes}"
    );
    assert!(
        !bytes.contains(REDACTED),
        "no PR address collapses:\n{bytes}"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// verdict — --intent, --lens
// ─────────────────────────────────────────────────────────────────────────────

fn record_verdict(log: &str, intent: &str, lens: &str) -> Output {
    run(&[
        "verdict", "--intent", intent, "--log", log, "--store", "--lens", lens, "--result",
        "approve",
    ])
}

#[test]
fn verdict_intent_field_matrix() {
    for (label, secret) in secrets() {
        let dir = scratch(&format!(
            "verdict-intent-{}",
            label.replace(['/', ' ', '-'], "_")
        ));
        let log = dir.join("log.json");
        std::fs::write(&log, "[]").unwrap();
        let out = record_verdict(log.to_str().unwrap(), secret, "security");
        assert_door_or_rest(&dir, &out, secret, &format!("verdict --intent={label}"));
    }
}

#[test]
fn verdict_lens_field_matrix() {
    for (label, secret) in secrets() {
        let dir = scratch(&format!(
            "verdict-lens-{}",
            label.replace(['/', ' ', '-'], "_")
        ));
        let log = dir.join("log.json");
        std::fs::write(&log, "[]").unwrap();
        let out = record_verdict(log.to_str().unwrap(), "i1", secret);
        assert_door_or_rest(&dir, &out, secret, &format!("verdict --lens={label}"));
    }
}

#[test]
fn verdict_intent_and_lens_are_free_text_not_addresses() {
    // verdict's payload stores `--intent` under the JSON key `intent` (NOT
    // `intent_id`) and `--lens` under `claims_checked` — NEITHER is an
    // identifier-address key, so both route through the FULL free-text scrub by
    // design (WJ-VERDICT / WG-SCRUB). A verdict is recorded against an already-
    // landed change, not a lookup address, so even a high-entropy ULID is
    // free-text-scrubbed at rest. This is NOT a leak (the secret invariant still
    // holds) and NOT a collapse-bug (verdict does not address by these fields) —
    // it is the intended free-text treatment, pinned here so the matrix is honest
    // about which fields are addresses (column b) and which are free text.
    let dir = scratch("verdict-freetext");
    let log = dir.join("log.json");
    std::fs::write(&log, "[]").unwrap();
    let out = record_verdict(log.to_str().unwrap(), ULID, "security");
    assert!(out.status.success(), "ULID intent must record: {out:?}");
    let bytes = at_rest_bytes_excluding_ac(&dir);
    // The high-entropy ULID is free-text-redacted at rest (intended).
    assert!(
        bytes.contains(REDACTED),
        "verdict --intent is free-text-scrubbed at rest (by design):\n{bytes}"
    );
    // A LOW-entropy slug intent, by contrast, survives the free-text engine
    // (it trips no detector) — so verdict over a human-readable id is addressable.
    let dir2 = scratch("verdict-slug");
    let log2 = dir2.join("log.json");
    std::fs::write(&log2, "[]").unwrap();
    let out2 = record_verdict(log2.to_str().unwrap(), "auth-hardening", "security");
    assert!(out2.status.success());
    let bytes2 = at_rest_bytes_excluding_ac(&dir2);
    assert!(
        bytes2.contains("auth-hardening"),
        "a low-entropy slug intent survives verdict's free-text scrub:\n{bytes2}"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// check — --def, --cmd, --pr, --principal, --toolchain
// ─────────────────────────────────────────────────────────────────────────────

/// Run an AD-HOC check (so `--cmd` is honoured) with `def`/`cmd`/`pr`/
/// `principal`/`toolchain` settable. `--cmd true` keeps the run fast + green.
#[allow(clippy::too_many_arguments)]
fn run_check(
    dir: &Path,
    log: &str,
    def: &str,
    cmd: &str,
    pr: &str,
    principal: &str,
    toolchain: &str,
) -> Output {
    run(&[
        "check",
        "--def",
        def,
        "--log",
        log,
        "--store",
        "--cmd",
        cmd,
        "--root",
        dir.to_str().unwrap(),
        "--pr",
        pr,
        "--principal",
        principal,
        "--toolchain",
        toolchain,
    ])
}

#[test]
fn check_def_field_matrix() {
    for (label, secret) in secrets() {
        let dir = scratch(&format!(
            "check-def-{}",
            label.replace(['/', ' ', '-'], "_")
        ));
        let log = dir.join("log.json");
        std::fs::write(&log, "[]").unwrap();
        let out = run_check(
            &dir,
            log.to_str().unwrap(),
            secret,
            "true",
            "pr-1",
            "ops",
            "tc-1",
        );
        assert_door_or_rest(&dir, &out, secret, &format!("check --def={label}"));
    }
}

#[test]
fn check_cmd_field_matrix() {
    for (label, secret) in secrets() {
        let dir = scratch(&format!(
            "check-cmd-{}",
            label.replace(['/', ' ', '-'], "_")
        ));
        let log = dir.join("log.json");
        std::fs::write(&log, "[]").unwrap();
        // The cmd embeds the secret as an echo arg; --def is ad-hoc so cmd runs.
        let cmd = format!("echo {secret}");
        let out = run_check(
            &dir,
            log.to_str().unwrap(),
            "adhoc-check",
            &cmd,
            "pr-1",
            "ops",
            "tc-1",
        );
        assert_door_or_rest(&dir, &out, secret, &format!("check --cmd={label}"));
    }
}

#[test]
fn check_pr_field_matrix() {
    for (label, secret) in secrets() {
        let dir = scratch(&format!("check-pr-{}", label.replace(['/', ' ', '-'], "_")));
        let log = dir.join("log.json");
        std::fs::write(&log, "[]").unwrap();
        let out = run_check(
            &dir,
            log.to_str().unwrap(),
            "adhoc-check",
            "true",
            secret,
            "ops",
            "tc-1",
        );
        assert_door_or_rest(&dir, &out, secret, &format!("check --pr={label}"));
    }
}

#[test]
fn check_principal_field_matrix() {
    for (label, secret) in secrets() {
        let dir = scratch(&format!(
            "check-princ-{}",
            label.replace(['/', ' ', '-'], "_")
        ));
        let log = dir.join("log.json");
        std::fs::write(&log, "[]").unwrap();
        let out = run_check(
            &dir,
            log.to_str().unwrap(),
            "adhoc-check",
            "true",
            "pr-1",
            secret,
            "tc-1",
        );
        assert_door_or_rest(&dir, &out, secret, &format!("check --principal={label}"));
    }
}

#[test]
fn check_toolchain_field_matrix() {
    for (label, secret) in secrets() {
        let dir = scratch(&format!("check-tc-{}", label.replace(['/', ' ', '-'], "_")));
        let log = dir.join("log.json");
        std::fs::write(&log, "[]").unwrap();
        // --toolchain feeds toolchain_digest, a digest-NAMED field — a non-digest
        // secret value must NOT survive the value-gated exemption (WH-SCRUB).
        let out = run_check(
            &dir,
            log.to_str().unwrap(),
            "adhoc-check",
            "true",
            "pr-1",
            "ops",
            secret,
        );
        assert_door_or_rest(&dir, &out, secret, &format!("check --toolchain={label}"));
    }
}

/// KNOWN RESIDUAL — ESCALATE (out of the WJ-INT fence).
///
/// `check --toolchain <structural-secret>` is correctly `[REDACTED]` in the
/// `--log` event payload (the porcelain scrub boundary WJ-INT owns redacts the
/// `toolchain_digest` digest-named field when its value is NOT digest-shaped).
/// BUT the engine's Action Cache (`run_memoized` → `FileAc`) caches the full
/// `CheckResult` — including the RAW `toolchain_digest` — into `<log>.ac` on a
/// SEPARATE write path that does NOT route through the scrub boundary. So the
/// secret survives VERBATIM in `<log>.ac`.
///
/// This is checks/engine logic (`crates/hugit-cli/src/checks/run.rs` +
/// `hugit-checks`), OUTSIDE the WJ-INT owned-files fence (ident.rs / redact.rs /
/// main.rs tournament path). This test PINS the current reality so the leak is
/// recorded, not hidden — and so the fix is detectable: when the `.ac` write is
/// routed through the structural scrub, this assertion flips and the test fails,
/// signalling the residual is closed (update it to assert cleanliness then).
#[test]
fn check_toolchain_leaks_into_ac_known_residual_escalate() {
    let dir = scratch("check-ac-residual");
    let log = dir.join("log.json");
    std::fs::write(&log, "[]").unwrap();
    let _ = run_check(
        &dir,
        log.to_str().unwrap(),
        "adhoc-check",
        "true",
        "pr-1",
        "ops",
        GHP,
    );

    // The WJ-owned boundary (the --log) IS clean — the secret is redacted there.
    let log_bytes = std::fs::read_to_string(&log).unwrap();
    assert!(
        !log_bytes.contains(secret_needle(GHP)),
        "WJ boundary: the --log MUST redact a toolchain secret:\n{log_bytes}"
    );

    // The engine's `.ac` sidecar leaks it (the KNOWN residual). If this file ever
    // stops containing the raw secret, the leak was fixed — flip this assertion.
    let ac = dir.join("log.json.ac");
    let ac_bytes = std::fs::read_to_string(&ac).unwrap_or_default();
    assert!(
        ac_bytes.contains(secret_needle(GHP)),
        "RESIDUAL PINNED: if the .ac no longer leaks the toolchain secret, the \
         engine AC-store fix landed — update this test to assert .ac cleanliness. \
         .ac bytes:\n{ac_bytes}"
    );
}

#[test]
fn check_address_pr_and_toolchain_survive() {
    let dir = scratch("check-addr");
    let log = dir.join("log.json");
    std::fs::write(&log, "[]").unwrap();
    // A 40-hex pr_id (address) and a 64-hex toolchain digest (real content
    // address) must SURVIVE the structural/value-gated scrubs.
    let tc64 = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
    let out = run_check(
        &dir,
        log.to_str().unwrap(),
        "adhoc-check",
        "true",
        HEX40,
        "ops",
        tc64,
    );
    assert!(out.status.success(), "address check must run: {out:?}");
    let bytes = at_rest_bytes(&dir);
    assert!(
        bytes.contains(HEX40),
        "40-hex pr_id survives at rest:\n{bytes}"
    );
    assert!(
        bytes.contains(tc64),
        "64-hex toolchain digest survives at rest:\n{bytes}"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// tournament — --intent error echo (WJ-INT fix #3): a nonexistent secret
// `--intent` must NOT echo raw in the structured `intent_not_found` error.
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn tournament_intent_error_is_scrubbed_matrix() {
    for (label, secret) in secrets() {
        let dir = scratch(&format!("tourney-{}", label.replace(['/', ' ', '-'], "_")));
        let log = dir.join("log.json");
        // A log that DOES carry an intent.landed so the missing-id branch fires.
        let store = dir.join("store.json");
        let seed = new_intent(store.to_str().unwrap(), "camp-x", "real-intent");
        let _ = seed; // store-only seed; the tournament log path needs an intent on --log.
        // Build a --log with one landed intent so the existence check can FAIL on
        // the secret id (an empty log also fails existence, but this proves the
        // real not-found branch over a populated log).
        run(&[
            "intent",
            "new",
            "--charter",
            "x",
            "--campaign",
            "camp-x",
            "--id",
            "real-intent",
            "--store",
            store.to_str().unwrap(),
            "--log",
            log.to_str().unwrap(),
        ]);
        let out = run(&[
            "tournament",
            "-n",
            "2",
            "--intent",
            secret,
            "--log",
            log.to_str().unwrap(),
        ]);
        assert_eq!(
            out.status.code(),
            Some(2),
            "nonexistent intent is exit-2: {label}"
        );
        let s = String::from_utf8_lossy(&out.stdout);
        let needle = secret_needle(secret);
        assert!(
            !s.contains(needle),
            "tournament error must NOT echo the raw `--intent` secret ({label}): {s}"
        );
        assert!(
            s.contains("intent_not_found"),
            "the error is intent_not_found ({label}): {s}"
        );
    }
}
