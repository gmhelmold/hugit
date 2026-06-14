//! `GET /v1/repos/{repo}/releases` → `ReleasesVm` and its nested types.
//! Transcribed BYTE-FOR-FIELD from the canonical source. All Eq (no f64).

use serde::{Deserialize, Serialize};

/// One downloadable asset, digest always shown.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReleaseAssetVm {
    pub name: String,
    pub size: String,
    pub digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReleaseVm {
    pub version: String,
    pub age: String,
    pub title: String,
    /// True renders the "Latest" pill.
    pub latest: bool,
    pub notes: Vec<String>,
    pub attest_line: String,
    #[serde(default)]
    pub attest_hash: String,
    pub assets: Vec<ReleaseAssetVm>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReleasesVm {
    pub repo: String,
    pub total_count: usize,
    pub stable_count: usize,
    pub year: String,
    pub releases: Vec<ReleaseVm>,
    pub attestation_note: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn releases_vm_round_trips() {
        let vm = ReleasesVm {
            repo: "humangr/corelink-server".to_string(),
            total_count: 8,
            stable_count: 5,
            year: "2026".to_string(),
            releases: vec![ReleaseVm {
                version: "v0.4.0".to_string(),
                age: "há 2 dias".to_string(),
                title: "Wave L — structural hardening".to_string(),
                latest: true,
                notes: vec!["deny-by-default identifier scrub".to_string()],
                attest_line: "atestada — árvore 7e22…b4 · SLSA".to_string(),
                attest_hash: "7e22…b4".to_string(),
                assets: vec![ReleaseAssetVm {
                    name: "hugit-v0.4.0-darwin-arm64.tar.gz".to_string(),
                    size: "8.2 MB".to_string(),
                    digest: "sha256:9f2e…a1".to_string(),
                }],
            }],
            attestation_note: "release sem atestação não existe aqui.".to_string(),
        };
        let json = serde_json::to_string(&vm).expect("serializes");
        let reparsed: ReleasesVm = serde_json::from_str(&json).expect("deserializes");
        assert_eq!(vm, reparsed, "ReleasesVm round-trip is lossless");
        let no_hash = r#"{"version":"v0.3.0","age":"há 7 dias","title":"Wave K","latest":false,"notes":[],"attest_line":"atestada","assets":[]}"#;
        let r: ReleaseVm = serde_json::from_str(no_hash).expect("parses without attest_hash");
        assert_eq!(r.attest_hash, "");
    }
}
