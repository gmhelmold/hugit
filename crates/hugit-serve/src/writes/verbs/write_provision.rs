//! `write_provision` — the `POST /v1/repos` self-service repo-create verb
//! (W-PROVISION, v0: CREATE-EMPTY only; `import_git_url` is DEFERRED pending an
//! SSRF audit — the frozen contract's one owner/infra gate).
//!
//! Unlike the other Wave-2 verbs, provision does NOT ride
//! [`with_write`](crate::writes::with_write): that door LOADS the repo's head first
//! (→ 404 for an absent log) and gates on `authorize_write` over the loaded meta —
//! but provision CREATES a repo that does not exist yet. Instead it seeds the genesis
//! `<owner_tenant>/<repo>.json` event-log directly, atomically, via a create-only
//! ([`CasToken::Absent`]) persist.
//!
//! ## The one atomic op (fail-closed, no half-state)
//!
//! The DURABLE artifact is the genesis event-log with a single chain-anchored
//! `repo.meta{visibility, owner_tenant}` record (seq 0). It lands via ONE
//! create-only persist:
//! - Local source → an `Absent`-token compare (create only if still absent),
//! - R2 source → a conditional `If-None-Match: *` PUT.
//!
//! So the genesis either lands whole or not at all: a duplicate / concurrent create
//! fails the precondition (→ 409, no clobber), any other persist fault returns 5xx
//! and NOTHING is created (no 404-but-half-provisioned repo). The runtime git-seam
//! insert happens ONLY AFTER the durable persist succeeds and is a pure in-memory
//! op (it cannot half-create the durable repo).
//!
//! ## Security
//! - `owner_tenant` is DERIVED from the caller's `clerk:{org}:{user}` principal —
//!   NEVER a request field, so a caller can never create a repo under another
//!   tenant, and the body can never spoof ownership.
//! - Anonymous / operator → 401 (a real user creates their OWN repo; there is no
//!   god-create / anon-create over the public door).
//! - The genesis `repo.meta` sets the SAME authz predicate the read gate + the
//!   git-wire read gate re-decide from on every request, so the created repo is
//!   immediately private-to-its-owner (or public), 404-no-oracle to everyone else.

use hugit_refstore::{Endpoint, EventLog};
use serde::Deserialize;
use serde_json::json;

use crate::authz::REPO_META_KIND;
use crate::error::EngineErr;
use crate::state::{AppState, MAX_REPOS_PER_TENANT};
use crate::writes::{CasToken, LogSink};

/// The `POST /v1/repos` request body (v0). `import_git_url` is intentionally ABSENT
/// (deferred — a remote-git egress needs an SSRF audit first); an unknown field is
/// ignored so a client that sends `import_git_url` still creates an EMPTY repo (it
/// does not error, but the field has no effect in v0).
#[derive(Debug, Deserialize)]
pub struct CreateRepoReq {
    /// The repo slug: `[a-z0-9._-]`, 1..=64, no path separator, no leading dot.
    pub name: String,
    /// `"public" | "private"`; absent/null → `"private"` (fail-safe default).
    #[serde(default)]
    pub visibility: Option<String>,
}

/// A successful provision — the routable slug + the resolved visibility.
#[derive(Debug)]
pub struct ProvisionOk {
    /// The routable repo slug (single URL segment) the repo is served under.
    pub repo: String,
    /// The owning tenant (derived from the caller), recorded in the genesis meta.
    pub owner_tenant: String,
    /// `"public" | "private"`.
    pub visibility: String,
}

/// The max provisioned-name length (frozen contract: 1..=64).
const MAX_NAME_LEN: usize = 64;

