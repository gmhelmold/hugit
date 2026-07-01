//! Server state + repo→log resolution + the verified-load chokepoint.
//!
//! The `{repo}` slug arrives from the URL, so it is validated as a single safe
//! path segment (no traversal) BEFORE it is used — a hostile
//! `..%2F..%2Fetc%2Fpasswd` can never escape the source. Loading ALWAYS routes
//! through the engine's single verified loader
//! (`hugit_cli::checks::load_event_log[_from_bytes]` → `rehydrate_and_verify` →
//! `verify_chain`, PS-13) — a tampered chain fails CLOSED as 503, never
//! projected, REGARDLESS of source (local file or R2 object).
//!
//! ## Source (engine-storage)
//! - **Local** (`HUGIT_SERVE_LOG_DIR`): `<dir>/<repo>.json` — the dev/test default.
//! - **R2** (`HUGIT_SERVE_R2_*`): `<tenant_id>/<repo>.json` from the dedicated
//!   `<your-r2-bucket>` bucket over the S3 API, SigV4-signed
//!   ([`crate::sigv4`]). The bucket key contract is CoreLink's
//!   (`<tenant_id>/<repo>.json`; tenant = Clerk `publicMetadata.tenant_id`). Until
//!   real Clerk auth (the P2 identity seam) the tenant is the configured
//!   `HUGIT_SERVE_R2_TENANT_ID` (the single dev tenant) — disclosed, not faked.

use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use hugit_refstore::EventLog;

use crate::cas::LiveOidIndex;
use crate::error::EngineErr;
use crate::sigv4;
use crate::token::{SessionExchangeClient, SessionExchangeConfig, TokenStore};
use crate::writes::CasToken;

/// Where the engine reads canonical event logs from.
#[derive(Clone)]
pub enum LogSource {
    /// Local directory: `<dir>/<repo>.json`.
    Local { dir: PathBuf },
    /// R2 (S3-compatible) bucket: `<tenant_id>/<repo>.json`.
    R2(Box<R2Config>),
}

/// R2 read-source config (engine-storage Option A — the engine reads R2 directly).
#[derive(Clone)]
pub struct R2Config {
    /// `https://<account_id>.r2.cloudflarestorage.com` (no trailing slash).
    pub endpoint: String,
    /// `<account_id>.r2.cloudflarestorage.com` (the signed `host` header).
    pub host: String,
    /// e.g. `example-bucket`.
    pub bucket: String,
    /// SigV4 region — R2 uses `auto`.
    pub region: String,
    pub key_id: String,
    pub secret: String,
    /// The tenant prefix; the configured dev tenant until the P2 Clerk seam.
    pub tenant_id: String,
    /// Sync HTTP client with a bounded timeout (no hanging reads).
    pub agent: ureq::Agent,
}

/// The per-repo git content seam — one entry per repo whose git dir / CAS
/// manifests were loaded at boot. The forge serves MANY repos from ONE engine;
/// each repo's git objects, HEAD root-tree, and refs are independent. A repo with
/// no `RepoState` (not in the [`AppState::repos`] map) has no git content seam →
/// its `blob`/`edit` reads + git-wire clone/fetch 404 honestly (not a fake-empty).
#[derive(Clone)]
pub struct RepoState {
    /// The git object source for this repo's file-content reads (`blob`/`edit`)
    /// and the clone/fetch wire. Always `Some` for a loaded repo (an entry only
    /// exists when the content seam wired); paired with `git_root_tree`.
    pub git_source: Arc<dyn hugit_proto::ObjectSource + Send + Sync>,
    /// The oid of HEAD's root tree in `git_source`, resolved once at boot.
    pub git_root_tree: gix_hash::ObjectId,
    /// This repo's git refs (`ref name → tip oid hex`) for the smart-HTTP wire
    /// serving (`git clone`/`git fetch`). Read from the SAME source the objects
    /// were enumerated from — refs and objects MUST be consistent (advertising a
    /// tip whose closure is not in the source would 404 mid-clone). Never empty
    /// for a loaded repo. A snapshot of this map IS the `hugit_proto::RefView` for
    /// the clone advertisement.
    ///
    /// INTERIOR-MUTABLE: a successful CAS-mode `git push` advances the pushed
    /// `ref → new tip` in-process via [`apply_cas_push_inmemory`](Self::apply_cas_push_inmemory),
    /// so the very next advertise shows the new tip with NO engine reboot. This is
    /// what makes a SECOND push to the ref see the correct base (a stale advertise
    /// would make the client send a stale `old`, which the durably-reloaded log
    /// rejects as a false non-fast-forward).
    pub git_refs: LiveRefs,
    /// The on-disk git dir (`GIT_DIR` mode only; `None` in CAS mode). When `Some`,
    /// the receive-pack write path persists a push's loose objects + ref to it
    /// (via [`write_cas`](Self::write_cas)). A CAS-mode repo has no local dir — its
    /// push sink is [`cas_write`](Self::cas_write) instead.
    pub git_dir: Option<PathBuf>,
    /// The CAS-mode push write seam (`None` in GIT_DIR mode, and `None` for a
    /// CAS-mode repo loaded with the receive-pack deploy flag OFF — the stock
    /// default, so there is NO behavior change). When `Some`, a push buffers into a
    /// [`CasRw`](crate::cas::CasRw), flushes the objects to the CoreLink CAS, and
    /// advances the `refs.json` + `oid-index.json` manifests in hugit's R2.
    pub cas_write: Option<CasWriteSeam>,
    /// CAS mode ONLY: a shared handle to the SAME `oid → blake3` index the
    /// `git_source` ([`crate::cas::LazyCasObjectSource`]) reads. A successful CAS
    /// push merges the just-pushed objects' `oid → blake3` entries here, so a clone
    /// of the new tip resolves its closure from the CAS with NO reboot. `None` in
    /// GitDir mode (no behavior change — its push path is unchanged) and for a
    /// `set_repo_git` test seed. Paired with the [`git_refs`](Self::git_refs)
    /// hot-swap and advanced BEFORE the ref tip (fail-closed: the tip is never
    /// observable before its objects are resolvable in-memory).
    pub live_oid_index: Option<LiveOidIndex>,
}

/// An interior-mutable, shared `ref name → tip oid hex` map — the live counterpart
/// of the boot-loaded refs. Shared (via one `Arc`) between the git-wire read path
/// (the advertise / clone) and the CAS-mode push finalize, so a pushed tip is
/// advertised immediately, no reboot. The accept loop (`server::serve_on`) is
/// single-threaded, so the `RwLock` is uncontended.
#[derive(Clone)]
pub struct LiveRefs(Arc<RwLock<BTreeMap<String, String>>>);

impl LiveRefs {
    /// Wrap a boot-loaded `ref → oid` map.
    #[must_use]
    pub fn new(refs: BTreeMap<String, String>) -> Self {
        Self(Arc::new(RwLock::new(refs)))
    }

    /// An owned snapshot of the LIVE refs (the advertisement's `RefView` source).
    /// Cheap: ref maps are small (a handful of branches), so a clone-per-advertise
    /// is negligible and keeps the read sites working with an owned `BTreeMap`.
    #[must_use]
    pub fn snapshot(&self) -> BTreeMap<String, String> {
        self.0.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Whether the LIVE ref set is empty (a not-loaded / refless repo).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.read().unwrap_or_else(|e| e.into_inner()).is_empty()
    }

    /// Atomically set `ref_name`'s tip to `oid` in the LIVE map (a push hot-swap).
    pub fn set_ref(&self, ref_name: &str, oid: &str) {
        self.0
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert(ref_name.to_string(), oid.to_string());
    }

    /// Atomically remove `ref_name` from the LIVE map (a delete-ref hot-swap). A
    /// no-op if the ref is absent (the durable finalize already validated presence;
    /// this only mirrors the committed removal into the in-memory advertise).
    pub fn remove_ref(&self, ref_name: &str) {
        self.0
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .remove(ref_name);
    }
}

impl From<BTreeMap<String, String>> for LiveRefs {
    fn from(refs: BTreeMap<String, String>) -> Self {
        Self::new(refs)
    }
}

/// The write sink a push resolves to: GIT_DIR mode (loose objects + ref to a local
/// dir) or CAS mode (buffer→flush to the CoreLink CAS, ref + index to R2 manifests).
/// Returned by [`RepoState::open_writer`]; consumed by the receive-pack handler.
pub enum RepoWriter {
    /// GIT_DIR mode — the existing local-dir path (objects + `git update-ref`).
    GitDir {
        /// The write-capable git-dir CAS (loose-object sink).
        cas: hugit_proto::write::store::GitDirCas,
        /// The dir the pushed ref is `git update-ref`'d into.
        git_dir: PathBuf,
    },
    /// CAS mode — the buffering adapter plus the seam needed to commit the push
    /// (flush the objects + rewrite the R2 manifests).
    Cas {
        /// The buffering receive→CAS write adapter (objects flush on `finalize`).
        /// Boxed alongside `seam` so the `Cas` variant doesn't dwarf `GitDir`
        /// (clippy `large_enum_variant`); field access auto-derefs through the
        /// `Box`, so call sites are unchanged.
        cas: Box<crate::cas::CasRw>,
        /// The CAS client + R2 manifest store + tenant/slug for the commit. Boxed
        /// so the `Cas` variant doesn't dwarf `GitDir` (clippy `large_enum_variant`);
        /// field access auto-derefs through the `Box`, so call sites are unchanged.
        seam: Box<CasWriteSeam>,
    },
}

/// The CAS-mode push write seam: the CoreLink CAS client (the pushed-object sink),
/// hugit's R2 (the mutable `refs.json` + `oid-index.json` manifest store), and the
/// tenant/slug those manifests key under. Present only for a CAS-mode repo loaded
/// with the receive-pack deploy flag ON.
#[derive(Clone)]
pub struct CasWriteSeam {
    /// The CoreLink CAS client — the [`CasRw`](crate::cas::CasRw) flush sink AND the
    /// read fall-through for the repo's pre-push closure.
    pub cas_client: crate::cas::CasClient,
    /// The CAS tenant the repo's objects + manifests live under.
    pub tenant: String,
    /// The repo slug the `<tenant>/<repo>/{refs,oid-index}.json` manifests key on.
    pub repo_slug: String,
    /// hugit's R2 — the mutable-manifest store (refs.json + oid-index.json).
    pub r2: R2Config,
}

/// The boot-derived handles needed to mint a fresh repo's CAS-mode git seam at
/// runtime (W-PROVISION). Present only when the engine booted in CAS mode. Cloned
/// per provision into the new repo's [`RepoState`] (its `git_source` +
/// [`CasWriteSeam`]) so the created repo is immediately push/clone-live with no
/// redeploy. Carries no per-repo state — it is a pure factory of the shared client
/// + tenant + manifest store.
#[derive(Clone)]
pub struct ProvisionTemplate {
    /// The CoreLink CAS client (the git-object read/write sink for provisioned repos).
    pub cas_client: crate::cas::CasClient,
    /// The CAS tenant provisioned repos' objects + manifests key under.
    pub tenant: String,
    /// hugit's R2 — the mutable `refs.json`/`oid-index.json` manifest store.
    pub r2: R2Config,
    /// Whether the receive-pack write path is enabled at boot — a provisioned repo
    /// gets a [`CasWriteSeam`] (so its first push works) ONLY when this is on, exactly
    /// mirroring the boot-load gate.
    pub receive_pack_enabled: bool,
}

/// The well-known git SHA-1 of the EMPTY tree — the `git_root_tree` of a freshly
/// provisioned EMPTY repo (no commits yet). A path resolve against it finds nothing
/// (→ an honest 404 for `blob`/`edit`) until the first push adds content.
pub const EMPTY_TREE_OID_HEX: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";

impl CasWriteSeam {
    /// Open a fresh [`CasRw`](crate::cas::CasRw) for a push: re-read the repo's
    /// current `oid-index.json` from R2 so the receive-pack anchor's
    /// "already-live-in-CAS" leg reflects the latest committed closure (not a
    /// possibly-stale boot snapshot). Fail-closed on an R2 read/parse fault.
    pub fn open_writer(&self) -> Result<crate::cas::CasRw, String> {
        let key = crate::cas::oid_index_key(&self.tenant, &self.repo_slug);
        let existing = match <R2Config as crate::cas::R2Get>::get_object(&self.r2, &key)? {
            Some(bytes) => crate::cas::parse_oid_index(&bytes)?,
            None => crate::cas::OidIndex::new(),
        };
        Ok(crate::cas::CasRw::new(self.cas_client.clone(), existing))
    }
}

impl RepoState {
    /// A write-capable [`Cas`](hugit_proto::write::store::Cas) for this repo's
    /// GIT_DIR push path, or `None` when there is no on-disk git dir (CAS mode).
    /// CAS-mode pushes use [`open_writer`](Self::open_writer) instead.
    #[must_use]
    pub fn write_cas(&self) -> Option<hugit_proto::write::store::GitDirCas> {
        self.git_dir
            .as_ref()
            .and_then(|d| hugit_proto::write::store::GitDirCas::new(d).ok())
    }

    /// The repo's HEAD commit oid — the LIVE default-branch tip (`refs/heads/main`,
    /// else `master`, else the first head; the same picker the clone advertise uses).
    /// `None` for a refless repo or a tip that is not a valid oid. Read from the
    /// interior-mutable [`git_refs`](Self::git_refs) snapshot so a just-pushed tip is
    /// reflected immediately (the live hot-swap). This is the start commit for the
    /// blob "Histórico" per-path history walk.
    #[must_use]
    pub fn head_commit(&self) -> Option<gix_hash::ObjectId> {
        let refs = self.git_refs.snapshot();
        let branch = crate::git::pick_default_branch(&refs)?;
        let tip = refs.get(&branch)?;
        gix_hash::ObjectId::from_hex(tip.as_bytes()).ok()
    }

    /// Whether this repo has ANY push write seam (a local git dir OR a CAS write
    /// seam). `false` → receive-pack 404s for it (no oracle). The deploy flag
    /// ([`AppState::write_path_enabled`]) is a SEPARATE, earlier gate.
    #[must_use]
    pub fn has_write_seam(&self) -> bool {
        self.git_dir.is_some() || self.cas_write.is_some()
    }

    /// Open the push write sink. `Ok(None)` → no write seam (the handler 404s).
    /// `Ok(Some(w))` → the sink. `Err` → a seam exists but could not be opened
    /// (e.g. an R2 read fault building the CAS writer) → the handler reports a
    /// transient `ng`, never a fake success and never an oracle. GIT_DIR mode takes
    /// precedence (a repo is one mode or the other).
    pub fn open_writer(&self) -> Result<Option<RepoWriter>, String> {
        if let Some(dir) = &self.git_dir {
            let cas = hugit_proto::write::store::GitDirCas::new(dir)
                .map_err(|e| format!("git-dir write seam: {e}"))?;
            return Ok(Some(RepoWriter::GitDir {
                cas,
                git_dir: dir.clone(),
            }));
        }
        if let Some(seam) = &self.cas_write {
            let cas = seam.open_writer()?;
            return Ok(Some(RepoWriter::Cas {
                cas: Box::new(cas),
                seam: Box::new(seam.clone()),
            }));
        }
        Ok(None)
    }

