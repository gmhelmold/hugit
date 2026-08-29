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

use std::collections::{BTreeMap, HashSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};
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
    /// A SECOND agent with a TIGHT timeout ([`crate::cas::LAZY_LOAD_FETCH_TIMEOUT`]),
    /// used ONLY by [`get_object_bounded`](Self::get_object_bounded) on the accept-loop
    /// lazy-load-on-miss path — so a slow/throttling R2 manifest read cannot stall the
    /// single-threaded accept loop for the standard 30s.
    pub fast_agent: ureq::Agent,
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
    /// The cached-clone-pack seam (WP-BC): where a pre-assembled full-clone pack is
    /// stored/read so an anonymous full `git clone` streams ONE R2 object instead of
    /// walking the whole object closure per request. `Some` ONLY for a CAS-backed repo
    /// whose receive-pack write seam is present (the build PUTs the pack + pointer, so
    /// it needs the SAME write-scoped [`R2Config`] the [`CasWriteSeam`] uses). `None`
    /// for GIT_DIR mode, a CAS repo with receive-pack OFF (no write cred → the build
    /// would 403 on PUT), and test seeds — the clone then falls back to the slow walk
    /// (never a wrong pack; the serve side re-checks `refset_sha`).
    pub clone_cache: Option<crate::clone_pack::CloneCacheSeam>,
}

/// The interior of a [`LiveRefs`]: the `ref → oid` map plus a monotonic LOCAL
/// generation counter. Every local mutation ([`LiveRefs::set_ref`] /
/// [`LiveRefs::remove_ref`] — the push hot-swaps) bumps `generation` UNDER the same
/// write lock as the map edit, so the background refresher can detect a hot-swap that
/// landed AFTER it snapshotted the generation but BEFORE it installs a (now-stale) R2
/// read — and skip the clobbering install ([`LiveRefs::replace_if_unchanged`]).
struct LiveRefsInner {
    refs: BTreeMap<String, String>,
    /// Bumped on every local `set_ref`/`remove_ref` hot-swap. NOT a durable/R2 value
    /// (R2 ETags are opaque + unordered and `RefsManifest` carries no generation), so
    /// it is a purely in-process ordering token between the push path and the refresher.
    generation: u64,
}

/// An interior-mutable, shared `ref name → tip oid hex` map — the live counterpart
/// of the boot-loaded refs. Shared (via one `Arc`) between the git-wire read path
/// (the advertise / clone) and the CAS-mode push finalize, so a pushed tip is
/// advertised immediately, no reboot. The accept loop (`server::serve_on`) is
/// single-threaded, so the `RwLock` is uncontended.
#[derive(Clone)]
pub struct LiveRefs(Arc<RwLock<LiveRefsInner>>);

impl LiveRefs {
    /// Wrap a boot-loaded `ref → oid` map (generation starts at 0).
    #[must_use]
    pub fn new(refs: BTreeMap<String, String>) -> Self {
        Self(Arc::new(RwLock::new(LiveRefsInner {
            refs,
            generation: 0,
        })))
    }

    /// An owned snapshot of the LIVE refs (the advertisement's `RefView` source).
    /// Cheap: ref maps are small (a handful of branches), so a clone-per-advertise
    /// is negligible and keeps the read sites working with an owned `BTreeMap`.
    #[must_use]
    pub fn snapshot(&self) -> BTreeMap<String, String> {
        self.0
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .refs
            .clone()
    }

