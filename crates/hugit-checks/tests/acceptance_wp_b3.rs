//! WP-B3 acceptance tests — affected-targets v0.
//!
//! Acceptance test for the affected-targets contract (WP-B3).
//! Oracle:   tests/acceptance/wp-b3/run.sh.
//!
//! Naming convention (binding, pre-decided by the lead):
//!   item_<n>_<slug>

use hugit_checks::affected::policy;
use hugit_checks::affected::{AffectedSet, BuildGraph, Ecosystem, FullSetReason, PackageNode};
use hugit_checks::affected::{cargo, pnpm, turbo};

// ─── item 1: golden sets cargo / pnpm / turbo ────────────────────────────────

/// ① Golden affected-set for the cargo ecosystem fixture.
///
/// Fixture: `crates/hugit-checks/affected/fixtures/cargo/graph.json`
/// Graph topology:
///   crate-a → crate-c
///   crate-b → crate-c
///   crate-c (leaf)
///
/// Assertions (golden sets):
///   - changed: crate-c → affected: {crate-a, crate-b, crate-c}
///   - changed: crate-a → affected: {crate-a}
///   - changed: crate-b → affected: {crate-b}
#[test]
fn item_1_golden_sets_cargo() {
    let graph = cargo::graph_from_fixture(cargo::FIXTURE_JSON).expect("cargo fixture must parse");

    // changed: crate-c/src/lib.rs
    let set = graph.affected(&["crate-c/src/lib.rs"]);
    assert!(!set.is_full_set);
    assert!(
        set.packages.contains("crate-a"),
        "crate-a must be affected when crate-c changes"
    );
    assert!(
        set.packages.contains("crate-b"),
        "crate-b must be affected when crate-c changes"
    );
    assert!(
        set.packages.contains("crate-c"),
        "crate-c itself must be in affected set"
    );

    // changed: crate-a/src/main.rs
    let set_a = graph.affected(&["crate-a/src/main.rs"]);
    assert!(!set_a.is_full_set);
    assert!(
        set_a.packages.contains("crate-a"),
        "crate-a in its own affected set"
    );
    assert!(
        !set_a.packages.contains("crate-b"),
        "crate-b NOT affected when only crate-a changes"
    );
    assert!(
        !set_a.packages.contains("crate-c"),
        "crate-c NOT affected when only crate-a changes"
    );

    // changed: crate-b/src/main.rs
    let set_b = graph.affected(&["crate-b/src/main.rs"]);
    assert!(!set_b.is_full_set);
    assert!(
        set_b.packages.contains("crate-b"),
        "crate-b in its own affected set"
    );
    assert!(
        !set_b.packages.contains("crate-a"),
        "crate-a NOT affected when only crate-b changes"
    );
}

/// ① Golden affected-set for the pnpm ecosystem fixture.
///
/// Fixture: `crates/hugit-checks/affected/fixtures/pnpm/graph.json`
/// Graph topology:
///   pkg-ui  → pkg-shared
///   pkg-api → pkg-shared
///   pkg-shared (leaf)
///
/// Assertions (golden sets):
///   - changed: pkg-shared → affected: {pkg-api, pkg-shared, pkg-ui}
///   - changed: pkg-ui     → affected: {pkg-ui}
#[test]
fn item_1_golden_sets_pnpm() {
    let graph = pnpm::graph_from_fixture(pnpm::FIXTURE_JSON).expect("pnpm fixture must parse");

    // changed: pkg-shared/index.ts
    let set = graph.affected(&["pkg-shared/index.ts"]);
    assert!(!set.is_full_set);
    assert!(
        set.packages.contains("pkg-ui"),
        "pkg-ui affected when pkg-shared changes"
    );
    assert!(
        set.packages.contains("pkg-api"),
        "pkg-api affected when pkg-shared changes"
    );
    assert!(
        set.packages.contains("pkg-shared"),
        "pkg-shared itself in affected set"
    );

    // changed: pkg-ui/App.tsx
    let set_ui = graph.affected(&["pkg-ui/App.tsx"]);
    assert!(!set_ui.is_full_set);
    assert!(
        set_ui.packages.contains("pkg-ui"),
        "pkg-ui in its own affected set"
    );
    assert!(
        !set_ui.packages.contains("pkg-api"),
        "pkg-api NOT affected when only pkg-ui changes"
    );
    assert!(
        !set_ui.packages.contains("pkg-shared"),
        "pkg-shared NOT affected when only pkg-ui changes"
    );
}