    /// The live in-process ref hot-swap (the frozen design's step 2): after a
    /// SUCCESSFUL CAS-mode `finalize_cas_push` (durable CAS objects + R2 manifests
    /// already committed), refresh THIS engine's in-memory view so the new tip
    /// serves with NO reboot.
    ///
    /// `index_add` is the push's `git_oid → blake3` additions, reused VERBATIM from
    /// the [`CasRw`](crate::cas::CasRw) that finalized the push — never re-read from
    /// R2 (the in-memory state must never be ahead of the durable store; reusing the
    /// already-committed additions keeps the two byte-identical).
    ///
    /// ## Ordering — fail-closed
    ///
    /// The oid-index is merged FIRST, the ref tip advanced LAST — the same order the
    /// durable manifests use ([`crate::cas::commit_cas_push_manifests`]). So at no
    /// observable moment does the advertise name a tip whose object closure is not
    /// already resolvable in-memory. Every pushed oid is parsed BEFORE any mutation,
    /// so a malformed oid aborts the refresh leaving the in-memory view wholly
    /// consistent (stale, never inconsistent): the durable push still succeeded and
    /// the correct tip loads on the next reboot.
    ///
    /// `Err` ⇒ the in-memory refresh was NOT applied (the ref was NOT advanced); the
    /// caller keeps serving the prior tip (a read never regresses) and the durable
    /// store remains the source of truth.
    pub fn apply_cas_push_inmemory(
        &self,
        ref_name: &str,
        new_oid: &str,
        index_add: &std::collections::HashMap<String, String>,
        consumed_bases: &[String],
    ) -> Result<(), String> {
        // Parse every pushed git oid BEFORE touching any in-memory state, so a
        // malformed oid leaves the view unchanged (fail-closed, never half-applied).
        let mut parsed: Vec<(gix_hash::ObjectId, String)> = Vec::with_capacity(index_add.len());
        for (oid_hex, blake3) in index_add {
            let oid = gix_hash::ObjectId::from_hex(oid_hex.as_bytes()).map_err(|e| {
                format!("push hot-swap: pushed oid {oid_hex:?} is not a valid git oid: {e}")
            })?;
            parsed.push((oid, blake3.clone()));
        }
        // 1. oid-index FIRST — the new objects become resolvable before the tip moves.
        match &self.live_oid_index {
            Some(index) => index.merge(parsed),
            // No live index but objects to record ⇒ we cannot guarantee the new tip's
            // closure is resolvable in-memory. Refuse to advance the ref (fail-closed:
            // never advertise an unresolvable tip). CAS-mode repos always carry a live
            // index, so this is a defensive guard, not a reachable path.
            None if !parsed.is_empty() => {
                return Err(
                    "push hot-swap: no live oid-index to record the pushed objects \
                     (refusing to advertise an unresolvable tip)"
                        .to_string(),
                );
            }
            None => {}
        }
        // 2. RE-ASSERT thin-pack base resolvability (defence-in-depth) — DONE here.
        // Post-#206 a push may resolve a delta against a base that lives ONLY in the
        // repo's prior closure (a thin-pack / REF_DELTA base the pushed pack omits).
        // `consumed_bases` is exactly the existing-closure oids THIS push resolved a
        // base from. Before advancing the tip, every one MUST be present in the
        // read-path `live_oid_index` — else the advertise would name a tip whose
        // closure the read path can't resolve (a clone would 500 mid-stream).
        //
        // Today this is safe BY CONSTRUCTION under `max_instances:1`: a consumed base
        // ∈ `CasRw.existing` == R2's `oid-index.json` == the `live_oid_index` boot
        // seed, so the check never fires. The guard exists to catch a FUTURE
        // divergence (HA / a stale live index) — fail-closed: refuse to advance (the
        // push is already durable; the prior tip keeps serving and the correct tip
        // loads on reboot — the existing failure path). The `None` index case is
        // already refused by the `None if !parsed.is_empty()` guard above; this
        // base-check only runs when there IS a live index to consult.
        if let Some(index) = &self.live_oid_index {
            for base_hex in consumed_bases {
                let base = gix_hash::ObjectId::from_hex(base_hex.as_bytes()).map_err(|e| {
                    format!(
                        "push hot-swap: consumed thin-pack base {base_hex:?} is not a valid \
                         git oid: {e}"
                    )
                })?;
                if !index.contains(&base) {
                    return Err(format!(
                        "push hot-swap: thin-pack base {base_hex} resolved during the push is \
                         absent from the live oid-index (refusing to advertise an unresolvable tip)"
                    ));
                }
            }
        }
        // 3. ref tip LAST — the new objects (step 1) AND every consumed base (step 2)
        // are now proven resolvable on the read path, so the advertised tip's closure
        // is whole the moment it appears.
        self.git_refs.set_ref(ref_name, new_oid);
        Ok(())
    }

    /// The live in-process delete-ref hot-swap (the frozen design's step 3): after a
    /// SUCCESSFUL CAS-mode [`crate::cas::finalize_cas_delete`] (the durable refs.json
    /// rewrite + the `ref.delete` event already committed), remove the ref from THIS
    /// engine's in-memory advertise so the deleted branch stops being advertised with
    /// NO reboot.
    ///
    /// The oid-index is deliberately NOT touched — a git delete-ref drops only the ref
    /// pointer, never the objects (no GC), so the deleted tip's closure stays resolvable
    /// for any OTHER ref that names it. Called ONLY AFTER the durable finalize, never
    /// ahead of R2 (the in-memory view never leads the durable store), mirroring the
    /// [`apply_cas_push_inmemory`](Self::apply_cas_push_inmemory) ordering discipline.
    pub fn apply_cas_delete_inmemory(&self, ref_name: &str) {
        self.git_refs.remove_ref(ref_name);
    }
}

/// Immutable server configuration.
///
/// MULTI-REPO: one engine instance serves many repos (the forge model). The
/// event-log source is shared (it keys by `<repo>.json` already); the per-repo
/// git content seam lives in [`repos`](Self::repos), resolved per request by the
/// `{repo}` URL slug. An unknown slug has no entry → a uniform 404 (no oracle).
#[derive(Clone)]
pub struct AppState {
    /// Where event logs are read from (local dir | R2). SHARED across repos — it
    /// already keys by `<repo>.json` / `<tenant>/<repo>.json`, so the one source
    /// serves every repo's log; the `{repo}` slug selects the object.
    pub source: LogSource,
    /// The Wave-1 dev Bearer token (the P2-Clerk stub). Fail-closed: required.
    pub dev_token: String,
    /// CoreLink session-exchange client for `POST /v1/token` (Option B). `None` →
    /// dev-token-only; the endpoint 404s without it (presence not disclosed). The
    /// P2 Clerk identity seam: hugit forwards the Clerk JWT, CoreLink verifies it.
    pub exchange: Option<Arc<SessionExchangeClient>>,
    /// In-process engine-token store. Always present so the lookup gate compiles
    /// uniformly; stays empty until a Clerk exchange mints a token. Single-host
    /// (the multi-instance shared store is the same P2 seam as the idem ledger).
    pub token_store: Arc<TokenStore>,
    /// The per-repo git content seam: `repo slug → RepoState`. Loaded once at boot
    /// (one entry per `HUGIT_SERVE_GIT_DIR` / `HUGIT_SERVE_CAS_REPO` list member).
    /// Empty when no content seam is wired → every repo's `blob`/`edit` + git wire
    /// 404 honestly. A `{repo}` not in the map is served with NO git seam (the same
    /// honest 404 as a not-wired engine), independent of whether the repo's LOG
    /// exists — the read API still works from `source`, only the git content is
    /// absent for un-loaded repos.
    ///
    /// This is the IMMUTABLE boot set. A repo created at runtime via
    /// [`POST /v1/repos`](crate::writes::verbs::write_provision) lands in the
    /// interior-mutable [`repos_runtime`](Self::repos_runtime) overlay instead;
    /// [`repo_state`](Self::repo_state) consults both. Keeping the boot set a plain
    /// map preserves the borrow-returning `repo_state` signature the git wire path
    /// depends on (a `RwLock` guard cannot hand out a `&RepoState`).
    pub repos: std::collections::HashMap<String, RepoState>,
    /// The interior-mutable RUNTIME repo overlay (W-PROVISION): repos created via
    /// `POST /v1/repos` after boot, served with NO redeploy. Disjoint from
    /// [`repos`](Self::repos) (insert refuses a slug already present in either);
    /// [`repo_state`](Self::repo_state)/[`me_repo_logs`](Self::me_repo_logs)/
    /// [`git_serving_count`](Self::git_serving_count) union the two.
    ///
    /// The engine is SINGLE-THREADED + single-instance, so the `RwLock` is
    /// uncontended; it exists only to grant interior mutability behind the shared
    /// `&AppState` the request handlers hold (the same reason [`LiveRefs`] uses one).
    /// Entries are `&'static RepoState` (a leaked `Box`): a provisioned forge repo is
    /// PERMANENT for the process lifetime (there is no runtime de-provision in v0),
    /// so leaking is semantically exact — and it lets `repo_state` return a
    /// `&RepoState` (the leaked ref outlives every borrow) WITHOUT changing the
    /// signature the do-not-touch git wire path relies on. Bounded by the number of
    /// provisions in one engine lifetime (rare, human-driven). On reboot a
    /// provisioned repo's LOG re-loads from `source` (durable) but its git seam is
    /// gone until the `HUGIT_SERVE_CAS_REPO` list is updated — the runtime-repo-set
    /// PERSISTENCE is the owner/infra-gated follow-up named in the frozen contract.
    pub repos_runtime: Arc<RwLock<std::collections::HashMap<String, &'static RepoState>>>,
    /// The CAS-mode provisioning template (`Some` only when the engine booted in CAS
    /// mode — `HUGIT_SERVE_CAS_URL` set): the handles `POST /v1/repos` mints a new
    /// empty repo's git seam from (so it is push/clone-live in the same op). `None`
    /// in Local/dev mode → a provisioned repo's LOG is still created + readable, but
    /// it gets NO git seam (push/clone need CAS). Never a request field — derived
    /// from the boot env only.
    pub provision: Option<ProvisionTemplate>,
    /// Whether the receive-pack (git `push`) write path is enabled — the
    /// `self-hosted-alpha` deploy gate. Default **false** (push → 403), so a stock
    /// deploy never accepts a write. Set by `HUGIT_SERVE_RECEIVE_PACK=1` at boot
    /// (and only takes effect where a repo also has a `git_dir` write seam).
    pub write_path_enabled: bool,
    /// Break-glass: whether a Bearer that matches [`dev_token`](Self::dev_token) may
    /// be derived into the platform OPERATOR principal (`orchestrator:hugit`, sees
    /// all). **The PUBLIC prod deploy OMITS this flag → the dev-token confers ZERO
    /// elevation** (a matching Bearer degrades to an anonymous visitor — public
    /// reads only, nothing private/operator-gated). It is turned ON ONLY as a
    /// documented ops/bootstrap break-glass via `HUGIT_ALLOW_DEV_OPERATOR=1`.
    ///
    /// A real Clerk-minted session token (`clerk:{org}:{user}`, Tier-1) can NEVER
    /// become the operator regardless of this flag — the god-path this gates is the
    /// dev-token derivation ONLY (Tier-2). This is the go-live guarantee: a real
    /// user never holds a god-token.
    ///
    /// Default in [`from_env`](Self::from_env) (the PROD boot) is **false**
    /// (fail-closed). The explicit non-env dev/test constructor
    /// [`new`](Self::new) defaults it **true** (it is a dev/seed break-glass by
    /// nature; production never calls it) — a test that must exercise the
    /// no-god-path behavior flips the field to `false` directly.
    pub allow_dev_operator: bool,
}

/// The hard cap on repos a single tenant may hold in ONE engine lifetime (the boot
/// [`repos`](AppState::repos) set ∪ the runtime overlay). Generous for a real human
/// or org, but a HARD bound on an authenticated create-spam DoS: every successful
/// `POST /v1/repos` permanently leaks a `&'static RepoState`
/// ([`insert_runtime_repo`](AppState::insert_runtime_repo), never freed) AND writes a
/// durable genesis object, so an uncapped create-loop by ONE tenant would OOM the
/// single-instance, single-threaded engine → a full outage. Enforced in the provision
/// path (see [`crate::writes::verbs::write_provision`]) via
/// [`count_owned_repos`](AppState::count_owned_repos) BEFORE the leak + the durable
/// write. The `repos_runtime` doc note ("bounded by rare human-driven provisions") is
/// now MECHANICALLY enforced by this cap, not merely assumed.
pub const MAX_REPOS_PER_TENANT: usize = 100;

impl AppState {
    /// The receive-pack flag gate for `hugit_proto::receive_pack` — `self-hosted-alpha`
    /// ON iff [`write_path_enabled`](Self::write_path_enabled).
    #[must_use]
    pub fn receive_flag_gate(&self) -> hugit_proto::write::flag::FlagGate {
        if self.write_path_enabled {
            hugit_proto::write::flag::FlagGate::self_hosted_alpha()
        } else {
            hugit_proto::write::flag::FlagGate::new()
        }
    }

    /// Enable the receive-pack write path (test seed / explicit opt-in).
    pub fn enable_write_path(&mut self) {
        self.write_path_enabled = true;
    }
    /// Build from env. `HUGIT_ENGINE_DEV_TOKEN` is always required (fail-closed).
    /// If `HUGIT_SERVE_R2_ACCOUNT_ID` is set → the R2 source (all `R2_*` required);
    /// else the Local source (`HUGIT_SERVE_LOG_DIR` required).
    pub fn from_env() -> Result<Self, String> {
        let dev_token = std::env::var("HUGIT_ENGINE_DEV_TOKEN").map_err(|_| {
            "HUGIT_ENGINE_DEV_TOKEN is not set (fail-closed: refusing to start without an auth token)"
                .to_string()
        })?;
        if dev_token.trim().is_empty() {
            return Err("HUGIT_ENGINE_DEV_TOKEN is empty (fail-closed)".to_string());
        }

        // R2 is selected by EITHER the native ACCOUNT_ID or the S3-standard ENDPOINT
        // (so a standard cred file, which carries _ENDPOINT not _ACCOUNT_ID, selects R2).
        let r2_selected = std::env::var("HUGIT_SERVE_R2_ACCOUNT_ID").is_ok()
            || std::env::var("HUGIT_SERVE_R2_ENDPOINT").is_ok();
        let source = if r2_selected {
            Self::r2_from_env()?
        } else {
            let dir = std::env::var("HUGIT_SERVE_LOG_DIR")
                .map_err(|_| "HUGIT_SERVE_LOG_DIR is not set".to_string())?;
            LogSource::Local {
                dir: PathBuf::from(dir),
            }
        };
        let exchange = SessionExchangeConfig::from_env()?.map(|cfg| Arc::new(cfg.into_client()));
        let token_store = Arc::new(TokenStore::new());

        // The per-repo git content seam (`blob`/`edit` reads + the clone/fetch
        // wire), one entry per loaded repo. MULTI-REPO: both source vars accept a
        // comma-separated SET of repos (one entry = single-repo, unchanged).
        // Source precedence (F): `HUGIT_SERVE_CAS_URL` set → the live CoreLink CAS
        // (mutable refs/oid-index manifests from hugit's R2, immutable objects from
        // the CAS), one `HUGIT_SERVE_CAS_REPO` slug per list member; else
        // `HUGIT_SERVE_GIT_DIR` → one local git dir per list member; else no content
        // seam → the content reads 404 honestly (NOT a fake blank file). Loaded once
        // at boot. Fail-closed: a content seam configured but loading NO repo is a
        // fatal boot error (a misconfigured seam refuses to start); an UN-configured
        // seam (neither var set) is the honest no-git default (empty map).
        let repos = Self::load_repos_from_env()?;
        // The runtime provisioning template (W-PROVISION): the CAS handles a
        // `POST /v1/repos` mints a new repo's git seam from. `Some` only in CAS mode.
        let provision = Self::provision_template_from_env()?;

        let write_path_enabled = receive_pack_enabled();

        // Fail-closed multi-instance guard (WP-IFMATCH): a >1-instance deploy that
        // mutates the shared manifests is safe ONLY with the conditional manifest-PUT
        // path active. Refuse to boot otherwise (mechanized single-writer invariant).
        multi_instance_guard(
            write_path_enabled,
            allow_multi_instance(),
            CONDITIONAL_MANIFEST_PUT_ACTIVE,
        )?;
        if allow_multi_instance() && write_path_enabled {
            eprintln!(
                "[hugit-serve] multi-instance permitted: the conditional (If-Match) \
                 manifest-PUT path is active — concurrent refs.json/oid-index.json writes \
                 are compare-and-swap guarded (WP-IFMATCH)."
            );
        }

        Ok(Self {
            source,
            dev_token,
            exchange,
            token_store,
            repos,
            repos_runtime: Arc::new(RwLock::new(std::collections::HashMap::new())),
            provision,
            write_path_enabled,
            // PROD default: OFF. Without `HUGIT_ALLOW_DEV_OPERATOR=1` the dev-token
            // is NOT a god-token — the public door has zero operator god-path.
            allow_dev_operator: dev_operator_allowed(),
        })
    }

