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
use std::sync::atomic::{AtomicU64, Ordering};

fn hugit_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_hugit"))
}

// T-5 fix: a monotonic per-call counter ensures every scratch() invocation
// gets a unique directory even when multiple test functions share the same tag
// string AND cargo runs them in parallel within this binary.  PID alone is
// insufficient because the static tags ("camp-addr", "intent-ulid", …) are
// each used by exactly one test fn but the remove_dir_all + create_dir_all
// sequence is not atomic — two fns colliding on the same name corrupt each
// other's working directory.  The counter produces a strictly distinct suffix
// per call, making collisions structurally impossible regardless of tag value.
static SCRATCH_CTR: AtomicU64 = AtomicU64::new(0);

/// A fresh scratch dir per call (process-id + monotonic counter + tag).
///
/// Each call gets a unique directory: the AtomicU64 counter is incremented
/// before the path is built, so even parallel test functions that share a tag
/// string can never land on the same directory.
fn scratch(tag: &str) -> PathBuf {
    let n = SCRATCH_CTR.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("hugit-wjmatrix-{tag}-{}-{n}", std::process::id()));
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
const XOXB: &str = "xo\x78b-2222222222-3333333333-leaklensslack01";
/// CoreLink PAT (another prefix the old hand-list omitted).
const CLP: &str = "clp_live_9f8e7d6c5b4a3210fedcba9876543210";
/// `Bearer ` token (omitted by the old hand-list).
const BEARER: &str = "Bearer abc123def456ghi789jklmnop";
/// SHORT modern OpenAI project key — the hyphenated `sk-proj-<id>` that the
/// length-gated scan missed before WJ-INT.
const SK_PROJ: &str = "sk-proj-leaklens99999";
/// Connection string with an embedded password.
const CONN: &str = "postgres://dbuser:S3cr3tLeakPassw0rd@db.internal:5432/app";

// ── PREFIX-LESS high-entropy specimens (the Round-8 / L-A ROOT) ──────────────
// These carry NO prefix in any allowlist; under the OLD secret-allowlist they
// rode EVERY identifier field VERBATIM into the forever-log (the confirmed live
// leak: AWS key in `--id` → store count 2). L-A's deny-by-default address gate
// now redacts them at rest (or the door rejects them at input). They are the
// authoritative addition F3 demanded — the matrix now enumerates the threat
// model, not the secret allowlist.
//
/// AWS secret access key shape — 41-char dense base64, no prefix.
const AWS: &str = "wJalrXUtnFEMIK7MDENGbPxRfiCYEXAMPLEKEY123";
/// SendGrid API key shape — `SG.` + two dense base64 segments (no listed prefix).
const SENDGRID: &str = "S\x47.aBcDeFgHiJkLmNoPqRsTuV.wXyZ0123456789aBcDeFgHiJkLmNoPqRsTuVwXyZ012";
/// Stripe restricted live key shape — `rk_live_` + dense alnum (no listed prefix).
const STRIPE: &str = "rk_\x6Cive_51HxYzAbCdEfGhIjKlMnOpQrStUvWxYz0123456789";
/// Dense 32-char base64 token — no prefix at all.
const B64: &str = "aB3xZ9qL2mK7pR4tY8wN6vC1dF5gH0jS";