/// ① Golden affected-set for the turbo ecosystem fixture.
///
/// Fixture: `crates/hugit-checks/affected/fixtures/turbo/graph.json`
/// Graph topology:
///   app-web → lib-core, lib-ui
///   app-api → lib-core
///   lib-ui  → lib-core
///   lib-core (leaf)
///
/// Assertions (golden sets):
///   - changed: lib-core → affected: {app-api, app-web, lib-core, lib-ui}
///   - changed: lib-ui   → affected: {app-web, lib-ui}
///   - changed: app-web  → affected: {app-web}
#[test]
fn item_1_golden_sets_turbo() {
    let graph = turbo::graph_from_fixture(turbo::FIXTURE_JSON).expect("turbo fixture must parse");

    // changed: lib-core/index.ts
    let set = graph.affected(&["lib-core/index.ts"]);
    assert!(!set.is_full_set);
    assert!(set.packages.contains("lib-core"), "lib-core itself in set");
    assert!(
        set.packages.contains("lib-ui"),
        "lib-ui affected when lib-core changes"
    );
    assert!(
        set.packages.contains("app-web"),
        "app-web affected when lib-core changes"
    );
    assert!(
        set.packages.contains("app-api"),
        "app-api affected when lib-core changes"
    );

    // changed: lib-ui/Button.tsx
    let set_ui = graph.affected(&["lib-ui/Button.tsx"]);
    assert!(!set_ui.is_full_set);
    assert!(set_ui.packages.contains("lib-ui"), "lib-ui in its own set");
    assert!(
        set_ui.packages.contains("app-web"),
        "app-web affected when lib-ui changes"
    );
    assert!(
        !set_ui.packages.contains("app-api"),
        "app-api NOT affected when only lib-ui changes"
    );
    assert!(
        !set_ui.packages.contains("lib-core"),
        "lib-core NOT affected when only lib-ui changes"
    );

    // changed: app-web/page.tsx
    let set_web = graph.affected(&["app-web/page.tsx"]);
    assert!(!set_web.is_full_set);
    assert!(
        set_web.packages.contains("app-web"),
        "app-web in its own set"
    );
    assert!(
        !set_web.packages.contains("lib-core"),
        "lib-core NOT affected when only app-web changes"
    );
}

// ─── item 2: root edit → full set ────────────────────────────────────────────

/// ② An edit to the workspace-root Cargo.toml invalidates the full graph.
///
/// Rule: root manifest edit → full_set (is_full_set = true,
/// reason = RootManifestEdited). All packages are in the returned set.
#[test]
fn item_2_root_edit_full_set_cargo() {
    let graph = cargo::graph_from_fixture(cargo::FIXTURE_JSON).expect("cargo fixture must parse");
    let all_count = graph.packages.len();

    let set = graph.affected(&["Cargo.toml"]);
    assert!(set.is_full_set, "root Cargo.toml edit must return full set");
    assert_eq!(
        set.packages.len(),
        all_count,
        "full set must contain all {} packages",
        all_count
    );
    assert!(
        matches!(
            set.full_set_reason,
            Some(FullSetReason::RootManifestEdited { .. })
        ),
        "reason must be RootManifestEdited"
    );
}

/// ② An edit to pnpm-workspace.yaml invalidates the full pnpm graph.
#[test]
fn item_2_root_edit_full_set_pnpm() {
    let graph = pnpm::graph_from_fixture(pnpm::FIXTURE_JSON).expect("pnpm fixture must parse");
    let all_count = graph.packages.len();

    let set = graph.affected(&["pnpm-workspace.yaml"]);
    assert!(
        set.is_full_set,
        "root pnpm-workspace.yaml edit must return full set"
    );
    assert_eq!(
        set.packages.len(),
        all_count,
        "full set must contain all {} packages",
        all_count
    );
    assert!(
        matches!(
            set.full_set_reason,
            Some(FullSetReason::RootManifestEdited { .. })
        ),
        "reason must be RootManifestEdited"
    );
}