    /// Build the CAS-mode provisioning template from env (W-PROVISION): the CAS
    /// client + tenant + R2 manifest store a `POST /v1/repos` mints a new repo's git
    /// seam from. `Ok(None)` when the engine is NOT in CAS mode (`HUGIT_SERVE_CAS_URL`
    /// unset/empty) — provisioning then still creates the LOG (readable) but no git
    /// seam. Reuses the SAME env the CAS-mode boot load reads; builds no network
    /// connection (pure config). Fail-closed: CAS mode configured but a missing
    /// tenant / R2 manifest cred is a fatal boot error (a half-configured provisioner
    /// refuses to start rather than silently disabling create).
    fn provision_template_from_env() -> Result<Option<ProvisionTemplate>, String> {
        let cas_selected = std::env::var("HUGIT_SERVE_CAS_URL")
            .ok()
            .is_some_and(|v| !v.trim().is_empty());
        if !cas_selected {
            return Ok(None);
        }
        let cas_client = crate::cas::CasClient::from_env()
            .map_err(|e| format!("provision template: CAS client not configured: {e}"))?;
        let tenant = std::env::var("HUGIT_SERVE_CAS_TENANT_ID").map_err(|_| {
            "HUGIT_SERVE_CAS_TENANT_ID is not set (CAS mode; provisioning needs it)".to_string()
        })?;
        let r2 = R2Config::from_env()
            .map_err(|e| format!("provision template needs the R2 manifest store: {e}"))?;
        Ok(Some(ProvisionTemplate {
            cas_client,
            tenant,
            r2,
            receive_pack_enabled: receive_pack_enabled(),
        }))
    }

    /// Build an EMPTY CAS-mode [`RepoState`] for a freshly provisioned repo — the
    /// git seam that makes the new repo push/clone-live in the same op (W-PROVISION).
    /// `None` when the engine has no [`ProvisionTemplate`] (Local/dev mode, no CAS):
    /// the caller then creates the LOG only (readable), no git seam. The state has NO
    /// objects and NO refs yet (the empty-tree root, an empty ref map, an empty live
    /// oid-index) — it gains content on the first `git push`, which the
    /// [`CasWriteSeam`] (present iff receive-pack is enabled) accepts and finalizes to
    /// the CAS + R2 manifests, hot-swapping the new tip into `git_refs`.
    #[must_use]
    pub fn build_empty_cas_repo_state(&self, slug: &str) -> Option<RepoState> {
        let t = self.provision.as_ref()?;
        // A lazy CAS source over an EMPTY index: reads resolve nothing until the first
        // push merges oid→blake3 entries into the SAME live index the source reads.
        let cas_src = crate::cas::LazyCasObjectSource::new(BTreeMap::new(), t.cas_client.clone());
        let live_oid_index = cas_src.live_index_handle();
        let git_source: Arc<dyn hugit_proto::ObjectSource + Send + Sync> = Arc::new(cas_src);
        let git_root_tree = gix_hash::ObjectId::from_hex(EMPTY_TREE_OID_HEX.as_bytes())
            .expect("the well-known empty-tree oid is valid hex");
        let cas_write = t.receive_pack_enabled.then(|| CasWriteSeam {
            cas_client: t.cas_client.clone(),
            tenant: t.tenant.clone(),
            repo_slug: slug.to_string(),
            r2: t.r2.clone(),
        });
        Some(RepoState {
            git_source,
            git_root_tree,
            git_refs: LiveRefs::new(BTreeMap::new()),
            git_dir: None,
            cas_write,
            live_oid_index: Some(live_oid_index),
        })
    }

    /// Insert a freshly provisioned repo's git seam into the interior-mutable RUNTIME
    /// overlay so it is served (read + receive-pack + clone) with NO engine reboot
    /// (W-PROVISION). Refuses a slug already present in EITHER the boot
    /// [`repos`](Self::repos) set or the runtime overlay (fail-closed: never clobber a
    /// loaded repo's seam) → [`EngineErr::cas_conflict`] (the caller maps it to a 409).
    ///
    /// The [`RepoState`] is leaked to `&'static` (a provisioned repo is permanent for
    /// the process; see [`repos_runtime`](Self::repos_runtime)) so
    /// [`repo_state`](Self::repo_state) can hand out a `&RepoState`.
    ///
    /// # Errors
    /// `409 CAS_CONFLICT` — the slug already has a loaded seam.
    pub fn insert_runtime_repo(&self, slug: &str, repo: RepoState) -> Result<(), EngineErr> {
        if self.repos.contains_key(slug) {
            return Err(EngineErr::cas_conflict());
        }
        let mut guard = self
            .repos_runtime
            .write()
            .unwrap_or_else(|e| e.into_inner());
        if guard.contains_key(slug) {
            return Err(EngineErr::cas_conflict());
        }
        // Leak: a provisioned forge repo is served for the engine's whole lifetime
        // (no runtime de-provision in v0), so the box is never freed — this is exact,
        // not a mistake, and is what lets `repo_state` return a `&RepoState`.
        let leaked: &'static RepoState = Box::leak(Box::new(repo));
        guard.insert(slug.to_string(), leaked);
        Ok(())
    }

    /// Whether `slug` already names a loaded repo (boot OR runtime) — the fast,
    /// in-memory duplicate pre-check for provisioning (the authoritative no-clobber
    /// guard is the create-only [`CasToken::Absent`] genesis persist).
    #[must_use]
    pub fn has_repo_seam(&self, slug: &str) -> bool {
        self.repos.contains_key(slug)
            || self
                .repos_runtime
                .read()
                .unwrap_or_else(|e| e.into_inner())
                .contains_key(slug)
    }

    /// Load the per-repo git content seam set from env. Returns an empty map when
    /// no content seam is configured (the honest no-git default). Fail-closed: a
    /// CONFIGURED seam (either var set, non-empty) that loads no repo, or any
    /// per-repo load error, is a fatal boot error.
    fn load_repos_from_env() -> Result<std::collections::HashMap<String, RepoState>, String> {
        let mut repos = std::collections::HashMap::new();

        let cas_repos = std::env::var("HUGIT_SERVE_CAS_URL")
            .ok()
            .filter(|v| !v.trim().is_empty())
            .and(
                std::env::var("HUGIT_SERVE_CAS_REPO")
                    .ok()
                    .filter(|v| !v.trim().is_empty()),
            );
        if let Some(list) = cas_repos {
            // CAS mode: one `HUGIT_SERVE_CAS_REPO` slug per list member, all over
            // the SAME CAS client + R2 manifest store.
            let cas = crate::cas::CasClient::from_env()
                .map_err(|e| format!("HUGIT_SERVE_CAS_*: CAS client not configured: {e}"))?;
            let tenant = std::env::var("HUGIT_SERVE_CAS_TENANT_ID").map_err(|_| {
                "HUGIT_SERVE_CAS_TENANT_ID is not set (CAS source selected)".to_string()
            })?;
            let r2 = R2Config::from_env()
                .map_err(|e| format!("CAS source needs the R2 manifest store: {e}"))?;
            for repo in split_repo_list(&list) {
                if !is_safe_repo_slug(repo) {
                    return Err(format!(
                        "HUGIT_SERVE_CAS_REPO contains an unsafe repo slug: {repo:?}"
                    ));
                }
                // LAZY boot: read only the manifests + resolve HEAD's tree; objects
                // are fetched from the CAS on demand at serve time.
                let (cas_src, root, refs) =
                    crate::cas::load_manifests_from_cas(cas.clone(), &r2, &tenant, repo)?;
                // A shared handle to the lazy source's live oid→blake3 index, so a
                // successful push can merge new entries into the SAME cell it reads.
                let live_oid_index = cas_src.live_index_handle();
                let src: Arc<dyn hugit_proto::ObjectSource + Send + Sync> = Arc::new(cas_src);
                // The CAS-mode push write seam — populated ONLY when the receive-pack
                // deploy flag is on, so a stock deploy (flag unset/`0`) gets `None`:
                // identical to the prior behavior (push gated off, no write seam). The
                // flag (`write_path_enabled`) remains THE gate; this just gives an
                // enabled deploy the objects sink + manifest store a CAS push needs.
                let cas_write = if receive_pack_enabled() {
                    Some(CasWriteSeam {
                        cas_client: cas.clone(),
                        tenant: tenant.clone(),
                        repo_slug: repo.to_string(),
                        r2: r2.clone(),
                    })
                } else {
                    None
                };
                repos.insert(
                    repo.to_string(),
                    RepoState {
                        git_source: src,
                        git_root_tree: root,
                        git_refs: LiveRefs::new(refs),
                        git_dir: None, // CAS mode: no local dir; push sink is cas_write
                        cas_write,
                        live_oid_index: Some(live_oid_index),
                    },
                );
            }
            if repos.is_empty() {
                return Err("HUGIT_SERVE_CAS_REPO is empty (CAS source selected)".to_string());
            }
            return Ok(repos);
        }

        match std::env::var("HUGIT_SERVE_GIT_DIR") {
            Ok(list) if !list.trim().is_empty() => {
                // Local git-dir mode: one dir per comma-separated list member. Each
                // member is either a bare `path` (the served slug is the dir's
                // basename, e.g. `/srv/git/hugit` → `hugit`) or an explicit
                // `slug=path` (so a checkout dir whose name is not the repo slug can
                // still be served under the right name). A single bare dir = the
                // unchanged single-repo config.
                for member in split_repo_list(&list) {
                    let (slug, dir) = parse_git_dir_member(member)?;
                    let (cas, root, refs) = load_git_dir(dir)?;
                    let src: Arc<dyn hugit_proto::ObjectSource + Send + Sync> = Arc::new(cas);
                    repos.insert(
                        slug,
                        RepoState {
                            git_source: src,
                            git_root_tree: root,
                            git_refs: LiveRefs::new(refs),
                            git_dir: Some(PathBuf::from(dir)), // push writes objects+ref here
                            cas_write: None,                   // GIT_DIR mode: no CAS seam
                            // GitDir push is unchanged (durable ref via `git update-ref`,
                            // re-read on the next boot): no in-memory oid-index hot-swap.
                            live_oid_index: None,
                        },
                    );
                }
                if repos.is_empty() {
                    return Err("HUGIT_SERVE_GIT_DIR is empty".to_string());
                }
                Ok(repos)
            }
            // No content seam configured → the honest no-git default.
            _ => Ok(repos),
        }
    }

    fn r2_from_env() -> Result<LogSource, String> {
        Ok(LogSource::R2(Box::new(R2Config::from_env()?)))
    }

    /// Explicit Local constructor (tests). No git content seam (empty `repos`) —
    /// blob/edit reads + git wire 404 honestly until a repo's git seam is wired
    /// (a deploy `HUGIT_SERVE_GIT_DIR`, or [`set_repo_git`](Self::set_repo_git) in
    /// a test).
    #[must_use]
    pub fn new(log_dir: PathBuf, dev_token: String) -> Self {
        Self {
            source: LogSource::Local { dir: log_dir },
            dev_token,
            exchange: None,
            token_store: Arc::new(TokenStore::new()),
            repos: std::collections::HashMap::new(),
            repos_runtime: Arc::new(RwLock::new(std::collections::HashMap::new())),
            provision: None,
            write_path_enabled: false,
            // The explicit dev/test/seed constructor enables the break-glass by
            // default (production boots via `from_env`, which is default-OFF). A
            // test asserting the no-god-path (flag-OFF) behavior sets this to
            // `false` on the returned state.
            allow_dev_operator: true,
        }
    }

    /// Look up the per-repo git content seam for `repo`, or `None` when this repo
    /// has no git seam loaded (not in the map). Callers map `None` to the SAME
    /// honest 404/empty as a not-wired engine — never a 500, never an oracle.
    ///
    /// Consults the IMMUTABLE boot [`repos`](Self::repos) first, then the RUNTIME
    /// overlay ([`repos_runtime`](Self::repos_runtime)) — so a repo provisioned via
    /// `POST /v1/repos` is served immediately, no reboot. The runtime entry is a
    /// leaked `&'static RepoState`, so the returned borrow is valid for any lifetime
    /// (the read lock is released before this returns; nothing borrows the guard).
    #[must_use]
    pub fn repo_state(&self, repo: &str) -> Option<&RepoState> {
        if let Some(r) = self.repos.get(repo) {
            return Some(r);
        }
        // The runtime entry is `&'static`, so `.copied()` detaches it from the read
        // guard (the guard drops at end-of-fn; the returned ref does not borrow it).
        self.repos_runtime
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(repo)
            .copied()
    }

