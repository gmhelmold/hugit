//! O-1 (T-8) — PROPERTY tests for the three-axis memo key security spine.
//!
//! The review sweep (`docs/review/sweep-2026-06-12/tests.md`, T-8) found the
//! memo key had ONLY hand-crafted fixtures. This is the class the mode-bit
//! stale-green (Wave N) came from: an axis silently dropped from the key serves a
//! cached green where the real run now fails. This suite generates random
//! (tree, def, toolchain) inputs and asserts:
//!
//!   - **Determinism:** identical inputs => identical key, always.
//!   - **Axis sensitivity:** changing ANY single axis => a DIFFERENT key (no axis
//!     is silently dropped). Proven both on the raw `compute_memo_key` and on the
//!     full `derive_memo_key` (tree-folding + def-digest) path.
//!   - **No trivial collision:** two different axis tuples => different keys.
//!
//! Property failure => a real bug (a stale-green hole); reported, not papered
//! over.

use hugit_checks::client::memo_key::{FileContent, compute_def_digest, derive_memo_key};
use hugit_contracts::CheckDef;
use hugit_refstore::compute_memo_key;
use proptest::prelude::*;

/// A hex-ish digest string (the axes are lowercase-hex digests in production, but
/// the formula is byte-string agnostic — any UTF-8 works).
fn digest() -> impl Strategy<Value = String> {
    proptest::string::string_regex("[a-f0-9]{0,40}").unwrap()
}

/// Build a `CheckDef` from generated parts (def_digest is recomputed by the
/// derive path, so the incoming value is irrelevant).
fn check_def() -> impl Strategy<Value = CheckDef> {
    (
        proptest::string::string_regex("[a-z ./_-]{1,20}").unwrap(),
        proptest::collection::vec(
            proptest::string::string_regex("[a-z0-9_./*-]{1,12}").unwrap(),
            0..=3,
        ),
        proptest::string::string_regex("[a-z0-9:._-]{1,16}").unwrap(),
        proptest::string::string_regex("[a-z0-9:._-]{0,16}").unwrap(),
        proptest::collection::vec(
            proptest::string::string_regex("[a-z0-9_./*-]{1,12}").unwrap(),
            0..=3,
        ),
    )
        .prop_map(
            |(command, inputs, toolchain_ref, env_manifest, glob_set)| CheckDef {
                def_digest: String::new(),
                command,
                inputs,
                toolchain_ref,
                env_manifest,
                glob_set,
            },
        )
}

/// A generated workspace tree: path -> framed bytes.
fn files() -> impl Strategy<Value = Vec<(String, FileContent)>> {
    proptest::collection::vec(
        (
            proptest::string::string_regex(
                "[a-z][a-z0-9_]{0,6}(/[a-z0-9_]{1,6}){0,2}\\.[a-z]{1,3}",
            )
            .unwrap(),
            proptest::collection::vec(any::<u8>(), 0..=24),
        ),
        0..=4,
    )
    .prop_map(|v| {
        // Dedup paths (a tree cannot have two entries for one path).
        let mut seen = std::collections::BTreeSet::new();
        v.into_iter()
            .filter(|(p, _)| seen.insert(p.clone()))
            .collect()
    })
}