/// Validate a provisioned repo `name` against the frozen contract:
/// `[a-z0-9._-]`, length 1..=64, no path separator, no leading dot, and (stricter,
/// traversal-safe) never `.`/`..`/containing `..`. A stricter subset of
/// [`crate::state::is_safe_repo_slug`] (lowercase-only, ≤64), so a valid name is
/// always a safe URL slug + R2 key.
///
/// # Errors
/// `400 INVALID_REQUEST` on any violation (fail-closed).
pub fn validate_name(name: &str) -> Result<(), EngineErr> {
    let bad = |m: &str| Err(EngineErr::invalid_request(m.to_string()));
    if name.is_empty() {
        return bad("o nome do repositório não pode ser vazio");
    }
    if name.len() > MAX_NAME_LEN {
        return bad("o nome do repositório excede 64 caracteres");
    }
    if name.starts_with('.') {
        return bad("o nome do repositório não pode começar com ponto");
    }
    if name == "." || name == ".." || name.contains("..") {
        return bad("o nome do repositório não pode conter \"..\"");
    }
    if name.contains('/') || name.contains('\\') {
        return bad("o nome do repositório não pode conter separador de caminho");
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-'))
    {
        return bad("o nome do repositório só aceita [a-z0-9._-]");
    }
    Ok(())
}

/// Normalize + validate the requested visibility. Absent/null → `"private"`.
///
/// # Errors
/// `400 INVALID_REQUEST` if present and not exactly `"public"`/`"private"`.
fn resolve_visibility(req: &CreateRepoReq) -> Result<&'static str, EngineErr> {
    match req.visibility.as_deref().map(str::trim) {
        None | Some("") | Some("private") => Ok("private"),
        Some("public") => Ok("public"),
        Some(_) => Err(EngineErr::invalid_request(
            "visibility deve ser \"public\" ou \"private\"",
        )),
    }
}

/// Derive `owner_tenant` from the caller's engine-resolved principal chain. ONLY a
/// real tenant principal (`clerk:{org}:{user}` with a non-empty org) may self-create;
/// the platform operator (`orchestrator:*`), an anonymous request (empty chain), and
/// any unrecognized/malformed principal are refused (no god-create / anon-create).
///
/// **ONE identity definition (GDPR1 audit B2 — the divergence fix).** The derived org
/// MUST be a valid [`is_safe_account_slug`](crate::state::is_safe_account_slug) — the
/// SAME predicate the account-erase store keys on. This closes a right-to-erasure DoS:
/// without it an org like `Org_A`/`acme.co`/>64-chars could OWN repos (creation/authz/
/// enumeration key on the raw string) but could NEVER stage/persist an erasure (404 on
/// the unsafe slug) — an ownable-but-unerasable class, with the enumeration identity
/// (raw org) and the store-key identity (`[a-z0-9-]`) diverging. Enforcing it HERE — the
/// one place both provision AND account-erase derive the tenant — makes ownership and
/// erasability share a single, traversal-safe identity by construction. A non-conforming
/// org can own NOTHING (fail-closed in the safe direction), so it can never be stranded
/// with un-erasable data.
///
/// # Errors
/// - `401 UNAUTHORIZED` — a non-tenant principal (operator/anon/unknown/malformed).
/// - `400 INVALID_REQUEST` — a tenant whose org is not a safe account slug (the identity
///   divergence guard; a well-behaved Clerk org slug is `[a-z0-9-]` and always passes).
pub fn derive_owner_tenant(principal: &[String]) -> Result<String, EngineErr> {
    let refuse = || {
        Err(EngineErr::unauthorized(
            "crie um repositório autenticado como um tenant (clerk); \
             operador/anônimo não pode criar",
        ))
    };
    let Some(first) = principal.first() else {
        return refuse(); // empty chain = anonymous
    };
    let Some(rest) = first.strip_prefix("clerk:") else {
        return refuse(); // operator (orchestrator:) or any non-clerk prefix
    };
    let org = match rest.split(':').next().unwrap_or("") {
        "" => return refuse(), // "clerk:" / "clerk::user" — malformed, no org
        org => org,
    };
    // ONE identity: the org must be a store-safe account slug, or ownership AND
    // erasability would diverge (audit B2). Fail-closed in the safe direction.
    if !crate::state::is_safe_account_slug(org) {
        return Err(EngineErr::invalid_request(
            "identidade de tenant inválida: o org do Clerk deve ser [a-z0-9-] (≤64)",
        ));
    }
    Ok(org.to_string())
}