    /// Whether the LIVE ref set is empty (a not-loaded / refless repo).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .refs
            .is_empty()
    }

    /// The current LOCAL generation. The refresher captures this at read-START (before
    /// the R2 GET) and passes it to [`replace_if_unchanged`](Self::replace_if_unchanged)
    /// so a hot-swap that lands during the read is not overwritten by the stale snapshot.
    #[must_use]
    pub fn generation(&self) -> u64 {
        self.0.read().unwrap_or_else(|e| e.into_inner()).generation
    }

    /// Atomically set `ref_name`'s tip to `oid` in the LIVE map (a push hot-swap).
    /// Bumps the generation under the same write lock so a concurrent stale refresh
    /// cannot silently revert this hot-swap.
    pub fn set_ref(&self, ref_name: &str, oid: &str) {
        let mut g = self.0.write().unwrap_or_else(|e| e.into_inner());
        g.refs.insert(ref_name.to_string(), oid.to_string());
        g.generation = g.generation.wrapping_add(1);
    }

    /// Atomically remove `ref_name` from the LIVE map (a delete-ref hot-swap). A
    /// no-op on the map if the ref is absent (the durable finalize already validated
    /// presence; this only mirrors the committed removal into the in-memory advertise),
    /// but the generation still bumps so the ordering token advances on every hot-swap.
    pub fn remove_ref(&self, ref_name: &str) {
        let mut g = self.0.write().unwrap_or_else(|e| e.into_inner());
        g.refs.remove(ref_name);
        g.generation = g.generation.wrapping_add(1);
    }

    /// Atomically REPLACE the whole LIVE map with `refs` (WP-B5 read-after-write): the
    /// background refresher loads the authoritative durable `refs.json` and installs it, so
    /// a second engine instance picks up another instance's push/delete within the refresh
    /// window (bounded staleness, ≤ the TTL). A full replace (not a merge) is correct
    /// because the durable manifest is the COMPLETE authoritative ref set — an add
    /// propagates, and a ref deleted on another instance (dropped from the manifest) drops
    /// here too. Does NOT consult the generation — the caller has decided this install
    /// wins; the generation guard lives in [`replace_if_unchanged`](Self::replace_if_unchanged).
    pub fn replace(&self, refs: BTreeMap<String, String>) {
        self.0.write().unwrap_or_else(|e| e.into_inner()).refs = refs;
    }

    /// Install `refs` ONLY if the LOCAL generation still equals `expected_generation`
    /// (check + replace atomic under one write lock). Returns `true` on install, `false`
    /// if a local hot-swap ([`set_ref`](Self::set_ref)/[`remove_ref`](Self::remove_ref))
    /// bumped the generation since the caller snapshotted it — in which case this install
    /// is SKIPPED so the fresher local tip survives.
    ///
    /// This closes the WP-B5 monotonic-guard defect: the refresher issues an unversioned
    /// R2 GET, and a concurrent push's conditional PUT + `set_ref` hot-swap can land
    /// AFTER that GET starts. Without the guard the refresher's `replace` would clobber
    /// the just-applied new tip with the stale pre-push snapshot for up to one interval,
    /// re-advertising the OLD tip after the client already got `ok`.
    pub fn replace_if_unchanged(
        &self,
        refs: BTreeMap<String, String>,
        expected_generation: u64,
    ) -> bool {
        let mut g = self.0.write().unwrap_or_else(|e| e.into_inner());
        if g.generation != expected_generation {
            return false; // a local hot-swap landed during the read — keep the fresher tip
        }
        g.refs = refs;
        true
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
    /// Derive the [`clone_pack::CloneCacheSeam`](crate::clone_pack::CloneCacheSeam)
    /// for this repo from the SAME write-scoped [`R2Config`] + tenant/slug the push
    /// finalize uses. A clone-pack BUILD PUTs the pack object + `current.json`
    /// pointer, so it MUST ride the write-scoped credential (a read-only cred 403s on
    /// PUT); reusing the receive-pack seam's `r2` guarantees that by construction.
    #[must_use]
    pub fn clone_cache_seam(&self) -> crate::clone_pack::CloneCacheSeam {
        crate::clone_pack::CloneCacheSeam {
            r2: self.r2.clone(),
            tenant: self.tenant.clone(),
            repo_slug: self.repo_slug.clone(),
        }
    }

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
    /// An OPTIONAL SECOND operator Bearer (`HUGIT_ENGINE_DEV_TOKEN_EXTRA`), validated
    /// ALONGSIDE `dev_token`. Enables adding a new ops credential with ZERO downtime —
    /// the primary `dev_token` a sibling (githugr) already sends is left untouched, so
    /// there is no rotation window. `None` when the env var is unset/whitespace.
    pub dev_token_extra: Option<String>,
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
    /// provisions in one engine lifetime (rare, human-driven). A provisioned repo's
    /// git seam is now RECOVERED on demand — [`repo_state_or_load`](Self::repo_state_or_load)
    /// lazy-loads it from the durable R2 manifests on the first request, so it survives
    /// a reboot AND is served by an instance that did not create it (closing the
    /// count≥2 new-repo-404 + reboot-loss gap, #96) without needing the
    /// `HUGIT_SERVE_CAS_REPO` list to be updated.
    pub repos_runtime: Arc<RwLock<std::collections::HashMap<String, &'static RepoState>>>,
    /// Negative cache for [`repo_state_or_load`](Self::repo_state_or_load): slug → the
    /// `now_ms()` of the last R2 lazy-load MISS. A repo not found in R2 is recorded so
    /// a 404-probe storm cannot re-hit R2 (`load_manifests_from_cas`) on EVERY request
    /// and stall the single-threaded accept loop (the read-latency-DoS class); a miss
    /// is only re-probed after [`REPO_LOAD_MISS_COOLDOWN_MS`]. Interior-mutable behind
    /// the shared `&AppState` (same pattern as [`repos_runtime`](Self::repos_runtime));
    /// the single-threaded accept loop keeps it uncontended.
    pub repo_load_misses: Arc<RwLock<std::collections::HashMap<String, u64>>>,
    /// GLOBAL rate-limiter on lazy-load ATTEMPTS that actually touch R2 (L2): a single
    /// bounded fixed-window counter `(window_start_ms, count)` — NOT a per-slug map, so
    /// it cannot itself leak (the per-slug [`repo_load_misses`](Self::repo_load_misses)
    /// negative cache does not stop a VARIED-slug 404 storm; each distinct nonexistent
    /// slug would otherwise force a fresh R2 round-trip). At most
    /// [`MAX_LAZY_LOADS_PER_WINDOW`] attempts per [`LAZY_LOAD_WINDOW_MS`] reach R2; over
    /// budget, [`repo_state_or_load`](Self::repo_state_or_load) returns `None` FAST
    /// without touching R2. The single-threaded accept loop keeps the counter race-free.
    pub repo_lazy_load_budget: Arc<RwLock<(u64, u32)>>,
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
    /// CAS `batch_read` self-check result for the first CAS-backed repo, surfaced on
    /// `/readyz` as `"cas_batch_read"`. **Interior-mutable + shared** (`Arc<RwLock>`)
    /// because the probe is run OFF the boot path on a detached thread that writes the
    /// result here when it lands — boot NEVER blocks on it (FIX: the synchronous
    /// ~18s probe with [`crate::cas::BATCH_REQUEST_CHUNK`] = 256 blew the container
    /// startup deadline → crash-loop outage). Values: `"probing"` (a CAS-backed repo
    /// loaded, the detached probe has not finished yet — the initial value after a
    /// deploy), `"ok <found>/<req> <ms>ms"` (the batch-plane works), `"empty-index"`
    /// (no objects indexed — newly provisioned repo), `"err:<detail>"` (batch_read
    /// failed — the failure `/readyz` surfaces, incl. a spawn/panic guard), or
    /// `"unprobed"` (no CAS-backed repo loaded at boot, Local/git-dir mode).
    pub cas_batch_read_health: Arc<RwLock<String>>,
    /// The per-repo clone-pack BUILD in-progress guard (WP-BC): the set of repo slugs
    /// whose background full-clone-pack assembly is currently running. Prevents two
    /// concurrent builds of one repo (a boot bootstrap + a post-push rebuild, or two
    /// pushes). A slug is inserted before the detached build thread spawns and removed
    /// when it ends (in ALL paths — success, error, panic). Interior-mutable behind the
    /// shared `&AppState` the handlers hold; the `Mutex` is near-uncontended (a build is
    /// rare + the critical section is a set insert/remove).
    pub clone_pack_building: Arc<Mutex<HashSet<String>>>,
    /// Whether PAT (Personal Access Token) git/API auth is ENABLED — the
    /// `HUGIT_SERVE_PAT_AUTH=1` gate (default **false**). A NEW auth surface, so it
    /// ships DISABLED behind an adversarial review; when off, [`pat_index`](Self::pat_index)
    /// is never built or consulted and a `ghgr_pat_` credential authenticates NOTHING
    /// (falls through to the existing tiers → anonymous/401). See
    /// `docs/design/2026-07-05-pat-git-auth-wire.md`.
    ///
    /// **Fail-closed on multi-instance:** [`from_env`](Self::from_env) REFUSES to boot
    /// with this on AND `>1` instances permitted — the in-memory index is
    /// single-instance-authoritative (a PAT minted on A is absent from B until B
    /// reboots), the same cross-instance staleness class as the ref hot-swap, gated on
    /// the B5 read-after-write seam.
    pub pat_auth_enabled: bool,
    /// The in-memory PAT auth index: `sha256(secret) → PatAuth` (owner principal +
    /// scopes + expiry). Built at boot from every `_accounts/*` log (when
    /// [`pat_auth_enabled`](Self::pat_auth_enabled)); refreshed on create/revoke
    /// ([`pat_index_insert_if_enabled`](Self::pat_index_insert_if_enabled) /
    /// [`pat_index_remove_if_enabled`](Self::pat_index_remove_if_enabled)) so a
    /// mint/revoke takes effect immediately, no reboot. Hot-path lookup is O(1)
    /// in-memory (NO R2 read per auth — a per-request R2 GET would be a latency DoS on
    /// the single-threaded engine). Interior-mutable behind the shared `&AppState`
    /// (same pattern as [`LiveRefs`]/`repos_runtime`). The key is the SECRET hash, so a
    /// leaked index/hash cannot forge a token (resolution hashes the presented secret).
    pub pat_index:
        Arc<RwLock<std::collections::HashMap<String, crate::writes::verbs::write_token::PatAuth>>>,
    /// Best-effort in-memory PAT last-used tracker: `pat_id → last-used Unix ms`, updated on
    /// a successful PAT auth (`resolve_pat_from_auth`). Surfaced as `PatMetaVm.last_used_at`
    /// on `GET /v1/me/account` so a user can spot idle/leaked tokens (the whole point of
    /// user-managed PATs). **Cheap on the accept loop** — an in-memory map write, NO durable
    /// I/O (a `pat.used` append per auth would be a log-bloat + accept-loop-latency DoS).
    /// **Honest limitation:** in-memory only, so it **resets on engine restart** (0 = not
    /// observed used since boot) and is per-instance under `max_instances>1` — a durable
    /// off-loop flush (like the B5 ref refresher) is the tracked follow-up. Keyed by the
    /// non-secret `pat_id` (never the secret / its hash).
    pub pat_last_used: Arc<RwLock<std::collections::HashMap<String, u64>>>,
    /// The CoreLink physical CAS-erase seam config (GDPR1 slice-2), read fail-closed from
    /// `CORELINK_ERASE_URL` + `CORELINK_ERASE_AUTH_KEY` at boot. `None` → the erase seam is
    /// NOT configured, so the operator-execute route is DISABLED (404 — presence not
    /// disclosed) and the executor could only ever claim `partial`. `Some` holds the
    /// validated config (SSRF-allowlisted host + a non-empty key); a fresh per-call
    /// [`HttpCasErase`](crate::writes::erasure::HttpCasErase) is built from it in the route
    /// (the client is `!Sync` — a per-erasure-run local — so the SHARED `AppState` holds
    /// only the `Send + Sync` config, never the client). The key never Debug-prints
    /// (redacting `Debug` on `EraseConfig`).
    pub erase_config: Option<crate::writes::erasure::EraseConfig>,
    /// The engine-wide precomputed per-path blob-history index (#70(a)): `slug →
    /// (head, path → touching revisions)`, built OFF the accept loop (at boot +
    /// post-push) so a DEEP-history file's "Histórico" drawer serves from the index
    /// instead of an exhausting live walk that trips the 2 s budget → empty. A lookup
    /// MISS (no index yet, a stale HEAD, or an un-indexed path) falls back to the live
    /// wall-clock-bounded [`hugit_proto::blob_history`] walk — a pure enhancement over
    /// that safety bound, never a replacement. Interior-mutable behind the shared
    /// `&AppState` (same pattern as [`LiveRefs`]/`repos_runtime`); cheap to clone.
    pub blob_history_index: crate::blob_history_index::BlobHistoryStore,
    /// The engine-wide precomputed per-repo CODE SEARCH index: `slug → (head, indexed
    /// files)`, built OFF the accept loop (at boot + post-push) so `/search` serves
    /// real code matches from an in-memory scan instead of a per-request CAS grep that
    /// fans out a synchronous R2 fetch PER FILE — the latency DoS that wedged prod once
    /// (`/search` bounded by result-count is still a per-object-fetch DoS on the
    /// single-threaded engine). A query consults this map ONLY; a MISS (no index yet /
    /// stale HEAD / a private repo, which is never indexed) is honest-empty, NEVER a
    /// live fetch. Only PUBLIC (anonymous-readable) repos are indexed (visibility gate
    /// at build time). Interior-mutable behind the shared `&AppState` (same pattern as
    /// [`LiveRefs`]/`repos_runtime`); cheap to clone.
    pub search_index: crate::search_index::SearchStore,
    /// The engine-wide, content-addressed HOME-RENDER cache: `root_tree_oid → (tree
    /// listing, README)`. The `/home` read lists the root tree + resolves the README on
    /// EVERY request — ~0.5–1 s of CAS tree-walk on the single-threaded lazy-CAS engine,
    /// re-run per request because the walk does not warm. This cache stores the two
    /// CAS-expensive outputs keyed by the content-addressed root tree oid: a HIT is a
    /// pure in-memory op (ZERO CAS walk), a push produces a new tree oid → a MISS →
    /// automatic invalidation (a changed tree can never be a stale HIT, so no rebuild
    /// hook is needed for correctness). The FAST log-derived parts of the home VM are
    /// rebuilt FRESH per request (never cached). Stored UNSCRUBBED; scrubbed at the read
    /// boundary in [`build_home`](crate::handlers::build_home). Byte-bounded with FIFO
    /// eviction. Interior-mutable behind the shared `&AppState` (same pattern as
    /// [`search_index`](Self::search_index)); cheap to clone.
    pub home_cache: crate::home_cache::HomeRenderCache,
    /// Cached, per-repo projected [`RepoMeta`](crate::authz::RepoMeta) — task #74
    /// (W-METENANT scaling follow-up). [`me_repo_logs`](Self::me_repo_logs) and
    /// [`count_owned_repos`](Self::count_owned_repos) used to call
    /// [`project_repo_meta`](crate::authz::project_repo_meta) PER repo PER request,
    /// re-walking each repo's ENTIRE event log just to derive 3 fields; on a
    /// per-tenant listing this is O(repos × log-size) on the single-threaded engine.
    /// This cache makes the common case O(1): populated ONCE at boot for every
    /// loaded repo ([`boot_populate_repo_meta_cache`](Self::boot_populate_repo_meta_cache),
    /// called from [`from_env`](Self::from_env)), and kept fresh by every write path
    /// that can change a repo's projected meta — provisioning a NEW repo
    /// ([`crate::writes::verbs::write_provision::provision`], via
    /// [`cache_repo_meta`](Self::cache_repo_meta)), a `repo.meta`
    /// visibility/owner-tenant update ([`crate::writes::verbs::write_repo_meta`],
    /// invalidated in [`dispatch_repo_write`](crate::server::dispatch_repo_write) via
    /// [`refresh_repo_meta_cache`](Self::refresh_repo_meta_cache)), and the GDPR1
    /// `repo.erased` tombstone
    /// ([`crate::writes::erasure::tombstone_repo`], same refresh hook).
    ///
    /// **Fail-safe by construction:** [`repo_meta_cached`](Self::repo_meta_cached) is
    /// the ONLY reader, and a cache MISS falls back to a LIVE
    /// [`project_repo_meta`] over the caller's already-loaded log — the exact
    /// pre-cache computation. A gap in cache coverage can only ever cost time, never
    /// correctness. Interior-mutable behind the shared `&AppState` (same pattern as
    /// `repos_runtime`/`pat_index`); the single-threaded accept loop keeps the lock
    /// uncontended.
    pub repo_meta_cache: Arc<RwLock<std::collections::HashMap<String, crate::authz::RepoMeta>>>,
    /// Per-owner_tenant advisory locks that serialize the G10 storage-cap
    /// CHECK→size.json-COMMIT critical section across the DETACHED receive-pack workers
    /// (each holds a CLONE of this `AppState`, so this registry MUST live behind an
    /// `Arc` to be shared). Without it the check-then-commit is a TOCTOU: two concurrent
    /// pushes for the same owner_tenant each read the pre-push aggregate, both pass the
    /// cap, then both bump `size.json` (a monotonic CAS-merge that never rejects) — the
    /// aggregate overshoots the per-owner_tenant (and, for same-repo pushes, per-repo)
    /// ceiling by up to N×pack-cap. [`storage_quota_lock`](Self::storage_quota_lock)
    /// hands out one lock per key; the receive-pack finalize holds its guard across the
    /// quota check AND the durable commit, so a same-tenant push cannot pass the cap
    /// until the prior push's `size.json` is committed and visible to its check.
    /// Bounded by the tenant/repo count (each entry is a zero-byte `Mutex<()>`); the
    /// outer `Mutex` is held only for the O(1) lookup, never across a push.
    pub storage_quota_locks: Arc<Mutex<std::collections::HashMap<String, Arc<Mutex<()>>>>>,
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

/// The #76 BOOT tenant-registry reconcile BOUND — how many durable repos a single off-loop
/// boot pass heals into their owner tenant's `_tenants/{org}.json` registry. Bounded so the
/// pass is finite even as the platform grows; the remainder heals on the next boot or the
/// provision no-clobber path (idempotent, genesis-authoritative).
const BOOT_RECONCILE_MAX_REPOS: usize = 512;

/// The WP-B5 read-after-write ref-cache refresh interval: the background refresher reloads
/// every loaded repo's durable `refs.json` this often, so a second engine instance sees
/// another instance's push/delete within this window. clw signed off **2s** for
/// `max_instances=2` — the durable CAS + conditional If-Match `refs.json` PUT make the
/// ≤2s staleness UX-only (a stale-base push is rejected non-fast-forward + retried), never a
/// lost update. Upgrade to a zero-staleness conditional-GET (Option 1) before widening past 2.
const REFS_REFRESH_INTERVAL_MS: u64 = 2_000;

/// The GDPR1 erasure AUTO-EXECUTOR sweep cadence — a background reconciler, MINUTES not
/// seconds (this is NOT latency-sensitive: the erasure grace window is hours/days, so a
/// 5-minute sweep lands every matured request well inside any bound). The whole cascade runs
/// on the sweep thread, OFF the accept loop, so this interval only paces the reconciler; it
/// never affects request liveness.
const ERASURE_AUTO_EXECUTE_INTERVAL_MS: u64 = 5 * 60 * 1_000;

/// The maximum number of accounts the sweep DRIVES an irreversible erasure on per pass — a
/// runaway guard. The sweep is off-loop so its duration is fine, but an unbounded drive over
/// a huge matured backlog is capped; the remainder lands on the NEXT sweep, idempotently (a
/// completed account is a no-op, a `partial` retries).
const ERASURE_AUTO_EXECUTE_MAX_PER_SWEEP: usize = 32;

/// Cooldown before re-probing R2 for a repo a lazy-load found ABSENT — bounds the
/// accept-loop R2 cost of a 404-probe storm (see [`AppState::repo_load_misses`] +
/// [`AppState::repo_state_or_load`]).
const REPO_LOAD_MISS_COOLDOWN_MS: u64 = 30_000;

/// L2 global lazy-load rate limit: at most this many lazy-load attempts that touch R2
/// per [`LAZY_LOAD_WINDOW_MS`] window. Bounds a VARIED-slug 404 storm on the unauth git
/// wire (which the per-slug negative cache cannot stop) so it cannot flood the
/// single-threaded accept loop with R2 round-trips. Generous for real first-request
/// traffic, a hard ceiling on abuse.
const MAX_LAZY_LOADS_PER_WINDOW: u32 = 20;

/// The fixed window for [`MAX_LAZY_LOADS_PER_WINDOW`].
const LAZY_LOAD_WINDOW_MS: u64 = 1_000;

/// Wall-clock ms since the Unix epoch (the negative-cache timestamp source). A clock
/// fault degrades to `0` → a stale-but-safe miss re-probe, never a panic.
fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

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

    /// The R2 read handle for the CAS content bucket (`<tenant>/<repo>/oid-index.json`, the
    /// digests a repo references) — the SAME `R2Config` (one bucket, `from_env`) that serves
    /// the event logs, so it reads any key in the bucket. `None` in Local/dev mode (no R2).
    /// Used by the GDPR1 erase route to build the [`R2OidIndexDigests`](crate::writes::erasure::R2OidIndexDigests)
    /// digest source. `R2Config` implements [`crate::cas::R2Get`].
    #[must_use]
    pub fn cas_r2_read(&self) -> Option<&R2Config> {
        match &self.source {
            LogSource::R2(r2) => Some(r2),
            LogSource::Local { .. } => None,
        }
    }

    /// The CAS tenant hugit's git content is keyed under (`HUGIT_SERVE_CAS_TENANT_ID`, the
    /// single shared `d863fafb` — see the state doc). `None` when unset/empty. The prefix
    /// for `oid-index.json` reads + the erase seam's tenant field.
    #[must_use]
    pub fn cas_tenant() -> Option<String> {
        std::env::var("HUGIT_SERVE_CAS_TENANT_ID")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    }

    /// Refresh ONE repo's in-memory ref cache from its durable `refs.json` (WP-B5
    /// read-after-write). FAIL-SAFE: an ABSENT manifest or ANY read/parse fault leaves the
    /// current live cache UNCHANGED (retry the next tick) — never clobbers a good advertise
    /// with an empty/partial map. MONOTONIC-SAFE: the local generation is captured BEFORE
    /// the R2 GET and the install goes through [`LiveRefs::replace_if_unchanged`], so a
    /// push hot-swap that lands DURING this (unversioned) read is never reverted by the
    /// stale pre-push snapshot — the fresher local tip wins and this install is skipped.
    /// Pure over an [`crate::cas::R2Get`] double (testable without a live R2).
    pub(crate) fn refresh_repo_refs_once(
        r2: &dyn crate::cas::R2Get,
        tenant: &str,
        slug: &str,
        live: &LiveRefs,
    ) {
        // Snapshot the local generation BEFORE any R2 I/O: a push's `set_ref`/`remove_ref`
        // that bumps it after this point makes the install below a no-op (fresher tip wins).
        let gen_at_start = live.generation();
        // Install the durable manifest ONLY on a clean read+parse; an absent manifest
        // (`Ok(None)`), a read fault (`Err`), or a parse fault all keep the current live
        // cache (retry next tick) — never install a corrupt/empty map.
        if let Ok(Some(bytes)) = r2.get_object(&crate::cas::refs_manifest_key(tenant, slug))
            && let Ok(manifest) = crate::cas::parse_refs_manifest(&bytes)
        {
            // Skip the install if a local hot-swap advanced the generation during the read.
            live.replace_if_unchanged(manifest.refs, gen_at_start);
        }
    }

    /// Spawn the background ref-cache refresher (WP-B5 read-after-write — the
    /// `max_instances>1` fungibility fix, clw-designed 2026-07-06). NO-OP in Local/git-dir
    /// mode (no durable `refs.json` store to reload). In CAS mode a dedicated DETACHED
    /// thread reloads every loaded repo's `refs.json` every [`REFS_REFRESH_INTERVAL_MS`], so
    /// a SECOND engine instance picks up another instance's push/delete within that window
    /// (bounded staleness ≤ the interval; clw signed off 2s for `max_instances=2`).
    ///
    /// **Strictly OFF the single-threaded accept loop:** the thread only writes the
    /// Arc-shared [`LiveRefs`] (exactly what the push hot-swap already does), and it clones
    /// the per-repo `LiveRefs` handles BEFORE any R2 I/O so it never holds a lock across a
    /// network read — the accept loop NEVER blocks on this. **Safe (not just fast):** a
    /// stale advertise is UX-only, never a lost update — every durable ref mutation goes
    /// through the receive-pack log compare-and-swap + the conditional If-Match `refs.json`
    /// PUT, so a push on a stale base is rejected non-fast-forward and the client retries.
    /// A spawn failure is non-fatal (the engine degrades to the pre-B5 reboot-refresh).
    fn spawn_refs_refresh_loop(&self) {
        if self.cas_r2_read().is_none() {
            return; // Local/git-dir: no durable manifest store to refresh from
        }
        let Some(tenant) = Self::cas_tenant() else {
            return;
        };
        let state = self.clone();
        let _ = std::thread::Builder::new()
            .name("hugit-refs-refresh".to_string())
            .spawn(move || {
                loop {
                    std::thread::sleep(std::time::Duration::from_millis(REFS_REFRESH_INTERVAL_MS));
                    let Some(r2) = state.cas_r2_read() else {
                        break; // mode changed (never, in practice) → stop cleanly
                    };
                    // Collect (slug, LiveRefs) WITHOUT holding a lock across the R2 reads
                    // (LiveRefs is an Arc clone — cheap; the runtime lock is released first).
                    let mut targets: Vec<(String, LiveRefs)> = state
                        .repos
                        .iter()
                        .map(|(s, rs)| (s.clone(), rs.git_refs.clone()))
                        .collect();
                    {
                        let rt = state
                            .repos_runtime
                            .read()
                            .unwrap_or_else(|e| e.into_inner());
                        targets.extend(rt.iter().map(|(s, rs)| (s.clone(), rs.git_refs.clone())));
                    }
                    for (slug, live) in targets {
                        Self::refresh_repo_refs_once(r2, &tenant, &slug, &live);
                    }
                }
            });
    }

    /// Spawn the GDPR1 erasure AUTO-EXECUTOR — a DETACHED background sweep that drives every
    /// MATURED governing `erasure.requested` to the SAME irreversible cascade the manual
    /// operator route runs, so a real user's Art.17 request AUTO-COMPLETES within a bounded
    /// window (no cron/scheduler otherwise exists — the go-live audit's gap). Structured
    /// EXACTLY like [`spawn_refs_refresh_loop`]: it clones the handles, loops with
    /// `thread::sleep`, and runs the WHOLE cascade ON THIS thread — so its duration NEVER
    /// blocks the single-threaded accept loop. This is ALSO the fix for the manual-cascade
    /// accept-loop DoS the audit found (the O(all-repos) partition scan + per-digest blocking
    /// erase now runs off-loop, never inline on a request).
    ///
    /// FAIL-CLOSED + env-gated: spawns ONLY when [`erasure_auto_execute_enabled`] AND CAS
    /// mode AND [`erase_config`](Self::from_env) is set ([`should_spawn_auto_executor`]).
    /// DEFAULT OFF — a routine deploy never auto-fires an irreversible erasure until the owner
    /// flips it on at go-live. It drives the STANDARD
    /// [`execute_account_erasure_composed`](crate::writes::erasure::execute_account_erasure_composed)
    /// (never a custom path), so every cascade gate — the DSR-id legitimacy, the exact-superset
    /// partition, the durable terminal `executed`/`partial` claim — is enforced UNCHANGED; it
    /// does NOT touch the manual operator route (an in-flight Track-A verify uses that).
    fn spawn_erasure_auto_executor_loop(&self) {
        if !should_spawn_auto_executor(
            erasure_auto_execute_enabled(),
            self.cas_r2_read().is_some(),
            self.erase_config.is_some(),
        ) {
            return; // fail-closed default: OFF (not enabled, or no CAS/erase seam)
        }
        let Some(tenant) = Self::cas_tenant() else {
            return; // CAS mode requires a tenant (belt-and-suspenders with cas_r2_read)
        };
        let state = self.clone();
        let spawn = std::thread::Builder::new()
            .name("hugit-erasure-auto".to_string())
            .spawn(move || {
                eprintln!(
                    "[hugit-serve] GDPR1 erasure auto-executor ENABLED — sweeping every {}s \
                     (off the accept loop)",
                    ERASURE_AUTO_EXECUTE_INTERVAL_MS / 1_000
                );
                loop {
                    std::thread::sleep(Duration::from_millis(ERASURE_AUTO_EXECUTE_INTERVAL_MS));
                    state.run_erasure_auto_sweep(&tenant);
                }
            });
        if spawn.is_err() {
            eprintln!(
                "[hugit-serve] erasure auto-executor thread spawn failed — matured requests \
                 will NOT auto-complete until a reboot (the manual operator route is unaffected)"
            );
        }
    }

    /// ONE reconciliation pass: enumerate account slugs, and for each MATURED governing
    /// request drive the standard cascade. Per-account ISOLATED + fail-closed — a
    /// load/execute fault OR a panic on one account is logged (`eprintln!`) and SKIPPED; it
    /// never halts the sweep or crashes the thread. Bounded per pass
    /// ([`ERASURE_AUTO_EXECUTE_MAX_PER_SWEEP`]); the remainder lands next sweep (idempotent).
    fn run_erasure_auto_sweep(&self, tenant: &str) {
        let slugs = match self.source.list_account_slugs() {
            Ok(s) => s,
            Err(e) => {
                eprintln!(
                    "[hugit-serve] erasure auto-exec: account listing failed ({}) — skipping \
                     this sweep",
                    e.reason
                );
                return;
            }
        };
        let mut driven = 0usize;
        for slug in slugs {
            if driven >= ERASURE_AUTO_EXECUTE_MAX_PER_SWEEP {
                eprintln!(
                    "[hugit-serve] erasure auto-exec: per-sweep cap \
                     ({ERASURE_AUTO_EXECUTE_MAX_PER_SWEEP}) reached — remaining matured requests \
                     run next sweep"
                );
                break;
            }
            let now = now_unix_ms();
            let grace_ms = crate::server::erasure_grace_ms();
            // Belt-and-suspenders isolation: the cascade already fail-closes on every fault,
            // but a per-account catch_unwind guarantees a single account's panic never takes
            // down the sweep thread (matching the other detached loops).
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                self.auto_execute_one(tenant, &slug, grace_ms, now)
            }));
            match outcome {
                Ok(Ok(Some(o))) => {
                    driven += 1;
                    eprintln!("[hugit-serve] erasure auto-exec: account `{slug}` executed → {o:?}");
                }
                Ok(Ok(None)) => {} // not matured / no governing request / no DSR id — skip
                Ok(Err(e)) => {
                    eprintln!(
                        "[hugit-serve] erasure auto-exec: account `{slug}` faulted ({}) — skipped \
                         (sweep continues)",
                        e.reason
                    );
                }
                Err(_) => {
                    eprintln!(
                        "[hugit-serve] erasure auto-exec: account `{slug}` drive PANICKED — \
                         skipped (sweep continues)"
                    );
                }
            }
        }
    }

    /// Drive the cascade for ONE account IFF its standing request is matured + governing +
    /// DSR-legitimate ([`should_auto_execute`](crate::writes::erasure::should_auto_execute)).
    /// `Ok(None)` = intentionally SKIPPED (no governing request, grace not elapsed, or no DSR
    /// id — fail-closed, never a physical erase without an anchored legitimacy id). `Ok(Some)`
    /// = the STANDARD cascade ran (executed/partial/already-executed). `Err` = a load/execute
    /// fault (the caller logs + continues). Drives the EXACT same
    /// [`execute_account_erasure_composed`](crate::writes::erasure::execute_account_erasure_composed)
    /// the operator route uses, as the OPERATOR principal ([`crate::server::dev_principal`]) —
    /// the SUBJECT (whose data is erased) is read from the standing record, never invented.
    /// `pub` so the HOLE-2 background reconciler leg can be driven END-TO-END from an
    /// integration test through the SAME per-account sweep entry the loop calls (never a
    /// direct `execute_account_erasure` primitive call — the audit flagged direct-primitive
    /// false-greens). Prod callers remain the in-crate sweep.
    pub fn auto_execute_one(
        &self,
        tenant: &str,
        slug: &str,
        grace_ms: u64,
        now: u64,
    ) -> Result<Option<crate::writes::erasure::ErasureOutcome>, EngineErr> {
        use crate::writes::erasure::{
            R2OidIndexDigests, execute_account_erasure_composed, read_standing_erasure_request,
            should_auto_execute, should_reconcile_pii,
        };
        let (log, _) = self.load_account_log(slug)?;
        let standing = read_standing_erasure_request(&log);
        let will_auto_execute = should_auto_execute(standing.as_ref(), grace_ms, now);
        if !will_auto_execute {
            // HOLE-2 BACKGROUND PII-COMPLETION RECONCILER. `read_standing_erasure_request`
            // treats `executed` as superseding (returns `None`), so an account whose post-claim
            // `shred_and_redact_on_execute` faulted mid-flight (executed persisted, but cleartext
            // un-redacted + key un-shredded) is INVISIBLE to `should_auto_execute` — the #317
            // self-heal was unreachable in production. Detect that stranded state on the SAME
            // sweep (a SEPARATE path that does NOT touch the standing-request supersession
            // semantics) and idempotently re-drive the PII step to convergence. Fail-closed (a
            // fault just retries next sweep) + idempotent (a converged account no longer matches
            // the predicate, and a still-live re-drive no-ops once the key is shredded).
            if should_reconcile_pii(&log, will_auto_execute) {
                self.reconcile_pii_completion(slug, &log, now)?;
                return Ok(Some(
                    crate::writes::erasure::ErasureOutcome::AlreadyExecuted,
                ));
            }
            return Ok(None);
        }
        // `should_auto_execute` guarantees `Some` with a non-empty `dsr_id`.
        let standing = standing.expect("should_auto_execute implies Some");
        let dsr_id = standing
            .dsr_id
            .as_deref()
            .expect("should_auto_execute implies a non-empty DSR id");
        // Physical erase requires CAS mode + the configured seam. The spawn guard asserted
        // both, but re-check per drive — fail-closed, never a silent no-erase.
        let Some(r2) = self.cas_r2_read() else {
            return Ok(None);
        };
        let Some(erase_cfg) = self.erase_config.as_ref() else {
            return Ok(None);
        };
        let digests = R2OidIndexDigests { r2 };
        let erase = erase_cfg.clone().into_client();
        let outcome = execute_account_erasure_composed(
            self,
            &digests,
            &standing.subject,
            crate::server::dev_principal(),
            now,
            &erase,
            tenant,
            dsr_id,
        )?;
        Ok(Some(outcome))
    }

    /// Idempotently re-drive the provenance-PII completion for an account STRANDED with
    /// `erasure.executed` but no `erasure.pii_shredded` (HOLE-2). Called ONLY by
    /// [`auto_execute_one`](Self::auto_execute_one) after
    /// [`should_reconcile_pii`](crate::writes::erasure::should_reconcile_pii) confirmed the
    /// stranded state — a SEPARATE path from the cascade that never touches the
    /// standing-request supersession semantics.
    ///
    /// Drives the EXISTING idempotent primitive
    /// [`shred_and_redact_on_execute`](crate::provenance_pii_redact::shred_and_redact_on_execute):
    /// it redacts the account log + tenant registry + every tombstoned repo log and shreds the
    /// key LAST, using the None-tolerant read-only key path (already-shredded + accountability
    /// present ⇒ a clean no-op, never a key re-mint). The tombstoned repo slugs are the durable
    /// owned set from [`plan_account_erasure`]; the DSR id is recovered from the (superseded)
    /// `erasure.requested` record so the healed accountability record keeps the original handle.
    ///
    /// Fail-closed: any durable fault returns `Err` (the sweep logs + retries next pass — never
    /// a false "healed"). Needs no CAS-erase transport (the byte-GC already ran before the
    /// `executed` claim); it is a pure durable-log convergence step, valid in any source mode.
    fn reconcile_pii_completion(
        &self,
        account: &str,
        log: &EventLog,
        now: u64,
    ) -> Result<(), EngineErr> {
        let plan = crate::writes::erasure::plan_account_erasure(self, account)?;
        let repo_slugs: Vec<String> = plan.repos.iter().map(|l| l.repo.clone()).collect();
        let dsr_id = crate::writes::erasure::erasure_request_dsr_id(log).unwrap_or_default();
        crate::provenance_pii_redact::shred_and_redact_on_execute(
            self,
            account,
            &repo_slugs,
            &dsr_id,
            now,
        )
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
        // OPTIONAL second operator token — ADDITIVE, never replaces `dev_token`, so a
        // new ops credential can be introduced with zero downtime to the sibling that
        // shares the primary. Absent / whitespace-only → None (no phantom empty token
        // that would match a blank Bearer).
        let dev_token_extra = std::env::var("HUGIT_ENGINE_DEV_TOKEN_EXTRA")
            .ok()
            .filter(|t| !t.trim().is_empty());

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
        // WP-B5: keyed for cross-instance fungibility when HUGIT_ENGINE_TOKEN_KEY is
        // shared across instances (else a per-boot random key = single-host, as before).
        let token_store = Arc::new(TokenStore::from_env());

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
        let (repos, selfcheck_probe) = Self::load_repos_from_env()?;
        // FIX: the CAS `batch_read` self-probe runs OFF the boot path. The shared
        // health cell starts at `"probing"` when a CAS repo is loaded (a detached
        // thread overwrites it when the probe lands), else `"unprobed"`. The probe
        // once ran SYNCHRONOUSLY here (~18s at chunk 256) and blew the Cloudflare
        // Container startup deadline → crash-loop → outage; boot now NEVER blocks on it.
        let cas_batch_read_health = Arc::new(RwLock::new(
            if selfcheck_probe.is_some() {
                "probing"
            } else {
                "unprobed"
            }
            .to_string(),
        ));
        if let Some(probe) = selfcheck_probe {
            let health = Arc::clone(&cas_batch_read_health);
            let spawn = std::thread::Builder::new()
                .name("hugit-cas-selfprobe".into())
                .spawn(move || {
                    // A probe panic is ISOLATED to this thread (never crashes boot):
                    // run under catch_unwind → an unwind records an honest err string
                    // rather than propagating. The detached thread outlives boot; the
                    // shared `Arc` keeps the health cell alive.
                    let result =
                        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| probe.run()))
                            .unwrap_or_else(|_| "err:selfprobe-panicked".to_string());
                    *health.write().unwrap_or_else(|e| e.into_inner()) = result;
                });
            if spawn.is_err() {
                // Thread exhaustion: record an honest err (the cell would otherwise stay
                // "probing" forever). The un-run closure + its `Arc` clone were dropped.
                *cas_batch_read_health
                    .write()
                    .unwrap_or_else(|e| e.into_inner()) = "err:selfprobe-spawn-failed".to_string();
                eprintln!("hugit-serve: CAS batch_read self-probe thread spawn failed");
            }
        }
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

        // PAT git/API auth (slice 2b) — DISABLED by default (`HUGIT_SERVE_PAT_AUTH=1`
        // to enable). Fail-closed on multi-instance: the in-memory index is
        // single-instance-authoritative, so refuse to boot with PAT auth on AND >1
        // instances permitted (a PAT minted on A would be absent from B until B
        // reboots — gated on the B5 read-after-write seam).
        let pat_auth_enabled = pat_auth_enabled_env();
        pat_auth_multi_instance_guard(pat_auth_enabled, allow_multi_instance())?;

        // Fail-closed: on a multi-instance deploy the engine-token signing key MUST be the
        // SHARED `HUGIT_ENGINE_TOKEN_KEY` (else `TokenStore::from_env` falls back to a
        // per-boot random key per instance → cross-instance 401s, the pre-#128 failure the
        // stateless HMAC token closed). Refuse to boot when >1 instances are permitted but
        // the key is absent/blank.
        let engine_token_key_present = std::env::var("HUGIT_ENGINE_TOKEN_KEY")
            .map(|v| !v.trim().is_empty())
            .unwrap_or(false);
        engine_token_key_multi_instance_guard(allow_multi_instance(), engine_token_key_present)?;

        // The physical CAS-erase seam config (GDPR1 slice-2). Fail-closed: a set-but-invalid
        // URL/key aborts boot (never a half-configured erase seam). Absent → `None` (the
        // operator-execute route is disabled). This is the ONE irreversible-delete seam.
        let erase_config = crate::writes::erasure::EraseConfig::from_env()?;

        let state = Self {
            source,
            dev_token,
            dev_token_extra,
            exchange,
            token_store,
            repos,
            repos_runtime: Arc::new(RwLock::new(std::collections::HashMap::new())),
            repo_load_misses: Arc::new(RwLock::new(std::collections::HashMap::new())),
            repo_lazy_load_budget: Arc::new(RwLock::new((0, 0))),
            provision,
            write_path_enabled,
            // PROD default: OFF. Without `HUGIT_ALLOW_DEV_OPERATOR=1` the dev-token
            // is NOT a god-token — the public door has zero operator god-path.
            allow_dev_operator: dev_operator_allowed(),
            cas_batch_read_health,
            clone_pack_building: Arc::new(Mutex::new(HashSet::new())),
            pat_auth_enabled,
            pat_index: Arc::new(RwLock::new(std::collections::HashMap::new())),
            pat_last_used: Arc::new(RwLock::new(std::collections::HashMap::new())),
            erase_config,
            blob_history_index: crate::blob_history_index::BlobHistoryStore::new(),
            search_index: crate::search_index::SearchStore::new(),
            home_cache: crate::home_cache::HomeRenderCache::new(),
            repo_meta_cache: Arc::new(RwLock::new(std::collections::HashMap::new())),
            storage_quota_locks: Arc::new(Mutex::new(std::collections::HashMap::new())),
        };
        // Populate the PAT index from the durable `_accounts/*` logs (only when
        // enabled). Boot-scan faults are fail-closed-DENY (the affected PATs simply
        // won't authenticate → 401) and NEVER crash boot — PAT auth is an additive,
        // flag-gated surface, not a boot prerequisite.
        if pat_auth_enabled {
            state.boot_build_pat_index();
        }
        // Task #74 (W-METENANT scaling follow-up): populate the repo-meta cache once
        // at boot for every loaded repo, so the very first `/v1/me/*` request already
        // hits the cache instead of a cold per-repo log walk. Best-effort — an
        // unloadable candidate is simply left uncached (the fail-safe
        // `repo_meta_cached` fallback covers it, identical to pre-cache behavior).
        state.boot_populate_repo_meta_cache();
        // WP-B5: start the background ref-cache refresher (read-after-write fungibility).
        // NO-OP outside CAS mode; strictly off the accept loop. Spawn LAST (after the state
        // is fully built) so the thread sees the loaded repos.
        state.spawn_refs_refresh_loop();
        // #76: heal the per-tenant repo registry toward genesis truth (register-side undercount
        // + legacy git-ingest back-fill). Detached, catch_unwind-isolated, bounded, CAS-mode
        // only; NEVER blocks boot. Spawn LAST (after the state is fully built).
        state.spawn_boot_reconcile_tenant_registry();
        // GDPR1 Art.17 auto-executor (env-gated, fail-closed DEFAULT OFF). A detached
        // off-loop sweep that auto-completes matured erasure requests (no cron otherwise) AND
        // moves the irreversible cascade off the single-threaded accept loop (the DoS fix).
        // NO-OP unless HUGIT_ERASURE_AUTO_EXECUTE=1 AND CAS mode AND the erase seam is
        // configured. Spawn LAST (after the state is fully built).
        state.spawn_erasure_auto_executor_loop();
        Ok(state)
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
        // The clone-pack cache rides the SAME write-scoped seam (absent when
        // receive-pack is off → no write cred → no cache, slow-walk fallback).
        let clone_cache = cas_write.as_ref().map(CasWriteSeam::clone_cache_seam);
        Some(RepoState {
            git_source,
            git_root_tree,
            git_refs: LiveRefs::new(BTreeMap::new()),
            git_dir: None,
            cas_write,
            live_oid_index: Some(live_oid_index),
            clone_cache,
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
        // PR-1b: normalize slug so a caller passing "foo.git" stores under "foo",
        // matching `resolve_repo_slug`'s lookup. `normalize_repo_slug` is defined
        // below (alongside `is_safe_repo_slug`); we use it here to ensure
        // `insert_runtime_repo` agrees with `resolve_repo_slug` on the canonical
        // form. The empty result is invalid — is_safe_repo_slug rejects "".
        let normalized = crate::state::normalize_repo_slug(slug);
        if normalized.is_empty() {
            return Err(EngineErr::invalid_request(
                "repository slug normalizes to empty",
            ));
        }
        if self.repos.contains_key(&normalized) {
            return Err(EngineErr::cas_conflict());
        }
        let mut guard = self
            .repos_runtime
            .write()
            .unwrap_or_else(|e| e.into_inner());
        if guard.contains_key(&normalized) {
            return Err(EngineErr::cas_conflict());
        }
        // Leak: a provisioned forge repo is served for the engine's whole lifetime
        // (no runtime de-provision in v0), so the box is never freed — this is exact,
        // not a mistake, and is what lets `repo_state` return a `&RepoState`.
        let leaked: &'static RepoState = Box::leak(Box::new(repo));
        guard.insert(normalized, leaked);
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
    ///
    /// Also returns a DETACHABLE CAS `batch_read` self-probe for the FIRST CAS-backed
    /// repo ([`crate::cas::CasSelfcheckProbe`]), or `None` when no CAS-backed repo is
    /// loaded (Local/git-dir mode). The caller ([`Self::from_env`]) runs it on a
    /// detached thread OFF the boot path (FIX: the synchronous probe blew the container
    /// startup deadline). This function itself makes NO probe network call — it only
    /// captures the handle; boot is never blocked by it.
    fn load_repos_from_env() -> Result<
        (
            std::collections::HashMap<String, RepoState>,
            Option<crate::cas::CasSelfcheckProbe>,
        ),
        String,
    > {
        let mut repos = std::collections::HashMap::new();
        let mut selfcheck_probe: Option<crate::cas::CasSelfcheckProbe> = None;

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
                // are fetched from the CAS on demand at serve time. The SAME builder
                // the lazy-load-on-miss path uses ([`build_cas_repo_state`]), so a
                // boot repo and a runtime-discovered repo are constructed identically.
                // Boot can afford the standard timeouts + throttle-retry (it is off the
                // request path); the typed load error maps to the fatal boot string.
                let (repo_state, probe) = build_cas_repo_state(
                    &cas,
                    &r2,
                    &tenant,
                    repo,
                    receive_pack_enabled(),
                    crate::cas::LoadMode::Boot,
                )
                .map_err(|e| format!("boot-load of CAS repo {repo:?}: {e}"))?;
                // Capture the boot CAS connectivity self-probe for the FIRST CAS repo
                // ONLY (additional repos share the same CAS client → a second probe is
                // redundant). It is NOT run here — the caller spawns it on a detached
                // thread OFF the boot path (FIX: the synchronous probe blew the
                // container startup deadline). Never fails boot.
                if selfcheck_probe.is_none() {
                    selfcheck_probe = Some(probe);
                }
                repos.insert(repo.to_string(), repo_state);
            }
            if repos.is_empty() {
                return Err("HUGIT_SERVE_CAS_REPO is empty (CAS source selected)".to_string());
            }
            return Ok((repos, selfcheck_probe));
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
                    // Snapshot + LIVE loose-object fallback so a pushed tip
                    // (object plane updated after boot) clones back with NO reboot.
                    let src: Arc<dyn hugit_proto::ObjectSource + Send + Sync> =
                        Arc::new(GitDirLooseObjectSource::new(cas, dir));
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
                            // GIT_DIR mode has no CAS/R2 write seam → no clone-pack cache
                            // (a clone slow-walks the on-disk git objects, which is fast).
                            clone_cache: None,
                        },
                    );
                }
                if repos.is_empty() {
                    return Err("HUGIT_SERVE_GIT_DIR is empty".to_string());
                }
                // GIT_DIR mode has no CAS batch-read plane → no probe (`/readyz` shows
                // "unprobed"). `selfcheck_probe` stays `None`.
                Ok((repos, None))
            }
            // No content seam configured → the honest no-git default (no probe).
            _ => Ok((repos, None)),
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
            dev_token_extra: None,
            exchange: None,
            token_store: Arc::new(TokenStore::new()),
            repos: std::collections::HashMap::new(),
            repos_runtime: Arc::new(RwLock::new(std::collections::HashMap::new())),
            repo_load_misses: Arc::new(RwLock::new(std::collections::HashMap::new())),
            repo_lazy_load_budget: Arc::new(RwLock::new((0, 0))),
            provision: None,
            write_path_enabled: false,
            // The explicit dev/test/seed constructor enables the break-glass by
            // default (production boots via `from_env`, which is default-OFF). A
            // test asserting the no-god-path (flag-OFF) behavior sets this to
            // `false` on the returned state.
            allow_dev_operator: true,
            cas_batch_read_health: Arc::new(RwLock::new("unprobed".to_string())),
            clone_pack_building: Arc::new(Mutex::new(HashSet::new())),
            // PAT auth OFF by default even in the dev/test constructor — a test that
            // exercises the PAT path flips `pat_auth_enabled` on the returned state.
            pat_auth_enabled: false,
            pat_index: Arc::new(RwLock::new(std::collections::HashMap::new())),
            pat_last_used: Arc::new(RwLock::new(std::collections::HashMap::new())),
            // No physical erase seam in the dev/test constructor (the operator-execute
            // route is disabled). A test wiring the executor sets it explicitly.
            erase_config: None,
            blob_history_index: crate::blob_history_index::BlobHistoryStore::new(),
            search_index: crate::search_index::SearchStore::new(),
            home_cache: crate::home_cache::HomeRenderCache::new(),
            // Empty: `new()`'s boot `repos` set is always empty too (tests wire repos
            // via `set_repo_git`/`insert_runtime_repo`, not a real boot load), so there
            // is nothing to pre-populate. A test exercising the cache calls
            // `cache_repo_meta`/`refresh_repo_meta_cache` explicitly.
            repo_meta_cache: Arc::new(RwLock::new(std::collections::HashMap::new())),
            storage_quota_locks: Arc::new(Mutex::new(std::collections::HashMap::new())),
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

    /// [`repo_state`](Self::repo_state), then a LAZY-LOAD-ON-MISS from the durable R2
    /// manifests (#96). A repo provisioned via `POST /v1/repos` lands ONLY in the
    /// creating instance's [`repos_runtime`](Self::repos_runtime) (leaked `&'static`)
    /// and is NOT in any other instance's boot set — so at `max_instances ≥ 2` (or
    /// after a reboot) another instance `repo_state`-misses it and 404s. This closes
    /// that: on a miss, reconstruct the repo from R2 via the boot builder
    /// ([`build_cas_repo_state`]) + register it, so ANY instance serves ANY durably
    /// provisioned repo on first request — no reboot, no `HUGIT_SERVE_CAS_REPO` edit.
    ///
    /// FAIL-CLOSED + accept-loop-SAFE:
    /// - Only in CAS mode ([`provision`](Self::provision) `Some`); Local/git-dir has a
    ///   fixed on-disk set → straight `repo_state`.
    /// - An unsafe slug → `None` (never an R2 probe for a traversal string).
    /// - A NEGATIVE CACHE ([`repo_load_misses`](Self::repo_load_misses)) skips the R2
    ///   probe for a slug found AUTHORITATIVELY absent within [`REPO_LOAD_MISS_COOLDOWN_MS`].
    /// - A GLOBAL budget ([`repo_lazy_load_budget`](Self::repo_lazy_load_budget), L2)
    ///   bounds how many attempts per [`LAZY_LOAD_WINDOW_MS`] touch R2 — so a VARIED-slug
    ///   404 storm (which the per-slug cache cannot stop) cannot flood the accept loop.
    /// - The load is BOUNDED ([`LoadMode::LazyLoad`](crate::cas::LoadMode::LazyLoad), L1):
    ///   no throttle-retry sleeps + a tight timeout, so it never stalls the loop ~12s+.
    /// - Only an AUTHORITATIVELY-ABSENT ([`RepoLoadError::Absent`](crate::cas::RepoLoadError::Absent))
    ///   result is negative-cached (L3); a TRANSIENT fault returns `None` WITHOUT caching
    ///   (the next request retries — a real repo is never false-404'd for the cooldown).
    /// - W1: an ABSENT `refs.json` whose durable GENESIS LOG exists (provisioned but never
    ///   pushed) is served as an EMPTY CAS seam (so first push/clone work on ANY instance),
    ///   not 404'd.
    /// - The single-threaded accept loop means no same-instance race on the load/insert.
    #[must_use]
    pub fn repo_state_or_load(&self, repo: &str) -> Option<&RepoState> {
        if let Some(r) = self.repo_state(repo) {
            return Some(r);
        }
        // CAS mode only + a store-safe slug (never probe R2 for a traversal string).
        let tmpl = self.provision.as_ref()?;
        if !is_safe_repo_slug(repo) {
            return None;
        }
        // Negative cache: a recent AUTHORITATIVE miss short-circuits without touching R2.
        if self.repo_load_miss_recent(repo) {
            return None;
        }
        // L2: consume a global budget token. Over budget → None FAST, no R2 round-trip
        // (checked BEFORE `build_cas_repo_state`, so an over-budget attempt never touches
        // R2). `decide_lazy_load` re-checks this and refuses to invoke the loader.
        let budget_ok = self.lazy_load_budget_take();
        let act = decide_lazy_load(
            budget_ok,
            // The bounded (no-retry, tight-timeout) lazy-load — invoked ONLY when in
            // budget (the closure is not called otherwise).
            || {
                build_cas_repo_state(
                    &tmpl.cas_client,
                    &tmpl.r2,
                    &tmpl.tenant,
                    repo,
                    tmpl.receive_pack_enabled,
                    crate::cas::LoadMode::LazyLoad,
                )
                .map(|(state, _probe)| state)
            },
            // W1 genesis-existence: the authoritative durable log predicate the rest of
            // the engine uses (`load_verified` → `source.fetch` → `Ok(Some)` iff seeded).
            || self.load_verified(repo).is_ok(),
        );
        match act {
            LazyLoadAct::Insert(repo_state) => {
                // Register the discovered repo. `insert_runtime_repo` is create-only;
                // a `CAS_CONFLICT` just means it is already present (benign) — either
                // way re-read through `repo_state` for the leaked `&'static` borrow.
                let _ = self.insert_runtime_repo(repo, *repo_state);
                self.repo_state(repo)
            }
            LazyLoadAct::InsertEmpty => {
                // W1: provisioned-but-never-pushed on another instance — mint the SAME
                // empty CAS seam `provision()` would, so the first push/clone works here.
                let repo_state = self.build_empty_cas_repo_state(repo)?;
                let _ = self.insert_runtime_repo(repo, repo_state);
                self.repo_state(repo)
            }
            LazyLoadAct::NegativeCache => {
                // Authoritatively absent (refs.json 404, no genesis log) → cache + 404.
                self.mark_repo_load_miss(repo);
                None
            }
            // Transient fault or over budget → honest miss, NO negative cache (retry next).
            LazyLoadAct::NoCacheRetry => None,
        }
    }

    /// Consume one token from the global lazy-load budget (L2). Returns `true` iff an
    /// attempt is allowed to touch R2 this window; `false` once
    /// [`MAX_LAZY_LOADS_PER_WINDOW`] have been spent in the current [`LAZY_LOAD_WINDOW_MS`].
    /// A fixed-window counter — a new window resets it. Single-threaded loop → race-free.
    fn lazy_load_budget_take(&self) -> bool {
        let now = now_ms();
        let mut g = self
            .repo_lazy_load_budget
            .write()
            .unwrap_or_else(|e| e.into_inner());
        let (window_start, count) = *g;
        if now.saturating_sub(window_start) >= LAZY_LOAD_WINDOW_MS {
            // A fresh window — reset and spend this token.
            *g = (now, 1);
            true
        } else if count < MAX_LAZY_LOADS_PER_WINDOW {
            *g = (window_start, count + 1);
            true
        } else {
            false
        }
    }

    /// G11 — resolve a request's repo id (a BARE name from the URL/wire, or an already
    /// user-scoped/explicit stored slug) to the STORED slug all downstream loads key on,
    /// honoring the user-scoped namespace WITH legacy back-compat:
    ///
    /// 1. an already-scoped/multi-segment slug (contains `/`) is used VERBATIM — the
    ///    operator / an internal caller may address the full stored key directly;
    /// 2. a TENANT caller PREFERS their user-scoped key `<tenant>/<name>` when it EXISTS
    ///    (an in-memory seam or a durable verified log) — so the owner reaches their own
    ///    repo by the bare name and two tenants' identically-named repos never collide;
    /// 3. BACK-COMPAT FALLBACK: otherwise the legacy flat `<name>` key — so every repo
    ///    provisioned under the pre-G11 flat scheme keeps resolving unchanged (no
    ///    destructive migration), and a genuinely-absent name resolves here too → an
    ///    honest 404 downstream.
    ///
    /// The security consequence (audit G11): a cross-tenant / anon caller composing a
    /// bare name never resolves ANOTHER tenant's user-scoped repo — the scoped key is
    /// keyed on the CALLER's tenant, and the legacy fallback only ever reaches a
    /// pre-G11 flat repo (whose own `authorize_read`/`authorize_write` gate still
    /// decides). New user-scoped repos are addressable by the bare name ONLY by their
    /// owning tenant (a composite `<owner>/<name>` id is not routable over the
    /// single-segment git wire — the documented v0 limitation).
    #[must_use]
    pub fn resolve_repo_slug(&self, name: &str, principal: &[String]) -> String {
        let name = normalize_repo_slug(name);
        if name.is_empty() {
            return String::new();
        }
        if name.contains('/') {
            return name;
        }
        if let Some(tenant) = tenant_org(principal) {
            let scoped = format!("{tenant}/{name}");
            if self.repo_slug_exists(&scoped) {
                return scoped;
            }
        }
        name.to_string()
    }

    /// Whether a STORED slug currently resolves to a real repo — a loaded in-memory seam
    /// (cheap, no I/O) or a durable chain-verified log (one bounded probe). The existence
    /// oracle [`resolve_repo_slug`](Self::resolve_repo_slug) consults to decide the
    /// user-scoped-vs-legacy key. A tamper/transport fault (non-404) counts as "does not
    /// resolve here" so resolution falls through to the legacy key (fail-safe: never
    /// strand a legacy repo behind a transient scoped-key read fault).
    #[must_use]
    fn repo_slug_exists(&self, slug: &str) -> bool {
        self.has_repo_seam(slug) || self.load_verified(slug).is_ok()
    }

    /// Whether `repo` was lazy-load-missed within [`REPO_LOAD_MISS_COOLDOWN_MS`] (the
    /// negative-cache short-circuit for [`repo_state_or_load`]).
    fn repo_load_miss_recent(&self, repo: &str) -> bool {
        let now = now_ms();
        self.repo_load_misses
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(repo)
            .is_some_and(|&at| now.saturating_sub(at) < REPO_LOAD_MISS_COOLDOWN_MS)
    }

    /// Record a lazy-load MISS for `repo` (negative cache; see [`repo_load_miss_recent`]).
    fn mark_repo_load_miss(&self, repo: &str) {
        self.repo_load_misses
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert(repo.to_string(), now_ms());
    }

    /// Boot-time [`repo_meta_cache`](Self::repo_meta_cache) population (task #74):
    /// project every LOADED boot repo's [`RepoMeta`](crate::authz::RepoMeta) ONCE, so
    /// the very first `/v1/me/*` request after boot already hits the cache. Computes
    /// every candidate BEFORE taking the write lock (never holds it across the
    /// per-repo load/verify I/O — same discipline as
    /// [`spawn_refs_refresh_loop`](Self::spawn_refs_refresh_loop)). Best-effort: an
    /// unloadable/untrusted candidate log is simply SKIPPED (never cached) — the
    /// fail-safe [`repo_meta_cached`](Self::repo_meta_cached) fallback covers it on
    /// the next read, identical to the pre-cache behavior.
    fn boot_populate_repo_meta_cache(&self) {
        let names: Vec<String> = self.repos.keys().cloned().collect();
        let computed: Vec<(String, crate::authz::RepoMeta)> = names
            .into_iter()
            .filter_map(|name| {
                self.load_verified(&name)
                    .ok()
                    .map(|log| (name, crate::authz::project_repo_meta(&log)))
            })
            .collect();
        let mut cache = self
            .repo_meta_cache
            .write()
            .unwrap_or_else(|e| e.into_inner());
        for (name, meta) in computed {
            cache.insert(name, meta);
        }
    }

    /// Read `repo`'s projected [`RepoMeta`](crate::authz::RepoMeta) from the
    /// [`repo_meta_cache`](Self::repo_meta_cache); on a cache MISS, project it LIVE
    /// from the caller-supplied (already loaded/verified) `log` — the EXACT
    /// computation every caller ran before this cache existed. A miss can only ever
    /// cost the same as today, never a wrong answer: this is the fail-safe that makes
    /// every write-path invalidation a performance concern only, never a correctness
    /// one.
    #[must_use]
    pub fn repo_meta_cached(&self, repo: &str, log: &EventLog) -> crate::authz::RepoMeta {
        if let Some(meta) = self
            .repo_meta_cache
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(repo)
        {
            return meta.clone();
        }
        crate::authz::project_repo_meta(log)
    }

    /// Cache an ALREADY-COMPUTED [`RepoMeta`](crate::authz::RepoMeta) for `repo` —
    /// the provisioning hook: `write_provision::provision` builds + durably persists
    /// a repo's genesis log itself, so it can project the meta from that in-memory
    /// log directly (no redundant reload) and hand it here immediately after the
    /// durable commit succeeds.
    pub fn cache_repo_meta(&self, repo: &str, meta: crate::authz::RepoMeta) {
        self.repo_meta_cache
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert(repo.to_string(), meta);
    }

    /// The write-path invalidation hook (task #74's hard constraint): recompute +
    /// store `repo`'s [`RepoMeta`](crate::authz::RepoMeta) from its CURRENT durable
    /// log. Call this immediately after ANY durable write that can change a repo's
    /// projected meta — a `repo.meta` visibility/owner-tenant update, or a GDPR1
    /// `repo.erased` tombstone — so the cache can never diverge from the log for
    /// longer than one write.
    ///
    /// **Fail-safe on the reload itself:** if the post-write load/verify fails (never
    /// expected right after a durable persist, but never assumed), the entry is
    /// REMOVED rather than left holding a pre-write value — a subsequent read falls
    /// back to a live projection ([`repo_meta_cached`](Self::repo_meta_cached))
    /// instead of ever serving a stale cached answer.
    pub fn refresh_repo_meta_cache(&self, repo: &str) {
        match self.load_verified(repo) {
            Ok(log) => {
                let meta = crate::authz::project_repo_meta(&log);
                self.repo_meta_cache
                    .write()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(repo.to_string(), meta);
            }
            Err(_) => {
                self.repo_meta_cache
                    .write()
                    .unwrap_or_else(|e| e.into_inner())
                    .remove(repo);
            }
        }
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

        // Operator: all loaded repos, bypassing the per-repo VISIBILITY gate (an
        // operator legitimately sees PRIVATE repos — that is the ops role). But a
        // GDPR1-erased repo is TERMINAL and must be gone from EVERY view, the
        // operator's included (#91): `authorize_read`/`authorize_write` already deny
        // an erased repo to everyone incl. the operator, so leaking its slug via
        // `/v1/me/*` while every other path 404s it is a real projection hole. So
        // the operator branch consults the SAME cached meta the tenant branch uses
        // and excludes ONLY `erased` (never visibility). An unloadable log is
        // skipped fail-closed (it cannot be aggregated anyway).
        if crate::authz::is_operator(principal) {
            return names
                .into_iter()
                .filter_map(|n| {
                    let log = self.load_verified(&n).ok()?;
                    if self.repo_meta_cached(&n, &log).erased {
                        return None; // erasure is terminal — gone from the operator view too
                    }
                    Some((n, log))
                })
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
            // Task #74: the cache, with a live fallback — never a different answer.
            let meta = self.repo_meta_cached(&name, &log);
            // THE SAME predicate the per-repo read gate runs — no second gate.
            if crate::authz::authorize_read(principal, &meta) {
                // G11: project the BARE display name (strip the caller's own
                // `<tenant>/` scope prefix) so the identity-scoped index stays
                // routable — the owner navigates/clones their repo by the bare name,
                // which `resolve_repo_slug` maps back to the user-scoped stored key. A
                // legacy flat slug has no prefix → returned unchanged.
                out.push((display_slug(&name, principal), log));
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

    /// Count the repos OWNED by `owner_tenant` — the denominator of the per-tenant DoS cap
    /// ([`MAX_REPOS_PER_TENANT`]) the provision path enforces BEFORE it leaks a
    /// `&'static RepoState` / writes a durable genesis.
    ///
    /// **WP-1 repoint (defects A + B):** this reads the DURABLE, chain-verified per-tenant
    /// registry (`_tenants/{owner_tenant}.json`, [`crate::tenant_registry`]) — ONE small
    /// per-tenant log fetch, O(1) w.r.t. the platform. It replaces the prior implementation
    /// that:
    /// - walked the in-memory boot∪runtime union, calling `load_verified` (a chain-verified
    ///   R2 fetch) per candidate ON THE ACCEPT LOOP — an O(all-platform-repos) scan (defect
    ///   B); AND
    /// - lost its count on a reboot (the runtime overlay is in-memory), so a tenant could
    ///   re-provision past the cap after a restart (defect A). The registry is DURABLE, so
    ///   the count is now stable across engine lifetimes.
    ///
    /// FAIL-CLOSED: returns `None` if the registry read/verify is indeterminate (a transport
    /// fault or a tamper/parse failure). An indeterminate count MUST refuse the create
    /// (never allow-by-default), so a read fault can never be leveraged to slip past the
    /// cap. An ABSENT registry is NOT indeterminate — it is a genuine empty owned-set
    /// (`Some(0)`) for a new tenant.
    #[must_use]
    pub fn count_owned_repos(&self, owner_tenant: &str) -> Option<usize> {
        let (log, _) = self.load_tenant_registry(owner_tenant).ok()?;
        Some(crate::tenant_registry::count(&log))
    }

    /// The per-`key` advisory lock that serializes the G10 storage-cap
    /// CHECK→`size.json`-COMMIT critical section (see
    /// [`storage_quota_locks`](Self::storage_quota_locks)). `key` is the owner_tenant
    /// (so ALL of an owner's pushes — even to different repos — serialize against the
    /// SAME per-owner_tenant aggregate) or, for a legacy/unowned repo, a repo-scoped
    /// fallback key. Returns the shared `Arc<Mutex<()>>`; the caller `.lock()`s it and
    /// holds the guard across BOTH the quota check and the durable `size.json` commit so
    /// two concurrent same-key pushes cannot both pass the cap and then overshoot. The
    /// outer registry `Mutex` is held only for this O(1) get-or-insert, NEVER across the
    /// push itself.
    #[must_use]
    pub fn storage_quota_lock(&self, key: &str) -> Arc<Mutex<()>> {
        let mut map = self
            .storage_quota_locks
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        Arc::clone(
            map.entry(key.to_string())
                .or_insert_with(|| Arc::new(Mutex::new(()))),
        )
    }

    /// Enumerate the `(slug, EventLog)` of every repo OWNED by `owner_tenant` — the
    /// authoritative candidate set (boot [`repos`] ∪ runtime overlay) filtered by the
    /// genesis `repo.meta{owner_tenant}` projection. The `(slug, log)`-returning twin
    /// of [`count_owned_repos`](Self::count_owned_repos), for the GDPR1 erasure PLANNER
    /// (it needs each owned repo's slug + head to build the tombstone plan).
    ///
    /// FAIL-CLOSED (same as `count_owned_repos`): returns `None` if ANY candidate log
    /// cannot be loaded/verified — an indeterminate ownership enumeration MUST NOT
    /// silently under-report the erasure set (a subject's repo must never be missed
    /// because of a transient read fault). The caller treats `None` as "cannot plan
    /// safely → refuse", never "no repos".
    ///
    /// Read-only; enumerates only the in-memory loaded set (never an R2 listing scan).
    #[must_use]
    pub fn owned_repo_logs(&self, owner_tenant: &str) -> Option<Vec<(String, EventLog)>> {
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

        let mut out: Vec<(String, EventLog)> = Vec::new();
        for name in names {
            // Fail-closed: an unloadable/untrusted candidate ⇒ indeterminate set.
            let log = self.load_verified(&name).ok()?;
            // Task #74: the cache, with a live fallback — never a different answer.
            let meta = self.repo_meta_cached(&name, &log);
            if meta.owner_tenant.as_deref() == Some(owner_tenant) {
                out.push((name, log));
            }
        }
        Some(out)
    }

    /// The DURABLE authoritative `(slug, EventLog)` of every repo owned by
    /// `owner_tenant` — the completeness-correct enumeration for GDPR1 erasure (the B1
    /// audit fix). Unlike [`owned_repo_logs`](Self::owned_repo_logs) (in-memory loaded
    /// set only), this UNIONS the durable store listing ([`LogSource::list_repo_slugs`])
    /// with the in-memory loaded set, so it can NEVER miss:
    /// - a **durable-but-unloaded** repo (provisioned, then dropped from the boot env —
    ///   its R2 log survives, its git seam is gone) — caught by the durable listing;
    /// - a **just-provisioned** repo not yet visible to an eventually-consistent listing
    ///   — caught by the in-memory union.
    ///
    /// FAIL-CLOSED: any listing fault OR any candidate log that will not load/verify →
    /// `Err(503)`. A subject must NEVER be told "erased" while a durable repo of theirs
    /// survives because the enumeration was silently incomplete (the worst erasure bug).
    ///
    /// Cost: one bounded durable listing + one verified load per candidate — acceptable
    /// on the rare, authorized erasure path (NOT a hot read).
    ///
    /// **Deliberately NOT wired to [`repo_meta_cache`](Self::repo_meta_cache) (task
    /// #74):** this runs on the irreversible GDPR1 erasure cascade, where a live
    /// `project_repo_meta` per candidate is cheap relative to the physical-GC legs it
    /// gates — reusing the cache here would buy no measurable perf and would add a
    /// second place to reason about cache freshness on the codebase's most
    /// security-sensitive path, for zero benefit.
    pub fn authoritative_owned_repo_logs(
        &self,
        owner_tenant: &str,
    ) -> Result<Vec<(String, EventLog)>, EngineErr> {
        // Durable listing ∪ in-memory loaded set (dedup) — the complete candidate set.
        let mut names: Vec<String> = self.source.list_repo_slugs()?;
        names.extend(self.repos.keys().cloned());
        {
            let runtime = self.repos_runtime.read().unwrap_or_else(|e| e.into_inner());
            names.extend(runtime.keys().cloned());
        }
        names.sort();
        names.dedup();

        let mut out = Vec::new();
        for name in names {
            // Fail-closed: an unloadable candidate is indeterminate — refuse the whole
            // plan (never under-report). An absent log (a listing/runtime race where the
            // object vanished) is a genuine 404 → skip; a 5xx propagates.
            match self.load_verified(&name) {
                Ok(log) => {
                    let meta = crate::authz::project_repo_meta(&log);
                    if meta.owner_tenant.as_deref() == Some(owner_tenant) {
                        out.push((name, log));
                    }
                }
                Err(e) if e.status == 404 => continue,
                Err(e) => return Err(e),
            }
        }
        Ok(out)
    }

    /// GDPR1 slice-2 (clw's #1 blast-radius bar): partition EVERY repo in the tenant into
    /// the SUBJECT's repos vs the SURVIVING repos, from the SAME durable authoritative
    /// candidate set as [`authoritative_owned_repo_logs`] (`list_repo_slugs()` — the
    /// durable R2 listing — ∪ the in-memory loaded overlay) with the SAME fail-closed
    /// discipline. Returns `(subject_repos, surviving_repos)`.
    ///
    /// The exclusive-digest partition is only safe if the SURVIVING set is COMPLETE: an
    /// omitted surviving repo → a shared digest mis-classified exclusive → a retained
    /// user's data physically erased. So this NEVER returns a partial surviving set — an
    /// unloadable candidate (5xx) propagates (`Err`), exactly like its sibling. Two
    /// fail-safe choices: (a) an UNOWNED repo (no `owner_tenant`) is classified SURVIVING
    /// (when ownership is absent, RETAIN its digests — never delete on ambiguity); (b) a
    /// 404 (a listing/runtime race where the object vanished) is skipped from BOTH sides,
    /// which is safe — a repo that no longer exists references nothing.
    ///
    /// # Errors
    /// `503` — any listing/load fault (fail-closed; the caller MUST NOT then erase).
    ///
    /// **Deliberately NOT wired to [`repo_meta_cache`](Self::repo_meta_cache)** — same
    /// reasoning as [`authoritative_owned_repo_logs`](Self::authoritative_owned_repo_logs):
    /// not a hot read, and this partition runs inside the SAME cascade that tombstones
    /// repos, so keeping it on a live projection sidesteps any question of whether an
    /// in-cascade cache refresh landed before this call reads it.
    pub fn erasure_repo_partition(
        &self,
        subject: &str,
    ) -> Result<(Vec<String>, Vec<String>), EngineErr> {
        let mut names: Vec<String> = self.source.list_repo_slugs()?;
        names.extend(self.repos.keys().cloned());
        {
            let runtime = self.repos_runtime.read().unwrap_or_else(|e| e.into_inner());
            names.extend(runtime.keys().cloned());
        }
        names.sort();
        names.dedup();

        let mut subject_repos = Vec::new();
        let mut surviving_repos = Vec::new();
        for name in names {
            match self.load_verified(&name) {
                Ok(log) => {
                    let meta = crate::authz::project_repo_meta(&log);
                    if meta.owner_tenant.as_deref() == Some(subject) {
                        subject_repos.push(name);
                    } else {
                        // Other-owned OR unowned → SURVIVING (retain — fail-safe).
                        surviving_repos.push(name);
                    }
                }
                Err(e) if e.status == 404 => continue, // vanished between list + load
                Err(e) => return Err(e),               // 5xx → fail-closed, never partial
            }
        }
        Ok((subject_repos, surviving_repos))
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
    ///
    /// PR-1b: normalizes the slug via `normalize_repo_slug` so a caller passing
    /// "foo.git" stores under "foo" (matching `resolve_repo_slug`'s lookup).
    pub fn set_repo_git(
        &mut self,
        repo: impl Into<String>,
        git_source: Arc<dyn hugit_proto::ObjectSource + Send + Sync>,
        git_root_tree: gix_hash::ObjectId,
        git_refs: BTreeMap<String, String>,
    ) {
        let normalized = crate::state::normalize_repo_slug(&repo.into());
        self.repos.insert(
            normalized,
            RepoState {
                git_source,
                git_root_tree,
                git_refs: LiveRefs::new(git_refs),
                git_dir: None,
                cas_write: None,
                live_oid_index: None,
                clone_cache: None,
            },
        );
    }

    /// TEST-SUPPORT: attach a clone-pack cache seam to an already-seeded boot repo
    /// (the wire test seeds a repo via [`set_repo_git`](Self::set_repo_git), then
    /// points its clone cache at a mock R2). `#[doc(hidden)]`; a no-op for an unknown
    /// slug. Not used by any production path (boot/provision populate `clone_cache`
    /// from the receive-pack write seam).
    #[doc(hidden)]
    pub fn set_repo_clone_cache(&mut self, repo: &str, seam: crate::clone_pack::CloneCacheSeam) {
        if let Some(rs) = self.repos.get_mut(repo) {
            rs.clone_cache = Some(seam);
        }
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
        let src: Arc<dyn hugit_proto::ObjectSource + Send + Sync> =
            Arc::new(GitDirLooseObjectSource::new(cas, dir));
        let normalized = crate::state::normalize_repo_slug(&repo.into());
        self.repos.insert(
            normalized,
            RepoState {
                git_source: src,
                git_root_tree: root,
                git_refs: LiveRefs::new(refs),
                git_dir: Some(PathBuf::from(dir)),
                cas_write: None,
                live_oid_index: None,
                clone_cache: None,
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

    /// Load-or-create a per-account event log (GDPR1) + its head [`CasToken`], from
    /// the reserved `_accounts/{account}.json` store (NOT the repo namespace). Unlike
    /// [`load_verified_with_token`] (absent → 404), an ABSENT account log is the
    /// EMPTY world with a create-only [`CasToken::Absent`] token — the first erasure
    /// request seeds the genesis. A present log is chain-verified by the SAME PS-13
    /// chokepoint (a tamper/parse fault → 503, never a fake-empty world).
    ///
    /// # Errors
    /// - `404 NOT_FOUND` — an unsafe account slug (traversal-safe fail-closed; never
    ///   builds an `_accounts/../…` key).
    /// - `503 ENGINE_UNAVAILABLE` — a store transport fault or a chain-verify failure.
    pub fn load_account_log(&self, account: &str) -> Result<(EventLog, CasToken), EngineErr> {
        if !is_safe_account_slug(account) {
            return Err(EngineErr::not_found());
        }
        match self.source.fetch_account(account)? {
            // Present: chain-verify (same gate as a repo log).
            Some((bytes, label, token)) => {
                let log = hugit_cli::checks::load_event_log_from_bytes(&bytes, Path::new(&label))
                    .map_err(|e| {
                    EngineErr::unavailable(format!("account log read/verify failed ({})", e.kind()))
                })?;
                Ok((log, token))
            }
            // Absent: the empty world — create-only on the first request.
            None => Ok((EventLog::new(), CasToken::Absent)),
        }
    }

    /// Durably persist a per-account event log (GDPR1) back to `_accounts/{account}.json`
    /// as a COMPARE-AND-SWAP against `expected`. Fail-closed on an unsafe slug (404) —
    /// the write can never reach a traversal key.
    pub fn persist_account_log(
        &self,
        account: &str,
        log: &EventLog,
        expected: &CasToken,
    ) -> Result<(), EngineErr> {
        if !is_safe_account_slug(account) {
            return Err(EngineErr::not_found());
        }
        let bytes = serde_json::to_vec(log.records())
            .map_err(|e| EngineErr::unavailable(format!("account log serialize failed: {e}")))?;
        self.source.persist_account(account, &bytes, expected)
    }

    // ── ADR-0004 leg 1: the durable per-subject KEY store (CSPRNG + shred) ─────
    //
    // The erasable secret behind the Art.17 crypto-shred: a CSPRNG 32-byte key per subject,
    // persisted under the reserved `_subject_keys/` keyspace (survives restart), fetched to
    // derive `subj:<hmac>` pseudonyms, and SHREDDED (durable + irreversible) on a completed
    // erase — after which the subject's pseudonyms can no longer be linked to cleartext.

    /// Fetch a subject's durable key, or `None` if never minted OR already shredded. `Err`
    /// on a durable fault (never read as "no key" — that would mint a duplicate/under-shred).
    pub fn subject_key_for(
        &self,
        subject: &str,
    ) -> Result<Option<crate::provenance_pii::SubjectKey>, EngineErr> {
        if !is_safe_account_slug(subject) {
            return Err(EngineErr::not_found());
        }
        Ok(self
            .source
            .fetch_subject_key(subject)?
            .map(crate::provenance_pii::SubjectKey::from_bytes))
    }

    /// Fetch-or-mint a subject's durable key (idempotent; a repeat returns the SAME key so a
    /// subject's pseudonym is stable across its records). Mints 32 CSPRNG bytes on first use
    /// and persists them create-only; a concurrent mint race adopts the winner's key. `Err`
    /// on a durable fault.
    pub fn subject_key_ensure(
        &self,
        subject: &str,
    ) -> Result<crate::provenance_pii::SubjectKey, EngineErr> {
        use rand::RngCore as _;
        if !is_safe_account_slug(subject) {
            return Err(EngineErr::not_found());
        }
        if let Some(b) = self.source.fetch_subject_key(subject)? {
            return Ok(crate::provenance_pii::SubjectKey::from_bytes(b));
        }
        let mut bytes = [0u8; 32];
        // ThreadRng is a cryptographically-secure PRNG (OS-seeded, ChaCha-based) — the
        // CSPRNG the ADR requires for real key material.
        rand::rng().fill_bytes(&mut bytes);
        match self.source.create_subject_key(subject, &bytes) {
            Ok(()) => Ok(crate::provenance_pii::SubjectKey::from_bytes(bytes)),
            // A concurrent mint won the create-only race — adopt ITS durable key so the
            // pseudonym stays stable (never last-writer-wins on key material).
            Err(e) if e.is_cas_conflict() => {
                let b = self.source.fetch_subject_key(subject)?.ok_or_else(|| {
                    EngineErr::unavailable("subject key vanished after a create conflict")
                })?;
                Ok(crate::provenance_pii::SubjectKey::from_bytes(b))
            }
            Err(e) => Err(e),
        }
    }

    /// FORWARD write-path pseudonymisation (ADR-0004 leg 2), kill-switch-gated. Map a live
    /// request's `principal_chain` to its stored, pseudonymous form so a NEW provenance record
    /// carries `subj:<hmac>` instead of the cleartext `clerk:{org}:{user}` at the point of
    /// append. Thin gate over [`provenance_pii_redact::pseudonymize_write_principal_chain`]:
    ///
    /// - Default **ON**. The ops kill-switch `HUGIT_SERVE_PROV_PSEUDONYM=0|false|off` returns
    ///   the chain UNCHANGED (identity) — so a deploy can disable forward pseudonymisation
    ///   without a rollback if it ever needs to (leaves the erase-time redaction untouched).
    /// - Fail-closed: a durable key-store fault propagates as `Err` (the write aborts rather
    ///   than storing cleartext or a wrong pseudonym).
    ///
    /// AUTHZ-NEUTRAL BY CONSTRUCTION: this rewrites ONLY the stored `principal_chain`, never the
    /// `owner_tenant`/`user` PAYLOAD fields the authz + PAT readers key on (ADR-0004 leg 3 — the
    /// gates read the projected `owner_tenant` + the LIVE request principal, never the stored
    /// chain). A stable pseudonym per org also keeps any principal-keyed idempotency match
    /// consistent (same org ⇒ same pseudonym while the key lives).
    pub fn pseudonymize_write_chain(&self, chain: &[String]) -> Result<Vec<String>, EngineErr> {
        if !prov_pseudonym_enabled() {
            return Ok(chain.to_vec()); // ops kill-switch: forward pseudonymisation OFF
        }
        crate::provenance_pii_redact::pseudonymize_write_principal_chain(self, chain)
    }

    /// SHRED a subject's durable key — the irreversible Art.17 action. Idempotent (an absent
    /// key → `Ok`). After this, [`subject_key_for`](Self::subject_key_for) returns `None` and
    /// every one of the subject's pseudonyms is permanently unrecoverable. `Err` on a durable
    /// fault (the caller must NOT then claim the cleartext erased).
    pub fn subject_key_shred(&self, subject: &str) -> Result<(), EngineErr> {
        if !is_safe_account_slug(subject) {
            return Err(EngineErr::not_found());
        }
        self.source.shred_subject_key(subject)
    }

    // ── per-tenant repo REGISTRY (WP-1: the durable cap denominator) ──────────
    //
    // The authoritative, chain-verified per-tenant owned-repo set/count under the reserved
    // `_tenants/{org}.json` keyspace (see [`crate::tenant_registry`]). It is DURABLE, so
    // the cap survives a reboot (defect A) and is read O(1) w.r.t. the platform (defect B).
    // The `org` key MUST be a safe account slug (the SAME `[a-z0-9-]≤64` identity provision
    // derives — ONE identity for ownership + erasability + the registry key), fail-closed
    // to 404 so a malformed org can never build a `_tenants/../evil.json` traversal key.

    /// Load-or-create a tenant's durable repo registry + its head [`CasToken`] from the
    /// reserved `_tenants/{org}.json` store. Like [`load_account_log`], an ABSENT registry
    /// is the EMPTY owned-set with a create-only [`CasToken::Absent`] token (a new tenant),
    /// NOT a 404. A present registry is chain-verified by the SAME PS-13 chokepoint (a
    /// tamper/parse fault → 503, never a fake-empty world that would undercount the cap).
    ///
    /// # Errors
    /// - `404 NOT_FOUND` — an unsafe org slug (traversal-safe fail-closed).
    /// - `503 ENGINE_UNAVAILABLE` — a store transport fault or a chain-verify failure.
    pub fn load_tenant_registry(&self, org: &str) -> Result<(EventLog, CasToken), EngineErr> {
        if !is_safe_account_slug(org) {
            return Err(EngineErr::not_found());
        }
        match self.source.fetch_tenant(org)? {
            Some((bytes, label, token)) => {
                let log = hugit_cli::checks::load_event_log_from_bytes(&bytes, Path::new(&label))
                    .map_err(|e| {
                    EngineErr::unavailable(format!(
                        "tenant registry read/verify failed ({})",
                        e.kind()
                    ))
                })?;
                Ok((log, token))
            }
            None => Ok((EventLog::new(), CasToken::Absent)),
        }
    }

    /// Durably persist a tenant's repo registry back to `_tenants/{org}.json` as a
    /// COMPARE-AND-SWAP against `expected`. Fail-closed on an unsafe org slug (404).
    pub fn persist_tenant_registry(
        &self,
        org: &str,
        log: &EventLog,
        expected: &CasToken,
    ) -> Result<(), EngineErr> {
        if !is_safe_account_slug(org) {
            return Err(EngineErr::not_found());
        }
        let bytes = serde_json::to_vec(log.records()).map_err(|e| {
            EngineErr::unavailable(format!("tenant registry serialize failed: {e}"))
        })?;
        self.source.persist_tenant(org, &bytes, expected)
    }

    /// Register `repo` into `org`'s durable owned-set via a bounded compare-and-swap
    /// (mirrors the write-door / tombstone CAS loop). IDEMPOTENT — a repo already in the
    /// set is a no-op (no double-count, no redundant persist). FAIL-CLOSED — a durable
    /// persist fault propagates (the caller decides whether to surface or best-effort it).
    ///
    /// This is the SECOND durable write of a provision (after the genesis create, the
    /// source of truth). It is APPEND-ONLY on the registry chain (never a rewrite → the
    /// chain still verifies), attributed to the creating principal (chain-derived class).
    pub fn register_repo_in_tenant(
        &self,
        org: &str,
        repo: &str,
        principal_chain: &[String],
        at: u64,
    ) -> Result<(), EngineErr> {
        for _attempt in 0..crate::writes::MAX_CAS_ATTEMPTS {
            let (mut log, token) = self.load_tenant_registry(org)?;
            let head_len = log.records().len();
            if !crate::tenant_registry::append_register(&mut log, repo, principal_chain, at)? {
                return Ok(()); // already registered — idempotent no-op
            }
            // ART.17 leg 2 (registry door): the record was BUILT on the CLEARTEXT principal so
            // its D14 class derivation (`asserted_class`) saw the real caller — now rewrite JUST
            // the appended tail's stored `principal_chain` to `subj:<hmac>` and re-hash it
            // NATIVELY (the exact `repseudonymize_tail` mechanism #319 uses at the write doors),
            // so `_tenants/{org}.json` never durably stores the cleartext `clerk:{org}:{user}`.
            // The fold keys on the `repo` PAYLOAD field, never the principal, so the count /
            // ownership (the cap denominator) is unaffected. Fail-closed on a key-store fault
            // (the register aborts rather than storing cleartext — the caller best-efforts it and
            // the count self-heals on a later touch).
            let log = crate::writes::repseudonymize_tail(&log, head_len, self)?;
            match self.persist_tenant_registry(org, &log, &token) {
                Ok(()) => return Ok(()),
                Err(e) if e.is_cas_conflict() => continue, // head moved — reload + retry
                Err(e) => return Err(e),
            }
        }
        Err(EngineErr::unavailable(
            "registro de repositório do tenant sob contenção — tente novamente",
        ))
    }

    /// Remove `repo` from `org`'s durable owned-set (the GDPR1 erase-tombstone decrement —
    /// the cap is a HOLD-count, erasable-down). IDEMPOTENT — a repo already absent is a
    /// no-op. Same bounded-CAS + fail-closed discipline as [`register_repo_in_tenant`].
    pub fn unregister_repo_from_tenant(
        &self,
        org: &str,
        repo: &str,
        principal_chain: &[String],
        at: u64,
    ) -> Result<(), EngineErr> {
        for _attempt in 0..crate::writes::MAX_CAS_ATTEMPTS {
            let (mut log, token) = self.load_tenant_registry(org)?;
            let head_len = log.records().len();
            if !crate::tenant_registry::append_unregister(&mut log, repo, principal_chain, at)? {
                return Ok(()); // already decremented — idempotent no-op
            }
            // ART.17 leg 2 (registry door): rewrite the appended tail's stored chain to the
            // pseudonym + re-hash natively — same mechanism as [`register_repo_in_tenant`]. The
            // erase-tombstone decrement is authored by the OPERATOR (`orchestrator:`, a
            // passthrough for the mapper), so this is a no-op there, but it keeps BOTH registry
            // doors consistent so no cleartext `clerk:` principal can enter `_tenants/{org}.json`
            // from either side.
            let log = crate::writes::repseudonymize_tail(&log, head_len, self)?;
            match self.persist_tenant_registry(org, &log, &token) {
                Ok(()) => return Ok(()),
                Err(e) if e.is_cas_conflict() => continue,
                Err(e) => return Err(e),
            }
        }
        Err(EngineErr::unavailable(
            "baixa de repositório do tenant sob contenção — tente novamente",
        ))
    }

    /// SELF-HEALING reconcile toward the genesis source of truth (WP-1 atomicity story):
    /// the genesis create is the FIRST, authoritative durable write; the registry register
    /// is the SECOND. If the engine crashes BETWEEN them, a repo exists durably but is not
    /// yet countable — an undercount. This heals that drift: given a `repo` whose durable
    /// genesis EXISTS and is OWNED by `org`, ensure it is registered (idempotent). Called
    /// on the provision no-clobber path (a retry of the same slug re-registers the crashed
    /// repo before the 409), so the count converges to genesis-truth. Never registers a
    /// repo that is not durably owned by `org` (verified via `load_verified`), so it can
    /// never inflate the count for a foreign / non-existent repo.
    ///
    /// Best-effort by contract: a reconcile fault is surfaced as `Err` for the caller to
    /// log-and-continue (the repo is already readable; the registry heals on a later
    /// touch), never a hard failure of the surrounding operation.
    pub fn reconcile_tenant_repo(
        &self,
        org: &str,
        repo: &str,
        principal_chain: &[String],
        at: u64,
    ) -> Result<(), EngineErr> {
        // Only reconcile a repo that DURABLY exists AND is owned by `org` (genesis is the
        // source of truth). An ABSENT genesis (404) → nothing to reconcile (Ok, no phantom
        // registration); a 5xx (indeterminate) propagates.
        let log = match self.load_verified(repo) {
            Ok(log) => log,
            Err(e) if e.status == 404 => return Ok(()), // no genesis → nothing to heal
            Err(e) => return Err(e),
        };
        let meta = crate::authz::project_repo_meta(&log);
        if meta.owner_tenant.as_deref() != Some(org) {
            return Ok(()); // not owned by org → not this registry's entry
        }
        self.register_repo_in_tenant(org, repo, principal_chain, at)
    }

    /// Kick off the #76 BOOT-RECONCILE on a DETACHED thread — boot NEVER blocks on it
    /// (mirrors [`boot_build_pat_index`](Self::boot_build_pat_index) / the refs-refresh loop:
    /// detached, `catch_unwind`, bounded, gated to CAS mode). It heals the per-tenant repo
    /// registry toward genesis truth on two axes:
    ///
    /// - the **registry write-side MED** — the `_tenants/{org}.json` register is a best-effort
    ///   SECOND durable write after the genesis create, so a transient R2 fault on that PUT
    ///   leaves a durable repo UNREGISTERED (a persistent undercount that would let
    ///   different-slug creates accumulate past [`MAX_REPOS_PER_TENANT`]); and
    /// - the **legacy back-fill** — git-ingested repos (hugit/githugr) that predate the
    ///   registry and were never registered at all.
    ///
    /// For each durable repo whose genesis verifies it calls the idempotent +
    /// genesis-authoritative [`reconcile_tenant_repo`](Self::reconcile_tenant_repo), which
    /// registers it under its genesis-declared owner ONLY — never double-counts, never
    /// registers a foreign-owned repo. NO-OP outside CAS mode. A spawn failure degrades to the
    /// pre-#76 heal-on-touch (the provision no-clobber path still reconciles).
    fn spawn_boot_reconcile_tenant_registry(&self) {
        if self.cas_r2_read().is_none() {
            return; // Local/git-dir: the durable-registry heal is a CAS-mode concern
        }
        let state = self.clone();
        let spawn = std::thread::Builder::new()
            .name("hugit-tenant-reconcile".to_string())
            .spawn(move || {
                // A panic on the reconcile pass is isolated (never crashes a detached thread /
                // the engine), exactly like the other off-loop boot tasks.
                let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    state.run_boot_reconcile_tenant_registry();
                }));
            });
        if spawn.is_err() {
            eprintln!(
                "[hugit-serve] tenant-registry boot reconcile thread spawn failed — the durable \
                 repo registry heals on the next provision touch / reboot instead"
            );
        }
    }

    /// ONE reconcile pass (see [`spawn_boot_reconcile_tenant_registry`]). Enumerate the durable
    /// repo slugs and reconcile each VERIFIED genesis into its owner tenant's registry. Per-repo
    /// ISOLATED + fail-closed: a listing/load/reconcile fault on one repo is logged + skipped,
    /// never halting the pass. Bounded by [`BOOT_RECONCILE_MAX_REPOS`]. Logs a one-line summary.
    fn run_boot_reconcile_tenant_registry(&self) {
        let slugs = match self.source.list_repo_slugs() {
            Ok(s) => s,
            Err(e) => {
                eprintln!(
                    "[hugit-serve] tenant-registry reconcile: repo listing failed ({}) — skipping",
                    e.reason
                );
                return;
            }
        };
        let operator: Vec<String> = vec!["orchestrator:hugit".to_string()];
        let at = now_unix_ms();
        let (mut ensured, mut skipped) = (0usize, 0usize);
        for slug in slugs.into_iter().take(BOOT_RECONCILE_MAX_REPOS) {
            // The genesis-declared owner is the ONLY tenant this repo may register under.
            let log = match self.load_verified(&slug) {
                Ok(log) => log,
                Err(e) if e.status == 404 => continue, // no genesis → nothing durable to heal
                Err(_) => {
                    skipped += 1;
                    continue; // indeterminate read → skip (heals next boot)
                }
            };
            let Some(owner) = crate::authz::project_repo_meta(&log).owner_tenant else {
                continue; // no declared owner → not a registrable genesis
            };
            // `reconcile_tenant_repo` re-verifies ownership + is idempotent (an already-registered
            // repo is a no-op; a foreign-owned repo is refused), so this can never double-count
            // nor register a repo under the wrong tenant.
            match self.reconcile_tenant_repo(&owner, &slug, &operator, at) {
                Ok(()) => ensured += 1,
                Err(e) => {
                    skipped += 1;
                    eprintln!(
                        "[hugit-serve] tenant-registry reconcile: repo {slug:?} skipped ({}) — \
                         heals on a later touch",
                        e.reason
                    );
                }
            }
        }
        eprintln!(
            "[hugit-serve] tenant-registry boot reconcile: {ensured} repo(s) ensured-registered, \
             {skipped} skipped"
        );
    }

    // ── PAT auth index (slice 2b) ────────────────────────────────────────────
    // The in-memory `sha256(secret) → PatAuth` index the hot-path resolver consults.
    // All three methods are NO-OPS when `pat_auth_enabled` is false.

    /// Kick off the PAT index build on a DETACHED thread — boot NEVER blocks on it.
    /// Called once at boot (from `from_env`) when PAT auth is enabled. The engine
    /// starts with an EMPTY index (a PAT 401s — fail-closed-DENY — until the warm-up
    /// lands, typically sub-second), then the thread MERGES the scanned tokens in.
    ///
    /// **Why detached (not synchronous):** the scan does one verified R2 fetch PER
    /// account, sequentially — synchronous at boot it would blow the Cloudflare
    /// Container startup deadline as accounts grow (the chunk-256 boot-crash class).
    /// This mirrors the CAS `batch_read` self-probe, moved off-boot for the SAME reason
    /// (see [`from_env`]). A build fault logs + leaves whatever merged (those PATs 401),
    /// never a crash; a panic is isolated by `catch_unwind`.
    ///
    /// **Merge, not overwrite:** the thread INSERTS each scanned entry into the live
    /// index without clearing it, so a `token_create` that lands DURING the warm-up
    /// (its `pat_index_insert_if_enabled` key is absent from the scan) survives. The
    /// only residual is a `token_revoke` in the same one-time warm-up window whose
    /// `pat.revoked` post-dates the scan's read: the scanned (still-live) entry is
    /// re-merged and authenticates until the next reboot — the SAME self-healing,
    /// single-instance in-memory-eviction property already reviewed for the steady
    /// state (and strictly narrower: a one-time boot window).
    fn boot_build_pat_index(&self) {
        let source = self.source.clone();
        let index = Arc::clone(&self.pat_index);
        let spawn = std::thread::Builder::new()
            .name("hugit-pat-index".into())
            .spawn(move || {
                let built = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    build_pat_index_from_source(&source)
                }))
                .unwrap_or_else(|_| {
                    eprintln!("[hugit-serve] PAT index build panicked — leaving an empty index");
                    std::collections::HashMap::new()
                });
                let n = built.len();
                // Merge (insert-only, never clear) so a create during warm-up survives.
                let mut idx = index.write().unwrap_or_else(|e| e.into_inner());
                for (hash, pat) in built {
                    idx.insert(hash, pat);
                }
                eprintln!("[hugit-serve] PAT auth: indexed {n} live token(s) (warm)");
            });
        if spawn.is_err() {
            eprintln!(
                "[hugit-serve] PAT index build thread spawn failed — PAT auth starts with an \
                 EMPTY index (those tokens will 401 until a reboot)"
            );
        }
    }

    /// Insert a freshly-minted token into the live index (immediate authentication, no
    /// reboot). No-op when PAT auth is off.
    pub fn pat_index_insert_if_enabled(
        &self,
        secret_hash: String,
        pat: crate::writes::verbs::write_token::PatAuth,
    ) {
        if !self.pat_auth_enabled {
            return;
        }
        self.pat_index
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert(secret_hash, pat);
    }

    /// Drop a revoked token from the live index (immediate deny, no reboot). No-op when
    /// PAT auth is off.
    pub fn pat_index_remove_if_enabled(&self, secret_hash: &str) {
        if !self.pat_auth_enabled {
            return;
        }
        self.pat_index
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .remove(secret_hash);
    }

    /// Resolve an `Authorization` header VALUE to its owning [`PatAuth`] via the live
    /// index, or `None`. Accepts BOTH `Bearer <secret>` and the git-CLI
    /// `Basic base64(user:secret)` (the PAT is the password). Returns `None` when PAT
    /// auth is disabled, the header carries no `ghgr_pat_` candidate, or the token is
    /// unknown/revoked/expired — an O(1) in-memory lookup, NO per-request R2 read.
    #[must_use]
    pub fn resolve_pat_from_auth(
        &self,
        auth_value: &str,
        now_ms: u64,
    ) -> Option<crate::writes::verbs::write_token::PatAuth> {
        if !self.pat_auth_enabled {
            return None;
        }
        let candidates = crate::writes::verbs::write_token::candidate_pat_secrets(auth_value);
        if candidates.is_empty() {
            return None;
        }
        let idx = self.pat_index.read().unwrap_or_else(|e| e.into_inner());
        for cand in &candidates {
            if let Some(pat) = crate::writes::verbs::write_token::resolve_pat(&idx, cand, now_ms) {
                // Record best-effort last-used, keyed by the non-secret pat id (derived from
                // the matched candidate). Drop the pat_index read lock first so the two
                // locks never nest. Cheap (in-memory), no durable I/O on the accept loop.
                let id = crate::writes::verbs::write_token::pat_id_of_secret(cand);
                drop(idx);
                self.record_pat_used(id, now_ms);
                return Some(pat);
            }
        }
        None
    }

    /// Record a PAT's last-used timestamp (best-effort, in-memory — see
    /// [`pat_last_used`](Self::pat_last_used)). Keyed by the non-secret `pat_id`. Called on
    /// a successful [`resolve_pat_from_auth`]; a monotonic `max` guard means an
    /// out-of-order call never rewinds the stamp.
    fn record_pat_used(&self, pat_id: String, now_ms: u64) {
        let mut m = self
            .pat_last_used
            .write()
            .unwrap_or_else(|e| e.into_inner());
        let slot = m.entry(pat_id).or_insert(0);
        *slot = (*slot).max(now_ms);
    }

    /// A snapshot of the `pat_id → last-used ms` map for the account projection
    /// ([`build_me_account`](crate::handlers::build_me_account) merges it into
    /// `PatMetaVm.last_used_at`). Cheap: a handful of PATs per account.
    #[must_use]
    pub fn pat_last_used_snapshot(&self) -> std::collections::HashMap<String, u64> {
        self.pat_last_used
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// A cheap, in-memory snapshot of which repos have a clone-pack build in progress —
    /// surfaced on `/readyz` as `clonepack` (clone-pack legibility). NO R2 read (the
    /// liveness probe must stay fast): reads only the in-process `clone_pack_building`
    /// guard set. `"idle"` when none, else `"building:<slug>[,<slug>…]"` (sorted). The
    /// slugs are `is_safe_repo_slug` by construction (only real repos are inserted), so
    /// the string is JSON-safe; `/readyz` char-filters defensively regardless.
    #[must_use]
    pub fn clone_pack_building_snapshot(&self) -> String {
        let set = self
            .clone_pack_building
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if set.is_empty() {
            return "idle".to_string();
        }
        let mut slugs: Vec<&str> = set.iter().map(String::as_str).collect();
        slugs.sort_unstable();
        format!("building:{}", slugs.join(","))
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
                // G11: a user-scoped slug (`<owner>/<name>`) nests one level, so ensure
                // the parent dir exists before the temp-write + rename (the flat case is
                // a no-op — the parent IS `dir`, already created above).
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent).map_err(|e| {
                        EngineErr::unavailable(format!("local log subdir create failed: {e}"))
                    })?;
                }
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

    /// Fetch a per-account event log's raw bytes + head [`CasToken`] under the
    /// reserved `_accounts/{account}.json` sub-prefix (GDPR1). `Ok(None)` = absent
    /// (the first erasure request → a create-genesis); `Ok(Some(..))` = present;
    /// `Err` = a transport/IO fault (→ 503). Structurally OUTSIDE the repo namespace
    /// (the `_accounts/` prefix + the `/` in the key are unreachable by
    /// [`is_safe_repo_slug`]), so an account log can never be served/cloned as a repo.
    fn fetch_account(
        &self,
        account: &str,
    ) -> Result<Option<(Vec<u8>, String, CasToken)>, EngineErr> {
        match self {
            LogSource::Local { dir } => {
                let path = dir.join("_accounts").join(format!("{account}.json"));
                match std::fs::read(&path) {
                    Ok(b) => {
                        let token = CasToken::Version(content_hash(&b));
                        Ok(Some((b, path.display().to_string(), token)))
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
                    Err(e) => Err(EngineErr::unavailable(format!(
                        "local account log read failed: {e}"
                    ))),
                }
            }
            LogSource::R2(c) => {
                let key = format!("{}/_accounts/{account}.json", c.tenant_id);
                let label = format!("r2://{}/{key}", c.bucket);
                Ok(c.get_object_etag(&key)?
                    .map(|(bytes, token)| (bytes, label, token)))
            }
        }
    }

    /// Durably persist a per-account event log back to the reserved
    /// `_accounts/{account}.json`, as a COMPARE-AND-SWAP against `expected` (the token
    /// the matching [`fetch_account`] returned). Mirrors [`persist`]'s discipline:
    /// Local = re-read content-hash compare + atomic temp-write/rename; R2 = a
    /// conditional (`If-Match`/`If-None-Match`) PUT via [`R2Config::conditional_object_put`].
    /// A concurrent head move → [`EngineErr::cas_conflict`] (the door reloads + retries),
    /// NEVER last-writer-wins.
    fn persist_account(
        &self,
        account: &str,
        bytes: &[u8],
        expected: &CasToken,
    ) -> Result<(), EngineErr> {
        match self {
            LogSource::Local { dir } => {
                let adir = dir.join("_accounts");
                std::fs::create_dir_all(&adir).map_err(|e| {
                    EngineErr::unavailable(format!("local account dir create failed: {e}"))
                })?;
                let path = adir.join(format!("{account}.json"));
                let current = match std::fs::read(&path) {
                    Ok(b) => CasToken::Version(content_hash(&b)),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => CasToken::Absent,
                    Err(e) => {
                        return Err(EngineErr::unavailable(format!(
                            "local account log re-read failed: {e}"
                        )));
                    }
                };
                if !cas_matches(expected, &current) {
                    return Err(EngineErr::cas_conflict());
                }
                let tmp = adir.join(format!("{account}.json.tmp.{}", std::process::id()));
                std::fs::write(&tmp, bytes).map_err(|e| {
                    EngineErr::unavailable(format!("local account log write failed: {e}"))
                })?;
                std::fs::rename(&tmp, &path).map_err(|e| {
                    let _ = std::fs::remove_file(&tmp);
                    EngineErr::unavailable(format!("local account log rename failed: {e}"))
                })
            }
            LogSource::R2(c) => {
                // FAIL-CLOSED (same as the repo persist): an `Unsupported` token would
                // downgrade to an unconditional PUT (last-writer-wins). `conditional_object_put`
                // refuses it; map its outcome onto the door's EngineErr signals.
                let key = format!("{}/_accounts/{account}.json", c.tenant_id);
                c.conditional_object_put(&key, bytes, expected)
                    .map(|_| ())
                    .map_err(|e| match e {
                        crate::cas::ManifestPutError::Precondition => EngineErr::cas_conflict(),
                        crate::cas::ManifestPutError::Other(m) => {
                            eprintln!("[hugit-serve] account log PUT failed: {m}");
                            EngineErr::unavailable("engine storage write unavailable")
                        }
                    })
            }
        }
    }

    // ── ADR-0004: the durable per-subject KEY store (`_subject_keys/{subject}.key`) ──
    //
    // The erasable secret the Art.17 crypto-shred deletes. Structurally OUTSIDE the repo
    // namespace (the `_subject_keys/` prefix is unreachable by `is_safe_repo_slug`), so a
    // key object can never be served/cloned as a repo. The BODY is base64 of the 32 raw
    // key bytes; absence OR an undecodable body = "no live key" (never minted, or shredded).

    /// Fetch a subject's raw 32-byte key, or `None` if never minted OR shredded. `Err` on a
    /// transport/IO fault (→ the caller aborts; a durable fault must never be read as "no
    /// key", which would mint a duplicate or under-shred).
    fn fetch_subject_key(&self, subject: &str) -> Result<Option<[u8; 32]>, EngineErr> {
        use base64::Engine as _;
        let decode = |bytes: Vec<u8>| -> Option<[u8; 32]> {
            let raw = base64::engine::general_purpose::STANDARD
                .decode(bytes.trim_ascii())
                .ok()?;
            <[u8; 32]>::try_from(raw.as_slice()).ok()
        };
        match self {
            LogSource::Local { dir } => {
                let path = dir.join("_subject_keys").join(format!("{subject}.key"));
                match std::fs::read(&path) {
                    Ok(b) => Ok(decode(b)),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
                    Err(e) => Err(EngineErr::unavailable(format!(
                        "local subject-key read failed: {e}"
                    ))),
                }
            }
            LogSource::R2(c) => {
                let key = format!("{}/_subject_keys/{subject}.key", c.tenant_id);
                Ok(c.get_object(&key)?.and_then(decode))
            }
        }
    }

    /// Durably CREATE a subject's key (idempotent-safe: a create-only put that fails the CAS
    /// if another writer minted first, so a concurrent mint never overwrites the winner's
    /// key — the caller re-fetches on conflict). `bytes` is the 32-byte CSPRNG key.
    fn create_subject_key(&self, subject: &str, bytes: &[u8; 32]) -> Result<(), EngineErr> {
        use base64::Engine as _;
        let body = base64::engine::general_purpose::STANDARD
            .encode(bytes)
            .into_bytes();
        match self {
            LogSource::Local { dir } => {
                let kdir = dir.join("_subject_keys");
                std::fs::create_dir_all(&kdir).map_err(|e| {
                    EngineErr::unavailable(format!("local subject-key dir create failed: {e}"))
                })?;
                let path = kdir.join(format!("{subject}.key"));
                // Create-only: `O_EXCL` fails if a concurrent mint already wrote → cas_conflict
                // so the caller re-fetches the winner's key (never last-writer-wins).
                match std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&path)
                {
                    Ok(mut f) => {
                        use std::io::Write as _;
                        f.write_all(&body).map_err(|e| {
                            EngineErr::unavailable(format!("local subject-key write failed: {e}"))
                        })
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                        Err(EngineErr::cas_conflict())
                    }
                    Err(e) => Err(EngineErr::unavailable(format!(
                        "local subject-key create failed: {e}"
                    ))),
                }
            }
            LogSource::R2(c) => {
                let key = format!("{}/_subject_keys/{subject}.key", c.tenant_id);
                // Create-only (`If-None-Match: *`) via the CAS-token `Absent`.
                c.conditional_object_put(&key, &body, &CasToken::Absent)
                    .map(|_| ())
                    .map_err(|e| match e {
                        crate::cas::ManifestPutError::Precondition => EngineErr::cas_conflict(),
                        crate::cas::ManifestPutError::Other(m) => {
                            eprintln!("[hugit-serve] subject-key PUT failed: {m}");
                            EngineErr::unavailable("engine storage write unavailable")
                        }
                    })
            }
        }
    }

    /// Durably + irreversibly SHRED a subject's key — the Art.17 crypto-shred. Local removes
    /// the object; R2 OVERWRITES it with a non-decodable tombstone (no delete verb needed; an
    /// undecodable body reads as `None` — the key material is gone either way). Idempotent (an
    /// already-absent key → `Ok`). `Err` on a durable fault (the caller must NOT then claim
    /// the cleartext erased).
    fn shred_subject_key(&self, subject: &str) -> Result<(), EngineErr> {
        match self {
            LogSource::Local { dir } => {
                // OVERWRITE with a tombstone (NOT `remove_file`): a removed file could be
                // re-created by a later `create_subject_key`, re-minting a fresh key and
                // resurrecting a subject that was crypto-shredded. A tombstone decodes to
                // `None` (undecodable body) AND blocks the create-only mint (O_EXCL sees the
                // file), so a shredded subject is NON-re-mintable — matching the R2 backend.
                let kdir = dir.join("_subject_keys");
                std::fs::create_dir_all(&kdir).map_err(|e| {
                    EngineErr::unavailable(format!("local subject-key dir create failed: {e}"))
                })?;
                let path = kdir.join(format!("{subject}.key"));
                let tmp = kdir.join(format!("{subject}.key.tmp.{}", std::process::id()));
                std::fs::write(&tmp, b"SHREDDED").map_err(|e| {
                    EngineErr::unavailable(format!("local subject-key tombstone write failed: {e}"))
                })?;
                std::fs::rename(&tmp, &path).map_err(|e| {
                    let _ = std::fs::remove_file(&tmp);
                    EngineErr::unavailable(format!("local subject-key shred rename failed: {e}"))
                })
            }
            LogSource::R2(c) => {
                let key = format!("{}/_subject_keys/{subject}.key", c.tenant_id);
                // Overwrite with a tombstone: the key bytes are gone (irreversible); a later
                // fetch decodes nothing → `None`. Unconditional put (shred is terminal).
                c.put_object(&key, b"SHREDDED").map(|_| ())
            }
        }
    }

    /// Fetch a per-tenant repo REGISTRY's raw bytes + head [`CasToken`] under the reserved
    /// `_tenants/{org}.json` sub-prefix (WP-1). `Ok(None)` = absent (a new tenant → the
    /// empty owned-set, a create-genesis on the first register); `Ok(Some(..))` = present;
    /// `Err` = a transport/IO fault (→ 503). Structurally OUTSIDE the repo namespace (the
    /// `_tenants/` prefix is unreachable by [`is_safe_repo_slug`]), so the registry can
    /// never be served/cloned as a repo. Mirrors [`fetch_account`].
    fn fetch_tenant(&self, org: &str) -> Result<Option<(Vec<u8>, String, CasToken)>, EngineErr> {
        match self {
            LogSource::Local { dir } => {
                let path = dir.join("_tenants").join(format!("{org}.json"));
                match std::fs::read(&path) {
                    Ok(b) => {
                        let token = CasToken::Version(content_hash(&b));
                        Ok(Some((b, path.display().to_string(), token)))
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
                    Err(e) => Err(EngineErr::unavailable(format!(
                        "local tenant registry read failed: {e}"
                    ))),
                }
            }
            LogSource::R2(c) => {
                let key = format!("{}/_tenants/{org}.json", c.tenant_id);
                let label = format!("r2://{}/{key}", c.bucket);
                Ok(c.get_object_etag(&key)?
                    .map(|(bytes, token)| (bytes, label, token)))
            }
        }
    }

    /// Durably persist a per-tenant repo REGISTRY back to the reserved `_tenants/{org}.json`,
    /// as a COMPARE-AND-SWAP against `expected` (the token the matching [`fetch_tenant`]
    /// returned). Mirrors [`persist_account`]'s discipline: Local = re-read content-hash
    /// compare + atomic temp-write/rename; R2 = a conditional (`If-Match`/`If-None-Match`)
    /// PUT via [`R2Config::conditional_object_put`]. A concurrent head move →
    /// [`EngineErr::cas_conflict`] (the caller reloads + retries), NEVER last-writer-wins.
    fn persist_tenant(
        &self,
        org: &str,
        bytes: &[u8],
        expected: &CasToken,
    ) -> Result<(), EngineErr> {
        match self {
            LogSource::Local { dir } => {
                let tdir = dir.join("_tenants");
                std::fs::create_dir_all(&tdir).map_err(|e| {
                    EngineErr::unavailable(format!("local tenant dir create failed: {e}"))
                })?;
                let path = tdir.join(format!("{org}.json"));
                let current = match std::fs::read(&path) {
                    Ok(b) => CasToken::Version(content_hash(&b)),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => CasToken::Absent,
                    Err(e) => {
                        return Err(EngineErr::unavailable(format!(
                            "local tenant registry re-read failed: {e}"
                        )));
                    }
                };
                if !cas_matches(expected, &current) {
                    return Err(EngineErr::cas_conflict());
                }
                let tmp = tdir.join(format!("{org}.json.tmp.{}", std::process::id()));
                std::fs::write(&tmp, bytes).map_err(|e| {
                    EngineErr::unavailable(format!("local tenant registry write failed: {e}"))
                })?;
                std::fs::rename(&tmp, &path).map_err(|e| {
                    let _ = std::fs::remove_file(&tmp);
                    EngineErr::unavailable(format!("local tenant registry rename failed: {e}"))
                })
            }
            LogSource::R2(c) => {
                // FAIL-CLOSED (same as the repo/account persist): an `Unsupported` token
                // would downgrade to an unconditional PUT (last-writer-wins).
                let key = format!("{}/_tenants/{org}.json", c.tenant_id);
                c.conditional_object_put(&key, bytes, expected)
                    .map(|_| ())
                    .map_err(|e| match e {
                        crate::cas::ManifestPutError::Precondition => EngineErr::cas_conflict(),
                        crate::cas::ManifestPutError::Other(m) => {
                            eprintln!("[hugit-serve] tenant registry PUT failed: {m}");
                            EngineErr::unavailable("engine storage write unavailable")
                        }
                    })
            }
        }
    }

    /// Enumerate the DURABLE repo slugs in the store — the authoritative owned-set
    /// source the GDPR1 erasure planner needs (the B1 completeness fix; the in-memory
    /// loaded set can MISS a durable-but-unloaded repo — one provisioned then dropped
    /// from the boot env, whose R2 log survives). A repo log is EITHER a top-level
    /// `<slug>.json` (legacy flat key) OR — post-G11 — a user-scoped `<owner>/<name>.json`
    /// (the owner side a safe account slug, so never the reserved `_accounts`; the leaf
    /// never a `<slug>/refs.json`/`oid-index.json` manifest). The `_accounts/…` account
    /// logs and the per-repo `<slug>/refs.json`/`oid-index.json` (and the scoped
    /// `<owner>/<name>/refs.json`, extra `/`) manifests are EXCLUDED. FAIL-CLOSED (`Err`
    /// → 503) on any listing fault: an indeterminate enumeration must never silently
    /// under-report the erasure set (a scoped repo missed here would be un-erasable).
    fn list_repo_slugs(&self) -> Result<Vec<String>, EngineErr> {
        // The two reserved manifest leaves that share the `<seg>/<leaf>.json` shape of a
        // user-scoped log — excluded so a LEGACY flat repo's manifest is never mistaken
        // for a scoped repo log (provision also RESERVES these names, so a real scoped
        // repo can never BE `refs`/`oid-index`).
        let is_manifest_leaf = |leaf: &str| leaf == "refs" || leaf == "oid-index";
        match self {
            LogSource::Local { dir } => {
                let rd = match std::fs::read_dir(dir) {
                    Ok(rd) => rd,
                    // A not-yet-created dir is an empty world (no durable repos), not a
                    // fault — the boot loader creates it on first write.
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
                    Err(e) => {
                        return Err(EngineErr::unavailable(format!(
                            "local repo listing failed: {e}"
                        )));
                    }
                };
                let mut slugs = Vec::new();
                for entry in rd {
                    let entry = entry.map_err(|e| {
                        EngineErr::unavailable(format!("local repo listing entry failed: {e}"))
                    })?;
                    let ft = entry.file_type().ok();
                    let Some(name) = entry.file_name().to_str().map(str::to_string) else {
                        continue;
                    };
                    // A top-level `<slug>.json` FILE = a legacy flat repo log.
                    if ft.map(|t| t.is_file()).unwrap_or(false) {
                        if let Some(slug) = name.strip_suffix(".json")
                            && is_safe_repo_slug(slug)
                        {
                            slugs.push(slug.to_string());
                        }
                        continue;
                    }
                    // A `<owner>/` SUBDIR (owner = a safe account slug, never `_accounts`)
                    // = the G11 user-scoped namespace: enumerate its `<name>.json` logs as
                    // `<owner>/<name>` (the manifest leaves are excluded).
                    if !is_safe_account_slug(&name) {
                        continue; // `_accounts` (underscore) and any non-account dir
                    }
                    let sub = dir.join(&name);
                    let srd = match std::fs::read_dir(&sub) {
                        Ok(srd) => srd,
                        Err(_) => continue, // vanished/unreadable subdir → skip (best-effort)
                    };
                    for sentry in srd {
                        let sentry = sentry.map_err(|e| {
                            EngineErr::unavailable(format!("local repo listing entry failed: {e}"))
                        })?;
                        if sentry.file_type().map(|t| t.is_file()).unwrap_or(false)
                            && let Some(leaf) = sentry.file_name().to_str()
                            && let Some(rest) = leaf.strip_suffix(".json")
                            && !is_manifest_leaf(rest)
                            && is_safe_repo_slug(rest)
                        {
                            slugs.push(format!("{name}/{rest}"));
                        }
                    }
                }
                Ok(slugs)
            }
            LogSource::R2(c) => {
                let prefix = format!("{}/", c.tenant_id);
                let keys = c.list_keys(&prefix)?;
                let mut slugs = Vec::new();
                for key in keys {
                    let Some(rest) = key.strip_prefix(&prefix) else {
                        continue;
                    };
                    let Some(stem) = rest.strip_suffix(".json") else {
                        continue;
                    };
                    match stem.split_once('/') {
                        // `<slug>` — a legacy flat repo log (not a reserved `_`-prefixed key).
                        None => {
                            if !stem.starts_with('_') && is_safe_repo_slug(stem) {
                                slugs.push(stem.to_string());
                            }
                        }
                        // `<owner>/<name>` — a G11 user-scoped log. The owner must be a
                        // safe account slug (excludes `_accounts`); the leaf must not be a
                        // `refs`/`oid-index` MANIFEST (a legacy flat repo's manifest shares
                        // this shape); a scoped manifest (`<owner>/<name>/refs.json`) has a
                        // second `/` in `name` and is rejected by `is_safe_repo_slug`.
                        Some((owner, name)) => {
                            if is_safe_account_slug(owner)
                                && !is_manifest_leaf(name)
                                && is_safe_repo_slug(stem)
                            {
                                slugs.push(stem.to_string());
                            }
                        }
                    }
                }
                Ok(slugs)
            }
        }
    }

    /// Enumerate the account slugs in the reserved `_accounts/` sub-prefix — the PAT
    /// boot-scan source (slice 2b). Mirrors [`list_repo_slugs`] but for the account
    /// namespace: Local = the `_accounts/` SUBDIR's `<slug>.json` files; R2 = the
    /// `<tenant>/_accounts/` prefix's single-segment `<slug>.json` keys. Only
    /// [`is_safe_account_slug`] names count. A not-yet-created store is an empty world
    /// (`Ok(vec![])`), not a fault; a listing IO error is `Err` (the caller logs +
    /// starts with an empty index — fail-closed-DENY, never a crash).
    fn list_account_slugs(&self) -> Result<Vec<String>, EngineErr> {
        match self {
            LogSource::Local { dir } => {
                let adir = dir.join("_accounts");
                let rd = match std::fs::read_dir(&adir) {
                    Ok(rd) => rd,
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
                    Err(e) => {
                        return Err(EngineErr::unavailable(format!(
                            "local account listing failed: {e}"
                        )));
                    }
                };
                let mut slugs = Vec::new();
                for entry in rd {
                    let entry = entry.map_err(|e| {
                        EngineErr::unavailable(format!("local account listing entry failed: {e}"))
                    })?;
                    if entry.file_type().map(|t| t.is_file()).unwrap_or(false)
                        && let Some(name) = entry.file_name().to_str()
                        && let Some(slug) = name.strip_suffix(".json")
                        && is_safe_account_slug(slug)
                    {
                        slugs.push(slug.to_string());
                    }
                }
                Ok(slugs)
            }
            LogSource::R2(c) => {
                let prefix = format!("{}/_accounts/", c.tenant_id);
                let keys = c.list_keys(&prefix)?;
                let mut slugs = Vec::new();
                for key in keys {
                    let Some(rest) = key.strip_prefix(&prefix) else {
                        continue;
                    };
                    // Single-segment `<slug>.json` only (no nested `/`); the account
                    // namespace is flat.
                    if rest.contains('/') {
                        continue;
                    }
                    if let Some(slug) = rest.strip_suffix(".json")
                        && is_safe_account_slug(slug)
                    {
                        slugs.push(slug.to_string());
                    }
                }
                Ok(slugs)
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

/// Scan every `_accounts/*` log via `source` and build the `sha256(secret) → PatAuth`
/// index — the PURE, thread-movable scan the detached boot builder
/// ([`AppState::boot_build_pat_index`]) runs OFF the boot path (so a synchronous
/// per-account verified R2 fetch never blows the container startup deadline).
/// Fail-closed-DENY: a list fault → an empty index; a per-account fetch/verify fault →
/// that account skipped (its PATs 401). Chain-verifies each log exactly like
/// [`AppState::load_account_log`]. Never panics on a data fault.
fn build_pat_index_from_source(
    source: &LogSource,
) -> std::collections::HashMap<String, crate::writes::verbs::write_token::PatAuth> {
    let mut idx = std::collections::HashMap::new();
    let slugs = match source.list_account_slugs() {
        Ok(s) => s,
        Err(e) => {
            eprintln!(
                "[hugit-serve] PAT index scan: account listing failed ({}) — empty index",
                e.reason
            );
            return idx;
        }
    };
    for slug in slugs {
        match source.fetch_account(&slug) {
            Ok(Some((bytes, label, _token))) => {
                match hugit_cli::checks::load_event_log_from_bytes(&bytes, Path::new(&label)) {
                    Ok(log) => {
                        for (hash, pat) in
                            crate::writes::verbs::write_token::index_account_log(&log)
                        {
                            idx.insert(hash, pat);
                        }
                    }
                    Err(e) => eprintln!(
                        "[hugit-serve] PAT index scan: skipping account {slug} (verify failed: \
                         {}) — its tokens 401",
                        e.kind()
                    ),
                }
            }
            Ok(None) => {} // absent account log — nothing to index
            Err(e) => eprintln!(
                "[hugit-serve] PAT index scan: skipping account {slug} (fetch failed: {}) — its \
                 tokens 401",
                e.reason
            ),
        }
    }
    idx
}

/// The hard cap on ListObjectsV2 pages the durable-owned enumeration will follow — a
/// bounded scan (each page is up to 1000 keys, so 32 pages ≈ 32k objects, far beyond
/// any real tenant's repo count). Hitting it is treated FAIL-CLOSED by the caller (an
/// unbounded listing would be a DoS, and a silently-truncated one would UNDER-report
/// the erasure set — the GDPR1-B1 hole; so an over-cap listing is an explicit error).
const MAX_LIST_PAGES: usize = 32;

/// Extract the `<Key>…</Key>` values + the `<NextContinuationToken>` (when
/// `<IsTruncated>true</IsTruncated>`) from an S3/R2 ListObjectsV2 XML body — a tiny
/// hand parser (no XML dependency, consistent with the SigV4 signer's zero-dep stance).
/// Pure + testable: the durable-enumeration correctness (the B1 completeness fix) is
/// proven on this without a live R2.
fn parse_listv2_xml(xml: &str) -> (Vec<String>, Option<String>) {
    // Keys: every <Key>…</Key> (element content is not entity-escaped for our
    // slug/`.json` keys, which are RFC-3986-unreserved).
    let mut keys = Vec::new();
    let mut rest = xml;
    while let Some(open) = rest.find("<Key>") {
        let after = &rest[open + "<Key>".len()..];
        let Some(close) = after.find("</Key>") else {
            break;
        };
        keys.push(after[..close].to_string());
        rest = &after[close + "</Key>".len()..];
    }
    // Continuation token ONLY when the result is truncated (else a stale token would
    // loop). Both tags are single-valued at the ListBucketResult root.
    let truncated = extract_tag(xml, "IsTruncated").as_deref() == Some("true");
    let next = if truncated {
        extract_tag(xml, "NextContinuationToken")
    } else {
        None
    };
    (keys, next)
}

/// Extract the first `<tag>…</tag>` element's content, or `None`.
fn extract_tag(xml: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = xml.find(&open)? + open.len();
    let end = xml[start..].find(&close)? + start;
    Some(xml[start..end].to_string())
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

impl crate::writes::AccountLogSink for AppState {
    /// Load-or-create + chain-verify a per-account log (GDPR1) + its head token — the
    /// account-scoped twin of [`LogSink::load`], but keyed under `_accounts/` and NOT
    /// gated on `is_safe_repo_slug` (its own [`is_safe_account_slug`] runs inside).
    fn load_account(&self, account: &str) -> Result<(EventLog, CasToken), EngineErr> {
        self.load_account_log(account)
    }

    /// Persist a per-account log as a compare-and-swap — the account-scoped twin of
    /// [`LogSink::persist`].
    fn persist_account(
        &self,
        account: &str,
        log: &EventLog,
        expected: &CasToken,
    ) -> Result<(), EngineErr> {
        self.persist_account_log(account, log, expected)
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
        let fast_agent = ureq::AgentBuilder::new()
            .timeout(crate::cas::LAZY_LOAD_FETCH_TIMEOUT)
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
            fast_agent,
        })
    }

    /// TEST-SUPPORT: build an [`R2Config`] pointing at a LOCAL `http://` endpoint
    /// (a mock R2 the wire test spins up), with a short timeout + dummy credentials.
    /// `#[doc(hidden)]` + `pub` because integration tests are a separate crate that
    /// cannot construct a [`ureq::Agent`] (ureq is not a dev-dependency); this keeps
    /// the ureq surface inside the crate. NOT reachable from any production path.
    #[doc(hidden)]
    #[must_use]
    pub fn for_test_endpoint(endpoint: String, bucket: String) -> Self {
        R2Config {
            host: "mock-r2.local".to_string(),
            endpoint,
            bucket,
            region: "auto".to_string(),
            key_id: "test-key".to_string(),
            secret: "test-secret".to_string(),
            tenant_id: "test-tenant".to_string(),
            agent: ureq::AgentBuilder::new()
                .timeout(Duration::from_secs(5))
                .build(),
            fast_agent: ureq::AgentBuilder::new()
                .timeout(crate::cas::LAZY_LOAD_FETCH_TIMEOUT)
                .build(),
        }
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
        self.get_object_on(&self.agent, key)
    }

    /// As [`get_object`](Self::get_object), but through the TIGHT-timeout
    /// [`fast_agent`](Self::fast_agent) — the accept-loop lazy-load-on-miss variant,
    /// so a slow/throttling R2 manifest read fails fast instead of stalling the
    /// single-threaded accept loop for the standard 30s.
    pub fn get_object_bounded(&self, key: &str) -> Result<Option<Vec<u8>>, EngineErr> {
        self.get_object_on(&self.fast_agent, key)
    }

    /// The shared body of [`get_object`](Self::get_object) /
    /// [`get_object_bounded`](Self::get_object_bounded), parameterized by the HTTP
    /// agent (so the two differ ONLY in the timeout budget).
    fn get_object_on(&self, agent: &ureq::Agent, key: &str) -> Result<Option<Vec<u8>>, EngineErr> {
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
        let resp = agent
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

    /// List every object key under `prefix` via ListObjectsV2 (the DURABLE
    /// authoritative enumeration the GDPR1 erasure planner needs — the B1 completeness
    /// fix). Follows continuation-token pagination up to [`MAX_LIST_PAGES`]; a still-
    /// truncated listing at the cap is a FAIL-CLOSED error (never a silent truncation
    /// that would UNDER-report the subject's owned set). Generic-503 on any transport/
    /// non-2xx fault (no storage-topology leak to the client). Used only on the rare,
    /// authorized erasure path — NOT a hot read.
    pub fn list_keys(&self, prefix: &str) -> Result<Vec<String>, EngineErr> {
        let mut out = Vec::new();
        let mut token: Option<String> = None;
        for _page in 0..MAX_LIST_PAGES {
            // Canonical query: ASCII-sorted `k=v`, each value RFC-3986-encoded. Sorted
            // order is `continuation-token` < `list-type` < `prefix`.
            let mut params: Vec<(String, String)> = vec![
                ("list-type".to_string(), "2".to_string()),
                ("prefix".to_string(), sigv4::encode_query_value(prefix)),
            ];
            if let Some(t) = &token {
                params.push((
                    "continuation-token".to_string(),
                    sigv4::encode_query_value(t),
                ));
            }
            params.sort();
            let canonical_query = params
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect::<Vec<_>>()
                .join("&");
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            let signed = sigv4::sign_s3_list(
                &self.host,
                &self.bucket,
                &canonical_query,
                &self.key_id,
                &self.secret,
                &self.region,
                now,
            );
            // The wire URL MUST carry the byte-identical canonical query that was signed.
            let url = format!("{}/{}?{canonical_query}", self.endpoint, self.bucket);
            let resp = self
                .agent
                .get(&url)
                .set("Authorization", &signed.authorization)
                .set("x-amz-date", &signed.amz_date)
                .set("x-amz-content-sha256", &signed.content_sha256)
                .call();
            let body = match resp {
                Ok(r) => r.into_string().map_err(|e| {
                    eprintln!("hugit-serve: R2 LIST body read failed: {e}");
                    EngineErr::unavailable("engine storage read failed".to_string())
                })?,
                Err(e) => {
                    eprintln!("hugit-serve: R2 LIST failed for prefix {prefix:?}: {e}");
                    return Err(EngineErr::unavailable(
                        "engine storage temporarily unavailable".to_string(),
                    ));
                }
            };
            let (keys, next) = parse_listv2_xml(&body);
            out.extend(keys);
            match next {
                Some(t) => token = Some(t),
                None => return Ok(out), // not truncated → complete
            }
        }
        // Still truncated at the page cap: FAIL-CLOSED (never under-report the set).
        Err(EngineErr::unavailable(
            "listagem durável excedeu o limite de páginas (fail-closed)".to_string(),
        ))
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

    fn get_object_bounded(&self, key: &str) -> Result<Option<Vec<u8>>, String> {
        R2Config::get_object_bounded(self, key).map_err(|e| format!("R2 get {key}: {}", e.reason))
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

/// Build a CAS-backed [`RepoState`] by loading `repo`'s manifests from R2 + the CAS
/// (LAZY: read the manifests + resolve HEAD's tree; objects are fetched on demand).
///
/// The single construction path shared by boot ([`AppState::load_repos_from_env`])
/// AND the runtime lazy-load ([`AppState::repo_state_or_load`]) — so a repo listed in
/// the boot `HUGIT_SERVE_CAS_REPO` comma-list and a repo provisioned on ANOTHER
/// instance (discovered on first request) are built identically. Returns the state +
/// the CAS connectivity self-probe (boot uses it for the first repo; the lazy path
/// discards it).
///
/// `mode` picks the latency discipline: [`LoadMode::Boot`](crate::cas::LoadMode::Boot)
/// keeps the standard timeouts + throttle-retry; [`LoadMode::LazyLoad`](crate::cas::LoadMode::LazyLoad)
/// is bounded (no retry sleeps, tight timeout) for the single-threaded accept loop.
///
/// # Errors
/// [`RepoLoadError::Absent`](crate::cas::RepoLoadError::Absent) — `refs.json` is
/// authoritatively absent (a not-yet-provisioned/nonexistent repo).
/// [`RepoLoadError::Transient`](crate::cas::RepoLoadError::Transient) — a CAS/R2
/// transport/throttle/timeout/decode fault (retry may succeed; never negative-cache).
/// What [`AppState::repo_state_or_load`] should do with a lazy-load attempt — the pure
/// decision, factored out so the L1/L2/L3/W1 policy is unit-tested without a live CAS/R2
/// or the concrete `&'static` insert.
enum LazyLoadAct {
    /// The manifests loaded → insert this state + serve it. Boxed: `RepoState` is large
    /// and the other variants are unit (clippy::large_enum_variant).
    Insert(Box<RepoState>),
    /// W1: `refs.json` absent but the durable genesis log EXISTS (provisioned, never
    /// pushed) → mint + insert an EMPTY CAS seam (the caller builds it).
    InsertEmpty,
    /// Authoritatively absent (refs.json 404 AND no genesis log) → negative-cache + 404.
    NegativeCache,
    /// A TRANSIENT fault OR over budget → honest miss, do NOT negative-cache (retry next).
    NoCacheRetry,
}

/// The pure L1/L2/L3/W1 decision core of [`AppState::repo_state_or_load`].
///
/// * `budget_ok` — the L2 global-budget verdict. When `false`, `load` is NEVER invoked
///   (so an over-budget attempt does no R2 round-trip) → [`LazyLoadAct::NoCacheRetry`].
/// * `load` — the bounded lazy-load (invoked at most once, only when in budget).
/// * `genesis_exists` — the W1 durable-genesis-log predicate (invoked only on an
///   [`RepoLoadError::Absent`](crate::cas::RepoLoadError::Absent) result).
fn decide_lazy_load(
    budget_ok: bool,
    load: impl FnOnce() -> Result<RepoState, crate::cas::RepoLoadError>,
    genesis_exists: impl FnOnce() -> bool,
) -> LazyLoadAct {
    if !budget_ok {
        return LazyLoadAct::NoCacheRetry;
    }
    match load() {
        Ok(state) => LazyLoadAct::Insert(Box::new(state)),
        // L3 + W1: refs.json authoritatively absent. Only NOW consult the genesis log.
        Err(crate::cas::RepoLoadError::Absent) => {
            if genesis_exists() {
                LazyLoadAct::InsertEmpty
            } else {
                LazyLoadAct::NegativeCache
            }
        }
        // L3: a transient fault (5xx/timeout/throttle/decode/partial seam) is NEVER
        // negative-cached — a real repo must recover on the next request.
        Err(crate::cas::RepoLoadError::Transient(_)) => LazyLoadAct::NoCacheRetry,
    }
}

fn build_cas_repo_state(
    cas: &crate::cas::CasClient,
    r2: &R2Config,
    tenant: &str,
    repo: &str,
    receive_pack: bool,
    mode: crate::cas::LoadMode,
) -> Result<(RepoState, crate::cas::CasSelfcheckProbe), crate::cas::RepoLoadError> {
    let (cas_src, root, refs) =
        crate::cas::load_manifests_from_cas(cas.clone(), r2, tenant, repo, mode)?;
    let probe = cas_src.selfcheck_probe();
    // A shared handle to the lazy source's live oid→blake3 index, so a successful
    // push can merge new entries into the SAME cell it reads.
    let live_oid_index = cas_src.live_index_handle();
    let src: Arc<dyn hugit_proto::ObjectSource + Send + Sync> = Arc::new(cas_src);
    // The CAS-mode push write seam — populated ONLY when receive-pack is on (a stock
    // deploy gets `None`: identical to the read-only behavior).
    let cas_write = receive_pack.then(|| CasWriteSeam {
        cas_client: cas.clone(),
        tenant: tenant.to_string(),
        repo_slug: repo.to_string(),
        r2: r2.clone(),
    });
    // The clone-pack cache rides the SAME write-scoped seam a CAS push finalizes
    // through — so it exists exactly when a build CAN PUT (receive-pack on).
    let clone_cache = cas_write.as_ref().map(CasWriteSeam::clone_cache_seam);
    Ok((
        RepoState {
            git_source: src,
            git_root_tree: root,
            git_refs: LiveRefs::new(refs),
            git_dir: None, // CAS mode: no local dir; push sink is cas_write
            cas_write,
            live_oid_index: Some(live_oid_index),
            clone_cache,
        },
        probe,
    ))
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

/// Whether the GDPR1 erasure AUTO-EXECUTOR background sweep is enabled
/// (`HUGIT_ERASURE_AUTO_EXECUTE=1|true`). DEFAULT OFF — fail-closed: a routine deploy NEVER
/// auto-fires an irreversible Art.17 erasure until the owner DELIBERATELY flips this on at
/// go-live (after clw's re-audit). Mirrors [`receive_pack_enabled`].
fn erasure_auto_execute_enabled() -> bool {
    std::env::var("HUGIT_ERASURE_AUTO_EXECUTE")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

/// The write doors' identity abstraction (ADR-0004 leg 2). `store_chain` is the FLAG-GATED write
/// mapper (pseudonym when the kill-switch is ON, cleartext when OFF, may MINT the caller's key);
/// `lookup_pseudonym` reconstructs a principal's pseudonym READ-ONLY and FLAG-INDEPENDENTLY for
/// the idempotency match arm (no mint; `None` when there is no key), so a key first executed
/// while ON still dedups after the kill-switch flips OFF (no double-execute).
impl crate::writes::WriteIdentity for AppState {
    fn store_chain(&self, chain: &[String]) -> Result<Vec<String>, EngineErr> {
        self.pseudonymize_write_chain(chain)
    }
    fn lookup_pseudonym(&self, principal: &str) -> Result<Option<String>, EngineErr> {
        crate::provenance_pii_redact::readonly_principal_pseudonym(self, principal)
    }
}

/// The forward write-path pseudonymisation gate (ADR-0004 leg 2), **default ON**. This is an
/// ops KILL-SWITCH, not a feature flag: it ships ON so cleartext PII stops entering the chain,
/// and can be turned OFF (`HUGIT_SERVE_PROV_PSEUDONYM=0|false|off`) as a break-glass without a
/// binary rollback. Only an explicit disable value turns it off — any other value (typo, empty,
/// unset) keeps it ON (fail-safe toward MORE privacy, the opposite polarity of a new-surface
/// feature gate).
fn prov_pseudonym_enabled() -> bool {
    match std::env::var("HUGIT_SERVE_PROV_PSEUDONYM") {
        Ok(v) => !(v == "0" || v.eq_ignore_ascii_case("false") || v.eq_ignore_ascii_case("off")),
        Err(_) => true, // unset ⇒ ON (default)
    }
}

/// PURE spawn guard (testable without env): the auto-executor loop spawns ONLY when it is
/// `enabled` AND the engine is in CAS mode (`cas_mode` — physical erase reads oid-indexes
/// from R2) AND the physical-erase seam is configured (`erase_configured` — `state.erase_config`).
/// Any of the three false → NO thread (the fail-closed default: a routine deploy never
/// auto-erases).
#[must_use]
fn should_spawn_auto_executor(enabled: bool, cas_mode: bool, erase_configured: bool) -> bool {
    enabled && cas_mode && erase_configured
}

/// Unix-ms wall clock — the same unit the erasure grace anchor (`requested_at`) is measured
/// in. A pre-epoch clock (impossible in practice) reads `0`.
fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
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
/// merely necessary — for BOTH the UPDATE and the DELETE write paths. The refs.json
/// compare-and-swap RE-VALIDATES the pusher's/deleter's per-ref `expected` precondition
/// against the FRESH base on every attempt:
/// * UPDATE — [`crate::cas::commit_cas_push_manifests`] (FIX-IFMATCH-REMERGE). Before
///   that fix a 412 re-merge blindly re-applied the ref tip on the advanced base — a
///   cross-instance same-ref force-push silently lost-updated.
/// * DELETE — [`crate::cas::remove_cas_ref_from_manifest`] (the symmetric fix). Before
///   it a 412 re-merge blindly re-removed the ref on the advanced base — a delete that
///   raced a concurrent same-ref UPDATE silently deleted the update (a lost update).
///
/// With BOTH re-validates, a concurrent same-ref advance now fails closed (StaleRef, the
/// ref is neither overwritten NOR removed), so the guard's claim matches what the update
/// AND delete write paths actually enforce.
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

/// Whether PAT git/API auth (slice 2b) is enabled — `HUGIT_SERVE_PAT_AUTH=1`/`true`.
/// Default **false** (a NEW auth surface ships disabled behind an adversarial review).
fn pat_auth_enabled_env() -> bool {
    std::env::var("HUGIT_SERVE_PAT_AUTH")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

/// The fail-closed boot guard for PAT auth on a multi-instance deploy. The in-memory
/// PAT index is SINGLE-INSTANCE-authoritative (a token minted on instance A is absent
/// from B's index until B reboots — the same cross-instance staleness class as the ref
/// hot-swap). So refuse to boot when PAT auth is enabled AND `>1` instances are
/// permitted: enabling it there would make a freshly-minted token intermittently 401
/// depending on which instance served the request. Gated on the B5 read-after-write
/// seam. Pure (takes the two booleans) → unit-tested without env.
fn pat_auth_multi_instance_guard(
    pat_auth_enabled: bool,
    allow_multi_instance: bool,
) -> Result<(), String> {
    if pat_auth_enabled && allow_multi_instance {
        return Err(
            "HUGIT_SERVE_PAT_AUTH is set with HUGIT_SERVE_ALLOW_MULTI_INSTANCE — refusing \
             to boot: the in-memory PAT index is single-instance-authoritative, so a \
             minted/revoked token would be inconsistent across instances. Land the B5 \
             read-after-write seam before running PAT auth on max_instances>1."
                .to_string(),
        );
    }
    Ok(())
}

/// The fail-closed boot guard tying the shared engine-token signing key to a
/// multi-instance deploy. `TokenStore::from_env` falls back to a per-boot RANDOM signing
/// key when `HUGIT_ENGINE_TOKEN_KEY` is unset/blank — safe single-host, but on a
/// `>1`-instance deploy each instance would then sign with its OWN random key, so a token
/// minted on instance A is rejected by instance B: exactly the pre-#128 ~50% 401 failure
/// the stateless HMAC-signed engine token (WP-B5) was built to close. A secret-provisioning
/// slip on one instance would silently reproduce it. So refuse to boot when multi-instance
/// is permitted AND the key is absent/blank (mirrors the sibling single-instance-authoritative
/// guards). Pure (takes the two booleans) → unit-tested without env.
fn engine_token_key_multi_instance_guard(
    allow_multi_instance: bool,
    key_present: bool,
) -> Result<(), String> {
    if allow_multi_instance && !key_present {
        return Err(
            "HUGIT_SERVE_ALLOW_MULTI_INSTANCE is set but HUGIT_ENGINE_TOKEN_KEY is \
             absent/blank — refusing to boot: without a shared engine-token signing key \
             each instance falls back to its own per-boot random key, so a token minted on \
             one instance is rejected by another (the pre-#128 ~50% 401 failure). Set \
             HUGIT_ENGINE_TOKEN_KEY to the same value on every instance."
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
            // git convention: bare dir's trailing ".git" is not part of served slug
            let slug = base.strip_suffix(".git").unwrap_or(base);
            (slug.to_string(), dir)
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

/// A GIT_DIR-mode read object source: the boot snapshot (everything reachable at
/// boot) PLUS a live fallback to the git dir's on-disk LOOSE objects — the exact
/// sink the receive-pack path writes to ([`GitDirCas`](hugit_proto::write::store::GitDirCas)).
///
/// Why the fallback: `load_git_dir` snapshots the object plane once at boot, but
/// the ref advertisement hot-swaps a freshly-pushed tip immediately (live ref
/// hot-swap). Without the fallback a pushed tip whose OBJECTS postdate the boot
/// snapshot would advertise but 404 on clone-back until the next reboot (object
/// plane ≠ ref plane). The fallback makes GIT_DIR clone-back of a pushed tip work
/// with NO reboot — the loose file IS the canonical on-disk form git itself reads.
///
/// Fail-closed like the snapshot: a loose file whose inflated framing fails to
/// decode, or whose re-derived git SHA-1 does not equal the requested oid, is an
/// ERROR, never silently served bytes (`decode_loose` length-checks the header;
/// the content-address check below is the second verifier).
struct GitDirLooseObjectSource {
    /// The boot snapshot (fast path, already content-address-verified on insert).
    snapshot: hugit_proto::CasObjectSource,
    /// `<git_dir>/objects` — the fallback's loose-object fan-out directory.
    objects_dir: PathBuf,
}

impl GitDirLooseObjectSource {
    /// Wrap a boot snapshot with its git dir's `<git_dir>/objects` fallback.
    fn new(snapshot: hugit_proto::CasObjectSource, git_dir: &str) -> Self {
        Self {
            snapshot,
            objects_dir: PathBuf::from(git_dir).join("objects"),
        }
    }
}

impl hugit_proto::ObjectSource for GitDirLooseObjectSource {
    fn get(
        &self,
        oid: &gix_hash::ObjectId,
    ) -> Result<Option<hugit_proto::GitObject>, hugit_proto::PackError> {
        // Fast path: the boot snapshot already holds every boot-reachable object.
        if let Some(obj) = self.snapshot.get(oid)? {
            return Ok(Some(obj));
        }
        // Fallback: a loose object a push landed after the boot snapshot. Git's
        // loose fan-out is `objects/<first-2>/<rest>`, keyed by full hex.
        let hex = oid.to_hex().to_string();
        let (fan, rest) = hex.split_at(2);
        let path = self.objects_dir.join(fan).join(rest);
        let compressed = match std::fs::read(&path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => {
                return Err(hugit_proto::PackError::Source(format!(
                    "git-dir loose fallback: reading {}: {e}",
                    path.display()
                )));
            }
        };
        // The loose file is zlib(<type> <len>\0<body>); inflate to the framing
        // the CAS decoder expects, then decode length-checked + type-checked.
        // Reuses the write path's same helper (`hugit-proto`), so the byte shape
        // is exactly what `receive_pack` lands on disk (and the exact-pin holds).
        let cas_obj = hugit_proto::write::store::CasObject {
            oid: hex,
            bytes: compressed,
        };
        let framing = hugit_proto::write::store::inflate_loose_framing(&cas_obj)
            .map_err(|e| hugit_proto::PackError::Source(format!("git-dir loose inflate: {e}")))?;
        let (kind, body) = crate::cas::decode_loose(&framing)
            .map_err(|e| hugit_proto::PackError::Source(format!("git-dir loose decode: {e}")))?;
        let obj = hugit_proto::GitObject::new(kind, body);
        // Content-address check (the second verifier): the bytes under `oid` must
        // hash to `oid`. A mismatch is corrupt storage — never served into a clone.
        let actual = obj.try_oid()?;
        if &actual != oid {
            return Err(hugit_proto::PackError::AddressMismatch {
                stored: *oid,
                actual,
            });
        }
        Ok(Some(obj))
    }
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

/// The caller's owning tenant org (`clerk:{org}:{user}` → `org`) when it is a
/// store-safe account slug, else `None` (operator/anon/unknown/malformed, or an
/// org that is not [`is_safe_account_slug`]). The SOFT counterpart of
/// [`write_provision::derive_owner_tenant`](crate::writes::verbs::write_provision::derive_owner_tenant)
/// (which errors instead of `None`) — used to key the G11 user-scoped slug for READS
/// and to strip the display prefix in the identity-scoped views. A `None` caller has no
/// user-scoped namespace, so it resolves ONLY the legacy flat key (back-compat).
#[must_use]
fn tenant_org(principal: &[String]) -> Option<String> {
    let rest = principal.first()?.strip_prefix("clerk:")?;
    let org = rest.split(':').next().unwrap_or("");
    if org.is_empty() || !is_safe_account_slug(org) {
        return None;
    }
    Some(org.to_string())
}

/// Strip the caller's `<tenant>/` prefix from a STORED slug to get the DISPLAY name
/// (G11: the routable/display id stays the BARE name; the owner-tenant prefix is an
/// internal storage detail). A legacy flat slug (no prefix) is returned unchanged.
#[must_use]
fn display_slug(stored: &str, principal: &[String]) -> String {
    if let Some(tenant) = tenant_org(principal)
        && let Some(bare) = stored.strip_prefix(&format!("{tenant}/"))
    {
        return bare.to_string();
    }
    stored.to_string()
}

/// A single safe path segment: non-empty, ≤100 chars, ASCII alnum + `-_.`, never
/// `.`/`..`/containing `..` or a path separator. Blocks URL path-traversal into
/// arbitrary files / R2 keys. The building block of [`is_safe_repo_slug`].
#[must_use]
fn is_safe_repo_segment(seg: &str) -> bool {
    !seg.is_empty()
        && seg.len() <= 100
        && seg != "."
        && seg != ".."
        && !seg.contains("..")
        && !seg.contains('/')
        && !seg.contains('\\')
        && seg
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
}

/// A safe repo slug — EITHER a single safe segment (the legacy flat key) OR a G11
/// user-scoped `<owner_tenant>/<name>` stored key (exactly ONE `/`). For the scoped
/// shape the tenant side MUST be a safe ACCOUNT slug ([`is_safe_account_slug`]:
/// `[a-z0-9-]`, ≤64) — which excludes the reserved `_accounts` prefix (underscore) so a
/// scoped repo key can NEVER collide with the account store — and the name side a safe
/// single segment. A second `/` is rejected (the name side would then contain `/`), so
/// traversal (`../…`, `a/../b`) stays blocked per segment. Blocks URL path-traversal
/// into arbitrary files / R2 keys.
#[must_use]
pub fn is_safe_repo_slug(repo: &str) -> bool {
    match repo.split_once('/') {
        Some((tenant, name)) => is_safe_account_slug(tenant) && is_safe_repo_segment(name),
        None => is_safe_repo_segment(repo),
    }
}

/// Normalize a repo slug: case-sensitive strip of exactly ONE trailing `.git`
/// from a bare slug or the LEAF of a scoped `<owner>/<name>` slug. Central
/// single source of truth so every entrypoint (boot GIT_DIR loop, runtime
/// set_repo_git/insert_runtime_repo, resolve_repo_slug, validate_name
/// provision) sees the same canonical form.
///
/// Cases (exhaustive — see test `normalize_repo_slug_cases`):
/// - `"src"` → `"src"`
/// - `"src.git"` → `"src"`
/// - `"repo.git.git"` → `"repo.git"` (single strip, leaves second segment)
/// - `"my.git-tools"` → `"my.git-tools"` (suffix-only)
/// - `"src.GIT"` → `"src.GIT"` (case-sensitive, NOT stripped)
/// - `".git"` → `""` (empty — caller rejects via is_safe_repo_slug)
/// - `"a/b"` → `"a/b"` (owner normalized only if leaf ends with .git)
/// - `"a/b.git"` → `"a/b"` (strip leaf)
/// - `"a/b/c"` → `"a/b/c"` (only ONE split; multi-slash rejected by is_safe_repo_slug)
#[must_use]
pub fn normalize_repo_slug(slug: &str) -> String {
    if let Some((owner, leaf)) = slug.split_once('/') {
        let leaf = leaf.strip_suffix(".git").unwrap_or(leaf);
        format!("{owner}/{leaf}")
    } else {
        slug.strip_suffix(".git").unwrap_or(slug).to_string()
    }
}

/// Whether `account` is a safe account slug for the reserved `_accounts/{slug}` store
/// key (GDPR1). STRICTER than [`is_safe_repo_slug`]: the Clerk-org shape `[a-z0-9-]`,
/// 1..=64, NO dot (so a slug can never be `.`/`..` or embed a traversal), no path
/// separator. The account slug is DERIVED from a Clerk-minted principal
/// (`clerk:{org}:...`) so it is already this shape; this is the traversal-safe
/// defense-in-depth gate that runs before the slug is ever spliced into an R2 key —
/// a malformed org can NEVER produce a `_accounts/../evil.json` key (it fails here
/// first, → 404, no store touch).
#[must_use]
pub fn is_safe_account_slug(account: &str) -> bool {
    !account.is_empty()
        && account.len() <= 64
        && account
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

#[cfg(test)]
mod tests {
    use super::*;
    use hugit_proto::ObjectSource;
    use std::collections::HashMap;

    // ── GDPR1 erasure auto-executor spawn guard (pure, env-free) ─────────────────

    #[test]
    fn auto_executor_spawns_only_when_enabled_cas_and_erase_seam() {
        // The full truth table: the loop spawns ONLY on (enabled AND CAS AND erase-seam).
        assert!(
            should_spawn_auto_executor(true, true, true),
            "enabled + CAS + erase seam → spawn"
        );
        // Any single gate off → fail-closed no-spawn.
        assert!(
            !should_spawn_auto_executor(false, true, true),
            "not enabled (the DEFAULT, HUGIT_ERASURE_AUTO_EXECUTE unset) → NEVER spawn"
        );
        assert!(
            !should_spawn_auto_executor(true, false, true),
            "no CAS mode → no spawn (physical erase needs the R2 oid-index)"
        );
        assert!(
            !should_spawn_auto_executor(true, true, false),
            "no erase seam configured → no spawn (no half-live delete path)"
        );
        assert!(
            !should_spawn_auto_executor(false, false, false),
            "the routine-deploy default (all off) never auto-fires an irreversible erasure"
        );
    }

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

    // ── WP-B5 read-after-write ref refresh ────────────────────────────────────────

    /// A hermetic [`crate::cas::R2Get`]: key → bytes, or a forced read fault.
    struct MockR2 {
        objects: BTreeMap<String, Vec<u8>>,
        fault: bool,
    }
    impl crate::cas::R2Get for MockR2 {
        fn get_object(&self, key: &str) -> Result<Option<Vec<u8>>, String> {
            if self.fault {
                return Err("mock R2 fault".to_string());
            }
            Ok(self.objects.get(key).cloned())
        }
    }
    fn mock_r2(entries: &[(&str, &[u8])]) -> MockR2 {
        MockR2 {
            objects: entries
                .iter()
                .map(|(k, v)| ((*k).to_string(), v.to_vec()))
                .collect(),
            fault: false,
        }
    }
    fn seed_live(refs: &[(&str, &str)]) -> LiveRefs {
        LiveRefs::new(
            refs.iter()
                .map(|(r, o)| ((*r).to_string(), (*o).to_string()))
                .collect(),
        )
    }

    #[test]
    fn refresh_installs_the_durable_manifest_adds_updates_and_drops() {
        // Another instance advanced main v1→v2, added `feat`, and deleted `old`. The refresh
        // installs the durable manifest verbatim: update propagates, add propagates, and the
        // cross-instance delete propagates (a full replace, not a merge).
        let live = seed_live(&[("refs/heads/main", "v1"), ("refs/heads/old", "z")]);
        let manifest =
            br#"{"head":"refs/heads/main","refs":{"refs/heads/main":"v2","refs/heads/feat":"c"}}"#;
        let r2 = mock_r2(&[("d863fafb/alpha/refs.json", manifest)]);
        AppState::refresh_repo_refs_once(&r2, "d863fafb", "alpha", &live);
        let s = live.snapshot();
        assert_eq!(s.get("refs/heads/main").map(String::as_str), Some("v2"));
        assert_eq!(s.get("refs/heads/feat").map(String::as_str), Some("c"));
        assert!(
            !s.contains_key("refs/heads/old"),
            "a cross-instance delete propagates"
        );
    }

    #[test]
    fn refresh_absent_manifest_keeps_the_cache_fail_safe() {
        let live = seed_live(&[("refs/heads/main", "v1")]);
        let r2 = mock_r2(&[]); // no refs.json → get_object → None
        AppState::refresh_repo_refs_once(&r2, "t", "alpha", &live);
        assert_eq!(
            live.snapshot().get("refs/heads/main").map(String::as_str),
            Some("v1"),
            "an absent manifest never clobbers a good live cache with empty"
        );
    }

    #[test]
    fn refresh_read_fault_keeps_the_cache() {
        let live = seed_live(&[("refs/heads/main", "v1")]);
        let r2 = MockR2 {
            objects: BTreeMap::new(),
            fault: true,
        };
        AppState::refresh_repo_refs_once(&r2, "t", "alpha", &live);
        assert_eq!(
            live.snapshot().get("refs/heads/main").map(String::as_str),
            Some("v1"),
            "a read fault keeps the cache (retry next tick)"
        );
    }

    #[test]
    fn refresh_malformed_manifest_keeps_the_cache() {
        let live = seed_live(&[("refs/heads/main", "v1")]);
        let r2 = mock_r2(&[("t/alpha/refs.json", b"not json at all")]);
        AppState::refresh_repo_refs_once(&r2, "t", "alpha", &live);
        assert_eq!(
            live.snapshot().get("refs/heads/main").map(String::as_str),
            Some("v1"),
            "a malformed manifest never installs a corrupt/empty map"
        );
    }

    #[test]
    fn refresh_does_not_revert_a_concurrent_push_hot_swap() {
        // F1/WP-B5 monotonic guard: a push's hot-swap that lands AFTER the refresher
        // snapshots the generation but BEFORE it installs must NOT be reverted by the
        // stale pre-push snapshot the (unversioned) R2 GET carried. We simulate the race
        // by (1) letting the refresher read the OLD manifest into a fresh LiveRefs whose
        // generation we advance mid-read via a manual hot-swap, using the split primitives.
        let live = seed_live(&[("refs/heads/main", "v1")]);
        // Refresher captures the generation at read-START.
        let gen_at_start = live.generation();
        // A concurrent push hot-swaps `main` v1→v2 (bumps the generation) — this is the
        // just-`ok`'d tip that must survive.
        live.set_ref("refs/heads/main", "v2");
        // The refresher now tries to install the STALE pre-push snapshot (still v1). With
        // the monotonic guard it is a no-op because the generation advanced during the read.
        let installed = live.replace_if_unchanged(
            BTreeMap::from([("refs/heads/main".to_string(), "v1".to_string())]),
            gen_at_start,
        );
        assert!(
            !installed,
            "the stale install is skipped once a hot-swap landed"
        );
        assert_eq!(
            live.snapshot().get("refs/heads/main").map(String::as_str),
            Some("v2"),
            "the locally-newer hot-swap tip survives the stale refresh"
        );
    }

    #[test]
    fn refresh_installs_when_no_concurrent_hot_swap() {
        // The guard only skips on a RACE: with no intervening hot-swap the refresh installs
        // the durable manifest exactly as before (adds/updates/cross-instance deletes).
        let live = seed_live(&[("refs/heads/main", "v1")]);
        let manifest = br#"{"head":"refs/heads/main","refs":{"refs/heads/main":"v2"}}"#;
        let r2 = mock_r2(&[("t/alpha/refs.json", manifest)]);
        AppState::refresh_repo_refs_once(&r2, "t", "alpha", &live);
        assert_eq!(
            live.snapshot().get("refs/heads/main").map(String::as_str),
            Some("v2"),
            "absent a concurrent hot-swap the durable manifest installs verbatim"
        );
    }

    #[test]
    fn record_pat_used_is_monotonic_and_snapshotted() {
        let st = AppState::new(PathBuf::from("/tmp/logs"), "tok".to_string());
        st.record_pat_used("pat_x".to_string(), 100);
        st.record_pat_used("pat_x".to_string(), 50); // out-of-order → never rewinds
        st.record_pat_used("pat_y".to_string(), 200);
        let snap = st.pat_last_used_snapshot();
        assert_eq!(snap.get("pat_x"), Some(&100), "the monotonic max wins");
        assert_eq!(snap.get("pat_y"), Some(&200));
        assert_eq!(snap.get("pat_absent"), None, "an unseen pat has no stamp");
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
    fn safe_slugs_accept_user_scoped_keys() {
        // G11: a `<owner_tenant>/<name>` stored key is safe (owner = an account slug).
        for ok in ["org-a/foo", "acme-labs/my-repo", "t1/repo_1", "org-a/a.b"] {
            assert!(is_safe_repo_slug(ok), "{ok} should be safe (scoped)");
        }
    }

    #[test]
    fn unsafe_slugs_block_traversal() {
        for bad in [
            "",
            ".",
            "..",
            "../etc",
            "a\\b",
            "a..b",
            "../../etc/passwd",
            "a/b/c",            // two segments below the owner (not a valid scoped key)
            "org-a/../etc",     // traversal in the name side
            "_accounts/victim", // reserved account prefix is not an owner tenant
            "Org_A/foo",        // owner side is not a safe account slug (uppercase/_)
            "org.a/foo",        // owner side has a dot (not an account slug)
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
            clone_cache: None,
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
    fn clone_pack_building_snapshot_reports_idle_and_sorted_building_set() {
        let st = AppState::new(PathBuf::from("/tmp/logs"), "tok".to_string());
        // Empty guard set → idle.
        assert_eq!(st.clone_pack_building_snapshot(), "idle");
        // Insert out of order → the snapshot is sorted + prefixed.
        {
            let mut set = st.clone_pack_building.lock().unwrap();
            set.insert("githugr".to_string());
            set.insert("hugit".to_string());
        }
        assert_eq!(st.clone_pack_building_snapshot(), "building:githugr,hugit");
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
    fn git_dir_member_bare_path_strips_dot_git_suffix() {
        let (slug, dir) = parse_git_dir_member("/srv/git/src.git").expect("bare .git path");
        assert_eq!(slug, "src");
        assert_eq!(dir, "/srv/git/src.git");
        // bare ".git" alone → empty slug → is_safe fails
        assert!(parse_git_dir_member("/srv/git/.git").is_err());
        // double suffix only strips one
        let (slug2, _) = parse_git_dir_member("/srv/git/repo.git.git").expect("double suffix");
        assert_eq!(slug2, "repo.git");
        // non-suffix preserved
        let (slug3, _) = parse_git_dir_member("/srv/git/my.git-tools").expect("non-suffix");
        assert_eq!(slug3, "my.git-tools");
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

    #[test]
    fn normalize_repo_slug_cases() {
        // Bare slugs
        assert_eq!(normalize_repo_slug("src"), "src");
        assert_eq!(normalize_repo_slug("src.git"), "src");
        assert_eq!(normalize_repo_slug("repo.git.git"), "repo.git"); // single strip
        assert_eq!(normalize_repo_slug("my.git-tools"), "my.git-tools"); // suffix-only
        assert_eq!(normalize_repo_slug("src.GIT"), "src.GIT"); // case-sensitive
        assert_eq!(normalize_repo_slug(".git"), ""); // empty → fail-closed
        assert_eq!(normalize_repo_slug(""), ""); // empty stays empty
        // Scoped slugs
        assert_eq!(normalize_repo_slug("a/b"), "a/b");
        assert_eq!(normalize_repo_slug("a/b.git"), "a/b");
        assert_eq!(normalize_repo_slug("org-a/foo"), "org-a/foo");
        assert_eq!(normalize_repo_slug("org-a/foo.git"), "org-a/foo");
        assert_eq!(
            normalize_repo_slug("org-a/my.git-tools"),
            "org-a/my.git-tools"
        );
    }

    // ── GIT_DIR loose-object fallback: clone-back of a freshly-pushed tip ──────

    /// Build a REAL zlib-compressed loose object at `<root>/objects/<xx>/<rest>`
    /// (the exact shape `GitDirCas::put` / `receive_pack` lands on disk), using
    /// the same exact-pinned `flate2` as the write path. Returns the oid the
    /// canonical git hash computes over the loose pre-image `<kind> <len>\0<body>`.
    fn place_loose_object(root: &std::path::Path, body: &[u8]) -> String {
        use std::io::Write as _;
        let pre = {
            let mut framing = format!("blob {}\0", body.len()).into_bytes();
            framing.extend_from_slice(body);
            framing
        };
        // The oid the canonical store derives for a blob with this body — the DIGEST
        // is taken from `GitObject::oid()`, so the test asserts byte-identity to the
        // real content-address (no hand-rolled hash that could drift).
        let oid = hugit_proto::GitObject::new(hugit_proto::ObjectKind::Blob, body.to_vec())
            .oid()
            .to_hex()
            .to_string();
        let (fan, rest) = oid.split_at(2);
        let dir = root.join("objects").join(fan);
        std::fs::create_dir_all(&dir).expect("mk fan dir");
        let mut comp = Vec::new();
        {
            let mut enc =
                flate2::write::ZlibEncoder::new(&mut comp, flate2::Compression::default());
            enc.write_all(&pre).expect("compress");
            enc.finish().expect("finish");
        }
        std::fs::write(dir.join(rest), &comp).expect("write loose object");
        oid
    }

    #[test]
    fn git_dir_loose_fallback_serves_a_post_boot_push_without_reload() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let root = std::env::temp_dir().join(format!("gitdirloose-fallback-{nanos}"));
        std::fs::create_dir_all(root.join("objects")).expect("mk objects");
        // A boot-time object, already in the snapshot (fast path).
        let boot_body = b"boot-time blob";
        let boot_oid = place_loose_object(&root, boot_body);
        let mut snapshot = hugit_proto::CasObjectSource::new();
        snapshot.insert_raw(hugit_proto::ObjectKind::Blob, boot_body.to_vec());
        let src = GitDirLooseObjectSource::new(snapshot, root.to_str().unwrap());
        // Snapshot hit serves.
        let boot_id = gix_hash::ObjectId::from_hex(boot_oid.as_bytes()).unwrap();
        assert_eq!(
            src.get(&boot_id).unwrap().map(|o| o.data),
            Some(boot_body.to_vec()),
            "boot snapshot path unchanged"
        );
        // A PUSHED object (post-boot, loose on disk, NOT in the snapshot): the
        // ref advertisement hot-swaps it into the wire immediately, so the object
        // source MUST serve it with NO reload — this is the regression this fixes.
        let pushed_body = b"post-boot pushed blob";
        let pushed_oid = place_loose_object(&root, pushed_body);
        let pushed_id = gix_hash::ObjectId::from_hex(pushed_oid.as_bytes()).unwrap();
        assert!(
            !src.snapshot.contains(&pushed_id),
            "the pushed object must NOT be in the boot snapshot (defines the miss)"
        );
        let got = src.get(&pushed_id).expect("loose fallback serves the push");
        assert_eq!(
            got.map(|o| o.data),
            Some(pushed_body.to_vec()),
            "clone-back of a freshly-pushed tip resolves WITHOUT a reboot"
        );
        // A garbage loose file (corrupt storage) fails CLOSED, never serves bytes.
        let (fan, rest) = pushed_oid.split_at(2);
        std::fs::write(
            root.join("objects").join(fan).join(rest),
            b"\x00\x01\x02not-zlib",
        )
        .expect("write corrupt loose");
        assert!(
            src.get(&pushed_id).is_err(),
            "a corrupt loose object must error (fail-closed), not serve junk"
        );
        let _ = std::fs::remove_dir_all(&root);
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
    fn pat_auth_multi_instance_guard_fail_closed() {
        // PAT auth ON + multi-instance permitted → REFUSE boot (the in-memory index is
        // single-instance-authoritative; a mint on A would be absent from B).
        let err = pat_auth_multi_instance_guard(true, true).unwrap_err();
        assert!(err.contains("PAT index") && err.contains("single-instance"));
        // PAT auth ON, single instance → fine (the pinned prod posture).
        assert!(pat_auth_multi_instance_guard(true, false).is_ok());
        // PAT auth OFF → fine regardless of instance count.
        assert!(pat_auth_multi_instance_guard(false, true).is_ok());
        assert!(pat_auth_multi_instance_guard(false, false).is_ok());
    }

    #[test]
    fn engine_token_key_multi_instance_guard_fail_closed() {
        // Multi-instance permitted + key absent → REFUSE boot (each instance would sign
        // with its own per-boot random key → cross-instance 401s, the pre-#128 failure).
        let err = engine_token_key_multi_instance_guard(true, false).unwrap_err();
        assert!(err.contains("refusing to boot") && err.contains("HUGIT_ENGINE_TOKEN_KEY"));
        // Multi-instance + shared key present → fine (fungible tokens).
        assert!(engine_token_key_multi_instance_guard(true, true).is_ok());
        // Single instance → fine regardless of the key (per-boot random key is single-host safe).
        assert!(engine_token_key_multi_instance_guard(false, false).is_ok());
        assert!(engine_token_key_multi_instance_guard(false, true).is_ok());
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

    #[test]
    fn repo_state_or_load_local_mode_is_a_pure_passthrough() {
        // Local/dev mode has `provision: None` → NO lazy-load (there is a fixed
        // on-disk/in-memory set). So `repo_state_or_load` == `repo_state`: a known
        // repo resolves, an unknown one is an honest `None` (never an R2 probe).
        let st = state_with(&[("alpha", "{}")]);
        assert!(
            st.repo_state_or_load("alpha").is_some(),
            "known repo resolves"
        );
        assert!(
            st.repo_state_or_load("ghost").is_none(),
            "unknown repo in Local mode → None, no lazy-load (provision is None)"
        );
        // And it did NOT negative-cache in Local mode (the CAS-gate returns before the
        // cache is ever consulted) — a defensive check that the gate order is right.
        assert!(
            !st.repo_load_miss_recent("ghost"),
            "Local mode must not touch the negative cache"
        );
    }

    #[test]
    fn repo_load_negative_cache_marks_and_reads_back() {
        // The DoS guard: a slug found absent is marked, and a recent mark short-circuits
        // the next probe. A never-marked slug is not recent. (Cooldown is wall-clock;
        // a just-marked slug is unambiguously within the window.)
        let st = state_with(&[]);
        assert!(
            !st.repo_load_miss_recent("nope"),
            "unmarked slug is not recent"
        );
        st.mark_repo_load_miss("nope");
        assert!(
            st.repo_load_miss_recent("nope"),
            "a just-marked miss is recent → the next probe is skipped"
        );
        assert!(
            !st.repo_load_miss_recent("other"),
            "the negative cache is per-slug"
        );
    }

    // ── #96 lazy-load hardening: L2 (global budget) + L3/W1 (decision policy) ──────

    /// L2: the global lazy-load budget allows EXACTLY `MAX_LAZY_LOADS_PER_WINDOW`
    /// attempts per window, then denies — so a varied-slug 404 storm (which the
    /// per-slug negative cache cannot stop) cannot flood the accept loop with R2 hits.
    #[test]
    fn lazy_load_budget_caps_attempts_per_window() {
        let st = state_with(&[]);
        let mut allowed = 0u32;
        for _ in 0..(MAX_LAZY_LOADS_PER_WINDOW + 5) {
            if st.lazy_load_budget_take() {
                allowed += 1;
            }
        }
        assert_eq!(
            allowed, MAX_LAZY_LOADS_PER_WINDOW,
            "exactly the budget is spent within one window"
        );
    }

    /// L2: a fresh window resets the budget.
    #[test]
    fn lazy_load_budget_resets_next_window() {
        let st = state_with(&[]);
        for _ in 0..MAX_LAZY_LOADS_PER_WINDOW {
            assert!(st.lazy_load_budget_take());
        }
        assert!(
            !st.lazy_load_budget_take(),
            "over budget within the same window"
        );
        // Backdate the window start → the next take opens a fresh window.
        {
            let mut g = st.repo_lazy_load_budget.write().unwrap();
            let backdated = now_ms().saturating_sub(LAZY_LOAD_WINDOW_MS + 1);
            *g = (backdated, MAX_LAZY_LOADS_PER_WINDOW);
        }
        assert!(
            st.lazy_load_budget_take(),
            "a new window resets the budget to full"
        );
    }

    /// L2: over budget, the loader closure (which would do the R2 round-trip) is NEVER
    /// invoked, and the result is a no-cache retry.
    #[test]
    fn decide_over_budget_does_no_r2_call() {
        let loaded = std::cell::Cell::new(false);
        let act = decide_lazy_load(
            false, // over budget
            || {
                loaded.set(true);
                Err(crate::cas::RepoLoadError::Transient("unreachable".into()))
            },
            || panic!("genesis check must not run when over budget"),
        );
        assert!(matches!(act, LazyLoadAct::NoCacheRetry));
        assert!(
            !loaded.get(),
            "over budget → the R2-touching loader is never invoked"
        );
    }

    /// L3: an authoritatively-absent repo with NO genesis log → negative-cache + 404.
    #[test]
    fn decide_absent_without_genesis_negative_caches() {
        let act = decide_lazy_load(
            true,
            || Err(crate::cas::RepoLoadError::Absent),
            || false, // no durable genesis log.
        );
        assert!(matches!(act, LazyLoadAct::NegativeCache));
    }

    /// W1: an absent refs.json whose durable GENESIS LOG exists (provisioned but never
    /// pushed) → build an EMPTY CAS seam, NOT a 404 — so first push/clone work on any
    /// instance that did not create it.
    #[test]
    fn decide_absent_with_genesis_builds_empty_seam() {
        let genesis_checked = std::cell::Cell::new(false);
        let act = decide_lazy_load(
            true,
            || Err(crate::cas::RepoLoadError::Absent),
            || {
                genesis_checked.set(true);
                true // the durable genesis log exists.
            },
        );
        assert!(matches!(act, LazyLoadAct::InsertEmpty));
        assert!(
            genesis_checked.get(),
            "genesis existence is consulted on Absent"
        );
    }

    /// L3: a TRANSIENT fault is NEVER negative-cached (a real repo recovers next call),
    /// and the genesis check is not even consulted (it applies only to Absent).
    #[test]
    fn decide_transient_does_not_negative_cache() {
        let genesis_checked = std::cell::Cell::new(false);
        let act = decide_lazy_load(
            true,
            || Err(crate::cas::RepoLoadError::Transient("R2 5xx".into())),
            || {
                genesis_checked.set(true);
                true
            },
        );
        assert!(
            matches!(act, LazyLoadAct::NoCacheRetry),
            "transient → retry, never cache"
        );
        assert!(
            !genesis_checked.get(),
            "the genesis check runs only for Absent, never for Transient"
        );
    }

    fn operator() -> Vec<String> {
        vec!["orchestrator:hugit".to_string()]
    }
    fn tenant(org: &str) -> Vec<String> {
        vec![format!("clerk:{org}:user-1")]
    }

    // ── G11: user-scoped slug — durable enumeration, resolution, display ──────

    #[test]
    fn user_scoped_repo_enumerates_resolves_and_displays_bare() {
        // Seed a G11 user-scoped repo the way `provision` stores it: the durable log at
        // `<owner>/<name>.json` PLUS the in-memory seam under the scoped key.
        let dir = scratch_dir();
        let mut st = AppState::new(dir.clone(), "dev-token".to_string());
        std::fs::create_dir_all(dir.join("org-a")).unwrap();
        std::fs::write(
            dir.join("org-a").join("foo.json"),
            meta_log("private", "org-a"),
        )
        .unwrap();
        st.set_repo_git(
            "org-a/foo",
            Arc::new(hugit_proto::CasObjectSource::new()),
            gix_hash::ObjectId::empty_tree(gix_hash::Kind::Sha1),
            BTreeMap::new(),
        );

        // Durable enumeration (the GDPR1 erasure planner's source) MUST see the scoped
        // log — a scoped repo missed here would be un-erasable.
        let owned = st
            .authoritative_owned_repo_logs("org-a")
            .expect("enumeration");
        assert!(
            owned.iter().any(|(slug, _)| slug == "org-a/foo"),
            "durable listing enumerates the scoped log, got {:?}",
            owned.iter().map(|(s, _)| s).collect::<Vec<_>>()
        );

        // The owner resolves the bare name to the scoped stored key…
        assert_eq!(st.resolve_repo_slug("foo", &tenant("org-a")), "org-a/foo");
        // …but the identity-scoped index DISPLAYS the bare name (routable by the owner).
        assert_eq!(st.repos_for(&tenant("org-a")), vec!["foo".to_string()]);
        // A foreign tenant sees nothing and resolves only the (absent) legacy flat key.
        assert!(st.repos_for(&tenant("org-b")).is_empty());
        assert_eq!(st.resolve_repo_slug("foo", &tenant("org-b")), "foo");
    }

    // ── GDPR1 account store seam (_accounts/{slug}) ──────────────────────────

    #[test]
    fn is_safe_account_slug_accepts_clerk_org_shape_rejects_traversal() {
        for ok in ["org-a", "acme", "team-42", "x", &"a".repeat(64)] {
            assert!(is_safe_account_slug(ok), "{ok:?} is a valid account slug");
        }
        for bad in [
            "",              // empty
            &"a".repeat(65), // too long
            "Org-A",         // uppercase
            "a.b",           // dot (would allow `.`/`..` shapes)
            "..",            // traversal
            "../evil",       // traversal + sep
            "a/b",           // path sep
            "a\\b",          // backslash
            "org_a",         // underscore not in the clerk-org charset
            "org a",         // space
        ] {
            assert!(!is_safe_account_slug(bad), "{bad:?} must be rejected");
        }
    }

    /// Append one `erasure.requested` to a fresh log (the account genesis shape).
    fn erasure_log(account: &str) -> EventLog {
        let mut log = EventLog::new();
        let payload = serde_json::json!({"account":account,"subject":account,"state":"requested"})
            .to_string();
        let body = hugit_refstore::canonical_json(&payload).unwrap_or(payload);
        log.append_authorized(
            PrincipalClass::Orchestrator,
            Endpoint::Land,
            "erasure.requested",
            tenant(account),
            body,
            1,
        )
        .expect("append erasure.requested");
        log
    }

    #[test]
    fn account_log_load_or_create_then_cas_roundtrip() {
        let st = AppState::new(scratch_dir(), "dev-token".to_string());
        // Absent → the empty world with a create-only token (NOT a 404).
        let (log0, tok0) = st.load_account_log("org-a").expect("absent → empty world");
        assert!(log0.records().is_empty());
        assert_eq!(tok0, CasToken::Absent);

        // Create-only persist seeds the genesis.
        let log = erasure_log("org-a");
        st.persist_account_log("org-a", &log, &CasToken::Absent)
            .expect("create-genesis persists");

        // Reload: present + chain-verified, now with a Version token.
        let (log1, tok1) = st.load_account_log("org-a").expect("present");
        assert_eq!(log1.records().len(), 1);
        assert_eq!(log1.records()[0].kind, "erasure.requested");
        assert!(matches!(tok1, CasToken::Version(_)));

        // A stale create-only persist (Absent, but it now exists) fails the CAS.
        let e = st
            .persist_account_log("org-a", &log, &CasToken::Absent)
            .expect_err("create-only over an existing log must conflict");
        assert!(e.is_cas_conflict(), "stale Absent → cas_conflict");
    }

    #[test]
    fn account_log_is_scoped_and_not_a_repo() {
        // The account log lives under `_accounts/` — it is NOT reachable as a repo,
        // and an unsafe slug never touches the store (404, no traversal key built).
        let st = AppState::new(scratch_dir(), "dev-token".to_string());
        st.persist_account_log("org-a", &erasure_log("org-a"), &CasToken::Absent)
            .expect("genesis");
        // `_accounts/org-a` is not a servable repo slug (contains `/` conceptually /
        // the reserved prefix); a repo read for "org-a" is a plain absent 404.
        assert_eq!(
            st.load_verified("org-a").unwrap_err().status,
            404,
            "the account is not addressable as a repo"
        );
        // A traversal slug is refused before any store touch.
        assert_eq!(
            st.load_account_log("../evil").unwrap_err().status,
            404,
            "an unsafe account slug → 404, never a traversal key"
        );
        assert_eq!(
            st.persist_account_log("../evil", &erasure_log("x"), &CasToken::Absent)
                .unwrap_err()
                .status,
            404,
        );
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

    // ── count_owned_repos (the per-tenant DoS-cap denominator, WP-1 registry) ─

    #[test]
    fn count_owned_repos_reads_the_durable_per_tenant_registry() {
        // WP-1 repoint: the count is the SIZE of the durable `_tenants/{org}.json` set —
        // per-tenant by construction (a separate registry per org), so no ownership scan.
        let st = AppState::new(scratch_dir(), "dev-token".to_string());
        st.register_repo_in_tenant("org-a", "alpha", &tenant("org-a"), 1)
            .unwrap();
        st.register_repo_in_tenant("org-a", "beta", &tenant("org-a"), 2)
            .unwrap();
        st.register_repo_in_tenant("org-b", "gamma", &tenant("org-b"), 3)
            .unwrap();
        assert_eq!(st.count_owned_repos("org-a"), Some(2));
        assert_eq!(st.count_owned_repos("org-b"), Some(1));
        // A new tenant with NO registry is a genuine empty owned-set (Some(0)), NOT
        // indeterminate — an absent registry never blocks a first create.
        assert_eq!(st.count_owned_repos("org-z"), Some(0));
    }

    #[test]
    fn count_survives_a_reboot_and_holds_the_cap_defect_a() {
        // DEFECT A: the cap must be durable across engine lifetimes. Register repos, then
        // build a FRESH AppState over the SAME on-disk store with an EMPTY runtime overlay
        // (a reboot) — the durable registry still counts them, so a tenant cannot
        // re-provision past the cap after a restart.
        let dir = scratch_dir();
        {
            let st = AppState::new(dir.clone(), "dev-token".to_string());
            for i in 0..3 {
                st.register_repo_in_tenant("org-a", &format!("r{i}"), &tenant("org-a"), i as u64)
                    .unwrap();
            }
            assert_eq!(st.count_owned_repos("org-a"), Some(3));
        }
        // Reboot: a new AppState, no in-memory overlay, same durable `_tenants/` store.
        let rebooted = AppState::new(dir, "dev-token".to_string());
        assert_eq!(
            rebooted.count_owned_repos("org-a"),
            Some(3),
            "the durable registry holds the count across a reboot (defect A closed)"
        );
    }

    #[test]
    fn count_owned_repos_fails_closed_on_an_indeterminate_registry() {
        // With the WP-1 repoint the count reads ONLY the durable registry — so the
        // fail-closed axis is a CORRUPT/unverifiable registry (not a repo log). A tampered
        // `_tenants/{org}.json` that will not chain-verify makes the count indeterminate →
        // None (the provision path maps None to a refusal, so a read fault can never be
        // leveraged to slip past the cap).
        let dir = scratch_dir();
        let st = AppState::new(dir.clone(), "dev-token".to_string());
        st.register_repo_in_tenant("org-a", "alpha", &tenant("org-a"), 1)
            .unwrap();
        assert_eq!(st.count_owned_repos("org-a"), Some(1));
        // Corrupt the durable registry object (invalid JSON → load/verify fails → 503).
        std::fs::write(dir.join("_tenants").join("org-a.json"), b"not json").unwrap();
        assert_eq!(
            st.count_owned_repos("org-a"),
            None,
            "fail-closed on an indeterminate registry read"
        );
    }

    #[test]
    fn erase_decrement_is_a_hold_count_defect_c_substrate() {
        // WP-1 substrate for the erase-execute decrement: unregister removes a repo from
        // the durable set (the cap is erasable-DOWN), the twin of register.
        let st = AppState::new(scratch_dir(), "dev-token".to_string());
        st.register_repo_in_tenant("org-a", "alpha", &tenant("org-a"), 1)
            .unwrap();
        st.register_repo_in_tenant("org-a", "beta", &tenant("org-a"), 2)
            .unwrap();
        assert_eq!(st.count_owned_repos("org-a"), Some(2));
        st.unregister_repo_from_tenant("org-a", "alpha", &tenant("org-a"), 3)
            .unwrap();
        assert_eq!(st.count_owned_repos("org-a"), Some(1));
        // Idempotent: decrementing an already-absent repo is a no-op.
        st.unregister_repo_from_tenant("org-a", "alpha", &tenant("org-a"), 4)
            .unwrap();
        assert_eq!(st.count_owned_repos("org-a"), Some(1));
    }

    #[test]
    fn reconcile_re_registers_a_genesis_without_a_registry_entry() {
        // WP-1 self-heal: simulate a crash BETWEEN the genesis create and the registry
        // register — the genesis exists durably but the registry has NO entry (undercount).
        // The reconcile heals toward the genesis source of truth (re-registers it), so the
        // count converges. It NEVER registers a repo not durably owned by the tenant.
        let dir = scratch_dir();
        let st = AppState::new(dir.clone(), "dev-token".to_string());
        // Genesis landed (owned by org-a), registry register "crashed" (never ran).
        std::fs::write(dir.join("orphan.json"), meta_log("private", "org-a")).unwrap();
        assert_eq!(
            st.count_owned_repos("org-a"),
            Some(0),
            "the un-registered genesis is invisible to the count (the crash undercount)"
        );
        // Reconcile toward genesis-truth → the repo becomes countable.
        st.reconcile_tenant_repo("org-a", "orphan", &tenant("org-a"), 1)
            .expect("reconcile a durably-owned repo");
        assert_eq!(
            st.count_owned_repos("org-a"),
            Some(1),
            "the reconcile re-registered the genesis (self-healed)"
        );
        // Reconcile is idempotent (a second run does not double-count).
        st.reconcile_tenant_repo("org-a", "orphan", &tenant("org-a"), 2)
            .unwrap();
        assert_eq!(st.count_owned_repos("org-a"), Some(1));
        // Reconcile NEVER registers a repo not owned by the tenant: org-b's reconcile of
        // org-a's repo is a no-op (the genesis owner_tenant is org-a).
        st.reconcile_tenant_repo("org-b", "orphan", &tenant("org-b"), 3)
            .unwrap();
        assert_eq!(st.count_owned_repos("org-b"), Some(0));
        // And an absent genesis → nothing to reconcile (no phantom registration).
        st.reconcile_tenant_repo("org-a", "ghost", &tenant("org-a"), 4)
            .unwrap();
        assert_eq!(st.count_owned_repos("org-a"), Some(1));
    }

    #[test]
    fn boot_reconcile_heals_undercount_and_is_idempotent_no_double_count() {
        // #76 boot reconcile: durable genesis repos that are NOT in their tenant registry (the
        // best-effort register-PUT undercount + the legacy git-ingest back-fill) are
        // reconciled-in on boot; an already-registered repo is a no-op (never double-counted);
        // and a foreign-owned repo is never registered under the wrong tenant.
        let dir = scratch_dir();
        let st = AppState::new(dir.clone(), "dev-token".to_string());
        // org-a owns alpha+beta, NEITHER registered (undercount / legacy back-fill).
        std::fs::write(dir.join("alpha.json"), meta_log("private", "org-a")).unwrap();
        std::fs::write(dir.join("beta.json"), meta_log("private", "org-a")).unwrap();
        // org-b owns gamma, ALREADY registered.
        std::fs::write(dir.join("gamma.json"), meta_log("private", "org-b")).unwrap();
        st.register_repo_in_tenant("org-b", "gamma", &tenant("org-b"), 1)
            .unwrap();
        assert_eq!(
            st.count_owned_repos("org-a"),
            Some(0),
            "the un-registered genesis is invisible before the reconcile (the undercount)"
        );
        assert_eq!(st.count_owned_repos("org-b"), Some(1));

        // The boot pass heals org-a's undercount + back-fill, no-ops gamma.
        st.run_boot_reconcile_tenant_registry();
        assert_eq!(
            st.count_owned_repos("org-a"),
            Some(2),
            "both un-registered genesis repos are reconciled-in"
        );
        assert_eq!(
            st.count_owned_repos("org-b"),
            Some(1),
            "an already-registered repo stays a single count (no double-count, no wrong-tenant)"
        );

        // Idempotent: a second pass changes nothing.
        st.run_boot_reconcile_tenant_registry();
        assert_eq!(st.count_owned_repos("org-a"), Some(2));
        assert_eq!(st.count_owned_repos("org-b"), Some(1));
    }

    #[test]
    fn tenant_registry_is_scoped_and_not_a_repo() {
        // The registry lives under `_tenants/` — it is NEVER reachable/servable as a repo,
        // and an unsafe org slug never touches the store (404, no traversal key built).
        let st = AppState::new(scratch_dir(), "dev-token".to_string());
        st.register_repo_in_tenant("org-a", "alpha", &tenant("org-a"), 1)
            .unwrap();
        assert_eq!(
            st.load_verified("org-a").unwrap_err().status,
            404,
            "the tenant registry is not addressable as a repo"
        );
        // A traversal org slug is refused before any store touch.
        assert_eq!(st.load_tenant_registry("../evil").unwrap_err().status, 404,);
        assert_eq!(
            st.register_repo_in_tenant("../evil", "x", &tenant("x"), 1)
                .unwrap_err()
                .status,
            404,
        );
    }

    #[test]
    fn tenant_registry_cas_roundtrip() {
        let st = AppState::new(scratch_dir(), "dev-token".to_string());
        // Absent → the empty owned-set with a create-only token.
        let (log0, tok0) = st
            .load_tenant_registry("org-a")
            .expect("absent → empty set");
        assert!(log0.records().is_empty());
        assert_eq!(tok0, CasToken::Absent);
        // Register, then a stale create-only persist must conflict.
        st.register_repo_in_tenant("org-a", "alpha", &tenant("org-a"), 1)
            .unwrap();
        let (log1, tok1) = st.load_tenant_registry("org-a").expect("present");
        assert_eq!(crate::tenant_registry::count(&log1), 1);
        assert!(matches!(tok1, CasToken::Version(_)));
        let e = st
            .persist_tenant_registry("org-a", &log1, &CasToken::Absent)
            .expect_err("create-only over an existing registry must conflict");
        assert!(e.is_cas_conflict(), "stale Absent → cas_conflict");
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

    // ── repo-meta cache (task #74, W-METENANT scaling follow-up) ────────────

    /// (a) A cache HIT returns EXACTLY what a live `project_repo_meta` over the same
    /// log would — caching must never change the answer, only the cost.
    #[test]
    fn cached_meta_matches_live_projection() {
        let st = state_with(&[("alpha", &meta_log("private", "org-a"))]);
        let log = st.load_verified("alpha").expect("load");
        let live = crate::authz::project_repo_meta(&log);

        // Populate the cache exactly like the write-path hooks do, then read it back.
        st.cache_repo_meta("alpha", live.clone());
        let cached = st.repo_meta_cached("alpha", &log);
        assert_eq!(cached, live, "a cache HIT must equal the live projection");
    }

    /// (c) A cache MISS (nothing was ever cached for this slug) falls back to the
    /// live projection — a gap in coverage never changes the answer.
    #[test]
    fn cache_miss_falls_back_to_live_projection() {
        let st = state_with(&[("alpha", &meta_log("public", "org-a"))]);
        let log = st.load_verified("alpha").expect("load");
        // Nothing was ever written to `repo_meta_cache` for "alpha" — `state_with`
        // (unlike `from_env`) never calls the boot populator.
        assert!(
            st.repo_meta_cache.read().unwrap().get("alpha").is_none(),
            "precondition: nothing cached yet"
        );
        let via_cache = st.repo_meta_cached("alpha", &log);
        let live = crate::authz::project_repo_meta(&log);
        assert_eq!(
            via_cache, live,
            "a cache MISS must return the same answer as calling project_repo_meta directly"
        );
    }

    /// (b) THE security-relevant case: a repo cached as visible/not-erased, then a
    /// `repo.meta`/`repo.erased` mutation lands on the durable log — `refresh_repo_meta_cache`
    /// (the invalidation hook every meta-mutating write path calls) MUST flip the
    /// cached answer, and the hot `me_repo_logs` path MUST reflect it on the very
    /// next call. Proves the invalidation, not just the happy path: a STALE cache
    /// here would let `authorize_read` grant a private/erased repo — a real leak.
    #[test]
    fn refresh_after_erasure_flips_the_cache_and_me_repo_logs_stops_serving_it() {
        let dir = scratch_dir();
        let mut st = AppState::new(dir.clone(), "dev-token".to_string());
        std::fs::write(dir.join("alpha.json"), meta_log("private", "org-a")).unwrap();
        st.set_repo_git(
            "alpha",
            Arc::new(hugit_proto::CasObjectSource::new()),
            gix_hash::ObjectId::empty_tree(gix_hash::Kind::Sha1),
            BTreeMap::new(),
        );

        // Seed the cache with the PRE-erasure meta (as boot population would) —
        // visible to its owner, not erased.
        let log0 = st.load_verified("alpha").expect("load");
        st.cache_repo_meta("alpha", crate::authz::project_repo_meta(&log0));
        assert_eq!(
            st.me_repo_logs(&tenant("org-a"))
                .iter()
                .map(|(s, _)| s.as_str())
                .collect::<Vec<_>>(),
            vec!["alpha"],
            "precondition: the owner sees the repo before erasure"
        );

        // Now durably append the terminal `repo.erased` tombstone directly to the log
        // (mirrors what `tombstone_repo` does), WITHOUT yet invalidating the cache —
        // proving the stale-cache risk this cache design must close.
        let mut log1 = log0.clone();
        log1.append_authorized(
            PrincipalClass::Orchestrator,
            Endpoint::Land,
            crate::writes::erasure::REPO_ERASED_KIND,
            vec!["orchestrator:hugit".into()],
            serde_json::json!({"reason":"erasure","state":"erased"}).to_string(),
            1,
        )
        .expect("append repo.erased");
        std::fs::write(
            dir.join("alpha.json"),
            serde_json::to_string(log1.records()).unwrap(),
        )
        .unwrap();

        // BEFORE the invalidation hook runs, the stale cache still hides the tombstone
        // from `me_repo_logs` — this is exactly the hole the write-path hooks close.
        assert_eq!(
            st.me_repo_logs(&tenant("org-a"))
                .iter()
                .map(|(s, _)| s.as_str())
                .collect::<Vec<_>>(),
            vec!["alpha"],
            "a stale cache would still (wrongly) serve the now-erased repo"
        );

        // The invalidation hook every meta-mutating write path calls.
        st.refresh_repo_meta_cache("alpha");

        // The cache itself now reflects `erased: true`.
        let cached = st
            .repo_meta_cache
            .read()
            .unwrap()
            .get("alpha")
            .cloned()
            .expect("refreshed entry present");
        assert!(cached.erased, "the cache must reflect the tombstone");

        // And the hot path — `me_repo_logs` — stops serving it to its own owner via
        // the cached meta (erasure is terminal, no exception for a tenant read).
        // (The operator branch ALSO excludes an erased repo — #91, covered by
        // `operator_me_repo_logs_excludes_an_erased_repo_but_keeps_private`; this
        // test proves the cache-consulting tenant branch.)
        assert!(
            st.me_repo_logs(&tenant("org-a")).is_empty(),
            "post-refresh, the owner must no longer see the erased repo"
        );
    }

    /// #91: an operator legitimately sees a PRIVATE repo, but a GDPR1-erased repo
    /// (terminal `repo.erased` tombstone) must be gone from the operator's `/v1/me/*`
    /// view too — else its slug leaks there while every other path 404s it. Proves
    /// the operator branch now excludes ONLY `erased` (still sees private).
    #[test]
    fn operator_me_repo_logs_excludes_an_erased_repo_but_keeps_private() {
        let dir = scratch_dir();
        let mut st = AppState::new(dir.clone(), "dev-token".to_string());

        // A private, NOT-erased repo the operator SHOULD still see.
        std::fs::write(dir.join("keep.json"), meta_log("private", "org-a")).unwrap();
        // A repo that will be erased (private meta + a terminal tombstone below).
        std::fs::write(dir.join("gone.json"), meta_log("private", "org-b")).unwrap();
        for slug in ["keep", "gone"] {
            st.set_repo_git(
                slug,
                Arc::new(hugit_proto::CasObjectSource::new()),
                gix_hash::ObjectId::empty_tree(gix_hash::Kind::Sha1),
                BTreeMap::new(),
            );
        }
        // Append the terminal `repo.erased` tombstone to `gone` durably.
        let gone0 = st.load_verified("gone").expect("load gone");
        let mut gone1 = gone0.clone();
        gone1
            .append_authorized(
                PrincipalClass::Orchestrator,
                Endpoint::Land,
                crate::writes::erasure::REPO_ERASED_KIND,
                vec!["orchestrator:hugit".into()],
                serde_json::json!({"reason":"erasure","state":"erased"}).to_string(),
                1,
            )
            .expect("append repo.erased");
        std::fs::write(
            dir.join("gone.json"),
            serde_json::to_string(gone1.records()).unwrap(),
        )
        .unwrap();
        // Boot-populate the cache (as `from_env` would) so the operator branch reads
        // the erased projection for `gone` and the private projection for `keep`.
        for slug in ["keep", "gone"] {
            let log = st.load_verified(slug).expect("load");
            st.cache_repo_meta(slug, crate::authz::project_repo_meta(&log));
        }

        assert_eq!(
            st.repos_for(&operator()),
            vec!["keep".to_string()],
            "operator sees the private repo but the erased repo is gone from /v1/me (#91)"
        );
    }

    /// A visibility flip (not just erasure) also invalidates correctly: a repo cached
    /// as `private` becomes `public` after a `repo.meta` update + refresh, and a
    /// FOREIGN tenant (previously denied) can now see it via `me_repo_logs`.
    #[test]
    fn refresh_after_visibility_change_flips_cross_tenant_read() {
        let dir = scratch_dir();
        let mut st = AppState::new(dir.clone(), "dev-token".to_string());
        std::fs::write(dir.join("alpha.json"), meta_log("private", "org-a")).unwrap();
        st.set_repo_git(
            "alpha",
            Arc::new(hugit_proto::CasObjectSource::new()),
            gix_hash::ObjectId::empty_tree(gix_hash::Kind::Sha1),
            BTreeMap::new(),
        );
        let log0 = st.load_verified("alpha").expect("load");
        st.cache_repo_meta("alpha", crate::authz::project_repo_meta(&log0));

        assert!(
            st.me_repo_logs(&tenant("org-b")).is_empty(),
            "precondition: a foreign tenant cannot see the private repo"
        );

        // Append a `repo.meta` record flipping visibility to public (mirrors
        // `write_repo_meta`).
        let mut log1 = log0.clone();
        log1.append_authorized(
            PrincipalClass::Orchestrator,
            Endpoint::Land,
            crate::authz::REPO_META_KIND,
            vec!["orchestrator:hugit".into()],
            serde_json::json!({"visibility":"public"}).to_string(),
            1,
        )
        .expect("append repo.meta");
        std::fs::write(
            dir.join("alpha.json"),
            serde_json::to_string(log1.records()).unwrap(),
        )
        .unwrap();

        // Still stale before the refresh hook runs.
        assert!(
            st.me_repo_logs(&tenant("org-b")).is_empty(),
            "stale cache still hides the now-public repo from a foreign tenant"
        );

        st.refresh_repo_meta_cache("alpha");

        assert_eq!(
            st.me_repo_logs(&tenant("org-b"))
                .iter()
                .map(|(s, _)| s.as_str())
                .collect::<Vec<_>>(),
            vec!["alpha"],
            "post-refresh, a foreign tenant sees the now-public repo"
        );
    }

    /// `refresh_repo_meta_cache` is fail-safe on the reload itself: if the repo can no
    /// longer be loaded (e.g. its log vanished), it REMOVES the stale entry rather
    /// than leaving a pre-write value behind — a subsequent read falls back live.
    #[test]
    fn refresh_removes_stale_entry_when_reload_fails() {
        let dir = scratch_dir();
        let mut st = AppState::new(dir.clone(), "dev-token".to_string());
        std::fs::write(dir.join("alpha.json"), meta_log("public", "org-a")).unwrap();
        st.set_repo_git(
            "alpha",
            Arc::new(hugit_proto::CasObjectSource::new()),
            gix_hash::ObjectId::empty_tree(gix_hash::Kind::Sha1),
            BTreeMap::new(),
        );
        let log0 = st.load_verified("alpha").expect("load");
        st.cache_repo_meta("alpha", crate::authz::project_repo_meta(&log0));
        assert!(st.repo_meta_cache.read().unwrap().contains_key("alpha"));

        // The log vanishes (simulates a transient/impossible-but-never-assumed fault).
        std::fs::remove_file(dir.join("alpha.json")).unwrap();
        st.refresh_repo_meta_cache("alpha");

        assert!(
            !st.repo_meta_cache.read().unwrap().contains_key("alpha"),
            "a failed reload must REMOVE the entry, never leave a stale one"
        );
    }

    /// Boot population (`from_env`'s path): a real `from_env`-style boot cannot run
    /// hermetically (needs env vars + possibly R2), so this proves the populator
    /// function directly against a `repos`-seeded state — the same shape `from_env`
    /// builds before calling it.
    #[test]
    fn boot_populate_repo_meta_cache_covers_every_loaded_repo() {
        let dir = scratch_dir();
        let mut st = AppState::new(dir.clone(), "dev-token".to_string());
        std::fs::write(dir.join("alpha.json"), meta_log("private", "org-a")).unwrap();
        std::fs::write(dir.join("beta.json"), meta_log("public", "org-b")).unwrap();
        for slug in ["alpha", "beta"] {
            st.set_repo_git(
                slug,
                Arc::new(hugit_proto::CasObjectSource::new()),
                gix_hash::ObjectId::empty_tree(gix_hash::Kind::Sha1),
                BTreeMap::new(),
            );
        }
        assert!(
            st.repo_meta_cache.read().unwrap().is_empty(),
            "precondition: nothing cached before boot population"
        );
        st.boot_populate_repo_meta_cache();

        let cache = st.repo_meta_cache.read().unwrap();
        assert_eq!(cache.len(), 2, "both loaded repos are cached");
        assert_eq!(
            cache.get("alpha").unwrap().owner_tenant.as_deref(),
            Some("org-a")
        );
        assert_eq!(
            cache.get("beta").unwrap().visibility,
            crate::authz::Visibility::Public
        );
    }

    // ── ListObjectsV2 XML parsing (the durable-enumeration B1 fix) ────────────

    #[test]
    fn parse_listv2_extracts_keys_and_no_token_when_complete() {
        let xml = "<?xml version=\"1.0\"?><ListBucketResult>\
            <Contents><Key>t/hugit.json</Key><Size>10</Size></Contents>\
            <Contents><Key>t/githugr.json</Key></Contents>\
            <IsTruncated>false</IsTruncated></ListBucketResult>";
        let (keys, next) = parse_listv2_xml(xml);
        assert_eq!(keys, vec!["t/hugit.json", "t/githugr.json"]);
        assert!(
            next.is_none(),
            "a complete listing yields no continuation token"
        );
    }

    #[test]
    fn parse_listv2_returns_the_token_only_when_truncated() {
        let truncated = "<ListBucketResult><Contents><Key>t/a.json</Key></Contents>\
            <IsTruncated>true</IsTruncated><NextContinuationToken>TOK123</NextContinuationToken>\
            </ListBucketResult>";
        let (keys, next) = parse_listv2_xml(truncated);
        assert_eq!(keys, vec!["t/a.json"]);
        assert_eq!(
            next.as_deref(),
            Some("TOK123"),
            "truncated → follow the token"
        );
        // A token present but NOT truncated must be ignored (no infinite loop).
        let not_truncated = "<ListBucketResult><IsTruncated>false</IsTruncated>\
            <NextContinuationToken>STALE</NextContinuationToken></ListBucketResult>";
        assert!(parse_listv2_xml(not_truncated).1.is_none());
    }

    #[test]
    fn list_repo_slugs_local_excludes_account_logs_and_manifests() {
        // Local mode: only top-level `<slug>.json` files are repo logs; the `_accounts`
        // subdir (account logs) is structurally excluded (it is a dir, not a top file).
        let dir = scratch_dir();
        std::fs::write(dir.join("alpha.json"), meta_log("private", "org-a")).unwrap();
        std::fs::write(dir.join("beta.json"), meta_log("public", "org-b")).unwrap();
        std::fs::create_dir_all(dir.join("_accounts")).unwrap();
        std::fs::write(dir.join("_accounts").join("org-a.json"), "[]").unwrap();
        let src = LogSource::Local { dir };
        let mut slugs = src.list_repo_slugs().expect("list");
        slugs.sort();
        assert_eq!(slugs, vec!["alpha", "beta"], "account logs are not repos");
    }

    #[test]
    fn storage_cap_rollup_counts_durable_but_unloaded_owned_repos() {
        // G10 rollup fail-open fix: `enforce_cas_storage_quota` used to roll the
        // per-owner_tenant cap over the IN-MEMORY loaded set only (`owned_repo_logs`), so
        // a DURABLE-but-UNLOADED owned repo (provisioned, then dropped from the boot env —
        // its `<slug>.json` log + R2 size.json survive, its git seam is gone) was silently
        // omitted → the aggregate under-counted → a push could slip past the 10 GiB
        // owner_tenant ceiling. The check now rolls up over `authoritative_owned_repo_logs`
        // (the DURABLE set: list_repo_slugs() ∪ the in-memory overlay), so the unloaded
        // repo still counts.
        let dir = scratch_dir();
        let mut st = AppState::new(dir.clone(), "dev-token".to_string());
        // `loaded` is durable AND in the in-memory seam; `dropped` is durable-only.
        std::fs::write(dir.join("loaded.json"), meta_log("private", "org-a")).unwrap();
        std::fs::write(dir.join("dropped.json"), meta_log("private", "org-a")).unwrap();
        st.set_repo_git(
            "loaded",
            Arc::new(hugit_proto::CasObjectSource::new()),
            gix_hash::ObjectId::empty_tree(gix_hash::Kind::Sha1),
            BTreeMap::new(),
        );

        // The OLD in-memory-only enumeration MISSES the unloaded repo (the fail-open).
        let inmem: Vec<String> = st
            .owned_repo_logs("org-a")
            .expect("in-memory enumeration")
            .into_iter()
            .map(|(s, _)| s)
            .collect();
        assert_eq!(
            inmem,
            vec!["loaded"],
            "owned_repo_logs sees only the loaded seam (the fail-open source)"
        );

        // The DURABLE authoritative enumeration — the set the storage-cap rollup now uses —
        // includes BOTH, so the unloaded repo can no longer slip the per-owner_tenant cap.
        let mut auth: Vec<String> = st
            .authoritative_owned_repo_logs("org-a")
            .expect("durable enumeration")
            .into_iter()
            .map(|(s, _)| s)
            .collect();
        auth.sort();
        assert_eq!(
            auth,
            vec!["dropped", "loaded"],
            "the durable rollup set includes the durable-but-unloaded owned repo"
        );
    }

    #[test]
    fn erasure_repo_partition_splits_subject_vs_surviving_from_the_durable_set() {
        // clw's #1 bar: the surviving set = EVERY non-subject repo from the SAME durable
        // authoritative listing the planner uses. org-b's repo is SURVIVING (retained).
        let st = state_with(&[
            ("alpha", &meta_log("private", "org-a")),
            ("beta", &meta_log("public", "org-a")),
            ("gamma", &meta_log("private", "org-b")),
        ]);
        let (mut subject, mut surviving) = st.erasure_repo_partition("org-a").expect("partition");
        subject.sort();
        surviving.sort();
        assert_eq!(subject, vec!["alpha", "beta"], "the subject's own repos");
        assert_eq!(
            surviving,
            vec!["gamma"],
            "another account's repo is SURVIVING (its digests are retained, never erased)"
        );
        // The subject side MUST match the planner's authoritative owned set exactly (same
        // durable + fail-closed discipline — no drift between the two derivations).
        let mut auth: Vec<String> = st
            .authoritative_owned_repo_logs("org-a")
            .unwrap()
            .into_iter()
            .map(|(s, _)| s)
            .collect();
        auth.sort();
        assert_eq!(
            subject, auth,
            "subject_repos == authoritative_owned_repo_logs slugs"
        );
    }
}