proptest! {
    // Bounded, deterministic for CI; no on-disk regression-seed persistence (the
    // suite is hermetic — a counterexample surfaces in the failure message).
    #![proptest_config(ProptestConfig {
        cases: 512,
        failure_persistence: None,
        ..ProptestConfig::default()
    })]

    // ── Raw three-axis formula (the single canonical realisation) ────────────

    /// Determinism: same (tree, def, toolchain) => identical key, always.
    #[test]
    fn raw_key_is_deterministic(a in digest(), b in digest(), c in digest()) {
        prop_assert_eq!(
            compute_memo_key(&a, &b, &c),
            compute_memo_key(&a, &b, &c),
            "memo key is not deterministic for ({:?},{:?},{:?})", a, b, c
        );
    }

    /// Axis 1 (tree) sensitivity: a != a' => different key.
    #[test]
    fn raw_tree_axis_is_load_bearing(a in digest(), ap in digest(), b in digest(), c in digest()) {
        prop_assume!(a != ap);
        prop_assert_ne!(
            compute_memo_key(&a, &b, &c),
            compute_memo_key(&ap, &b, &c),
            "tree axis silently dropped: ({:?} vs {:?}) gave the same key", a, ap
        );
    }

    /// Axis 2 (def) sensitivity: b != b' => different key.
    #[test]
    fn raw_def_axis_is_load_bearing(a in digest(), b in digest(), bp in digest(), c in digest()) {
        prop_assume!(b != bp);
        prop_assert_ne!(
            compute_memo_key(&a, &b, &c),
            compute_memo_key(&a, &bp, &c),
            "def axis silently dropped: ({:?} vs {:?}) gave the same key", b, bp
        );
    }

    /// Axis 3 (toolchain) sensitivity: c != c' => different key.
    #[test]
    fn raw_toolchain_axis_is_load_bearing(a in digest(), b in digest(), c in digest(), cp in digest()) {
        prop_assume!(c != cp);
        prop_assert_ne!(
            compute_memo_key(&a, &b, &c),
            compute_memo_key(&a, &b, &cp),
            "toolchain axis silently dropped: ({:?} vs {:?}) gave the same key", c, cp
        );
    }

    /// No trivial collision: two DIFFERENT axis tuples => different keys.
    #[test]
    fn raw_distinct_tuples_distinct_keys(
        a in digest(), b in digest(), c in digest(),
        a2 in digest(), b2 in digest(), c2 in digest(),
    ) {
        prop_assume!((&a, &b, &c) != (&a2, &b2, &c2));
        prop_assert_ne!(
            compute_memo_key(&a, &b, &c),
            compute_memo_key(&a2, &b2, &c2),
            "distinct tuples ({:?},{:?},{:?}) vs ({:?},{:?},{:?}) collided",
            a, b, c, a2, b2, c2
        );
    }

    // ── Full derive path (tree-folding + def-digest, the wedge's real key) ───

    /// Determinism end-to-end: same (def, files, toolchain) => identical key.
    #[test]
    fn derive_is_deterministic(def in check_def(), fs in files(), tc in digest()) {
        let f1: Vec<(&str, &FileContent)> = fs.iter().map(|(p, c)| (p.as_str(), c)).collect();
        let f2 = f1.clone();
        prop_assert_eq!(
            derive_memo_key(&def, f1, &tc),
            derive_memo_key(&def, f2, &tc),
            "derive_memo_key is not deterministic"
        );
    }

    /// Toolchain axis is load-bearing through the full derive path (the axis the
    /// Wave-N class is most adjacent to: a silently dropped execution-env axis).
    #[test]
    fn derive_toolchain_axis_is_load_bearing(
        def in check_def(), fs in files(), tc in digest(), tcp in digest(),
    ) {
        prop_assume!(tc != tcp);
        let f1: Vec<(&str, &FileContent)> = fs.iter().map(|(p, c)| (p.as_str(), c)).collect();
        let f2 = f1.clone();
        prop_assert_ne!(
            derive_memo_key(&def, f1, &tc),
            derive_memo_key(&def, f2, &tcp),
            "toolchain axis dropped on the derive path: {:?} vs {:?}", tc, tcp
        );
    }

    /// Def axis is load-bearing through the full derive path: any change to a
    /// def field that the digest covers (command/inputs/toolchain_ref/
    /// env_manifest/glob_set) that ALSO changes the canonical def_digest must
    /// change the key. We assume the digest differs (a pure glob_set change can
    /// re-scope the tree too, but the def_digest is one of the three axes either
    /// way).
    #[test]
    fn derive_def_axis_is_load_bearing(
        def in check_def(), def2 in check_def(), fs in files(), tc in digest(),
    ) {
        prop_assume!(compute_def_digest(&def) != compute_def_digest(&def2));
        // Hold the SAME glob_set so the tree axis is identical and ONLY the
        // def_digest axis differs — proving that axis alone is load-bearing.
        let mut def2 = def2;
        def2.glob_set = def.glob_set.clone();
        prop_assume!(compute_def_digest(&def) != compute_def_digest(&def2));
        let f1: Vec<(&str, &FileContent)> = fs.iter().map(|(p, c)| (p.as_str(), c)).collect();
        let f2 = f1.clone();
        prop_assert_ne!(
            derive_memo_key(&def, f1, &tc),
            derive_memo_key(&def2, f2, &tc),
            "def axis dropped on the derive path"
        );
    }
}