/// Every secret specimen + a label, for matrix iteration. The first six are
/// prefixed/structural (caught by the old allowlist); the last four are the
/// PREFIX-LESS high-entropy specimens L-A closes (the Round-8 root class). EVERY
/// identifier field × EVERY specimen must be door-rejected OR `[REDACTED]`.
fn secrets() -> Vec<(&'static str, &'static str)> {
    vec![
        ("ghp_", GHP),
        ("xoxb-", XOXB),
        ("clp_", CLP),
        ("Bearer", BEARER),
        ("sk-proj-", SK_PROJ),
        ("conn-string", CONN),
        ("aws-prefixless", AWS),
        ("sendgrid-prefixless", SENDGRID),
        ("stripe-prefixless", STRIPE),
        ("base64-prefixless", B64),
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
        AWS => "wJalrXUtnFEMIK7MDENGbPxRfiCYEXAMPLEKEY123",
        SENDGRID => "wXyZ0123456789aBcDeFgHiJkLmNoPqRsTuVwXyZ012",
        STRIPE => "51HxYzAbCdEfGhIjKlMnOpQrStUvWxYz0123456789",
        B64 => "aB3xZ9qL2mK7pR4tY8wN6vC1dF5gH0jS",
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

/// Bytes at rest EXCLUDING the `<log>.ac` Action-Cache sidecar. This view asserts
/// the WJ-INT scrub boundary (the `--log` + `.hugit/*` stores) is clean. The `.ac`
/// sidecar is a SEPARATE engine write path; it is now ALSO asserted clean directly
/// via [`assert_ac_clean`] (WK-AC closed the toolchain-digest leak — the door
/// rejects a secret-shaped axis and the FileAc write boundary is fail-closed).
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

/// Assert the `<log>.ac` Action-Cache sidecar carries ZERO verbatim bytes of
/// `secret` (WK-AC closure). The `.ac` is the engine write path that USED to leak
/// a secret-shaped memo axis (`toolchain_digest`) verbatim; it is now guarded by
/// the door (`validate_axis`) + the fail-closed FileAc write boundary, so it must
/// be clean for EVERY structural secret shape, permanently. The file may be absent
/// (door-rejected before any AC write) — an absent file is trivially clean.
fn assert_ac_clean(dir: &Path, secret: &str, ctx: &str) {
    let mut acc = String::new();
    collect(dir, &mut acc, &|p| p.to_string_lossy().ends_with(".ac"));
    let needle = secret_needle(secret);
    assert!(
        !acc.contains(needle) && !acc.contains(secret),
        "{ctx}: the .ac sidecar leaked the secret VERBATIM:\n{acc}"
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
// verdict — --tree-hash (K-SCRUB: the cas:/content-address exemption hole)
// ─────────────────────────────────────────────────────────────────────────────

fn record_verdict_tree_hash(log: &str, intent: &str, tree_hash: &str) -> Output {
    run(&[
        "verdict",
        "--intent",
        intent,
        "--log",
        log,
        "--store",
        "--lens",
        "security",
        "--result",
        "approve",
        "--tree-hash",
        tree_hash,
    ])
}

#[test]
fn verdict_tree_hash_field_matrix() {
    // K-SCRUB: `--tree-hash` is an identifier-address that reaches the forever-log
    // (and the `.ac`). EVERY structural secret in it — BARE and `cas:`-prefixed —
    // must be door-rejected OR `[REDACTED]` at rest, with ZERO verbatim bytes.
    for (label, secret) in secrets() {
        for (variant, value) in [
            ("bare", secret.to_string()),
            ("cas-prefixed", format!("cas:{secret}")),
        ] {
            let dir = scratch(&format!(
                "verdict-tree-hash-{variant}-{}",
                label.replace(['/', ' ', '-'], "_")
            ));
            let log = dir.join("log.json");
            std::fs::write(&log, "[]").unwrap();
            let out = record_verdict_tree_hash(log.to_str().unwrap(), "i1", &value);
            let ctx = format!("verdict --tree-hash={variant}={label}");
            assert_door_or_rest(&dir, &out, secret, &ctx);
            // The `.ac` sidecar (a separate engine write path) must also be clean.
            assert_ac_clean(&dir, secret, &ctx);
        }
    }
}

#[test]
fn verdict_tree_hash_cas_credential_does_not_survive() {
    // THE confirmed P0: `verdict … --tree-hash "cas:ghp_…"` stored the PAT
    // VERBATIM in the forever-log because ANY `cas:`-prefixed value was
    // blanket-exempted from the scrub. Value-gated now: a credential behind `cas:`
    // is NOT a content address — door-rejected OR redacted, never verbatim.
    let dir = scratch("verdict-tree-hash-cas-ghp");
    let log = dir.join("log.json");
    std::fs::write(&log, "[]").unwrap();
    let leak = format!("cas:{GHP}");
    let out = record_verdict_tree_hash(log.to_str().unwrap(), "i1", &leak);
    assert_door_or_rest(&dir, &out, GHP, "verdict --tree-hash=cas:ghp_");
    assert_ac_clean(&dir, GHP, "verdict --tree-hash=cas:ghp_ (.ac)");
}

#[test]
fn verdict_legit_cas_and_hex_tree_hash_survive_verbatim() {
    // The addressability half: a real `cas:<64-hex>` and a bare 64-hex tree-hash
    // MUST survive verbatim (the content address is load-bearing, never collapses).
    const HEX64: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
    let cas64 = format!("cas:{HEX64}");

    let dir = scratch("verdict-tree-hash-legit-cas");
    let log = dir.join("log.json");
    std::fs::write(&log, "[]").unwrap();
    let out = record_verdict_tree_hash(log.to_str().unwrap(), "auth-hardening", &cas64);
    assert!(
        out.status.success(),
        "legit cas: tree-hash must record: {out:?}"
    );
    let bytes = at_rest_bytes_excluding_ac(&dir);
    assert!(
        bytes.contains(&cas64),
        "a legit `cas:<64hex>` tree-hash must SURVIVE verbatim:\n{bytes}"
    );

    let dir2 = scratch("verdict-tree-hash-legit-hex");
    let log2 = dir2.join("log.json");
    std::fs::write(&log2, "[]").unwrap();
    let out2 = record_verdict_tree_hash(log2.to_str().unwrap(), "auth-hardening", HEX64);
    assert!(
        out2.status.success(),
        "legit hex tree-hash must record: {out2:?}"
    );
    let bytes2 = at_rest_bytes_excluding_ac(&dir2);
    assert!(
        bytes2.contains(HEX64),
        "a legit bare 64-hex tree-hash must SURVIVE verbatim:\n{bytes2}"
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
        // WK-AC: a secret-shaped --def is rejected at the door, so the def_digest
        // axis can never reach the `.ac` either.
        assert_ac_clean(&dir, secret, &format!("check --def={label} [.ac]"));
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
        // WK-AC: the `.ac` sidecar is now permanently clean for the toolchain axis
        // (the door rejects + the write boundary is fail-closed).
        assert_ac_clean(&dir, secret, &format!("check --toolchain={label} [.ac]"));
    }
}

/// WK-AC CLOSURE: the `.ac` toolchain-digest leak is fixed.
///
/// A secret-shaped `--toolchain` is now REJECTED at the door (`validate_axis` in
/// `checks/run.rs` reuses the shared structural detector → exit-2
/// `secret_in_identifier`), AND — defense-in-depth — the FileAc write boundary
/// is fail-closed (it refuses to persist any memo axis carrying a secret shape).
/// So the secret NEVER reaches the `<log>.ac` sidecar verbatim. A LEGIT toolchain
/// digest still runs, the `.ac` is written normally, and a warm re-run is a real
/// cache HIT (the memo axis is preserved — the fix rejects secret input, it does
/// NOT scrub a stored axis, so the cache is intact).
#[test]
fn check_toolchain_secret_rejected_at_door_and_never_in_ac() {
    let dir = scratch("check-ac-closure");
    let log = dir.join("log.json");
    std::fs::write(&log, "[]").unwrap();
    let out = run_check(
        &dir,
        log.to_str().unwrap(),
        "adhoc-check",
        "true",
        "pr-1",
        "ops",
        GHP,
    );

    // The door rejects it: exit-2, structured `secret_in_identifier`.
    assert_eq!(
        out.status.code(),
        Some(2),
        "a secret --toolchain is rejected at the door (exit 2): {out:?}"
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("secret_in_identifier"),
        "the door surfaces the structured secret_in_identifier error:\n{stdout}"
    );

    // The `--log` is clean (no recording happened).
    let log_bytes = std::fs::read_to_string(&log).unwrap();
    assert!(
        !log_bytes.contains(secret_needle(GHP)),
        "the --log MUST NOT carry the rejected toolchain secret:\n{log_bytes}"
    );

    // CLOSURE: the `<log>.ac` sidecar carries ZERO verbatim bytes of the secret —
    // the door rejected before any AC write, and the write boundary is fail-closed
    // behind it. (The file may not even exist; if it does it is clean.)
    let ac = dir.join("log.json.ac");
    let ac_bytes = std::fs::read_to_string(&ac).unwrap_or_default();
    assert!(
        !ac_bytes.contains(secret_needle(GHP)) && !ac_bytes.contains(GHP),
        "CLOSURE: the .ac MUST NOT leak the toolchain secret:\n{ac_bytes}"
    );

    // A LEGIT toolchain digest still runs, writes the `.ac`, and a warm re-run is
    // a real cache HIT — the fix rejects secret INPUT, it never scrubs a stored
    // axis, so the memo key + cache survive.
    let dir2 = scratch("check-ac-closure-legit");
    let log2 = dir2.join("log.json");
    std::fs::write(&log2, "[]").unwrap();
    let legit_tc = "rustc-1.96.0-abc123def456";
    let cold = run_check(
        &dir2,
        log2.to_str().unwrap(),
        "adhoc-check",
        "true",
        "pr-1",
        "ops",
        legit_tc,
    );
    assert!(cold.status.success(), "a legit toolchain runs: {cold:?}");
    let cold_json = String::from_utf8_lossy(&cold.stdout);
    assert!(
        cold_json.contains("\"cache_hit\":false") || cold_json.contains("\"cache_hit\": false"),
        "the cold run is a MISS:\n{cold_json}"
    );
    // The `.ac` is written normally and carries the legit digest verbatim.
    let ac2 = dir2.join("log.json.ac");
    let ac2_bytes =
        std::fs::read_to_string(&ac2).expect("the .ac is written for a legit toolchain");
    assert!(
        ac2_bytes.contains(legit_tc),
        "the legit toolchain digest survives in the .ac (axis preserved):\n{ac2_bytes}"
    );
    // Warm re-run: same inputs → a real cache HIT (the cache still works).
    let warm = run_check(
        &dir2,
        log2.to_str().unwrap(),
        "adhoc-check",
        "true",
        "pr-1",
        "ops",
        legit_tc,
    );
    assert!(warm.status.success(), "the warm re-run succeeds: {warm:?}");
    let warm_json = String::from_utf8_lossy(&warm.stdout);
    assert!(
        warm_json.contains("\"cache_hit\":true") || warm_json.contains("\"cache_hit\": true"),
        "the warm re-run is a cache HIT — the cache still works:\n{warm_json}"
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

// ─────────────────────────────────────────────────────────────────────────────
// L-A (adversarial Round 8) — DENY-BY-DEFAULT: the prefix-less-credential ROOT
// ─────────────────────────────────────────────────────────────────────────────
//
// The Round-8 class root: a prefix-less high-entropy credential (AWS 40-char
// base64 secret key, SendGrid `SG.`, Stripe `rk_live_`, a dense 32-char base64
// token) rode EVERY identifier field VERBATIM into the forever-log under the old
// secret-allowlist. The control proved it was the EXEMPTION, not a detector gap:
// the SAME AWS key in a FREE-TEXT field redacted (count 0), but in `--id` it rode
// through (store count 2). These tests assert the inversion CLOSED the root over
// the REAL binary: every prefix-less specimen × the explicit AWS-in-every-field
// repro → count 0 at rest; and the legitimate addresses still survive.

/// THE live repro the audit reproduced, run over the real binary: the AWS key in
/// EVERY identifier field of intent/pr/campaign/check must leave ZERO verbatim
/// bytes at rest (door-rejected OR `[REDACTED]`). This is the count-0 closure.
#[test]
fn la_prefixless_aws_key_in_every_identifier_field_is_zero_at_rest() {
    // intent --id and --campaign
    {
        let dir = scratch("la-intent-id");
        let store = dir.join("store.json");
        let out = new_intent(store.to_str().unwrap(), "camp-x", AWS);
        assert_door_or_rest(&dir, &out, AWS, "L-A intent --id=AWS");
    }
    {
        let dir = scratch("la-intent-camp");
        let store = dir.join("store.json");
        let out = new_intent(store.to_str().unwrap(), AWS, "ok-id");
        assert_door_or_rest(&dir, &out, AWS, "L-A intent --campaign=AWS");
    }
    // pr --pr, --campaign, --run-id
    for (field, args) in [
        ("--pr", (AWS, "camp-x", "run-1")),
        ("--campaign", ("pr-1", AWS, "run-1")),
        ("--run-id", ("pr-1", "camp-x", AWS)),
    ] {
        let dir = scratch(&format!("la-pr-{}", field.trim_start_matches('-')));
        let log = dir.join("log.json");
        let (pr, camp, run) = args;
        let out = open_pr(log.to_str().unwrap(), pr, camp, run, "i1");
        assert_door_or_rest(&dir, &out, AWS, &format!("L-A pr {field}=AWS"));
    }
    // pr --principal (author-kind human + principal carries the key)
    {
        let dir = scratch("la-pr-principal");
        let log = dir.join("log.json");
        let out = run(&[
            "pr",
            "open",
            "--log",
            log.to_str().unwrap(),
            "--pr",
            "pr-1",
            "--campaign",
            "camp-x",
            "--author-kind",
            "human",
            "--principal",
            AWS,
            "--intent",
            "i1",
        ]);
        assert_door_or_rest(&dir, &out, AWS, "L-A pr --principal=AWS");
    }
    // campaign --campaign
    {
        let dir = scratch("la-camp");
        let log = dir.join("log.json");
        let out = open_campaign(log.to_str().unwrap(), AWS, "alice");
        assert_door_or_rest(&dir, &out, AWS, "L-A campaign --campaign=AWS");
    }
    // check --pr
    {
        let dir = scratch("la-check-pr");
        let log = dir.join("log.json");
        std::fs::write(&log, "[]").unwrap();
        let out = run_check(
            &dir,
            log.to_str().unwrap(),
            "adhoc-check",
            "true",
            AWS,
            "ops",
            "tc-1",
        );
        assert_door_or_rest(&dir, &out, AWS, "L-A check --pr=AWS");
    }
}

/// Every prefix-less high-entropy specimen (AWS / SendGrid / Stripe / dense
/// base64) in the `--id` identifier field is door-rejected OR `[REDACTED]` —
/// zero verbatim at rest. (The full per-field × per-specimen coverage is in the
/// `*_field_matrix` tests, which now iterate the extended `secrets()` set; this
/// pins the class root explicitly on the identifier field that leaked.)
#[test]
fn la_every_prefixless_specimen_in_id_is_zero_at_rest() {
    for (label, secret) in secrets() {
        let dir = scratch(&format!("la-spec-{}", label.replace(['/', ' ', '.'], "_")));
        let store = dir.join("store.json");
        let out = new_intent(store.to_str().unwrap(), "camp-x", secret);
        assert_door_or_rest(&dir, &out, secret, &format!("L-A intent --id={label}"));
    }
}

/// The ADDRESS-SURVIVAL half (no over-scrub): a ULID, a 40-hex sha, a slug, a
/// `cas:<64hex>`, and a small integer in the SAME identifier fields SURVIVE
/// verbatim (grep ≥ 1). A legit id wrongly redacted would break the address.
#[test]
fn la_legitimate_addresses_survive_in_identifier_fields() {
    const CAS64: &str = "cas:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
    // intent --id: ULID, 40-hex sha, slug, cas:, integer.
    for (tag, addr) in [
        ("ulid", ULID),
        ("hex40", HEX40),
        ("slug", "auth-hardening"),
        ("cas64", CAS64),
        ("integer", "42"),
    ] {
        let dir = scratch(&format!("la-addr-{tag}"));
        let store = dir.join("store.json");
        let out = new_intent(store.to_str().unwrap(), "camp-x", addr);
        assert!(
            out.status.success(),
            "L-A address `{tag}` (`{addr}`) must be accepted as --id: {out:?}"
        );
        let bytes = at_rest_bytes(&dir);
        assert!(
            bytes.contains(addr),
            "L-A address `{tag}` (`{addr}`) must SURVIVE verbatim at rest:\n{bytes}"
        );
        assert!(
            !bytes.contains(REDACTED),
            "L-A address `{tag}` must NOT collapse to the sentinel:\n{bytes}"
        );
    }
}