    /// The identity-scoped (`/v1/me/*`) repo set for `principal`, resolved to the
    /// authorized `(slug, verified-log)` pairs the me/* builders aggregate. This is
    /// the per-tenant repo index (W-METENANT) that CLOSES the cross-principal read
    /// exposure — the me/* views used to bind a hardcoded default repo and hand
    /// EVERY caller its data.
    ///
    /// It REUSES the SAME predicate the per-repo read gate runs
    /// ([`authz::authorize_read`](crate::authz::authorize_read)) — there is NO
    /// second visibility gate to drift:
    ///
    /// - **Operator** (`orchestrator:*`) → every loaded repo (the dev/bootstrap view).
    /// - A **tenant** (`clerk:{org}:{user}`) → each loaded repo it may
    ///   `authorize_read` (its own private repos + any public repo).
    /// - **Anonymous / unknown / malformed** principal → EMPTY. me/* is
    ///   identity-scoped: a caller with no tenant has no "my repos" (fail-closed —
    ///   never the default repo, never all public repos).
    ///
    /// FAIL-CLOSED on ambiguity: a repo in the map whose log cannot load/verify is
    /// EXCLUDED (never included by default), so no path can surface a repo the
    /// caller cannot read. Each candidate log is loaded EXACTLY ONCE (the
    /// single-thread engine must not double-fetch); the work is bounded by the
    /// loaded-repo count (small).
    #[must_use]
    pub fn me_repo_logs(&self, principal: &[String]) -> Vec<(String, EventLog)> {
        // Deterministic, sorted candidate order (stable aggregate output). Union the
        // IMMUTABLE boot set with the RUNTIME overlay so a repo provisioned via
        // `POST /v1/repos` appears in the caller's `/v1/me/*` views with no reboot.
        let mut names: Vec<String> = self.repos.keys().cloned().collect();
        {
            let runtime = self.repos_runtime.read().unwrap_or_else(|e| e.into_inner());
            for k in runtime.keys() {
                if !self.repos.contains_key(k) {
                    names.push(k.clone());
                }
            }
        }
        names.sort();
        names.dedup();

        // Operator: all loaded repos (bypass — no per-repo authz needed). An
        // unloadable log is skipped fail-closed (it cannot be aggregated anyway).
        if crate::authz::is_operator(principal) {
            return names
                .into_iter()
                .filter_map(|n| self.load_verified(&n).ok().map(|log| (n, log)))
                .collect();
        }

        // Only a well-formed TENANT principal has "my repos"; a genuinely
        // anonymous request, an unknown bearer, or a malformed principal gets NONE
        // (identity-scoped: no tenant ⇒ no repos — never all-public, never default).
        if !principal_is_tenant(principal) {
            return Vec::new();
        }

        let mut out: Vec<(String, EventLog)> = Vec::new();
        for name in names {
            // Fail-closed: an unloadable/untrusted log EXCLUDES the repo.
            let Ok(log) = self.load_verified(&name) else {
                continue;
            };
            let meta = crate::authz::project_repo_meta(&log);
            // THE SAME predicate the per-repo read gate runs — no second gate.
            if crate::authz::authorize_read(principal, &meta) {
                out.push((name, log));
            }
        }
        out
    }

    /// The per-tenant repo INDEX — the sorted slug set `principal` may see in the
    /// identity-scoped views. The authorization decision is
    /// [`me_repo_logs`](Self::me_repo_logs)'s (this is the slug projection of it),
    /// so the index and the aggregated data can never diverge.
    #[must_use]
    pub fn repos_for(&self, principal: &[String]) -> Vec<String> {
        self.me_repo_logs(principal)
            .into_iter()
            .map(|(slug, _)| slug)
            .collect()
    }

    /// Count the repos OWNED by `owner_tenant` across the authoritative in-memory
    /// set — the boot [`repos`](Self::repos) set ∪ the runtime overlay
    /// ([`repos_runtime`](Self::repos_runtime)). This is the EXACT set whose entries
    /// leak (a `&'static RepoState`, never freed) and whose genesis objects are
    /// durable, so it is the right denominator for the per-tenant DoS cap
    /// ([`MAX_REPOS_PER_TENANT`]) the provision path enforces BEFORE it leaks / writes.
    ///
    /// CHEAP: bounded by the loaded-repo count (each candidate log is loaded EXACTLY
    /// once). It NEVER enumerates the durable/R2 store — a listing scan there would
    /// itself be a DoS (the anti-pattern this cap exists to prevent).
    ///
    /// FAIL-CLOSED: returns `None` if ANY candidate log cannot be loaded/verified. An
    /// indeterminate count MUST refuse the create (never allow-by-default): an
    /// unloadable candidate is genuinely ambiguous re: whether it belongs to this
    /// tenant, so a transient read fault can never be leveraged to slip past the cap.
    #[must_use]
    pub fn count_owned_repos(&self, owner_tenant: &str) -> Option<usize> {
        // Deterministic candidate set: boot repos ∪ runtime overlay (dedup). Same
        // union `me_repo_logs` walks, but filtered by OWNERSHIP (not read-authz).
        let mut names: Vec<String> = self.repos.keys().cloned().collect();
        {
            let runtime = self.repos_runtime.read().unwrap_or_else(|e| e.into_inner());
            for k in runtime.keys() {
                if !self.repos.contains_key(k) {
                    names.push(k.clone());
                }
            }
        }
        names.sort();
        names.dedup();

        let mut count = 0usize;
        for name in names {
            // Fail-closed: an unloadable/untrusted candidate ⇒ indeterminate count.
            let log = self.load_verified(&name).ok()?;
            let meta = crate::authz::project_repo_meta(&log);
            if meta.owner_tenant.as_deref() == Some(owner_tenant) {
                count += 1;
            }
        }
        Some(count)
    }

    /// The number of repos whose git content seam is loaded (the `/readyz`
    /// capability count). Zero = git serving not live for any repo. Counts the boot
    /// set PLUS the runtime overlay (the two are disjoint by construction — insert
    /// refuses a duplicate slug).
    #[must_use]
    pub fn git_serving_count(&self) -> usize {
        self.repos.len()
            + self
                .repos_runtime
                .read()
                .unwrap_or_else(|e| e.into_inner())
                .len()
    }

    /// Wire (or replace) a repo's git content seam — the test/seed entrypoint. The
    /// live boot path populates `repos` from env; tests use this to seed a repo's
    /// git source + refs without an on-disk git dir.
    pub fn set_repo_git(
        &mut self,
        repo: impl Into<String>,
        git_source: Arc<dyn hugit_proto::ObjectSource + Send + Sync>,
        git_root_tree: gix_hash::ObjectId,
        git_refs: BTreeMap<String, String>,
    ) {
        self.repos.insert(
            repo.into(),
            RepoState {
                git_source,
                git_root_tree,
                git_refs: LiveRefs::new(git_refs),
                git_dir: None,
                cas_write: None,
                live_oid_index: None,
            },
        );
    }

    /// Wire a repo's git seam from an on-disk git dir — like the `HUGIT_SERVE_GIT_DIR`
    /// boot path but for one repo (the push test + a single-repo seed). Unlike
    /// [`set_repo_git`](Self::set_repo_git), this records the `git_dir`, so the
    /// receive-pack write path is live for it ([`RepoState::write_cas`]).
    pub fn set_repo_from_git_dir(
        &mut self,
        repo: impl Into<String>,
        dir: &str,
    ) -> Result<(), String> {
        let (cas, root, refs) = load_git_dir(dir)?;
        let src: Arc<dyn hugit_proto::ObjectSource + Send + Sync> = Arc::new(cas);
        self.repos.insert(
            repo.into(),
            RepoState {
                git_source: src,
                git_root_tree: root,
                git_refs: LiveRefs::new(refs),
                git_dir: Some(PathBuf::from(dir)),
                cas_write: None,
                live_oid_index: None,
            },
        );
        Ok(())
    }

    /// A short label of the active source (for the boot log; no secrets).
    #[must_use]
    pub fn source_label(&self) -> String {
        match &self.source {
            LogSource::Local { dir } => format!("local:{}", dir.display()),
            LogSource::R2(c) => format!("r2:{}/{}/<tenant>", c.host, c.bucket),
        }
    }

    /// Load + chain-verify a repo's event log. A non-existent log (or an unsafe
    /// slug) → 404 (no existence leak). Any parse / tamper / transport fault →
    /// 503 ENGINE_UNAVAILABLE (fail-honest — never a fake-empty VM). The verify
    /// is identical for both sources (the PS-13 chokepoint).
    pub fn load_verified(&self, repo: &str) -> Result<EventLog, EngineErr> {
        self.load_verified_with_token(repo).map(|(log, _)| log)
    }

    /// As [`load_verified`], but ALSO returns the [`CasToken`] for the head (the R2
    /// object ETag, or a local content hash) — the version the write-door's
    /// compare-and-swap persists against. The chain verify is the SAME single
    /// PS-13 chokepoint (this method IS the body; `load_verified` drops the token),
    /// so a write can never skip the verification a read performs.
    pub fn load_verified_with_token(&self, repo: &str) -> Result<(EventLog, CasToken), EngineErr> {
        if !is_safe_repo_slug(repo) {
            return Err(EngineErr::not_found());
        }
        let (bytes, label, token) = match self.source.fetch(repo)? {
            Some(b) => b,
            None => return Err(EngineErr::not_found()),
        };
        let log = hugit_cli::checks::load_event_log_from_bytes(&bytes, Path::new(&label)).map_err(
            |e| EngineErr::unavailable(format!("engine log read/verify failed ({})", e.kind())),
        )?;
        Ok((log, token))
    }
}

impl LogSource {
    /// Fetch a repo's raw event-log bytes + the head [`CasToken`] (R2 ETag, or a
    /// local content hash). `Ok(None)` = the object does not exist (→ 404, no
    /// existence leak); `Ok(Some((bytes, label, token)))` = present; `Err` = a
    /// transport/IO fault (→ 503). `label` is the source string for error context.
    fn fetch(&self, repo: &str) -> Result<Option<(Vec<u8>, String, CasToken)>, EngineErr> {
        match self {
            LogSource::Local { dir } => {
                let path = dir.join(format!("{repo}.json"));
                match std::fs::read(&path) {
                    Ok(b) => {
                        let token = CasToken::Version(content_hash(&b));
                        Ok(Some((b, path.display().to_string(), token)))
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
                    Err(e) => Err(EngineErr::unavailable(format!(
                        "local log read failed: {e}"
                    ))),
                }
            }
            LogSource::R2(c) => c.fetch(repo),
        }
    }

    /// Durably write a repo's raw event-log bytes back to the source, as a
    /// COMPARE-AND-SWAP against `expected` (the token the matching [`fetch`]
    /// returned). A concurrent head move → [`EngineErr::cas_conflict`] (the
    /// write-door reloads + retries), NEVER last-writer-wins.
    /// - **Local**: re-read + content-hash compare, then atomic temp-write + rename.
    /// - **R2**: a conditional signed PUT (`If-Match`/`If-None-Match`) — REQUIRES a
    ///   write-scoped credential; the standing engine cred is read-only by design,
    ///   so an R2 write fail-honestly returns 503 until the scoped RW grant is wired.
    fn persist(&self, repo: &str, bytes: &[u8], expected: &CasToken) -> Result<(), EngineErr> {
        match self {
            LogSource::Local { dir } => {
                std::fs::create_dir_all(dir).map_err(|e| {
                    EngineErr::unavailable(format!("local log dir create failed: {e}"))
                })?;
                let path = dir.join(format!("{repo}.json"));
                // CAS check: the on-disk head must still equal `expected` (a local
                // analogue of R2 `If-Match`). A residual TOCTOU remains between this
                // compare and the rename below — acceptable because Local is the
                // single-box dev/test source; the production multi-writer source is
                // R2, whose conditional PUT is atomic at the store.
                let current = match std::fs::read(&path) {
                    Ok(b) => CasToken::Version(content_hash(&b)),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => CasToken::Absent,
                    Err(e) => {
                        return Err(EngineErr::unavailable(format!(
                            "local log re-read failed: {e}"
                        )));
                    }
                };
                if !cas_matches(expected, &current) {
                    return Err(EngineErr::cas_conflict());
                }
                // Atomic: write a unique temp then rename over the target, so a
                // crash mid-write never leaves a torn `<repo>.json`.
                let tmp = dir.join(format!("{repo}.json.tmp.{}", std::process::id()));
                std::fs::write(&tmp, bytes)
                    .map_err(|e| EngineErr::unavailable(format!("local log write failed: {e}")))?;
                std::fs::rename(&tmp, &path).map_err(|e| {
                    let _ = std::fs::remove_file(&tmp);
                    EngineErr::unavailable(format!("local log rename failed: {e}"))
                })
            }
            LogSource::R2(c) => {
                // FAIL-CLOSED (audit hardening): an `Unsupported` token on the R2
                // write path means `fetch` got no ETag for an existing object — a
                // PUT would then be UNCONDITIONAL (last-writer-wins), silently
                // bypassing the CAS. R2 always returns an ETag, so this is
                // defense-in-depth: refuse the non-CAS write rather than degrade
                // silently. (The snapshot uploader's intentional unconditional
                // create uses `put`, never this engine write path.)
                if matches!(expected, CasToken::Unsupported) {
                    return Err(EngineErr::unavailable(
                        "engine storage returned no version token; refusing a non-CAS write",
                    ));
                }
                c.put_conditional(repo, bytes, expected).map(|_| ())
            }
        }
    }
}

/// The content-hash version of a raw log object — the local CAS token (a stand-in
/// for the R2 ETag). Hex SHA-256 of the exact bytes.
fn content_hash(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(bytes);
    hex::encode(h.finalize())
}

/// Whether the head a write swaps against (`expected`) still equals what is durably
/// present now (`current`). `Unsupported` always matches (the sink opts out of CAS).
fn cas_matches(expected: &CasToken, current: &CasToken) -> bool {
    match (expected, current) {
        (CasToken::Unsupported, _) => true,
        (CasToken::Absent, CasToken::Absent) => true,
        (CasToken::Version(a), CasToken::Version(b)) => a == b,
        _ => false,
    }
}

impl crate::writes::LogSink for AppState {
    /// Load + chain-verify (same gate as the read path; absent → 404) AND capture
    /// the head [`CasToken`] for the write-door's compare-and-swap.
    fn load(&self, repo: &str) -> Result<(EventLog, CasToken), EngineErr> {
        self.load_verified_with_token(repo)
    }

    /// Serialize the mutated log + durably persist it back to the source as a
    /// COMPARE-AND-SWAP against `expected` (the head this request loaded). A
    /// concurrent head move → [`EngineErr::cas_conflict`], which `with_write`
    /// catches to reload + retry — so a multi-writer / R2 deployment never drops a
    /// concurrent request's records (the load→persist gap holds no lock; the CAS,
    /// not a lock, is what makes the cycle safe).
    fn persist(&self, repo: &str, log: &EventLog, expected: &CasToken) -> Result<(), EngineErr> {
        if !is_safe_repo_slug(repo) {
            return Err(EngineErr::not_found());
        }
        let bytes = serde_json::to_vec(log.records())
            .map_err(|e| EngineErr::unavailable(format!("log serialize failed: {e}")))?;
        self.source.persist(repo, &bytes, expected)
    }
}

impl R2Config {
    /// Build from the `HUGIT_SERVE_R2_*` env (same vars the read server uses).
    /// `REGION` defaults to `auto` (R2). For the snapshot uploader the `KEY_ID`/
    /// `SECRET` are the one-shot READ+WRITE grant; for the server they are the
    /// standing read-only cred — same shape, different scope.
    pub fn from_env() -> Result<Self, String> {
        Self::from_vars(|k| std::env::var(k).ok())
    }