/// Build the genesis event-log: one chain-anchored `repo.meta{visibility,
/// owner_tenant}` record (seq 0) — the SAME record the read gate + the git-wire read
/// gate project their authz decision from. Attributed to the creating `principal`
/// (the real tenant), routed through the D14-guarded append under its asserted class.
///
/// # Errors
/// `503 ENGINE_UNAVAILABLE` if the guarded append is denied (fail-honest).
fn build_genesis_log(
    owner_tenant: &str,
    visibility: &str,
    principal: &[String],
    at: u64,
) -> Result<EventLog, EngineErr> {
    let payload_value = json!({ "owner_tenant": owner_tenant, "visibility": visibility });
    let payload = hugit_refstore::canonical_json(&payload_value.to_string())
        .unwrap_or_else(|| payload_value.to_string());
    // D14: assert the REAL caller's class (chain-derived, fail-closed) — a tenant
    // maps to the integration authority (Orchestrator), which `Endpoint::Land`
    // permits (identical to `write_repo_meta`).
    let class = crate::writes::asserted_class(principal)?;
    let mut log = EventLog::new();
    log.append_authorized(
        class,
        Endpoint::Land,
        REPO_META_KIND,
        principal.to_vec(),
        payload,
        at,
    )
    .map_err(|d| {
        EngineErr::unavailable(format!(
            "genesis repo.meta append denied: {}",
            d.reason.code()
        ))
    })?;
    Ok(log)
}

/// Best-effort SELF-HEALING reconcile on the no-clobber path: the slug already exists, so
/// if its durable genesis is owned by THIS tenant, ensure it is registered in the tenant's
/// durable set (heals a repo whose genesis landed but whose registry register crashed
/// mid-provision). Idempotent. A reconcile fault is logged, NEVER surfaced — the caller
/// still returns the authoritative 409; the registry heals on a later touch.
fn reconcile_on_conflict(
    state: &AppState,
    owner_tenant: &str,
    slug: &str,
    principal: &[String],
    at: u64,
) {
    if let Err(e) = state.reconcile_tenant_repo(owner_tenant, slug, principal, at) {
        eprintln!(
            "[hugit-serve] provision: tenant-registry reconcile for {slug:?} failed ({}); \
             the count heals on a later touch",
            e.code
        );
    }
}

/// A 409 "already exists for this owner" (no clobber). A distinct code from the
/// generic `CAS_CONFLICT` so the client can present "repo já existe" precisely.
fn repo_exists_err() -> EngineErr {
    EngineErr {
        status: 409,
        code: "REPO_EXISTS",
        reason: "repositório já existe para este tenant".to_string(),
    }
}

/// A `429 TOO_MANY_REPOS` — this tenant is at [`MAX_REPOS_PER_TENANT`] for the current
/// engine lifetime. A distinct code so the client can present the quota precisely (and
/// so it never collapses into the generic conflict codes).
fn too_many_repos_err() -> EngineErr {
    EngineErr {
        status: 429,
        code: "TOO_MANY_REPOS",
        reason: format!("limite de {MAX_REPOS_PER_TENANT} repositórios por tenant atingido"),
    }
}

