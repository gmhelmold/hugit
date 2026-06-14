//! `GET /v1/repos/{repo}/viewer-can` → `ViewerCanVm`.
//! Render gate only: the engine re-decides fail-closed on every verb.

use serde::{Deserialize, Serialize};

/// The per-repo capability set of the signed-in viewer (spec §4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ViewerCanVm {
    pub land: bool,
    pub verdict: bool,
    pub comment: bool,
    pub dispatch: bool,
    pub policy: bool,
    pub erasure: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn viewer_can_vm_round_trips() {
        let vm = ViewerCanVm {
            land: true,
            verdict: true,
            comment: true,
            dispatch: false,
            policy: false,
            erasure: false,
        };
        let json = serde_json::to_string(&vm).unwrap();
        let reparsed: ViewerCanVm = serde_json::from_str(&json).unwrap();
        assert_eq!(vm, reparsed, "ViewerCanVm round-trip is lossless");
    }
}