    /// Pure core of [`from_env`] — resolves the config from a `get(name)` lookup so
    /// it is testable without mutating global process env.
    ///
    /// Accepts BOTH the engine-native names AND the S3-standard names a CoreLink/AWS
    /// credential file ships with, so such a file can be `source`d verbatim (only
    /// `HUGIT_SERVE_R2_TENANT_ID` must be supplied separately — it is not part of a
    /// generic cred):
    /// - host: `HUGIT_SERVE_R2_ACCOUNT_ID` (native) **or** `…_ENDPOINT` (S3-standard).
    /// - key:  `HUGIT_SERVE_R2_KEY_ID` (native) **or** `…_ACCESS_KEY_ID`.
    /// - secret: `HUGIT_SERVE_R2_SECRET` (native) **or** `…_SECRET_ACCESS_KEY`.
    fn from_vars(get: impl Fn(&str) -> Option<String>) -> Result<Self, String> {
        let req = |k: &str| get(k).ok_or_else(|| format!("{k} is not set (R2 source selected)"));
        let either = |primary: &str, alias: &str| get(primary).or_else(|| get(alias));

        // Host from the native ACCOUNT_ID, else parsed from the S3-standard ENDPOINT.
        let (endpoint, host) = match get("HUGIT_SERVE_R2_ACCOUNT_ID") {
            Some(account_id) => {
                let host = format!("{account_id}.r2.cloudflarestorage.com");
                (format!("https://{host}"), host)
            }
            None => {
                let endpoint = get("HUGIT_SERVE_R2_ENDPOINT").ok_or_else(|| {
                    "neither HUGIT_SERVE_R2_ACCOUNT_ID nor HUGIT_SERVE_R2_ENDPOINT is set \
                     (R2 source selected)"
                        .to_string()
                })?;
                let host = endpoint
                    .trim_start_matches("https://")
                    .trim_start_matches("http://")
                    .split('/')
                    .next()
                    .unwrap_or("")
                    .to_string();
                if host.is_empty() {
                    return Err(format!("HUGIT_SERVE_R2_ENDPOINT is malformed: {endpoint}"));
                }
                (endpoint, host)
            }
        };
        let key_id =
            either("HUGIT_SERVE_R2_KEY_ID", "HUGIT_SERVE_R2_ACCESS_KEY_ID").ok_or_else(|| {
                "HUGIT_SERVE_R2_KEY_ID (or _ACCESS_KEY_ID) is not set (R2 source selected)"
                    .to_string()
            })?;
        let secret = either("HUGIT_SERVE_R2_SECRET", "HUGIT_SERVE_R2_SECRET_ACCESS_KEY")
            .ok_or_else(|| {
                "HUGIT_SERVE_R2_SECRET (or _SECRET_ACCESS_KEY) is not set (R2 source selected)"
                    .to_string()
            })?;
        let agent = ureq::AgentBuilder::new()
            .timeout(Duration::from_secs(30))
            .build();
        Ok(R2Config {
            endpoint,
            host,
            bucket: req("HUGIT_SERVE_R2_BUCKET")?,
            region: get("HUGIT_SERVE_R2_REGION").unwrap_or_else(|| "auto".to_string()),
            key_id,
            secret,
            tenant_id: req("HUGIT_SERVE_R2_TENANT_ID")?,
            agent,
        })
    }

    /// Fetch the raw object + head [`CasToken`] (the GET ETag). `pub` so the live
    /// R2 CAS round-trip proof (`tests/r2_cas_live.rs`, `#[ignore]`) can exercise
    /// the real fetch→put_conditional path.
    pub fn fetch(&self, repo: &str) -> Result<Option<(Vec<u8>, String, CasToken)>, EngineErr> {
        let key = format!("{}/{repo}.json", self.tenant_id);
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let signed = sigv4::sign_s3_get(
            &self.host,
            &self.bucket,
            &key,
            &self.key_id,
            &self.secret,
            &self.region,
            now,
        );
        // Path-style URL; tenant (UUID) + repo (validated slug) + ".json" are all
        // RFC-3986-unreserved, so the wire path equals the signed canonical URI.
        let url = format!("{}/{}/{key}", self.endpoint, self.bucket);
        let label = format!("r2://{}/{key}", self.bucket);
        let resp = self
            .agent
            .get(&url)
            .set("Authorization", &signed.authorization)
            .set("x-amz-date", &signed.amz_date)
            .set("x-amz-content-sha256", &signed.content_sha256)
            .call();
        match resp {
            Ok(r) => {
                // Capture the ETag (the CAS version) BEFORE consuming the body. R2
                // returns it quoted (e.g. `"abc…"`); preserve it verbatim so the
                // `If-Match` we send back round-trips byte-identically. A response
                // without an ETag (should not happen for a real object) opts out of
                // CAS for this head rather than failing the read.
                let token = match r.header("etag") {
                    Some(e) if !e.is_empty() => CasToken::Version(e.to_string()),
                    _ => CasToken::Unsupported,
                };
                let mut buf = Vec::new();
                r.into_reader().read_to_end(&mut buf).map_err(|e| {
                    // Detail (which carries the bucket/key `label`) to the SERVER log
                    // only — never echo storage internals to the public client.
                    eprintln!("hugit-serve: R2 body read failed for {label}: {e}");
                    EngineErr::unavailable("engine storage read failed".to_string())
                })?;
                Ok(Some((buf, label, token)))
            }
            Err(ureq::Error::Status(404, _)) => Ok(None),
            // A non-404 status OR a transport fault. `ureq`'s error Display embeds the
            // request URL (R2 host + bucket + tenant + key); on the PUBLIC read path
            // that would leak the storage topology in a 503 body. Log the specifics
            // server-side; return a GENERIC reason to the client (no existence/topology
            // oracle). Still fail-honest (503), never a fake-empty VM.
            Err(e) => {
                eprintln!("hugit-serve: R2 GET failed for {label}: {e}");
                Err(EngineErr::unavailable(
                    "engine storage temporarily unavailable".to_string(),
                ))
            }
        }
    }

    /// Fetch an ARBITRARY R2 object by its full key (no `<repo>.json` shaping) —
    /// the small generic GET the git-from-CAS loader needs for the mutable
    /// `<tenant>/<repo>/refs.json` + `<tenant>/<repo>/oid-index.json` objects.
    /// `Ok(None)` = absent (404); `Ok(Some(bytes))` = present; `Err` = a
    /// transport/non-404 fault. Mirrors [`fetch`]'s SigV4 + generic-503 discipline
    /// (no storage-topology leak to the client; specifics to the server log).
    pub fn get_object(&self, key: &str) -> Result<Option<Vec<u8>>, EngineErr> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let signed = sigv4::sign_s3_get(
            &self.host,
            &self.bucket,
            key,
            &self.key_id,
            &self.secret,
            &self.region,
            now,
        );
        let url = format!("{}/{}/{key}", self.endpoint, self.bucket);
        let label = format!("r2://{}/{key}", self.bucket);
        let resp = self
            .agent
            .get(&url)
            .set("Authorization", &signed.authorization)
            .set("x-amz-date", &signed.amz_date)
            .set("x-amz-content-sha256", &signed.content_sha256)
            .call();
        match resp {
            Ok(r) => {
                let mut buf = Vec::new();
                r.into_reader().read_to_end(&mut buf).map_err(|e| {
                    eprintln!("hugit-serve: R2 body read failed for {label}: {e}");
                    EngineErr::unavailable("engine storage read failed".to_string())
                })?;
                Ok(Some(buf))
            }
            Err(ureq::Error::Status(404, _)) => Ok(None),
            Err(e) => {
                eprintln!("hugit-serve: R2 GET failed for {label}: {e}");
                Err(EngineErr::unavailable(
                    "engine storage temporarily unavailable".to_string(),
                ))
            }
        }
    }

    /// As [`get_object`](Self::get_object), but ALSO captures the object's R2 ETag as
    /// a [`CasToken`] — the version a conditional (`If-Match`) manifest PUT swaps
    /// against (WP-IFMATCH). `Ok(None)` = absent (→ a create, `If-None-Match: *`). A
    /// present object whose GET response carries no ETag yields
    /// [`CasToken::Unsupported`]; the conditional write path treats that FAIL-CLOSED
    /// (it never degrades to an unconditional PUT). Mirrors [`fetch`](Self::fetch)'s
    /// SigV4 + generic-503 discipline (no storage-topology leak to the client).
    pub fn get_object_etag(&self, key: &str) -> Result<Option<(Vec<u8>, CasToken)>, EngineErr> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let signed = sigv4::sign_s3_get(
            &self.host,
            &self.bucket,
            key,
            &self.key_id,
            &self.secret,
            &self.region,
            now,
        );
        let url = format!("{}/{}/{key}", self.endpoint, self.bucket);
        let label = format!("r2://{}/{key}", self.bucket);
        let resp = self
            .agent
            .get(&url)
            .set("Authorization", &signed.authorization)
            .set("x-amz-date", &signed.amz_date)
            .set("x-amz-content-sha256", &signed.content_sha256)
            .call();
        match resp {
            Ok(r) => {
                // Capture the ETag (the CAS version) BEFORE consuming the body — R2
                // returns it quoted; preserve it verbatim so the `If-Match` we send
                // back round-trips byte-identically. No ETag ⇒ `Unsupported` (the
                // conditional writer refuses to degrade to an unconditional PUT).
                let token = match r.header("etag") {
                    Some(e) if !e.is_empty() => CasToken::Version(e.to_string()),
                    _ => CasToken::Unsupported,
                };
                let mut buf = Vec::new();
                r.into_reader().read_to_end(&mut buf).map_err(|e| {
                    eprintln!("hugit-serve: R2 body read failed for {label}: {e}");
                    EngineErr::unavailable("engine storage read failed".to_string())
                })?;
                Ok(Some((buf, token)))
            }
            Err(ureq::Error::Status(404, _)) => Ok(None),
            Err(e) => {
                eprintln!("hugit-serve: R2 GET (etag) failed for {label}: {e}");
                Err(EngineErr::unavailable(
                    "engine storage temporarily unavailable".to_string(),
                ))
            }
        }
    }

    /// PUT `body` to `<tenant_id>/<repo>.json` UNCONDITIONALLY (the one-shot
    /// snapshot upload — used by the `hugit-snapshot` bin, NOT the engine write
    /// path). The standing engine credential is read-only by design (a PUT 403s);
    /// this path is reached only with a read+WRITE credential. Returns the wire key.
    ///
    /// ⚠️ Unconditional by intent: this OVERWRITES the whole object (initial seeding).
    /// It deliberately does NOT compare-and-swap, so running it against a repo that is
    /// taking LIVE engine writes can clobber them — it is a seeding/operator tool, not
    /// a steady-state writer. Steady-state writes go through `LogSink::persist` (CAS).
    pub fn put(&self, repo: &str, body: &[u8]) -> Result<String, EngineErr> {
        self.put_conditional(repo, body, &CasToken::Unsupported)
    }

    /// PUT `body` as a COMPARE-AND-SWAP against `expected` (the head the matching
    /// [`fetch`] returned), via the S3/R2 conditional headers:
    /// - [`CasToken::Version(etag)`] → `If-Match: <etag>` (overwrite only if unchanged),
    /// - [`CasToken::Absent`] → `If-None-Match: *` (create only if still absent),
    /// - [`CasToken::Unsupported`] → no conditional header (unconditional PUT).
    ///
    /// R2 returns **412 Precondition Failed** when the precondition is not met
    /// (verified against the Cloudflare S3-compat docs); that maps to
    /// [`EngineErr::cas_conflict`] — the write-door's reload-and-retry signal. The
    /// conditional header is a standard (non-`x-amz`) HTTP header, so per the SigV4
    /// spec it need not be in `SignedHeaders`; it is sent UNSIGNED (keeping the
    /// proven signer untouched) and the live round-trip proves R2 honors it.
    /// A 2xx is success; anything else is an explicit error (never a silent partial).
    ///
    /// THREAT-MODEL NOTE (audit): because `If-Match` is unsigned, an on-path attacker
    /// who can rewrite the request (a MITM or a malicious/buggy intermediary) could
    /// STRIP it, downgrading the CAS to an unconditional overwrite. This is bounded by
    /// the fact that the engine talks to R2 over TLS DIRECTLY (no intermediary), so it
    /// is not exploitable in the deployed topology — it is NOT a general "tamper-proof"
    /// guarantee. Signing the header (adding it to `SignedHeaders`) would close even
    /// the intermediary case; deferred as the threat is out of the direct-TLS model.
    pub fn put_conditional(
        &self,
        repo: &str,
        body: &[u8],
        expected: &CasToken,
    ) -> Result<String, EngineErr> {
        let key = format!("{}/{repo}.json", self.tenant_id);
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let signed = sigv4::sign_s3_put(
            &self.host,
            &self.bucket,
            &key,
            body,
            &self.key_id,
            &self.secret,
            &self.region,
            now,
        );
        let url = format!("{}/{}/{key}", self.endpoint, self.bucket);
        let mut req = self
            .agent
            .put(&url)
            .set("Authorization", &signed.authorization)
            .set("x-amz-date", &signed.amz_date)
            .set("x-amz-content-sha256", &signed.content_sha256);
        // The conditional header that turns this PUT into a compare-and-swap.
        match expected {
            CasToken::Version(etag) => req = req.set("If-Match", etag),
            CasToken::Absent => req = req.set("If-None-Match", "*"),
            CasToken::Unsupported => {}
        }
        let resp = req.send_bytes(body);
        match resp {
            Ok(r) if (200..300).contains(&r.status()) => Ok(format!("r2://{}/{key}", self.bucket)),
            Ok(r) => {
                eprintln!("[hugit-serve] R2 PUT unexpected status {}", r.status());
                Err(EngineErr::unavailable("engine storage write unavailable"))
            }
            // The precondition failed: a concurrent writer moved the head. This is
            // the CAS-conflict retry signal, NOT a hard failure.
            Err(ureq::Error::Status(412, _)) => Err(EngineErr::cas_conflict()),
            Err(ureq::Error::Status(403, r)) => {
                eprintln!(
                    "[hugit-serve] R2 PUT 403 — credential is not write-scoped: {:?}",
                    r.status()
                );
                Err(EngineErr::unavailable("engine storage write unavailable"))
            }
            Err(ureq::Error::Status(s, _)) => {
                eprintln!("[hugit-serve] R2 PUT status {s}");
                Err(EngineErr::unavailable("engine storage write unavailable"))
            }
            Err(e) => {
                eprintln!("[hugit-serve] R2 PUT transport error: {e}");
                Err(EngineErr::unavailable("engine storage write unavailable"))
            }
        }
    }

    /// A conditional (compare-and-swap) PUT to an ARBITRARY R2 key — the manifest
    /// counterpart of [`put_conditional`](Self::put_conditional) (which shapes
    /// `<tenant>/<repo>.json`) used by the receive-pack finalize for
    /// `refs.json`/`oid-index.json` (WP-IFMATCH). It sends `If-Match: <etag>` (for
    /// [`CasToken::Version`]) or `If-None-Match: *` (for [`CasToken::Absent`], a
    /// create-only); the store's **412** maps to the distinct, retryable
    /// [`crate::cas::ManifestPutError::Precondition`]. Returns the NEW ETag on success
    /// (or `Unsupported` if the PUT response carried none).
    ///
    /// FAIL-CLOSED: an `Unsupported` EXPECTED token is REFUSED (never an unconditional
    /// PUT — that would re-open the lost-update race). Reuses the SAME SigV4 signer +
    /// unsigned-conditional-header approach as [`put_conditional`](Self::put_conditional)
    /// (the same direct-TLS threat model applies; see its note).
    pub fn conditional_object_put(
        &self,
        key: &str,
        body: &[u8],
        expected: &CasToken,
    ) -> Result<CasToken, crate::cas::ManifestPutError> {
        use crate::cas::ManifestPutError;
        // Fail-closed: refuse a non-CAS (unconditional) write outright.
        if matches!(expected, CasToken::Unsupported) {
            return Err(ManifestPutError::Other(
                "manifest storage returned no version token; refusing a non-CAS write".to_string(),
            ));
        }
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let signed = sigv4::sign_s3_put(
            &self.host,
            &self.bucket,
            key,
            body,
            &self.key_id,
            &self.secret,
            &self.region,
            now,
        );
        let url = format!("{}/{}/{key}", self.endpoint, self.bucket);
        let mut req = self
            .agent
            .put(&url)
            .set("Authorization", &signed.authorization)
            .set("x-amz-date", &signed.amz_date)
            .set("x-amz-content-sha256", &signed.content_sha256);
        match expected {
            CasToken::Version(etag) => req = req.set("If-Match", etag),
            CasToken::Absent => req = req.set("If-None-Match", "*"),
            // Refused above — this arm is unreachable but kept explicit (no silent
            // unconditional PUT).
            CasToken::Unsupported => {
                return Err(ManifestPutError::Other(
                    "refusing an unconditional manifest PUT".to_string(),
                ));
            }
        }
        let resp = req.send_bytes(body);
        match resp {
            Ok(r) if (200..300).contains(&r.status()) => {
                let new = match r.header("etag") {
                    Some(e) if !e.is_empty() => CasToken::Version(e.to_string()),
                    _ => CasToken::Unsupported,
                };
                Ok(new)
            }
            Ok(r) => {
                eprintln!(
                    "[hugit-serve] R2 conditional manifest PUT unexpected status {}",
                    r.status()
                );
                Err(ManifestPutError::Other(
                    "engine storage write unavailable".to_string(),
                ))
            }
            // The precondition failed: a concurrent writer moved the manifest. The
            // retry-on-fresh-base signal (NOT a hard failure).
            Err(ureq::Error::Status(412, _)) => Err(ManifestPutError::Precondition),
            Err(ureq::Error::Status(403, _)) => {
                eprintln!(
                    "[hugit-serve] R2 conditional manifest PUT 403 — credential is not write-scoped"
                );
                Err(ManifestPutError::Other(
                    "engine storage write unavailable".to_string(),
                ))
            }
            Err(ureq::Error::Status(s, _)) => {
                eprintln!("[hugit-serve] R2 conditional manifest PUT status {s}");
                Err(ManifestPutError::Other(
                    "engine storage write unavailable".to_string(),
                ))
            }
            Err(e) => {
                eprintln!("[hugit-serve] R2 conditional manifest PUT transport error: {e}");
                Err(ManifestPutError::Other(
                    "engine storage write unavailable".to_string(),
                ))
            }
        }
    }

    /// PUT `body` to an ARBITRARY R2 key UNCONDITIONALLY — the generic write the
    /// git ingest bin uses to publish `<tenant>/<repo>/refs.json` +
    /// `<tenant>/<repo>/oid-index.json` (the mutable manifests; the immutable git
    /// objects go to the CAS, not R2). Like [`put`], this needs the one-shot RW
    /// grant (the standing engine cred is read-only → 403). Returns the wire key.
    pub fn put_object(&self, key: &str, body: &[u8]) -> Result<String, EngineErr> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let signed = sigv4::sign_s3_put(
            &self.host,
            &self.bucket,
            key,
            body,
            &self.key_id,
            &self.secret,
            &self.region,
            now,
        );
        let url = format!("{}/{}/{key}", self.endpoint, self.bucket);
        let resp = self
            .agent
            .put(&url)
            .set("Authorization", &signed.authorization)
            .set("x-amz-date", &signed.amz_date)
            .set("x-amz-content-sha256", &signed.content_sha256)
            .send_bytes(body);
        match resp {
            Ok(r) if (200..300).contains(&r.status()) => Ok(format!("r2://{}/{key}", self.bucket)),
            Ok(r) => {
                eprintln!(
                    "[hugit-serve] R2 PUT (object) unexpected status {}",
                    r.status()
                );
                Err(EngineErr::unavailable("engine storage write unavailable"))
            }
            Err(ureq::Error::Status(403, r)) => {
                eprintln!(
                    "[hugit-serve] R2 PUT (object) 403 — credential is not write-scoped: {:?}",
                    r.status()
                );
                Err(EngineErr::unavailable("engine storage write unavailable"))
            }
            Err(ureq::Error::Status(s, _)) => {
                eprintln!("[hugit-serve] R2 PUT (object) status {s}");
                Err(EngineErr::unavailable("engine storage write unavailable"))
            }
            Err(e) => {
                eprintln!("[hugit-serve] R2 PUT (object) transport error: {e}");
                Err(EngineErr::unavailable("engine storage write unavailable"))
            }
        }
    }
}