/// The provision core (testable): validate → derive owner → per-tenant cap →
/// no-clobber pre-check → build genesis → CREATE-ONLY persist → runtime git-seam
/// insert. Returns the created repo's identity, or the mapped `EngineErr`.
///
/// Atomicity: the genesis persist ([`CasToken::Absent`]) is the single durable
/// commit point. Its precondition failure (a duplicate / concurrent create) → 409,
/// no clobber; any other persist fault propagates (5xx) and NOTHING is created. The
/// runtime seam insert runs ONLY after a durable success (it cannot leave a durable
/// half-state).
pub fn provision(
    state: &AppState,
    body: &[u8],
    principal: &[String],
    at: u64,
) -> Result<ProvisionOk, EngineErr> {
    // 1. Parse + validate the body (400 on bad JSON / name / visibility).
    let req: CreateRepoReq = serde_json::from_slice(body)
        .map_err(|e| EngineErr::invalid_request(format!("corpo inválido: {e}")))?;
    validate_name(&req.name)?;
    let visibility = resolve_visibility(&req)?;
    // 2. Derive the owner tenant from the CALLER (never the body) — 401 for
    //    operator/anon/unknown (no god-create / anon-create).
    let owner_tenant = derive_owner_tenant(principal)?;
    let slug = req.name.clone();

    // 2b. Per-tenant repo cap (DoS guard) — refuse BEFORE any durable write or the
    //     &'static RepoState leak. Every provision permanently leaks a RepoState
    //     (never freed) + writes a durable genesis object, so an uncapped
    //     authenticated create-loop by one tenant OOMs the single-instance engine.
    //     FAIL-CLOSED: an indeterminate count (an unloadable candidate) → 503, never
    //     allow-by-default (a transient read fault can't be used to slip past the cap).
    let owned = state.count_owned_repos(&owner_tenant).ok_or_else(|| {
        EngineErr::unavailable(
            "não foi possível determinar a contagem de repositórios do tenant (fail-closed)",
        )
    })?;
    if owned >= MAX_REPOS_PER_TENANT {
        return Err(too_many_repos_err());
    }

    // 3. Fast in-memory no-clobber pre-check (the authoritative guard is the
    //    create-only persist below): refuse if a seam is loaded OR a LOG already
    //    exists for the slug. A tampered/unreadable existing log (non-404) is NOT a
    //    free slot — surface it, never create over it (fail-closed).
    if state.has_repo_seam(&slug) {
        // SELF-HEALING reconcile (WP-1 atomicity): a repo whose genesis landed but whose
        // registry register crashed mid-provision is un-countable. A retry of the same
        // slug lands here — heal the registry toward the genesis source of truth before
        // the 409, so the count converges (best-effort: a heal fault is logged, the repo
        // is already readable). Idempotent for an already-registered repo.
        reconcile_on_conflict(state, &owner_tenant, &slug, principal, at);
        return Err(repo_exists_err());
    }
    match state.load_verified(&slug) {
        Ok(_) => {
            reconcile_on_conflict(state, &owner_tenant, &slug, principal, at);
            return Err(repo_exists_err()); // a log already exists → no clobber
        }
        Err(e) if e.status == 404 => {} // absent → free to create
        Err(e) => return Err(e),        // 503 (unreadable/tampered) → refuse
    }

    // 4. Build the genesis event-log (one repo.meta record, seq 0).
    let log = build_genesis_log(&owner_tenant, visibility, principal, at)?;

    // 5. THE atomic commit: persist the genesis as a CREATE-ONLY compare-and-swap.
    //    `Absent` → Local content-compare / R2 `If-None-Match: *`. A precondition
    //    failure (someone created it in the race) maps to 409 (no clobber); any
    //    other fault propagates (5xx) with NOTHING created.
    let sink: &dyn LogSink = state;
    match sink.persist(&slug, &log, &CasToken::Absent) {
        Ok(()) => {}
        Err(e) if e.is_cas_conflict() => return Err(repo_exists_err()),
        Err(e) => return Err(e),
    }

    // 5b. Cache the freshly durable genesis meta immediately (task #74, W-METENANT
    //     scaling follow-up): project it from the in-memory genesis `log` we just
    //     persisted (no redundant reload) so `count_owned_repos`/`me_repo_logs` see
    //     this NEW repo's ownership at the very next request, no cold log-walk.
    state.cache_repo_meta(&slug, crate::authz::project_repo_meta(&log));

    // 5c. THE SECOND durable write (WP-1): register the new repo into the tenant's durable
    //     `_tenants/{org}.json` set, so the per-tenant cap ([`count_owned_repos`]) is
    //     durable across reboots (defect A) and O(1) w.r.t. the platform (defect B).
    //     Ordered AFTER the genesis create — the genesis is the SOURCE OF TRUTH, so a repo
    //     is never counted before it durably exists. A crash between the two writes leaves
    //     an un-countable repo that SELF-HEALS: a retry of this slug reconciles it (step 3
    //     above), and the standalone [`reconcile_tenant_repo`] can back-fill any residual.
    //     Best-effort here (mirrors the runtime-seam insert below): the repo is durably
    //     created + readable; a register fault is logged, never a failure of the create
    //     (the count heals on the next touch — the fail-safe direction is a transient
    //     undercount, closed by reconcile, NOT a lost repo).
    if let Err(e) = state.register_repo_in_tenant(&owner_tenant, &slug, principal, at) {
        eprintln!(
            "[hugit-serve] provision: genesis for {slug:?} is durable, but the tenant-registry \
             register failed ({}); the repo is readable — the per-tenant count self-heals on \
             the next provision/reconcile of this slug",
            e.code
        );
    }

    // 6. DURABLY created. Now make it push/clone-live with no reboot: insert the
    //    empty CAS-mode git seam into the runtime overlay. In Local/dev mode (no CAS
    //    template) there is no seam to build — the repo is created + readable, and
    //    push/clone light up once a CAS deploy loads it (honest degradation). An
    //    insert conflict here cannot un-create the durable repo; the repo is already
    //    live for reads, so we surface the seam as best-effort (logged, not fatal).
    if let Some(repo_state) = state.build_empty_cas_repo_state(&slug)
        && let Err(e) = state.insert_runtime_repo(&slug, repo_state)
    {
        eprintln!(
            "[hugit-serve] provision: genesis for {slug:?} is durable, but the runtime \
             git-seam insert failed ({}); the repo is readable — push/clone light up on \
             the next reboot/load",
            e.code
        );
    }

    Ok(ProvisionOk {
        repo: slug,
        owner_tenant,
        visibility: visibility.to_string(),
    })
}

