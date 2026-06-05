//! Cargo workspace adapter for affected-target computation (B3 ①).
//!
//! Builds a [`BuildGraph`] from a `cargo metadata`-style JSON graph fixture.
//! In production this would invoke `cargo metadata --no-deps --format-version 1`;
//! in the acceptance suite it reads the committed JSON fixture.

use serde::Deserialize;

use super::{BuildGraph, Ecosystem, PackageNode};

/// A package entry in the cargo graph JSON fixture.
#[derive(Debug, Deserialize)]
pub struct CargoPackageEntry {
    pub name: String,
    pub path: String,
    pub deps: Vec<String>,
}

/// The cargo graph JSON fixture shape.
#[derive(Debug, Deserialize)]
pub struct CargoGraphFixture {
    pub ecosystem: String,
    pub workspace_root: String,
    pub packages: Vec<CargoPackageEntry>,
}

/// Build a [`BuildGraph`] from a cargo graph JSON fixture (the committed
/// `fixtures/cargo/graph.json`).
pub fn graph_from_fixture(json: &str) -> Result<BuildGraph, serde_json::Error> {
    let fixture: CargoGraphFixture = serde_json::from_str(json)?;
    let packages = fixture
        .packages
        .into_iter()
        .map(|p| PackageNode {
            name: p.name,
            path: p.path,
            direct_deps: p.deps,
        })
        .collect();

    // Root manifests for a cargo workspace: the workspace Cargo.toml.
    // An edit to Cargo.toml at the workspace root invalidates the full graph.
    let root_manifests = vec![fixture.workspace_root.clone(), "Cargo.toml".to_string()];

    Ok(BuildGraph {
        ecosystem: Ecosystem::Cargo,
        packages,
        root_manifests,
    })
}

/// The bundled cargo fixture (compiled into the binary for acceptance tests).
pub const FIXTURE_JSON: &str = include_str!("fixtures/cargo/graph.json");
