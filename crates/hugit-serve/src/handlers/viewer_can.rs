//! `GET /v1/repos/{repo}/viewer-can` → [`ViewerCanVm`].
//!
//! The per-repo capability set of the signed-in viewer (spec §4) — a RENDER GATE
//! only. It tells the window which write affordances to show; the engine
//! re-decides every verb fail-closed at its write-door, so this projection can
//! never grant a capability the engine would not.
//!
//! FAITHFUL to engine enforcement: every one of the 9 write verbs rides ONE
//! per-caller gate at the write-door — [`crate::authz::authorize_write`]
//! (OWNERSHIP: the operator `orchestrator:*`, or the owning tenant whose
//! `clerk:{org}` equals a set `owner_tenant`). So every write affordance here is
//! exactly `authorize_write(principal, meta)`: an owner/operator sees all-true; a
//! non-owner (even on a PUBLIC repo — public opens READS only) sees all-false, so
//! the window hides affordances the engine would 404. `policy`/`erasure`
//! additionally require a STEP-UP at the door (a per-request header, not a standing
//! capability), which this standing hint does not model.
//!
//! History (audit 2026-06-16): this previously projected the D14 matrix per
//! principal-CLASS AND was called with a hardcoded operator stub, so it reported
//! operator capabilities to EVERY viewer. The matrix was never the per-caller write
//! gate anyway (the verbs append as a fixed class; the real per-caller gate is
//! `authorize_write`), so the fix both threads the REAL caller AND projects the
//! REAL gate.
//!
//! No log free-text is echoed (six booleans), so nothing can leak.

use hugit_http_contracts::viewer_can::ViewerCanVm;

use crate::authz::{RepoMeta, authorize_write};

/// Build the viewer-capability render hint from the caller's principal chain + the
/// repo's authz metadata. Every field is `authorize_write(principal, meta)` — the
/// exact per-caller gate the write-door enforces. Fail-closed: an empty /
/// unclassifiable / non-owning principal projects all-false.
pub fn build_viewer_can(principal_chain: &[String], meta: &RepoMeta) -> ViewerCanVm {
    let can_write = authorize_write(principal_chain, meta);
    ViewerCanVm {
        land: can_write,
        verdict: can_write,
        comment: can_write,
        dispatch: can_write,
        // policy/erasure additionally need STEP-UP at the door (per-request header);
        // this standing hint reflects only the ownership gate.
        policy: can_write,
        erasure: can_write,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::authz::Visibility;

    fn private(owner: Option<&str>) -> RepoMeta {
        RepoMeta {
            visibility: Visibility::Private,
            owner_tenant: owner.map(str::to_string),
            erased: false,
        }
    }
    fn public() -> RepoMeta {
        RepoMeta {
            visibility: Visibility::Public,
            owner_tenant: Some("org-a".into()),
            erased: false,
        }
    }
    fn op() -> Vec<String> {
        vec!["orchestrator:hugit".into()]
    }
    fn tenant(org: &str) -> Vec<String> {
        vec![format!("clerk:{org}:u")]
    }
    fn bits(vm: &ViewerCanVm) -> [bool; 6] {
        [
            vm.land,
            vm.verdict,
            vm.comment,
            vm.dispatch,
            vm.policy,
            vm.erasure,
        ]
    }

    #[test]
    fn operator_gets_the_full_set() {
        assert_eq!(bits(&build_viewer_can(&op(), &private(None))), [true; 6]);
        assert_eq!(bits(&build_viewer_can(&op(), &public())), [true; 6]);
    }

    #[test]
    fn owning_tenant_gets_the_full_set() {
        assert_eq!(
            bits(&build_viewer_can(&tenant("org-a"), &private(Some("org-a")))),
            [true; 6]
        );
    }

    #[test]
    fn non_owner_is_all_false_even_on_a_public_repo() {
        // Public opens READS to all, but NOT writes — the hint must reflect that.
        assert_eq!(
            bits(&build_viewer_can(&tenant("org-b"), &public())),
            [false; 6]
        );
        assert_eq!(
            bits(&build_viewer_can(&tenant("org-b"), &private(Some("org-a")))),
            [false; 6]
        );
    }

    #[test]
    fn empty_or_unclassifiable_principal_is_all_false() {
        assert_eq!(bits(&build_viewer_can(&[], &public())), [false; 6]);
        assert_eq!(
            bits(&build_viewer_can(&["alien:x".into()], &public())),
            [false; 6]
        );
    }

    #[test]
    fn vm_round_trips() {
        let vm = build_viewer_can(&op(), &public());
        let j = serde_json::to_string(&vm).unwrap();
        assert_eq!(vm, serde_json::from_str::<ViewerCanVm>(&j).unwrap());
    }
}