/// `POST /v1/repos` — the HTTP entry point the server route calls. Runs
/// [`provision`] and shapes the `201`/error `(status, body)` pair.
///
/// On success: `201 { "repo": "<slug>", "owner_tenant": "<org>", "visibility":
/// "<...>", "ready": true }`.
///
/// NOTE (deviation, flagged): the frozen contract's response `repo` is
/// `"<owner_tenant>/<name>"`, but the engine routes/stores repos under a SINGLE URL
/// segment ([`is_safe_repo_slug`](crate::state::is_safe_repo_slug) rejects `/`), so a
/// composite `<owner_tenant>/<name>` id is NOT addressable. v0 returns the routable
/// single-segment slug as `repo` and surfaces `owner_tenant` as a sibling field.
#[must_use]
pub fn handle_provision(
    state: &AppState,
    body: &[u8],
    principal: &[String],
    at: u64,
) -> (u16, String) {
    match provision(state, body, principal, at) {
        Ok(ok) => {
            let body = json!({
                "repo": ok.repo,
                "owner_tenant": ok.owner_tenant,
                "visibility": ok.visibility,
                "ready": true,
            });
            (201, body.to_string())
        }
        Err(e) => (e.status, e.to_body()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::authz::{Visibility, authorize_read, project_repo_meta};
    use std::path::PathBuf;

    fn tmp_dir() -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "hugit-provision-{}-{}",
            std::process::id(),
            at_now()
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }
    fn at_now() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0)
    }
    fn tenant(org: &str) -> Vec<String> {
        vec![format!("clerk:{org}:user-1")]
    }
    fn operator() -> Vec<String> {
        vec!["orchestrator:hugit".to_string()]
    }
    fn state_local() -> AppState {
        // Local source (no CAS template) → the genesis-log path is exercised; the
        // git seam is skipped (readable-only), which is exactly right for a hermetic
        // test of the create/read/authz path.
        AppState::new(tmp_dir(), "dev-token".to_string())
    }
    fn body(name: &str, vis: Option<&str>) -> Vec<u8> {
        match vis {
            Some(v) => json!({ "name": name, "visibility": v })
                .to_string()
                .into_bytes(),
            None => json!({ "name": name }).to_string().into_bytes(),
        }
    }

    /// A Local `AppState` plus the on-disk log dir it reads (so a test can seed
    /// durable genesis logs the `count_owned_repos` cap consults).
    fn state_local_with_dir() -> (AppState, PathBuf) {
        let dir = tmp_dir();
        (AppState::new(dir.clone(), "dev-token".to_string()), dir)
    }

    /// Seed one repo OWNED by `owner`, COUNTABLE by the per-tenant cap (WP-1): write its
    /// durable genesis log, wire a (dummy) git seam into the boot `repos` set, AND register
    /// it into the tenant's durable `_tenants/{owner}.json` registry — the O(1) denominator
    /// [`count_owned_repos`] now reads. (A real `provision` does the same two durable writes;
    /// this mirrors the post-provision durable state.)
    fn seed_owned(st: &mut AppState, dir: &std::path::Path, name: &str, owner: &str) {
        let log = build_genesis_log(owner, "private", &tenant(owner), 1).expect("genesis");
        let json = serde_json::to_string(log.records()).expect("serialize genesis");
        std::fs::write(dir.join(format!("{name}.json")), json).expect("write log");
        st.set_repo_git(
            name,
            std::sync::Arc::new(hugit_proto::CasObjectSource::new()),
            gix_hash::ObjectId::empty_tree(gix_hash::Kind::Sha1),
            std::collections::BTreeMap::new(),
        );
        st.register_repo_in_tenant(owner, name, &tenant(owner), 1)
            .expect("register seeded repo into the tenant registry");
    }

    // ── name validation ──────────────────────────────────────────────────────

    #[test]
    fn valid_names_accepted() {
        for ok in ["a", "my-repo", "repo_1", "a.b", "x".repeat(64).as_str()] {
            assert!(validate_name(ok).is_ok(), "{ok:?} should be valid");
        }
    }

    #[test]
    fn invalid_names_rejected_400() {
        for bad in [
            "",                      // empty
            "x".repeat(65).as_str(), // too long
            ".hidden",               // leading dot
            "..",                    // dotdot
            "a..b",                  // contains ..
            "a/b",                   // path sep
            "a\\b",                  // backslash
            "UPPER",                 // uppercase not allowed
            "a b",                   // space
            "wat?",                  // punctuation
        ] {
            let e = validate_name(bad).expect_err("must reject");
            assert_eq!(e.status, 400, "{bad:?} → 400");
            assert_eq!(e.code, "INVALID_REQUEST");
        }
    }

    // ── owner_tenant derivation (401 for non-tenant) ─────────────────────────

    #[test]
    fn derive_owner_from_clerk() {
        assert_eq!(derive_owner_tenant(&tenant("org-a")).unwrap(), "org-a");
    }

    #[test]
    fn derive_owner_rejects_operator_anon_unknown() {
        for chain in [
            operator(),
            vec![],                          // anonymous
            vec!["weird:x".to_string()],     // unknown prefix
            vec!["clerk:".to_string()],      // malformed (no org)
            vec!["clerk::user".to_string()], // empty org
        ] {
            let e = derive_owner_tenant(&chain).expect_err("non-tenant must 401");
            assert_eq!(e.status, 401, "chain {chain:?} → 401");
            assert_eq!(e.code, "UNAUTHORIZED");
        }
    }

    #[test]
    fn derive_owner_rejects_org_that_is_not_a_safe_account_slug() {
        // Audit B2: ONE identity. An org that cannot be a store-safe account slug
        // cannot own anything (fail-closed 400) — so it can never be stranded
        // ownable-but-unerasable (the right-to-erasure DoS the divergence created).
        for bad_org in ["Org_A", "acme.co", "a_b", &"x".repeat(65), "UPPER"] {
            let chain = vec![format!("clerk:{bad_org}:user-1")];
            let e = derive_owner_tenant(&chain).expect_err("unsafe org must be refused");
            assert_eq!(
                e.status, 400,
                "org {bad_org:?} → 400 (identity divergence guard)"
            );
            assert_eq!(e.code, "INVALID_REQUEST");
        }
        // A conforming Clerk org slug still derives cleanly (the common case).
        assert_eq!(
            derive_owner_tenant(&tenant("acme-labs")).unwrap(),
            "acme-labs"
        );
    }

    // ── end-to-end provision (Local mode) ────────────────────────────────────

    #[test]
    fn provision_creates_readable_repo_owner_yes_foreign_and_anon_no() {
        let st = state_local();
        let ok = provision(&st, &body("proj", None), &tenant("org-a"), 1).expect("201");
        assert_eq!(ok.repo, "proj");
        assert_eq!(ok.owner_tenant, "org-a");
        assert_eq!(ok.visibility, "private"); // default

        // The genesis log is durable + readable: the owner passes the read gate, a
        // foreign tenant + anon are denied (→ 404 no-oracle at the route).
        let log = st.load_verified("proj").expect("genesis log is readable");
        let meta = project_repo_meta(&log);
        assert_eq!(meta.visibility, Visibility::Private);
        assert_eq!(meta.owner_tenant.as_deref(), Some("org-a"));
        assert!(authorize_read(&tenant("org-a"), &meta), "owner reads");
        assert!(
            !authorize_read(&tenant("org-b"), &meta),
            "foreign tenant denied"
        );
        assert!(!authorize_read(&[], &meta), "anon denied on private");
    }

    #[test]
    fn provision_public_visibility_is_recorded() {
        let st = state_local();
        let ok =
            provision(&st, &body("openrepo", Some("public")), &tenant("org-a"), 1).expect("201");
        assert_eq!(ok.visibility, "public");
        let meta = project_repo_meta(&st.load_verified("openrepo").unwrap());
        assert_eq!(meta.visibility, Visibility::Public);
        // A public repo opens anon reads (the clone gate).
        assert!(authorize_read(&[], &meta), "anon reads public");
    }

    #[test]
    fn provision_handle_returns_201_json() {
        let st = state_local();
        let (status, body) = handle_provision(&st, &body("h", None), &tenant("org-a"), 1);
        assert_eq!(status, 201);
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v["repo"], "h");
        assert_eq!(v["owner_tenant"], "org-a");
        assert_eq!(v["ready"], true);
    }

    #[test]
    fn duplicate_is_409_and_does_not_clobber() {
        let st = state_local();
        provision(&st, &body("dup", Some("public")), &tenant("org-a"), 1).expect("first 201");
        let before = st.load_verified("dup").unwrap();
        let before_vis = project_repo_meta(&before).visibility;

        // A second create for the SAME slug — even by the SAME tenant with a
        // different visibility — must 409 and NOT overwrite the original meta.
        let e = provision(&st, &body("dup", Some("private")), &tenant("org-a"), 2)
            .expect_err("duplicate must 409");
        assert_eq!(e.status, 409);
        let after = project_repo_meta(&st.load_verified("dup").unwrap());
        assert_eq!(
            after.visibility, before_vis,
            "the original meta is untouched"
        );
        assert_eq!(after.visibility, Visibility::Public);
    }

    #[test]
    fn operator_and_anon_cannot_create_401() {
        let st = state_local();
        for chain in [operator(), vec![]] {
            let e = provision(&st, &body("nope", None), &chain, 1).expect_err("401");
            assert_eq!(e.status, 401);
            // And NOTHING was created (no half-state): the slug stays absent.
            assert_eq!(
                st.load_verified("nope").unwrap_err().status,
                404,
                "a refused create leaves no log"
            );
        }
    }

    #[test]
    fn bad_name_is_400_and_creates_nothing() {
        let st = state_local();
        let e = provision(&st, &body("Bad Name", None), &tenant("org-a"), 1).expect_err("400");
        assert_eq!(e.status, 400);
        // No genesis written for an invalid name (lands-or-nothing).
        assert_eq!(st.git_serving_count(), 0, "no runtime seam leaked");
    }

    #[test]
    fn owner_tenant_comes_from_caller_not_body() {
        // A body that tries to smuggle an `owner_tenant` (or any extra field) has NO
        // effect: ownership is derived from the caller's principal only.
        let st = state_local();
        let spoof = json!({ "name": "sec", "owner_tenant": "org-victim", "visibility": "private" })
            .to_string()
            .into_bytes();
        let ok = provision(&st, &spoof, &tenant("org-attacker"), 1).expect("201");
        assert_eq!(
            ok.owner_tenant, "org-attacker",
            "owner is the caller, not the body"
        );
        let meta = project_repo_meta(&st.load_verified("sec").unwrap());
        assert_eq!(meta.owner_tenant.as_deref(), Some("org-attacker"));
        // The victim tenant cannot read it (it is NOT theirs).
        assert!(!authorize_read(&tenant("org-victim"), &meta));
    }

    // ── per-tenant repo cap (DoS guard) ──────────────────────────────────────

    #[test]
    fn under_cap_creates_normally() {
        // A tenant well under the cap creates without friction (the common case).
        let (mut st, dir) = state_local_with_dir();
        for i in 0..(MAX_REPOS_PER_TENANT - 1) {
            seed_owned(&mut st, &dir, &format!("r{i}"), "org-a");
        }
        assert_eq!(
            st.count_owned_repos("org-a"),
            Some(MAX_REPOS_PER_TENANT - 1)
        );
        // One more (reaching the cap) still succeeds.
        provision(&st, &body("last", None), &tenant("org-a"), 1).expect("under cap → 201");
    }

    #[test]
    fn at_cap_refuses_429_with_no_leak_and_no_write() {
        // Seed the tenant right up to the cap (durable log + a countable seam each).
        let (mut st, dir) = state_local_with_dir();
        for i in 0..MAX_REPOS_PER_TENANT {
            seed_owned(&mut st, &dir, &format!("r{i}"), "org-a");
        }
        assert_eq!(st.count_owned_repos("org-a"), Some(MAX_REPOS_PER_TENANT));

        // The next create for THIS tenant is refused — before any durable write and
        // before the &'static RepoState leak (the DoS the cap closes).
        let seams_before = st.git_serving_count();
        let e = provision(&st, &body("overflow", None), &tenant("org-a"), 1)
            .expect_err("at cap must refuse");
        assert_eq!(e.status, 429);
        assert_eq!(e.code, "TOO_MANY_REPOS");
        // No genesis written (fail-closed BEFORE the persist) — the slug stays absent.
        assert_eq!(
            st.load_verified("overflow").unwrap_err().status,
            404,
            "a capped create leaves no durable log"
        );
        // No runtime seam leaked (the count is unchanged).
        assert_eq!(
            st.git_serving_count(),
            seams_before,
            "a capped create leaks no RepoState"
        );
    }

    #[test]
    fn cap_is_per_tenant_not_global() {
        // org-a is at the cap; a DIFFERENT tenant is unaffected (the cap is scoped by
        // owner_tenant, derived from the caller — not a global create ceiling).
        let (mut st, dir) = state_local_with_dir();
        for i in 0..MAX_REPOS_PER_TENANT {
            seed_owned(&mut st, &dir, &format!("a{i}"), "org-a");
        }
        assert_eq!(st.count_owned_repos("org-a"), Some(MAX_REPOS_PER_TENANT));
        assert_eq!(st.count_owned_repos("org-b"), Some(0));
        // org-a refused, org-b creates fine.
        assert_eq!(
            provision(&st, &body("nope", None), &tenant("org-a"), 1)
                .expect_err("org-a at cap")
                .status,
            429
        );
        provision(&st, &body("fresh", None), &tenant("org-b"), 1).expect("org-b under cap → 201");
    }
}