/// ② An edit to the root turbo.json invalidates the full turbo graph.
#[test]
fn item_2_root_edit_full_set_turbo() {
    let graph = turbo::graph_from_fixture(turbo::FIXTURE_JSON).expect("turbo fixture must parse");
    let all_count = graph.packages.len();

    let set = graph.affected(&["turbo.json"]);
    assert!(set.is_full_set, "root turbo.json edit must return full set");
    assert_eq!(
        set.packages.len(),
        all_count,
        "full set must contain all {} packages",
        all_count
    );
    assert!(
        matches!(
            set.full_set_reason,
            Some(FullSetReason::RootManifestEdited { .. })
        ),
        "reason must be RootManifestEdited"
    );
}

// ─── item 3: unknown ecosystem → full set fail-open ──────────────────────────

/// ③ An unknown ecosystem returns the full set (fail-open policy).
///
/// "Fail-open" = over-run. When the ecosystem is unrecognized, return every
/// known package so no check is silently skipped. The returned set has
/// `is_full_set = true` and `full_set_reason = UnknownEcosystem`.
#[test]
fn item_3_unknown_ecosystem_fail_open() {
    let packages = vec![
        PackageNode {
            name: "pkg-a".into(),
            path: "pkg-a".into(),
            direct_deps: vec![],
        },
        PackageNode {
            name: "pkg-b".into(),
            path: "pkg-b".into(),
            direct_deps: vec!["pkg-a".into()],
        },
    ];
    let graph = BuildGraph {
        ecosystem: Ecosystem::Unknown("maven".into()),
        packages,
        root_manifests: vec![],
    };

    // Any change on an Unknown ecosystem → full set.
    let set = graph.affected(&["pkg-a/src/Main.java"]);
    assert!(
        set.is_full_set,
        "unknown ecosystem must return full set (fail-open)"
    );
    assert!(set.packages.contains("pkg-a"), "pkg-a in full set");
    assert!(set.packages.contains("pkg-b"), "pkg-b in full set");
    assert!(
        matches!(set.full_set_reason, Some(FullSetReason::UnknownEcosystem { ref hint }) if hint == "maven"),
        "reason must be UnknownEcosystem with the ecosystem tag"
    );

    // policy::apply_fail_open — direct policy API.
    let direct = policy::apply_fail_open("gradle", vec!["x".into(), "y".into()]);
    assert!(direct.is_full_set, "apply_fail_open must return full set");
    assert_eq!(direct.packages.len(), 2);
    assert!(matches!(direct.full_set_reason,
        Some(FullSetReason::UnknownEcosystem { ref hint }) if hint == "gradle"));

    // policy::is_recognized_ecosystem
    assert!(
        policy::is_recognized_ecosystem("cargo"),
        "cargo is recognized"
    );
    assert!(
        policy::is_recognized_ecosystem("pnpm"),
        "pnpm is recognized"
    );
    assert!(
        policy::is_recognized_ecosystem("turbo"),
        "turbo is recognized"
    );
    assert!(
        !policy::is_recognized_ecosystem("maven"),
        "maven is not recognized"
    );
    assert!(
        !policy::is_recognized_ecosystem("gradle"),
        "gradle is not recognized"
    );
    assert!(
        !policy::is_recognized_ecosystem(""),
        "empty string not recognized"
    );
}

/// ③ Verify AffectedSet::full + targeted constructors (coverage for the
/// output-shape the QueueApi consumers receive).
#[test]
fn item_3_affected_set_output_shape() {
    // targeted set
    let targeted = AffectedSet::targeted(vec!["a".into(), "b".into()]);
    assert!(!targeted.is_full_set);
    assert!(targeted.full_set_reason.is_none());
    assert_eq!(targeted.packages.len(), 2);

    // full set — unknown ecosystem
    let full = AffectedSet::full(
        vec!["a".into(), "b".into(), "c".into()],
        FullSetReason::UnknownEcosystem {
            hint: "other".into(),
        },
    );
    assert!(full.is_full_set);
    assert_eq!(full.packages.len(), 3);

    // full set — root manifest
    let full_root = AffectedSet::full(
        vec!["a".into()],
        FullSetReason::RootManifestEdited {
            path: "Cargo.toml".into(),
        },
    );
    assert!(full_root.is_full_set);
    assert!(matches!(full_root.full_set_reason,
        Some(FullSetReason::RootManifestEdited { ref path }) if path == "Cargo.toml"));
}
