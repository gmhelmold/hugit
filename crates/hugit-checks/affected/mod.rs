//! affected — build-graph reachable check-set computation (WP-B3).
//!
//! Given a tree delta (set of changed paths), compute the set of packages
//! ("affected targets") whose checks must run. Design per whitepaper §6.2:
//!
//!   affected(Δtree) = build-graph reachable check set
//!
//! Three ecosystem adapters (①):
//!   - [`cargo`]   — cargo workspace package graph
//!   - [`pnpm`]    — pnpm workspace package graph
//!   - [`turbo`]   — turbo.json task graph
//!
//! Root-edit rule (②): an edit to a workspace root manifest invalidates the
//! whole graph and returns the full check set.
//!
//! Fail-open policy (③): an unknown / unrecognized ecosystem returns the full
//! set — over-run is the safe direction for an affected-set.

pub mod cargo;
pub mod pnpm;
pub mod policy;
pub mod turbo;

use std::collections::BTreeSet;

/// A package or target identifier within a workspace.
pub type PackageName = String;

/// The computed affected set — an ordered, deduplicated set of package names
/// whose checks must run for a given change.
///
/// Emitted in the shape consumed by `QueueApi` (B4a union batching and B2a
/// glob sensitivity).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AffectedSet {
    /// Sorted, deduplicated set of affected package names.
    pub packages: BTreeSet<PackageName>,
    /// True when the set represents the full workspace (root edit or
    /// fail-open triggered).
    pub is_full_set: bool,
    /// The source of the full-set trigger, if any.
    pub full_set_reason: Option<FullSetReason>,
}

/// Why a full check-set was returned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FullSetReason {
    /// A root workspace manifest was among the changed paths (②).
    RootManifestEdited { path: String },
    /// The ecosystem is unrecognized — fail-open policy (③).
    UnknownEcosystem { hint: String },
}

impl AffectedSet {
    /// Construct a targeted (non-full) affected set.
    pub fn targeted(packages: impl IntoIterator<Item = PackageName>) -> Self {
        AffectedSet {
            packages: packages.into_iter().collect(),
            is_full_set: false,
            full_set_reason: None,
        }
    }

    /// Construct a full affected set with a reason.
    pub fn full(
        all_packages: impl IntoIterator<Item = PackageName>,
        reason: FullSetReason,
    ) -> Self {
        AffectedSet {
            packages: all_packages.into_iter().collect(),
            is_full_set: true,
            full_set_reason: Some(reason),
        }
    }
}

/// A package node in the build graph.
#[derive(Debug, Clone)]
pub struct PackageNode {
    /// Package name / identifier.
    pub name: PackageName,
    /// Relative path within the workspace root.
    pub path: String,
    /// Names of packages this package depends on (direct deps only; the graph
    /// walk computes transitive reach).
    pub direct_deps: Vec<PackageName>,
}

/// A workspace build graph — nodes + reverse-dependency index.
#[derive(Debug, Clone)]
pub struct BuildGraph {
    pub ecosystem: Ecosystem,
    pub packages: Vec<PackageNode>,
    /// Root-manifest paths that, when edited, invalidate the whole graph.
    pub root_manifests: Vec<String>,
}

/// Recognized workspace ecosystems.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ecosystem {
    Cargo,
    Pnpm,
    Turbo,
    Unknown(String),
}

impl BuildGraph {
    /// Compute the affected set for a given set of changed paths.
    ///
    /// Algorithm (whitepaper §6.2):
    ///   1. If any changed path is a root manifest → full set (rule ②).
    ///   2. Map changed paths to the packages they belong to.
    ///   3. BFS/DFS over the reverse-dependency graph from the changed
    ///      packages → transitive dependents.
    ///   4. Return the union of changed packages + transitive dependents.
    pub fn affected(&self, changed_paths: &[&str]) -> AffectedSet {
        // Rule ①③: Unknown ecosystem → fail-open full set.
        if let Ecosystem::Unknown(ref hint) = self.ecosystem {
            return AffectedSet::full(
                self.all_package_names(),
                FullSetReason::UnknownEcosystem { hint: hint.clone() },
            );
        }

        // Rule ②: root manifest edit → full set.
        for path in changed_paths {
            for root_manifest in &self.root_manifests {
                if path.contains(root_manifest.as_str()) || root_manifest.contains(*path) {
                    return AffectedSet::full(
                        self.all_package_names(),
                        FullSetReason::RootManifestEdited {
                            path: path.to_string(),
                        },
                    );
                }
            }
        }

        // Build reverse-dep index: dep_name → [packages that depend on it].
        let mut rev: std::collections::HashMap<&str, Vec<&str>> = std::collections::HashMap::new();
        for pkg in &self.packages {
            for dep in &pkg.direct_deps {
                rev.entry(dep.as_str()).or_default().push(pkg.name.as_str());
            }
        }

        // Map changed paths → owning packages (by path prefix).
        let mut seeds: BTreeSet<&str> = BTreeSet::new();
        for path in changed_paths {
            for pkg in &self.packages {
                if path.starts_with(pkg.path.as_str()) {
                    seeds.insert(pkg.name.as_str());
                }
            }
        }

        // BFS over reverse-dep graph from seeds.
        let mut affected: BTreeSet<PackageName> = BTreeSet::new();
        let mut queue: std::collections::VecDeque<&str> = seeds.iter().copied().collect();
        while let Some(name) = queue.pop_front() {
            if affected.insert(name.to_string())
                && let Some(dependents) = rev.get(name)
            {
                for dep in dependents {
                    if !affected.contains(*dep) {
                        queue.push_back(dep);
                    }
                }
            }
        }

        AffectedSet::targeted(affected)
    }

    /// All package names in this graph (used for full-set returns).
    pub fn all_package_names(&self) -> Vec<PackageName> {
        self.packages.iter().map(|p| p.name.clone()).collect()
    }
}

/// Fail-open policy entry point (③): given an unrecognized ecosystem tag,
/// return the full set of all known packages.
///
/// "Fail-open" means over-run (run everything) — the safe direction for an
/// affected-set. Contrast with the fail-CLOSED security/gate paths.
pub fn fail_open_full_set(
    ecosystem_hint: impl Into<String>,
    all_packages: impl IntoIterator<Item = PackageName>,
) -> AffectedSet {
    let hint = ecosystem_hint.into();
    AffectedSet::full(all_packages, FullSetReason::UnknownEcosystem { hint })
}
