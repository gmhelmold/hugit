//! `hugit meta set` — append a `repo.meta` record (visibility + owner_tenant).

use serde_json::json;

use super::{KIND_REPO_META, SetMetaArgs};
use crate::campaign::CampaignError;
use crate::campaign::world::{World, append_authorized_and_persist};

/// The two legal visibility values (mirror `authz::Visibility`). The engine
/// treats any non-`public` value as private (fail-safe), but the producer is
/// STRICT — a typo must not silently become "private".
const VISIBILITIES: [&str; 2] = ["public", "private"];

pub fn run(args: SetMetaArgs) -> Result<String, CampaignError> {
    // Visibility is a closed enum — reject anything else loudly (don't let a typo
    // fall through to the engine's fail-safe private default).
    if !VISIBILITIES.contains(&args.visibility.as_str()) {
        return Err(CampaignError::new(
            "invalid_visibility",
            format!(
                "--visibility must be one of {VISIBILITIES:?}; got {:?}",
                args.visibility
            ),
            "pass --visibility public  OR  --visibility private",
        ));
    }

    // owner_tenant (when set) is an IDENTIFIER, not free text — validate it on the
    // same door the campaign/intent keys use (rejects empty-after-trim and
    // credential-prefix shapes before it reaches the forever-log). Empty is legal
    // and means "unassigned" (operator-only).
    if !args.owner_tenant.is_empty() {
        crate::ident::validate_identifier(&args.owner_tenant, "--owner-tenant")
            .map_err(|e| CampaignError::new(e.kind, e.message, e.fix))?;
    }

    // Resolve the default --log ($HUGIT_LOG → .hugit/log.json) once.
    let log_path = crate::log_resolve::resolve_log_checked(args.log.clone()).map_err(|e| {
        CampaignError::new(
            e.kind(),
            e.to_json(),
            "repair blocked migration before appending repository metadata",
        )
    })?;

    // Lock-before-load across the whole load→mutate→persist (the canonical seam
    // discipline). bootstrap=true: a `repo.meta` may be the FIRST record on a
    // fresh repo's log (the seed case).
    let (lock, world) = World::lock_and_load(&log_path, true)?;

    // The engine projects `visibility` + `owner_tenant` off this payload
    // (`project_repo_meta`). owner_tenant is written verbatim; the engine maps an
    // empty string to `None` (unassigned).
    let payload = json!({
        "visibility": args.visibility,
        "owner_tenant": args.owner_tenant,
    })
    .to_string();

    // D14-guarded, scrub-on-append, atomic persist — the same chokepoint the
    // campaign verbs use. repo.meta is a human-owned policy mutation.
    append_authorized_and_persist(
        &lock,
        &world,
        &log_path,
        KIND_REPO_META,
        &args.by,
        payload,
        args.recorded_at.unwrap_or(0),
    )?;

    Ok(json!({
        "repo_meta_set": true,
        "visibility": args.visibility,
        "owner_tenant": args.owner_tenant,
        "by": args.by,
    })
    .to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch_log() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "hugit-repo-meta-{}-{:?}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("log.json")
    }

    fn records(path: &std::path::Path) -> Vec<serde_json::Value> {
        serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
    }

    #[test]
    fn set_writes_a_repo_meta_record_the_engine_can_project() {
        let log = scratch_log();
        let out = run(SetMetaArgs {
            log: Some(log.clone()),
            visibility: "private".into(),
            owner_tenant: "humangr".into(),
            by: "humangr".into(),
            recorded_at: Some(0),
        })
        .expect("set ok");
        assert!(out.contains("\"repo_meta_set\":true"));

        let recs = records(&log);
        let meta = recs
            .iter()
            .rev()
            .find(|r| r["kind"] == KIND_REPO_META)
            .expect("a repo.meta record exists");
        let payload: serde_json::Value =
            serde_json::from_str(meta["payload"].as_str().unwrap()).unwrap();
        assert_eq!(payload["visibility"], "private");
        assert_eq!(payload["owner_tenant"], "humangr");
    }

    #[test]
    fn rejects_a_bad_visibility() {
        let log = scratch_log();
        let err = run(SetMetaArgs {
            log: Some(log),
            visibility: "internal".into(),
            owner_tenant: String::new(),
            by: "humangr".into(),
            recorded_at: None,
        })
        .unwrap_err();
        assert!(err.to_json().contains("invalid_visibility"));
    }

    #[test]
    fn empty_owner_tenant_is_allowed_unassigned() {
        let log = scratch_log();
        run(SetMetaArgs {
            log: Some(log.clone()),
            visibility: "public".into(),
            owner_tenant: String::new(),
            by: "humangr".into(),
            recorded_at: Some(0),
        })
        .expect("empty owner_tenant ok");
        let recs = records(&log);
        let meta = recs
            .iter()
            .rev()
            .find(|r| r["kind"] == KIND_REPO_META)
            .unwrap();
        let payload: serde_json::Value =
            serde_json::from_str(meta["payload"].as_str().unwrap()).unwrap();
        assert_eq!(payload["owner_tenant"], "");
    }
}