/// hugit's R2 as the mutable-manifest source for the git-from-CAS loader: maps
/// the [`R2Config::get_object`] `EngineErr` to the loader's `String` error.
impl crate::cas::R2Get for R2Config {
    fn get_object(&self, key: &str) -> Result<Option<Vec<u8>>, String> {
        R2Config::get_object(self, key).map_err(|e| format!("R2 get {key}: {}", e.reason))
    }
}

/// hugit's R2 as the mutable-manifest write target for the git-ingest bin: maps
/// the [`R2Config::put_object`] `EngineErr` to the ingest's `String` error.
impl crate::cas::R2Put for R2Config {
    fn put_object(&self, key: &str, body: &[u8]) -> Result<(), String> {
        R2Config::put_object(self, key, body)
            .map(|_| ())
            .map_err(|e| format!("R2 put {key}: {}", e.reason))
    }
}

/// hugit's R2 as the VERSIONED manifest source for the receive-pack finalize's
/// conditional (If-Match) writes (WP-IFMATCH): the bytes + ETag base a conditional
/// PUT swaps against. Maps the `EngineErr` to the loader's `String` error.
impl crate::cas::R2GetVersioned for R2Config {
    fn get_object_versioned(&self, key: &str) -> Result<Option<(Vec<u8>, CasToken)>, String> {
        R2Config::get_object_etag(self, key).map_err(|e| format!("R2 get {key}: {}", e.reason))
    }
}

/// hugit's R2 as the CONDITIONAL manifest write target for the receive-pack finalize
/// (WP-IFMATCH): a compare-and-swap PUT surfacing the store's 412 distinctly so the
/// finalize can retry-on-fresh-base (never an unconditional clobber).
impl crate::cas::R2PutConditional for R2Config {
    fn put_object_conditional(
        &self,
        key: &str,
        body: &[u8],
        expected: &CasToken,
    ) -> Result<CasToken, crate::cas::ManifestPutError> {
        R2Config::conditional_object_put(self, key, body, expected)
    }
}

/// Whether `HUGIT_SERVE_RECEIVE_PACK` enables the git-push write path. The single
/// source of truth for BOTH the [`AppState::write_path_enabled`] flag and whether a
/// CAS-mode repo is loaded with a [`CasWriteSeam`] — so a stock deploy (the var
/// unset or `0`) has NO write seam and NO behavior change.
fn receive_pack_enabled() -> bool {
    std::env::var("HUGIT_SERVE_RECEIVE_PACK")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

/// The dev-token→operator break-glass gate ([`AppState::allow_dev_operator`]). The
/// PUBLIC prod deploy OMITS `HUGIT_ALLOW_DEV_OPERATOR`, so this is **false** by
/// default and the dev-token confers NO operator elevation. Set to `1` ONLY as a
/// documented ops/bootstrap break-glass. Strict `== "1"` (fail-closed: any other
/// value, including an empty string or a typo, keeps the god-path OFF).
fn dev_operator_allowed() -> bool {
    std::env::var("HUGIT_ALLOW_DEV_OPERATOR")
        .map(|v| v == "1")
        .unwrap_or(false)
}

/// Whether the conditional (If-Match) manifest-PUT path is compiled in as the ONLY
/// steady-state manifest write path. WP-IFMATCH wires it unconditionally, so this is
/// always `true`; it exists as a concrete predicate the multi-instance boot guard
/// asserts against (mechanized, not documented-only) — a future escape hatch that
/// disabled the conditional path would be caught by [`multi_instance_guard`] the
/// moment a deploy declared `max_instances>1`.
///
/// This `true` is HONEST only because the conditional path is now *sufficient*, not
/// merely necessary: the refs.json compare-and-swap RE-VALIDATES the pusher's per-ref
/// `expected` precondition against the FRESH base on every attempt
/// ([`crate::cas::commit_cas_push_manifests`], FIX-IFMATCH-REMERGE). Before that fix a
/// 412 re-merge blindly re-applied the ref tip on the advanced base — a cross-instance
/// same-ref force-push silently lost-updated, so the guard green-lit `max_instances>1`
/// with a safety the code did not provide. With the re-validate, a concurrent same-ref
/// advance now fails closed (StaleRef, the ref is not overwritten), so the guard's claim
/// matches what the write path actually enforces.
const CONDITIONAL_MANIFEST_PUT_ACTIVE: bool = true;

/// Whether the operator has DECLARED a multi-instance deploy (`max_instances>1`).
/// The engine cannot read the Cloudflare instance count in-process, so a >1 deploy
/// MUST acknowledge it via `HUGIT_SERVE_ALLOW_MULTI_INSTANCE`; the boot guard then
/// asserts the conditional manifest-PUT path is active before permitting it.
fn allow_multi_instance() -> bool {
    std::env::var("HUGIT_SERVE_ALLOW_MULTI_INSTANCE")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

/// The fail-closed multi-instance boot guard (WP-IFMATCH — the HARD pre-condition for
/// `max_instances>1`). A deploy running more than one engine instance concurrently
/// mutating the shared `refs.json`/`oid-index.json` manifests is SAFE ONLY if the
/// conditional (If-Match) manifest-PUT path is active (else the concurrent writes
/// lost-update). The instance count is not knowable in-process, so the operator
/// declares it via `HUGIT_SERVE_ALLOW_MULTI_INSTANCE`; this guard REFUSES to boot when
/// multi-instance is declared AND the git-push write path is enabled AND the conditional
/// path is NOT active. Single-instance (the default, var unset) is always allowed; a
/// read-only deploy (`write_path_enabled == false`) never mutates a manifest, so it is
/// allowed too. Pure (takes the three booleans) so it is unit-tested without env.
fn multi_instance_guard(
    write_path_enabled: bool,
    allow_multi_instance: bool,
    conditional_manifest_put_active: bool,
) -> Result<(), String> {
    if allow_multi_instance && write_path_enabled && !conditional_manifest_put_active {
        return Err(
            "HUGIT_SERVE_ALLOW_MULTI_INSTANCE is set with the git-push write path enabled, \
             but the conditional (If-Match) manifest-PUT path is NOT active — refusing to \
             boot: >1 instance would lost-update refs.json/oid-index.json. Wire the \
             conditional manifest PUT (WP-IFMATCH) before running max_instances>1."
                .to_string(),
        );
    }
    Ok(())
}

/// Split a comma-separated repo/dir list into trimmed, non-empty members. One
/// member = the unchanged single-repo config; many = the multi-repo forge set.
fn split_repo_list(list: &str) -> Vec<&str> {
    list.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect()
}

/// Parse one `HUGIT_SERVE_GIT_DIR` list member into `(served slug, git dir)`.
/// Two forms:
/// - `slug=path` — an explicit slug (the part before the FIRST `=`), so a checkout
///   dir whose name differs from the repo slug serves under the right name.
/// - `path` — the served slug is the dir's basename (final path component).
///
/// Fail-closed (→ a fatal boot error) on an empty path or a slug that is not a
/// safe slug — a misconfigured seam refuses to start rather than serving under an
/// ambiguous/unsafe name.
fn parse_git_dir_member(member: &str) -> Result<(String, &str), String> {
    let (slug, dir): (String, &str) = match member.split_once('=') {
        Some((slug, dir)) => (slug.trim().to_string(), dir.trim()),
        None => {
            let dir = member.trim();
            let base = Path::new(dir)
                .file_name()
                .and_then(|s| s.to_str())
                .ok_or_else(|| format!("HUGIT_SERVE_GIT_DIR has no repo basename: {dir:?}"))?;
            (base.to_string(), dir)
        }
    };
    if dir.is_empty() {
        return Err(format!(
            "HUGIT_SERVE_GIT_DIR member has an empty path: {member:?}"
        ));
    }
    if !is_safe_repo_slug(&slug) {
        return Err(format!(
            "HUGIT_SERVE_GIT_DIR slug is not a safe repo slug: {slug:?} (from {member:?})"
        ));
    }
    Ok((slug, dir))
}

/// Load a git directory into a [`CasObjectSource`] and resolve HEAD's root-tree
/// oid — the **live-infra seam** for the `blob`/`edit` file-content reads. Only
/// invoked when `HUGIT_SERVE_GIT_DIR` is set; not exercised by the hermetic
/// handler tests (those seed a `CasObjectSource` directly).
///
/// ## Why a `git` subprocess (not a reused hugit-proto reader)
/// hugit-proto exposes the projection/pack-assembly read path, but no public
/// entrypoint that ingests an on-disk git dir into a `CasObjectSource`; the only
/// such reader in the workspace lives in `hugit-mirror` (`import::history`,
/// `git cat-file`-backed), which is NOT a dependency of this serve crate. Rather
/// than fork that logic OR add a heavyweight new dependency, this loader shells
/// out to the local `git` binary — the same `cat-file`/`rev-list` plumbing
/// hugit-mirror uses — to enumerate every object reachable from HEAD and stream
/// it into the content-addressed store. The store verifies each object against
/// its oid on insert ([`CasObjectSource`]'s content-addressing invariant), so a
/// tampered git dir cannot smuggle mislabeled bytes into a served blob.
///
/// Returns `Err(String)` (→ a fatal boot error) if `git` is absent, the dir is
/// not a repo, HEAD does not resolve, or an object fails to parse — fail-closed:
/// a misconfigured content seam refuses to start rather than silently serving 404.
fn load_git_dir(
    git_dir: &str,
) -> Result<
    (
        hugit_proto::CasObjectSource,
        gix_hash::ObjectId,
        std::collections::BTreeMap<String, String>,
    ),
    String,
> {
    use hugit_proto::{CasObjectSource, ObjectKind};
    use std::process::Command;

    // Run `git -C <dir> <args...>`, capturing stdout bytes; map any failure to a
    // boot error string (no secret content — only the git dir path + git stderr).
    let git = |args: &[&str]| -> Result<Vec<u8>, String> {
        let out = Command::new("git")
            .arg("-C")
            .arg(git_dir)
            .args(args)
            .output()
            .map_err(|e| format!("HUGIT_SERVE_GIT_DIR: failed to spawn `git`: {e}"))?;
        if !out.status.success() {
            return Err(format!(
                "HUGIT_SERVE_GIT_DIR={git_dir}: `git {}` failed: {}",
                args.join(" "),
                String::from_utf8_lossy(&out.stderr).trim()
            ));
        }
        Ok(out.stdout)
    };

    // Resolve HEAD's root tree oid.
    let root_hex = String::from_utf8(git(&["rev-parse", "HEAD^{tree}"])?)
        .map_err(|e| format!("HUGIT_SERVE_GIT_DIR: HEAD tree oid is not UTF-8: {e}"))?;
    let root_hex = root_hex.trim();
    let root_tree = gix_hash::ObjectId::from_hex(root_hex.as_bytes())
        .map_err(|e| format!("HUGIT_SERVE_GIT_DIR: HEAD tree oid {root_hex:?} invalid: {e}"))?;

    // Enumerate every object reachable from ANY ref (`--all`, not just HEAD) so a
    // clone of any branch resolves its full closure from the CAS — the git wire
    // advertisement lists every ref, so every ref's reachable objects must be
    // present (a clone of a branch whose objects were not enumerated would 404
    // mid-stream). `<oid> [path]` per line.
    let listing = String::from_utf8(git(&["rev-list", "--objects", "--all"])?)
        .map_err(|e| format!("HUGIT_SERVE_GIT_DIR: rev-list output is not UTF-8: {e}"))?;

    // The reachable oid set (`rev-list --objects` emits `<oid> [path]` per line).
    let oids: Vec<&str> = listing
        .lines()
        .filter_map(|l| l.split_whitespace().next())
        .filter(|o| !o.is_empty())
        .collect();

    // Stream EVERY reachable object through ONE `git cat-file --batch` process
    // instead of spawning two `git` subprocesses per object. The launch repo has
    // thousands of objects (6.8k+ at this writing); per-object spawns made boot
    // take MINUTES — past the container healthcheck window, and boot is fail-closed
    // (a slow boot ⇒ an unhealthy container ⇒ a broken deploy). `--batch` reads
    // oids on stdin and emits, per object, a header line `<oid> SP <type> SP
    // <size> LF` followed by exactly `<size>` raw body bytes and a trailing LF.
    // The body is the loose-header-less object bytes — exactly what
    // `CasObjectSource` re-hashes against the oid.
    let mut cas = CasObjectSource::new();
    if !oids.is_empty() {
        use std::io::{Read, Write};

        let mut child = Command::new("git")
            .arg("-C")
            .arg(git_dir)
            .args(["cat-file", "--batch"])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| {
                format!("HUGIT_SERVE_GIT_DIR: failed to spawn `git cat-file --batch`: {e}")
            })?;

        // Feed oids on a writer thread while we drain stdout on this thread, and
        // drain stderr on a third — writing all oids before reading stdout would
        // deadlock once the OS pipe buffer fills (object bodies can be MBs).
        let oids_owned: Vec<String> = oids.iter().map(|s| s.to_string()).collect();
        let mut stdin = child.stdin.take().expect("piped stdin");
        let writer = std::thread::spawn(move || -> std::io::Result<()> {
            for oid in &oids_owned {
                stdin.write_all(oid.as_bytes())?;
                stdin.write_all(b"\n")?;
            }
            Ok(()) // drop(stdin) closes the pipe so cat-file finishes
        });
        let mut stderr_pipe = child.stderr.take().expect("piped stderr");
        let errs = std::thread::spawn(move || {
            let mut s = String::new();
            let _ = stderr_pipe.read_to_string(&mut s);
            s
        });

        let mut out = Vec::new();
        child
            .stdout
            .take()
            .expect("piped stdout")
            .read_to_end(&mut out)
            .map_err(|e| format!("HUGIT_SERVE_GIT_DIR: reading `git cat-file --batch`: {e}"))?;

        let writer_res = writer.join();
        let stderr_text = errs.join().unwrap_or_default();
        let status = child
            .wait()
            .map_err(|e| format!("HUGIT_SERVE_GIT_DIR: `git cat-file --batch` wait: {e}"))?;
        match writer_res {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                return Err(format!(
                    "HUGIT_SERVE_GIT_DIR: writing oids to `git cat-file --batch`: {e}"
                ));
            }
            Err(_) => {
                return Err("HUGIT_SERVE_GIT_DIR: the cat-file writer thread panicked".to_string());
            }
        }
        if !status.success() {
            return Err(format!(
                "HUGIT_SERVE_GIT_DIR: `git cat-file --batch` failed: {}",
                stderr_text.trim()
            ));
        }

        // Parse the stream: repeated `<oid> SP <type> SP <size> LF <body> LF`. A
        // `<oid> SP missing LF` line (no body) cannot occur for a rev-list oid but
        // is handled defensively.
        let mut i = 0usize;
        while i < out.len() {
            let nl = match out[i..].iter().position(|&b| b == b'\n') {
                Some(p) => i + p,
                None => break, // no trailing header — stop
            };
            let header = std::str::from_utf8(&out[i..nl])
                .map_err(|_| "HUGIT_SERVE_GIT_DIR: non-UTF-8 cat-file header".to_string())?;
            i = nl + 1;
            let mut parts = header.split(' ');
            let _oid = parts.next().unwrap_or("");
            let type_str = parts.next().unwrap_or("");
            if type_str == "missing" {
                continue; // no body follows
            }
            let size: usize = parts
                .next()
                .and_then(|s| s.parse().ok())
                .ok_or_else(|| format!("HUGIT_SERVE_GIT_DIR: bad cat-file header {header:?}"))?;
            let kind = match type_str {
                "blob" => ObjectKind::Blob,
                "tree" => ObjectKind::Tree,
                "commit" => ObjectKind::Commit,
                "tag" => ObjectKind::Tag,
                // A reachable object of an unknown type cannot occur from a healthy
                // git; skip its body rather than abort.
                _ => {
                    i += size + 1; // body + trailing LF
                    continue;
                }
            };
            if i + size > out.len() {
                return Err("HUGIT_SERVE_GIT_DIR: truncated cat-file object body".to_string());
            }
            // Insert under the oid the bytes hash to; the store rejects a mismatch
            // on the later `get`, so byte-identity to git is preserved.
            cas.insert_raw(kind, out[i..i + size].to_vec());
            i += size;
            if i < out.len() && out[i] == b'\n' {
                i += 1; // trailing LF after the body
            }
        }
    }

    // The refs for the git wire advertisement (`git clone`/`git fetch`). Read from
    // the SAME git dir as the objects so the two are consistent (the launch repo's
    // refs, not the event-log projection — they must agree with the CAS closure).
    // `for-each-ref` emits `<refname> <objectname>` per line (our chosen format).
    let refs_listing =
        String::from_utf8(git(&["for-each-ref", "--format=%(refname) %(objectname)"])?)
            .map_err(|e| format!("HUGIT_SERVE_GIT_DIR: for-each-ref output is not UTF-8: {e}"))?;
    let mut refs = std::collections::BTreeMap::new();
    for line in refs_listing.lines() {
        let mut it = line.split_whitespace();
        if let (Some(name), Some(oid)) = (it.next(), it.next()) {
            refs.insert(name.to_string(), oid.to_string());
        }
    }

    Ok((cas, root_tree, refs))
}

