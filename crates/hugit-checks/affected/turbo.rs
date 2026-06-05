//! turbo.json task-graph adapter for affected-target computation (B3 ①).
//!
//! Builds a [`BuildGraph`] from a turbo workspace package graph JSON fixture.
//! In production this would parse `turbo.json` + the workspace package list;
//! in the acceptance suite it reads the committed JSON fixture.

use serde::Deserialize;

use super::{BuildGraph, Ecosystem, PackageNode};

/// A package entry in the turbo graph JSON fixture.
#[derive(Debug, Deserialize)]
pub struct TurboPackageEntry {
    pub name: String,
    pub path: String,
    pub deps: Vec<String>,
}

/// The turbo graph JSON fixture shape.
#[derive(Debug, Deserialize)]
pub struct TurboGraphFixture {
    pub ecosystem: String,
    pub workspace_root: String,
    pub packages: Vec<TurboPackageEntry>,
}

/// Build a [`BuildGraph`] from a turbo graph JSON fixture (the committed
/// `fixtures/turbo/graph.json`).
pub fn graph_from_fixture(json: &str) -> Result<BuildGraph, serde_json::Error> {
    let fixture: TurboGraphFixture = serde_json::from_str(json)?;
    let packages = fixture
        .packages
        .into_iter()
        .map(|p| PackageNode {
            name: p.name,
            path: p.path,
            direct_deps: p.deps,
        })
        .collect();

    // Root manifests for a turbo workspace: turbo.json at the workspace root.
    // An edit to the root turbo.json invalidates the full graph.
    let root_manifests = vec![fixture.workspace_root.clone(), "turbo.json".to_string()];

    Ok(BuildGraph {
        ecosystem: Ecosystem::Turbo,
        packages,
        root_manifests,
    })
}

/// The bundled turbo fixture (compiled into the binary for acceptance tests).
pub const FIXTURE_JSON: &str = include_str!("fixtures/turbo/graph.json");
