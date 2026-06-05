//! pnpm workspace adapter for affected-target computation (B3 ①).
//!
//! Builds a [`BuildGraph`] from a pnpm workspace package graph JSON fixture.
//! In production this would parse `pnpm-workspace.yaml` + each `package.json`;
//! in the acceptance suite it reads the committed JSON fixture.

use serde::Deserialize;

use super::{BuildGraph, Ecosystem, PackageNode};

/// A package entry in the pnpm graph JSON fixture.
#[derive(Debug, Deserialize)]
pub struct PnpmPackageEntry {
    pub name: String,
    pub path: String,
    pub deps: Vec<String>,
}

/// The pnpm graph JSON fixture shape.
#[derive(Debug, Deserialize)]
pub struct PnpmGraphFixture {
    pub ecosystem: String,
    pub workspace_root: String,
    pub packages: Vec<PnpmPackageEntry>,
}

/// Build a [`BuildGraph`] from a pnpm graph JSON fixture (the committed
/// `fixtures/pnpm/graph.json`).
pub fn graph_from_fixture(json: &str) -> Result<BuildGraph, serde_json::Error> {
    let fixture: PnpmGraphFixture = serde_json::from_str(json)?;
    let packages = fixture
        .packages
        .into_iter()
        .map(|p| PackageNode {
            name: p.name,
            path: p.path,
            direct_deps: p.deps,
        })
        .collect();

    // Root manifests for a pnpm workspace: pnpm-workspace.yaml.
    // An edit to pnpm-workspace.yaml invalidates the full graph.
    let root_manifests = vec![
        fixture.workspace_root.clone(),
        "pnpm-workspace.yaml".to_string(),
    ];

    Ok(BuildGraph {
        ecosystem: Ecosystem::Pnpm,
        packages,
        root_manifests,
    })
}

/// The bundled pnpm fixture (compiled into the binary for acceptance tests).
pub const FIXTURE_JSON: &str = include_str!("fixtures/pnpm/graph.json");