/// `true` iff `principal` is a well-formed TENANT principal (`clerk:{org}:{user}`
/// with a NON-EMPTY org). Mirrors the tenant arm of `authz`'s (private) `caller`
/// classifier — kept minimal + fail-closed (an empty chain, an unknown prefix, or
/// a malformed `clerk:`/`clerk::user` → NOT a tenant). It exists only to answer
/// "does this caller have any 'my repos' at all"; the actual per-repo read
/// decision still routes through [`authz::authorize_read`](crate::authz::authorize_read),
/// so this never becomes a second visibility gate. Operator is classified
/// separately via [`authz::is_operator`](crate::authz::is_operator).
fn principal_is_tenant(principal: &[String]) -> bool {
    principal
        .first()
        .and_then(|p| p.strip_prefix("clerk:"))
        .map(|rest| !rest.split(':').next().unwrap_or("").is_empty())
        .unwrap_or(false)
}

/// A repo slug is a single safe path segment: non-empty, ≤100 chars, ASCII
/// alnum + `-_.`, never `.`/`..`/containing `..` or a path separator. Blocks URL
/// path-traversal into arbitrary files / R2 keys.
#[must_use]
pub fn is_safe_repo_slug(repo: &str) -> bool {
    !repo.is_empty()
        && repo.len() <= 100
        && repo != "."
        && repo != ".."
        && !repo.contains("..")
        && !repo.contains('/')
        && !repo.contains('\\')
        && repo
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// Build a `get`-style lookup over a fixed map (no global env mutation).
    fn vars(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let m: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |k: &str| m.get(k).cloned()
    }

    #[test]
    fn r2_config_accepts_engine_native_names() {
        let c = R2Config::from_vars(vars(&[
            ("HUGIT_SERVE_R2_ACCOUNT_ID", "acct123"),
            ("HUGIT_SERVE_R2_BUCKET", "example-bucket"),
            ("HUGIT_SERVE_R2_KEY_ID", "k"),
            ("HUGIT_SERVE_R2_SECRET", "s"),
            ("HUGIT_SERVE_R2_TENANT_ID", "test-tenant-1"),
        ]))
        .expect("native names");
        assert_eq!(c.host, "acct123.r2.cloudflarestorage.com");
        assert_eq!(c.endpoint, "https://acct123.r2.cloudflarestorage.com");
        assert_eq!(c.tenant_id, "test-tenant-1");
        assert_eq!(c.region, "auto"); // default
    }

    #[test]
    fn r2_config_accepts_s3_standard_cred_names() {
        // A CoreLink/AWS cred file (sourced verbatim) + the separately-set tenant.
        let c = R2Config::from_vars(vars(&[
            (
                "HUGIT_SERVE_R2_ENDPOINT",
                "https://acct123.r2.cloudflarestorage.com",
            ),
            ("HUGIT_SERVE_R2_ACCESS_KEY_ID", "k"),
            ("HUGIT_SERVE_R2_SECRET_ACCESS_KEY", "s"),
            ("HUGIT_SERVE_R2_BUCKET", "example-bucket"),
            ("HUGIT_SERVE_R2_REGION", "auto"),
            ("HUGIT_SERVE_R2_TENANT_ID", "test-tenant-1"),
        ]))
        .expect("S3-standard names");
        assert_eq!(c.host, "acct123.r2.cloudflarestorage.com");
        assert_eq!(c.endpoint, "https://acct123.r2.cloudflarestorage.com");
        assert_eq!(c.key_id, "k");
        assert_eq!(c.secret, "s");
    }

    #[test]
    fn r2_persist_with_unsupported_token_fails_closed_not_unconditional() {
        // Audit hardening: an `Unsupported` token on the R2 write path (fetch got no
        // ETag) must REFUSE the write (no silent last-writer-wins), short-circuiting
        // BEFORE any network PUT. Build an R2 source with dummy creds — the guard
        // returns first, so no request is ever sent.
        let cfg = R2Config::from_vars(vars(&[
            ("HUGIT_SERVE_R2_ACCOUNT_ID", "acct123"),
            ("HUGIT_SERVE_R2_BUCKET", "example-bucket"),
            ("HUGIT_SERVE_R2_KEY_ID", "k"),
            ("HUGIT_SERVE_R2_SECRET", "s"),
            ("HUGIT_SERVE_R2_TENANT_ID", "test-tenant-1"),
        ]))
        .expect("config");
        let source = LogSource::R2(Box::new(cfg));
        let err = source
            .persist("hugit", b"{}", &CasToken::Unsupported)
            .expect_err("an Unsupported token on R2 must fail-closed, not write unconditionally");
        assert_eq!(err.status, 503);
        assert_eq!(err.code, "ENGINE_UNAVAILABLE");
        assert!(
            err.reason.contains("no version token"),
            "the reason must name the refused non-CAS write, got: {}",
            err.reason
        );
    }

    #[test]
    fn r2_config_missing_host_source_is_an_error() {
        let err = match R2Config::from_vars(vars(&[
            ("HUGIT_SERVE_R2_KEY_ID", "k"),
            ("HUGIT_SERVE_R2_SECRET", "s"),
            ("HUGIT_SERVE_R2_BUCKET", "b"),
            ("HUGIT_SERVE_R2_TENANT_ID", "t"),
        ])) {
            Ok(_) => panic!("expected an error when neither ACCOUNT_ID nor ENDPOINT is set"),
            Err(e) => e,
        };
        assert!(
            err.contains("ACCOUNT_ID") && err.contains("ENDPOINT"),
            "{err}"
        );
    }

    #[test]
    fn safe_slugs_accept_normal_repos() {
        for ok in ["hugit", "my-repo", "repo_1", "a.b", "HuGR"] {
            assert!(is_safe_repo_slug(ok), "{ok} should be safe");
        }
    }

    #[test]
    fn unsafe_slugs_block_traversal() {
        for bad in [
            "",
            ".",
            "..",
            "../etc",
            "a/b",
            "a\\b",
            "a..b",
            "../../etc/passwd",
        ] {
            assert!(!is_safe_repo_slug(bad), "{bad} must be rejected");
        }
    }

    #[test]
    fn unsafe_slug_is_404_before_any_fetch() {
        let st = AppState::new(PathBuf::from("/nonexistent"), "tok".to_string());
        assert_eq!(st.load_verified("../etc/passwd").unwrap_err().status, 404);
    }

    #[test]
    fn absent_local_log_is_404() {
        let st = AppState::new(PathBuf::from("/nonexistent-dir-xyz"), "tok".to_string());
        assert_eq!(st.load_verified("hugit").unwrap_err().status, 404);
    }

    #[test]
    fn source_label_reflects_local() {
        let st = AppState::new(PathBuf::from("/tmp/logs"), "tok".to_string());
        assert!(st.source_label().starts_with("local:"));
    }

    #[test]
    fn new_state_has_no_repos_loaded() {
        let st = AppState::new(PathBuf::from("/tmp/logs"), "tok".to_string());
        assert_eq!(st.git_serving_count(), 0);
        assert!(st.repo_state("hugit").is_none());
    }

    /// A minimal RUNTIME `RepoState` for the interior-mutable overlay tests — a
    /// trivial empty object source + a live ref map + a live oid-index (mirrors the
    /// git.rs test seed; the runtime-insert path never reads objects).
    fn runtime_repo_state(refs: BTreeMap<String, String>) -> RepoState {
        let src: Arc<dyn hugit_proto::ObjectSource + Send + Sync> =
            Arc::new(hugit_proto::CasObjectSource::new());
        RepoState {
            git_source: src,
            git_root_tree: gix_hash::ObjectId::from_hex(EMPTY_TREE_OID_HEX.as_bytes()).unwrap(),
            git_refs: LiveRefs::new(refs),
            git_dir: None,
            cas_write: None,
            live_oid_index: Some(crate::cas::LiveOidIndex::new(BTreeMap::new())),
        }
    }

    /// W-PROVISION: a runtime-inserted repo is visible to `repo_state` (the read +
    /// receive-pack lookup) IMMEDIATELY, no reboot — and counts toward
    /// `git_serving_count`. The insert goes through the shared `&AppState` (interior
    /// mutability), exactly as `POST /v1/repos` does.
    #[test]
    fn runtime_insert_is_visible_to_repo_state_without_reboot() {
        let st = AppState::new(PathBuf::from("/tmp/logs"), "tok".to_string());
        assert!(st.repo_state("fresh").is_none(), "absent before insert");
        assert_eq!(st.git_serving_count(), 0);

        let mut refs = BTreeMap::new();
        refs.insert("refs/heads/main".to_string(), "a".repeat(40));
        st.insert_runtime_repo("fresh", runtime_repo_state(refs))
            .expect("runtime insert ok");

        // Immediately visible to the SAME state (no reload): repo_state finds it and
        // its refs advertise (what the receive-pack + clone paths consult).
        let rs = st.repo_state("fresh").expect("served after runtime insert");
        assert_eq!(
            rs.git_refs.snapshot().get("refs/heads/main"),
            Some(&"a".repeat(40))
        );
        assert_eq!(st.git_serving_count(), 1, "runtime repo counts");
        assert!(st.has_repo_seam("fresh"));
    }

    /// The runtime insert is FAIL-CLOSED against a duplicate slug (no clobber of a
    /// loaded seam) — in both the runtime overlay and against a boot-loaded slug.
    #[test]
    fn runtime_insert_refuses_duplicate_slug() {
        let st = AppState::new(PathBuf::from("/tmp/logs"), "tok".to_string());
        st.insert_runtime_repo("dup", runtime_repo_state(BTreeMap::new()))
            .expect("first insert ok");
        // Second insert of the same slug → 409 (never overwrite).
        let e = st
            .insert_runtime_repo("dup", runtime_repo_state(BTreeMap::new()))
            .expect_err("duplicate runtime slug refused");
        assert_eq!(e.status, 409);

        // And a slug already in the IMMUTABLE boot set is refused too.
        let mut boot = AppState::new(PathBuf::from("/tmp/logs"), "tok".to_string());
        boot.repos
            .insert("bootrepo".to_string(), runtime_repo_state(BTreeMap::new()));
        let e = boot
            .insert_runtime_repo("bootrepo", runtime_repo_state(BTreeMap::new()))
            .expect_err("boot-loaded slug refused");
        assert_eq!(e.status, 409);
    }

    /// Boot repos take precedence and both sets are unioned in the me/* index +
    /// serving count (the overlay is additive, boot path unchanged).
    #[test]
    fn runtime_overlay_is_additive_to_boot_set() {
        let mut st = AppState::new(PathBuf::from("/tmp/logs"), "tok".to_string());
        st.repos
            .insert("boot".to_string(), runtime_repo_state(BTreeMap::new()));
        st.insert_runtime_repo("run", runtime_repo_state(BTreeMap::new()))
            .expect("insert");
        assert!(st.repo_state("boot").is_some());
        assert!(st.repo_state("run").is_some());
        assert_eq!(
            st.git_serving_count(),
            2,
            "boot + runtime counted once each"
        );
    }

    #[test]
    fn split_repo_list_single_is_unchanged() {
        // One member = the single-repo config (backward-compatible).
        assert_eq!(split_repo_list("hugit"), vec!["hugit"]);
        assert_eq!(split_repo_list("/srv/git/hugit"), vec!["/srv/git/hugit"]);
    }

    #[test]
    fn split_repo_list_many_trims_and_drops_empty() {
        assert_eq!(
            split_repo_list(" hugit , acme ,, beta "),
            vec!["hugit", "acme", "beta"]
        );
    }

    #[test]
    fn git_dir_member_bare_path_uses_basename_slug() {
        let (slug, dir) = parse_git_dir_member("/srv/git/hugit").expect("bare path");
        assert_eq!(slug, "hugit");
        assert_eq!(dir, "/srv/git/hugit");
    }

    #[test]
    fn git_dir_member_explicit_slug_overrides_basename() {
        // `slug=path` serves a checkout dir under an explicit name.
        let (slug, dir) =
            parse_git_dir_member("hugit=/var/checkouts/launch-repo").expect("explicit slug");
        assert_eq!(slug, "hugit");
        assert_eq!(dir, "/var/checkouts/launch-repo");
    }

    #[test]
    fn git_dir_member_unsafe_slug_is_rejected() {
        // A traversal slug (explicit) is fail-closed.
        assert!(parse_git_dir_member("../etc=/srv/git/x").is_err());
        // A bare path whose basename is unsafe (`..`) is fail-closed.
        assert!(parse_git_dir_member("/srv/git/..").is_err());
        // An empty path is fail-closed.
        assert!(parse_git_dir_member("hugit=").is_err());
    }

    // ── WP-IFMATCH: the multi-instance boot guard + conditional-PUT fail-closed ──

    #[test]
    fn multi_instance_guard_refuses_multi_without_conditional_path() {
        // The ONE refusal case: >1 instance declared, write path on, conditional PUT
        // NOT active → refuse to boot (the lost-update race would be open).
        let err = multi_instance_guard(
            /* write_path_enabled */ true, /* allow_multi_instance */ true,
            /* conditional_manifest_put_active */ false,
        )
        .expect_err("multi-instance without the conditional path must refuse boot");
        assert!(
            err.contains("refusing to boot") && err.contains("max_instances>1"),
            "the error names the fail-closed refusal, got: {err}"
        );
    }

    #[test]
    fn multi_instance_guard_permits_when_conditional_path_active() {
        // The live posture (WP-IFMATCH wired): >1 instance + write path + conditional
        // active → permitted.
        assert!(multi_instance_guard(true, true, true).is_ok());
    }

    #[test]
    fn multi_instance_guard_allows_single_instance_and_readonly() {
        // Single instance (allow_multi = false) is always fine, even with the (defensive)
        // conditional-path-off case — no concurrency to race.
        assert!(multi_instance_guard(true, false, false).is_ok());
        assert!(multi_instance_guard(true, false, true).is_ok());
        // A read-only deploy never mutates a manifest → multi-instance is fine even
        // with the conditional path off.
        assert!(multi_instance_guard(false, true, false).is_ok());
    }

    #[test]
    fn conditional_object_put_refuses_unsupported_token_fails_closed() {
        // SECURITY: an `Unsupported` EXPECTED token (no ETag to swap against) must be
        // REFUSED before any network PUT — never a silent unconditional overwrite that
        // re-opens the lost-update race. Dummy creds; the guard returns first.
        use crate::cas::ManifestPutError;
        let cfg = R2Config::from_vars(vars(&[
            ("HUGIT_SERVE_R2_ACCOUNT_ID", "acct123"),
            ("HUGIT_SERVE_R2_BUCKET", "example-bucket"),
            ("HUGIT_SERVE_R2_KEY_ID", "k"),
            ("HUGIT_SERVE_R2_SECRET", "s"),
            ("HUGIT_SERVE_R2_TENANT_ID", "test-tenant-1"),
        ]))
        .expect("config");
        let err = cfg
            .conditional_object_put("t/hugit/refs.json", b"{}", &CasToken::Unsupported)
            .expect_err("an Unsupported token must fail-closed, not PUT unconditionally");
        match err {
            ManifestPutError::Other(msg) => assert!(
                msg.contains("no version token") || msg.contains("non-CAS"),
                "the reason must name the refused non-CAS write, got: {msg}"
            ),
            ManifestPutError::Precondition => {
                panic!("expected a fail-closed Other, not a Precondition")
            }
        }
    }
}

/// W-METENANT: the per-tenant repo index (`repos_for` / `me_repo_logs`) that
/// closes the cross-principal `/v1/me/*` read exposure. These prove the CORE
/// isolation invariant at the `AppState` layer: a tenant sees ONLY the repos it
/// may `authorize_read`, anonymous/unknown see NONE, the operator sees all, and
/// an unloadable repo is EXCLUDED fail-closed.
#[cfg(test)]
mod me_repos_tests {
    use super::*;
    use hugit_refstore::{Endpoint, PrincipalClass};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU64, Ordering};

    /// A unique scratch dir per call (parallel-test-safe — mirrors the integration
    /// harness: pid + nanos + a monotonic counter so two calls never collide).
    fn scratch_dir() -> PathBuf {
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let seq = SEQ.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "hugit-metenant-{}-{}-{}",
            std::process::id(),
            nanos,
            seq
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A serialized, chain-valid event log carrying exactly one `repo.meta` record.
    fn meta_log(visibility: &str, owner_tenant: &str) -> String {
        let mut log = EventLog::new();
        let payload =
            serde_json::json!({"visibility":visibility,"owner_tenant":owner_tenant}).to_string();
        let body = hugit_refstore::canonical_json(&payload).unwrap_or(payload);
        log.append_authorized(
            PrincipalClass::Orchestrator,
            Endpoint::Push,
            "repo.meta",
            vec!["o".into()],
            body,
            0,
        )
        .expect("append repo.meta");
        serde_json::to_string(log.records()).unwrap()
    }

    /// An `AppState` (Local source) with `repos` = `(slug, log_json)`. Each slug is
    /// BOTH written as a `<slug>.json` log AND inserted into the `repos` map (the
    /// git seam) — the exact shape the me/* index iterates.
    fn state_with(repos: &[(&str, &str)]) -> AppState {
        let dir = scratch_dir();
        let mut st = AppState::new(dir.clone(), "dev-token".to_string());
        for (slug, json) in repos {
            std::fs::write(dir.join(format!("{slug}.json")), json).unwrap();
            st.set_repo_git(
                *slug,
                Arc::new(hugit_proto::CasObjectSource::new()),
                gix_hash::ObjectId::empty_tree(gix_hash::Kind::Sha1),
                BTreeMap::new(),
            );
        }
        st
    }

    fn operator() -> Vec<String> {
        vec!["orchestrator:hugit".to_string()]
    }
    fn tenant(org: &str) -> Vec<String> {
        vec![format!("clerk:{org}:user-1")]
    }

    #[test]
    fn operator_sees_all_loaded_repos() {
        let st = state_with(&[
            ("alpha", &meta_log("private", "org-a")),
            ("beta", &meta_log("private", "org-b")),
        ]);
        assert_eq!(
            st.repos_for(&operator()),
            vec!["alpha".to_string(), "beta".to_string()],
            "operator (dev/orchestrator) sees every loaded repo"
        );
    }

    // ── count_owned_repos (the per-tenant DoS-cap denominator) ───────────────

    #[test]
    fn count_owned_repos_filters_by_owner_tenant() {
        // Ownership (not read-authz): a PUBLIC repo still counts toward its OWNER's
        // cap, and never toward another tenant's.
        let st = state_with(&[
            ("alpha", &meta_log("private", "org-a")),
            ("beta", &meta_log("public", "org-a")),
            ("gamma", &meta_log("private", "org-b")),
        ]);
        assert_eq!(st.count_owned_repos("org-a"), Some(2));
        assert_eq!(st.count_owned_repos("org-b"), Some(1));
        assert_eq!(st.count_owned_repos("org-z"), Some(0));
    }

    #[test]
    fn count_owned_repos_fails_closed_on_unloadable_candidate() {
        // A slug wired into the seam set but whose log cannot load/verify makes the
        // count indeterminate → None (the provision path maps None to a refusal, so a
        // read fault can never be leveraged to slip past the cap).
        let mut st = state_with(&[("alpha", &meta_log("private", "org-a"))]);
        st.set_repo_git(
            "ghost", // no backing `ghost.json` log → load_verified 404 → indeterminate
            Arc::new(hugit_proto::CasObjectSource::new()),
            gix_hash::ObjectId::empty_tree(gix_hash::Kind::Sha1),
            BTreeMap::new(),
        );
        assert_eq!(
            st.count_owned_repos("org-a"),
            None,
            "fail-closed on ambiguity"
        );
    }

    #[test]
    fn two_tenants_disjoint_private_repos_see_only_their_own() {
        // THE core cross-principal isolation invariant.
        let st = state_with(&[
            ("alpha", &meta_log("private", "org-a")),
            ("beta", &meta_log("private", "org-b")),
        ]);
        assert_eq!(
            st.repos_for(&tenant("org-a")),
            vec!["alpha".to_string()],
            "org-a sees ONLY its own private repo"
        );
        assert_eq!(
            st.repos_for(&tenant("org-b")),
            vec!["beta".to_string()],
            "org-b sees ONLY its own private repo — never org-a's"
        );
    }

    #[test]
    fn tenant_sees_public_plus_own_private_not_foreign_private() {
        let st = state_with(&[
            ("pubrepo", &meta_log("public", "org-a")),
            ("priv_a", &meta_log("private", "org-a")),
            ("priv_b", &meta_log("private", "org-b")),
        ]);
        // org-b: the public repo + its OWN private; NOT org-a's private.
        assert_eq!(
            st.repos_for(&tenant("org-b")),
            vec!["priv_b".to_string(), "pubrepo".to_string()],
            "a tenant sees public repos + its own private, never a foreign private"
        );
    }

    #[test]
    fn anonymous_and_unknown_and_malformed_get_empty() {
        // me/* is identity-scoped: no tenant ⇒ NO repos (never all-public, never
        // the default). Even a PUBLIC repo present is absent for these callers.
        let st = state_with(&[("pubrepo", &meta_log("public", "org-a"))]);
        assert!(
            st.repos_for(&[]).is_empty(),
            "anonymous (empty chain) → empty"
        );
        assert!(
            st.repos_for(&["weird:thing".to_string()]).is_empty(),
            "unknown bearer prefix → empty"
        );
        assert!(
            st.repos_for(&["clerk:".to_string()]).is_empty(),
            "malformed clerk (empty org) → empty"
        );
        assert!(
            st.repos_for(&["clerk::user".to_string()]).is_empty(),
            "clerk with empty org → empty"
        );
    }

    #[test]
    fn unloadable_repo_is_excluded_fail_closed() {
        // "ghost" is in the repos map but has NO <slug>.json → load fails → it must
        // be EXCLUDED (never included by default), for operator AND tenant.
        let dir = scratch_dir();
        let mut st = AppState::new(dir.clone(), "dev-token".to_string());
        std::fs::write(dir.join("good.json"), meta_log("public", "org-a")).unwrap();
        for slug in ["good", "ghost"] {
            st.set_repo_git(
                slug,
                Arc::new(hugit_proto::CasObjectSource::new()),
                gix_hash::ObjectId::empty_tree(gix_hash::Kind::Sha1),
                BTreeMap::new(),
            );
        }
        assert_eq!(
            st.repos_for(&operator()),
            vec!["good".to_string()],
            "operator: an unloadable repo is excluded fail-closed"
        );
        assert_eq!(
            st.repos_for(&tenant("org-a")),
            vec!["good".to_string()],
            "tenant: an unloadable repo is excluded fail-closed"
        );
    }

    #[test]
    fn me_repo_logs_returns_the_verified_logs_for_the_index() {
        // The (slug, log) pairs match the index — and each carries the repo's real
        // verified log (so the builders aggregate real data, not a stub).
        let st = state_with(&[
            ("alpha", &meta_log("private", "org-a")),
            ("beta", &meta_log("public", "org-b")),
        ]);
        let logs = st.me_repo_logs(&tenant("org-a"));
        let slugs: Vec<&str> = logs.iter().map(|(s, _)| s.as_str()).collect();
        // org-a: its own private "alpha" + the public "beta".
        assert_eq!(slugs, vec!["alpha", "beta"]);
        // Each log carries the repo.meta record (real verified log, not empty stub).
        for (_, log) in &logs {
            assert!(
                log.records().iter().any(|r| r.kind == "repo.meta"),
                "each returned log is the repo's real verified log"
            );
        }
    }
}
