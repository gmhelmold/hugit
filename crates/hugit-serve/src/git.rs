//! Git smart-HTTP wire serving — `git clone`/`git fetch` (`git-upload-pack`, the
//! READ side) AND `git push` (`git-receive-pack`, the WRITE side).
//!
//! The READ side serves any repo whose git seam is loaded. The WRITE side
//! (`git-receive-pack`) is gated, fail-closed: OFF unless `HUGIT_SERVE_RECEIVE_PACK=1`
//! AND the repo has an on-disk `git_dir` write seam AND the pusher passes
//! `authorize_write` (ownership). It wires the git client straight into
//! [`hugit_proto::write::receive::receive_pack`] (bound→unpack→verify→store→anchor→
//! append) and answers the `report-status`. A successful CAS-mode push then
//! refreshes the in-memory view IN PROCESS (the **live ref hot-swap**:
//! [`crate::state::RepoState::apply_cas_push_inmemory`]) so the new tip advertises +
//! clone-resolves with no reboot. v0 scope: a single non-delete ref per push,
//! self-contained pack (incremental/thin-pack reachability that consults the CAS for
//! server-side bases is the remaining follow-up — see
//! `docs/plan/2026-06-22-receive-pack-wave-design.md`).
//!
//! ## Protocol: smart-HTTP **v1** (the dumb-free "smart" transport)
//!
//! hugit-proto ([`hugit_proto::read`]) assembles a real git V2 *packfile* and
//! parses `want`/`have` pkt-lines, but it does NOT implement protocol-**v2**'s
//! `fetch` response framing (no `packfile` section header, no `acknowledgments`
//! section, no side-band-64k multiplexing). The wire version that interoperates
//! with a real `git` client given exactly "(NAK pkt-line) + (raw packfile)" is
//! **protocol v1 smart-HTTP** — so that is what this envelope speaks:
//!
//! 1. `GET  /<repo>/info/refs?service=git-upload-pack` →
//!    `# service=git-upload-pack\n` pkt-line, a flush, then the v1 ref
//!    advertisement: the first ref line carries a NUL-separated capability list,
//!    the rest are bare `<oid> <ref>` lines, terminated by a flush. Content-Type
//!    `application/x-git-upload-pack-advertisement`.
//! 2. `POST /<repo>/git-upload-pack` → parse the client's `want`/`have`/`done`
//!    pkt-lines ([`hugit_proto::WantHave::parse`]), assemble the pack
//!    ([`hugit_proto::serve_clone`] / [`hugit_proto::serve_fetch`]), and reply
//!    `0008NAK\n` followed by the raw packfile bytes. Content-Type
//!    `application/x-git-upload-pack-result`.
//!
//! Crucially, the advertised capabilities deliberately EXCLUDE `side-band-64k`
//! (and any ack-negotiation capability): the proto returns a bare packfile with
//! no side-band multiplexing, so the client must read the pack straight off the
//! `NAK\n`. Advertising side-band would make `git` wait for multiplexed framing
//! that never comes (a hang). The minimal honest capability set is what we send.
//!
//! ## Refs come from the SAME git dir as the objects
//!
//! The [`hugit_proto::RefView`] for the advertisement is the requested repo's
//! `git_refs` map — resolved PER-REPO from
//! [`AppState::repo_state`](crate::state::AppState::repo_state) (the engine serves
//! many repos). It is populated by `for-each-ref` over the exact git source the
//! objects were enumerated from. Refs and objects must be consistent; reading refs
//! from the event log (a different projection) could advertise a tip whose closure
//! is not in the CAS → a clone that 404s mid-stream. They are co-loaded,
//! fail-closed, at boot.
//!
//! ## Auth
//!
//! A clone is gated on the repo's **READ visibility** — the SAME
//! `authz::authorize_read` predicate every `/v1` read uses. The principal is
//! derived from an OPTIONAL `Authorization: Bearer <token>` the client may send
//! (real `git` can carry one via `http.extraHeader`):
//!
//! * **No / invalid / expired Bearer** → the ANONYMOUS principal (`&[]`). A
//!   public repo is served; a private/absent repo is a uniform 404. This is the
//!   fail-CLOSED default: a malformed credential is treated as anonymous (public
//!   only), never as authenticated.
//! * **A valid engine token** (Tier-1 of the `/v1` token store — a Clerk session
//!   exchange minted it) → the real `clerk:{org}:{user}` tenant principal. So the
//!   OWNING tenant clones its own PRIVATE repo, and a FOREIGN tenant gets the same
//!   404 as a non-existent repo (cross-tenant isolation — no existence oracle).
//!
//! The token validation is the IDENTICAL Tier-1 path `two_tier_auth` runs for the
//! receive-pack (push) write side ([`crate::token::TokenStore::lookup`] — the
//! constant-time SHA-256 compare + expiry sweep). The ONE deliberate difference:
//! the clone read path does NOT consult the Tier-2 **dev-token** fallback, so the
//! operator god-principal (`orchestrator:*` → sees-all) can NEVER be acquired over
//! the clone wire — a clone principal is ONLY a real validated tenant or anonymous
//! (go-live decision: a real user gets no god-token; the read wire must not inherit
//! the operator bypass). Both the `info/refs` advertise AND the `git-upload-pack`
//! POST run the SAME derivation + `authorize_read`, so neither route can leak a
//! repo the other hides. When the requested repo has no git seam loaded (not in the
//! [`AppState`](crate::state::AppState) repo map) every git route is a 404 (git
//! serving is not live for it — honest, no fake; no oracle for which repos are
//! git-served).

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use tiny_http::{Header, Method, Request, Response};

use crate::clone_pack::{self, CloneCacheSeam};
use crate::state::AppState;

/// The `git-upload-pack` service name (clone + fetch). `git-receive-pack` (push)
/// is intentionally NOT handled here (out of scope).
const UPLOAD_PACK: &str = "git-upload-pack";

/// The v1 capabilities we advertise. Deliberately minimal + side-band-FREE: the
/// proto serves a bare packfile (no side-band-64k multiplexing), so advertising
/// side-band would make the client await framing that never arrives. `agent` is
/// informational; `object-format=sha1` matches the proto's hash. `shallow` opts the
/// client into sending `deepen <N>` for `git clone --depth N` — the server answers
/// with a `shallow <oid>` section + a depth-bounded pack (see `build_shallow_pack_bytes`);
/// WITHOUT it git refuses `--depth` outright ("Server does not support shallow clients").
const ADVERTISED_CAPS: &str = "object-format=sha1 agent=hugit-serve shallow";

/// Whether `url`'s path is a git smart-HTTP route this module owns
/// (`/<repo>/info/refs`, `/<repo>/git-upload-pack`, OR `/<repo>/git-receive-pack`).
/// The main loop uses this as a pre-route short-circuit (like the SSE path)
/// because a packfile is binary and cannot ride the `(status, String)` path.
/// `git-receive-pack` (push) is included here so it receives the clear 403
/// message instead of being swallowed by the generic 404 routing.
#[must_use]
pub fn is_git_path(url: &str) -> bool {
    let path = url.split('?').next().unwrap_or("");
    let segs: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    matches!(
        segs.as_slice(),
        [_repo, "info", "refs"] | [_repo, UPLOAD_PACK] | [_repo, "git-receive-pack"]
    )
}

/// Whether `url` addresses the receive-pack (push) POST route — the ONLY route that
/// carries a packfile body, so the ONLY one that gets the larger pack-size cap
/// (`max_pack_bytes`) at body-read time; every other POST keeps the `/v1` JSON door's
/// [`crate::writes::MAX_BODY_BYTES`] (8 MiB).
pub fn is_receive_pack_path(url: &str) -> bool {
    let path = url.split('?').next().unwrap_or("");
    let segs: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    matches!(segs.as_slice(), [_repo, "git-receive-pack"])
}

/// Default dedicated ceiling on a received packfile BODY (64 MiB). Distinct from — and
/// larger than — the `/v1` JSON door's 8 MiB [`crate::writes::MAX_BODY_BYTES`], which
/// used to (wrongly) TRUNCATE a git push before the proto's own wire cap could reject
/// it. Overridable via `HUGIT_SERVE_MAX_PACK_BYTES`, clamped to `[1 MiB, 512 MiB]` so a
/// mis-set env can neither block every real push (floor) nor un-bound the accept-loop's
/// buffered read (ceiling). This is the FIRST wall (a clean 413 before any unpack); the
/// proto bomb-caps (32 MiB inflated / 1M objects / delta-pass) are the second.
pub const DEFAULT_MAX_PACK_BYTES: usize = 64 * 1024 * 1024;
const MIN_MAX_PACK_BYTES: usize = 1024 * 1024; // 1 MiB floor
const MAX_MAX_PACK_BYTES: usize = 512 * 1024 * 1024; // 512 MiB ceiling

/// The configured receive-pack body cap: `HUGIT_SERVE_MAX_PACK_BYTES` clamped to
/// `[1 MiB, 512 MiB]`, defaulting to [`DEFAULT_MAX_PACK_BYTES`].
#[must_use]
pub fn max_pack_bytes() -> usize {
    let raw = std::env::var("HUGIT_SERVE_MAX_PACK_BYTES")
        .ok()
        .and_then(|v| v.parse::<usize>().ok());
    clamp_max_pack_bytes(raw)
}

/// The pure clamp behind [`max_pack_bytes`] (env-free, hermetically testable): an
/// unset / unparseable value → the 64 MiB default; any value is clamped to
/// `[1 MiB, 512 MiB]`.
#[must_use]
pub fn clamp_max_pack_bytes(raw: Option<usize>) -> usize {
    raw.unwrap_or(DEFAULT_MAX_PACK_BYTES)
        .clamp(MIN_MAX_PACK_BYTES, MAX_MAX_PACK_BYTES)
}

/// Handle a git smart-HTTP request inline (binary-safe), consuming `request` in
/// every branch. Mirrors `respond_sse`: it owns the whole response because a
/// packfile is a `Vec<u8>` body with a git-specific Content-Type. `body` is the
/// already-read (capped) request body — empty for the GET advertisement, the
/// want/have pkt-lines for the upload-pack POST.
///
/// `io_budget` is the accept-loop's wall-clock I/O deadline (FIX-SOCKET-TIMEOUT),
/// threaded in like `respond_sse`: every response write here rides [`send`] →
/// `crate::server::respond_bounded`, so a small body (advertise, report) is written
/// INLINE while a LARGE clone/fetch PACK write is offloaded to a bounded worker — a
/// client that stops draining a big pack can NOT wedge the single-threaded loop.
pub fn respond_git(
    state: &AppState,
    method: &Method,
    url: &str,
    body: &[u8],
    request: Request,
    io_budget: Duration,
) {
    let path = url.split('?').next().unwrap_or("");
    let query = url.split('?').nth(1).unwrap_or("");
    let segs: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();

    match (method, segs.as_slice()) {
        (Method::Get, [repo, "info", "refs"]) => {
            // Only the upload-pack (read) service is served. A receive-pack probe
            // (push) is a 403 with a clear human message (not a silent 404) — the
            // caller explicitly probed a service we have categorically declined.
            // Any other/absent service is a 404 (no oracle for unknown services).
            let svc = query_param(query, "service");
            if svc == Some("git-receive-pack") {
                return handle_receive_advertise(state, repo, request, io_budget);
            }
            if svc != Some(UPLOAD_PACK) {
                return respond_not_found(request, io_budget);
            }
            // Derive the clone principal from an OPTIONAL Bearer (anonymous on
            // absent/invalid/expired — fail-closed), then gate on authorize_read.
            // SAME derivation the upload-pack POST runs (no split-route bypass).
            let principal = clone_principal(state, request.headers());
            // G11: map the bare URL slug to the caller's user-scoped stored key (legacy
            // fallback for pre-G11 flat repos). Same derivation the POST below runs.
            let repo = &state.resolve_repo_slug(repo, &principal);
            match advertise_refs(state, repo, &principal) {
                Some(body) => {
                    let body_len = body.len();
                    let resp = Response::from_data(body).with_status_code(200).with_header(
                        git_content_type("application/x-git-upload-pack-advertisement"),
                    );
                    send(request, resp, body_len, io_budget);
                }
                None => respond_not_found(request, io_budget),
            }
        }
        (Method::Post, [repo, "git-upload-pack"]) => {
            // IDENTICAL principal derivation + authz to the advertise above: the
            // POST must 404 for every case the advertise hides (no split-route
            // bypass where a pack is served for a repo the advertise concealed).
            let principal = clone_principal(state, request.headers());
            // G11: resolve the bare URL slug to the caller's user-scoped stored key
            // (legacy fallback) — IDENTICAL to the advertise, so no split-route bypass.
            let repo = &state.resolve_repo_slug(repo, &principal);
            // Do the CHEAP work INLINE on the accept loop — principal, read-authz +
            // ref snapshot, want parse + want-validation (which caps the walk ROOT
            // before any worker exists). On any failure → 404 inline, no worker.
            match prepare_upload_pack(state, repo, body, &principal) {
                Some((source, want, plan, seam)) => {
                    // Then HAND the whole heavy job OFF to a detached worker that OWNS
                    // this connection and RESPONDS itself (the reachability walk + pack
                    // assembly are one synchronous CAS fetch PER object — seconds on a
                    // real repo). The accept loop returns to accept IMMEDIATELY — it is
                    // NOT blocked for the clone's duration, so `/readyz` + other
                    // requests stay answerable while the clone builds. A spawn failure
                    // under a clone flood sheds a fast inline 503 (never a runaway walk
                    // inline). See `spawn_upload_pack_worker`. The `plan` + `seam` let
                    // the worker serve the cached full-clone pack (WP-BC) when it matches.
                    spawn_upload_pack_worker(source, want, plan, seam, request, io_budget);
                }
                None => respond_not_found(request, io_budget),
            }
        }
        // POST git-receive-pack (push) → the write path (gated). Other methods on
        // the receive-pack route → the clear 403 (not a silent 404).
        (Method::Post, [repo, "git-receive-pack"]) => {
            handle_receive_pack(state, repo, body, request, io_budget)
        }
        (_, [_repo, "git-receive-pack"]) => respond_push_forbidden(request, io_budget),
        // Any other method/shape on a git-looking path → 404, no oracle.
        _ => respond_not_found(request, io_budget),
    }
}

/// Derive the READ principal for a clone from an OPTIONAL `Authorization: Bearer`.
///
/// This is the authenticated-clone seam (the go-live cross-tenant read gate). It
/// reuses the EXACT Tier-1 token validation `crate::server::two_tier_auth` runs for
/// the receive-pack write side — [`crate::token::TokenStore::lookup`] (constant-time
/// SHA-256 compare + expiry sweep) — and maps a valid record to the SAME
/// `clerk:{org}:{user}` principal shape `authorize_read` classifies as a tenant.
///
/// Fail-CLOSED to ANONYMOUS (the empty chain `vec![]`) on every non-success:
/// * no `Authorization: Bearer` header at all (a plain anonymous `git clone`),
/// * an unknown / garbage / malformed token (Tier-1 `Invalid`),
/// * an EXPIRED engine token (Tier-1 `Expired`),
/// * a token-store lock fault (`lookup` already returns `Invalid` fail-closed).
///
/// "Anonymous" (not an error) is correct here because a clone of a PUBLIC repo
/// must still succeed without a credential — so an invalid Bearer degrades to the
/// public-only anonymous principal, NEVER to "authenticated as someone". A private
/// repo with an anonymous principal is denied by `authorize_read` → a uniform 404.
///
/// DELIBERATELY does NOT consult the Tier-2 **dev-token** fallback: the operator
/// god-principal (`orchestrator:*`, which `authorize_read` treats as sees-all) must
/// NEVER be acquirable over the clone wire (go-live: a real user gets no god-token;
/// the read wire derives ONLY a real validated tenant or anonymous). The dev/operator
/// bypass stays confined to the `/v1` POST write door + receive-pack, which call
/// `two_tier_auth` directly — it is NOT entangled here.
fn clone_principal(state: &AppState, headers: &[tiny_http::Header]) -> Vec<String> {
    // The full `Authorization` value (case-insensitive header name); absent →
    // anonymous. Handles Bearer AND the git-CLI HTTP-Basic (a PAT clone puts the token
    // in the Basic PASSWORD).
    let auth_value = headers
        .iter()
        .find(|h| {
            h.field
                .as_str()
                .as_str()
                .eq_ignore_ascii_case("Authorization")
        })
        .map(|h| h.value.as_str().to_string());
    let Some(auth_value) = auth_value else {
        return Vec::new(); // no credential → anonymous (public-clone gate)
    };

    // Tier-1: the engine-token store (Bearer only — a Clerk session exchange minted
    // it). A valid, unexpired record → the real tenant principal.
    if let Some(raw) = auth_value.strip_prefix("Bearer ")
        && let crate::token::LookupResult::Ok(rec) = state.token_store.lookup(raw)
    {
        return vec![format!("clerk:{}:{}", rec.org, rec.user)];
    }

    // Tier-1.5: a hugit PAT (Bearer or the Basic password). Resolves to the token
    // OWNER's real tenant principal — NEVER the operator (the resolver stores only
    // clerk owners; the dev-token god-path is deliberately still NOT consulted here). A
    // read-only PAT authenticates a clone (a read needs no write scope). No-op when PAT
    // auth is off. Anything unknown/expired/revoked → anonymous (public only).
    if let Some(pat) = state.resolve_pat_from_auth(&auth_value, now_ms()) {
        return pat.principal_chain();
    }
    Vec::new() // invalid/expired/unknown → fail-closed to anonymous (public only)
}

/// Build the v1 `info/refs` advertisement body for `repo`, or `None` (→ 404) when
/// git serving is not live, the repo is unsafe/absent, or `principal` is not
/// authorized to READ it. The auth gate is the SAME `authorize_read` predicate as
/// the `/v1` reads, evaluated against the clone principal (`clone_principal`):
/// anonymous for a public clone, a real tenant for an authed private clone.
fn advertise_refs(state: &AppState, repo: &str, principal: &[String]) -> Option<Vec<u8>> {
    let refs = git_refs_for(state, repo, principal)?;

    // pkt-line framing, built directly (the envelope owns the framing; the proto
    // owns the payload semantics). Smart-HTTP v1 info/refs:
    //   <pkt>"# service=git-upload-pack\n"  <flush>
    //   <pkt>"<oid> <ref>\0<caps>\n"  (first ref carries caps)
    //   <pkt>"<oid> <ref>\n"          (remaining refs)
    //   <flush>
    // The default branch (what HEAD resolves to). A clone needs HEAD advertised —
    // and the `symref=HEAD:<branch>` capability — to know which branch to check out
    // (without it, git fetches the refs but leaves an unborn HEAD / nothing checked
    // out). Prefer `main`, then `master`, then the first head, else the first ref.
    let head_branch = pick_default_branch(&refs);

    let mut out = Vec::new();
    pkt_line(&mut out, format!("# service={UPLOAD_PACK}\n").as_bytes());
    pkt_flush(&mut out);

    // The FIRST advertised line carries the capability list after a NUL (v1
    // contract). We advertise HEAD first, pointing at the default branch's tip,
    // with `symref=HEAD:<branch>` so the client sets its default branch correctly.
    // Then the real refs (name-sorted). `git_refs_for` returns None for an empty
    // map, so there is always at least one ref → a default branch → a HEAD tip.
    let mut first = true;
    if let Some(branch) = &head_branch
        && let Some(tip) = refs.get(branch)
    {
        let mut line = format!("{tip} HEAD\0{ADVERTISED_CAPS} symref=HEAD:{branch}");
        line.push('\n');
        pkt_line(&mut out, line.as_bytes());
        first = false;
    }

    for (name, oid) in &refs {
        let mut line = format!("{oid} {name}");
        if first {
            // No HEAD was advertised (degenerate: no branch); the first real ref
            // carries the caps instead, so the line is never capability-less.
            line.push('\0');
            line.push_str(ADVERTISED_CAPS);
            first = false;
        }
        line.push('\n');
        pkt_line(&mut out, line.as_bytes());
    }
    pkt_flush(&mut out);
    Some(out)
}

/// Pick the default branch ref to advertise as HEAD: `refs/heads/main`, else
/// `refs/heads/master`, else the first `refs/heads/*`, else the first ref of any
/// kind. `None` only for an empty map (which `git_refs_for` already rejects).
pub(crate) fn pick_default_branch(refs: &BTreeMap<String, String>) -> Option<String> {
    if refs.contains_key("refs/heads/main") {
        return Some("refs/heads/main".to_string());
    }
    if refs.contains_key("refs/heads/master") {
        return Some("refs/heads/master".to_string());
    }
    refs.keys()
        .find(|k| k.starts_with("refs/heads/"))
        .or_else(|| refs.keys().next())
        .cloned()
}

/// A shorthand for the object source shared with a detached clone worker: an owned
/// `Arc` over the repo's `Send + Sync` [`hugit_proto::ObjectSource`]. Cloning the
/// `Arc` gives the worker a `'static` handle that borrows nothing from the accept
/// loop; `Sync` makes concurrent clone workers reading the same CAS safe.
type SharedSource = Arc<dyn hugit_proto::ObjectSource + Send + Sync>;

/// The clone-cache decision (WP-BC) carried from the INLINE `prepare_upload_pack`
/// to the DETACHED worker. `full_clone` is the strict predicate that gates the
/// cached-pack fast path: `true` ONLY when the client asked for EXACTLY the full
/// advertised tip-set with no `have`s (an initial `git clone`), so the cached pack
/// — assembled for that same tip-set — is a byte-correct answer. `refs` is the
/// read-gated ref snapshot the wants were validated against; the serve side
/// recomputes [`clone_pack::refset_sha`] from it and refuses any cache whose pointer
/// disagrees (fail-open to the slow walk — never a wrong pack). A false positive here
/// would serve a wrong pack, so the predicate is set-EQUALITY strict; a false
/// negative merely slow-walks (safe).
struct ClonePlan {
    full_clone: bool,
    refs: BTreeMap<String, String>,
    /// `Some(depth)` when the client sent `deepen <depth>` (`git clone --depth N`,
    /// round 1): serve a depth-bounded pack + a `shallow` section. Mutually exclusive
    /// with `full_clone` (a shallow request never uses the cached full-clone pack —
    /// that pack carries the WHOLE history).
    shallow_depth: Option<u32>,
    /// The client's declared shallow boundary (`shallow <oid>` lines) — round 2 of a
    /// stateless HTTP shallow clone carries the boundary instead of `deepen`. Non-empty
    /// ALSO marks a shallow request (or the server skips the `shallow` section git
    /// requires each round → `expected shallow list`).
    shallow_client: Vec<hugit_proto::ObjectId>,
    /// Whether the client sent `done`. A shallow request WITHOUT `done` is round-1
    /// negotiation: reply with the `shallow` section ONLY (no pack); the pack rides the
    /// round-2 request that carries `done`.
    done: bool,
}

impl ClonePlan {
    /// A shallow (`--depth`) request — detected by `deepen` (round 1) OR the client's
    /// `shallow` lines (round 2). Both must take the shallow serve path.
    fn is_shallow(&self) -> bool {
        self.shallow_depth.is_some() || !self.shallow_client.is_empty()
    }
}

/// Whether the explicit `want` set is EXACTLY the full advertised tip-set (every
/// tip requested, nothing extra). This is the tip⊆wants half of the full-clone
/// predicate; combined with an empty `have` list it means an initial clone. Strict
/// set-equality: a want for a non-tip (already rejected by [`wants_all_advertised`])
/// or a missing tip → NOT a full clone → the slow walk (safe). Order-independent.
/// Whether an upload-pack response takes the clone-pack CACHE path (vs the slow walk).
/// True ONLY for a full clone of a repo that HAS a cache seam — for that repo the
/// pre-assembled pack IS the serve path, so a cache MISS is a retryable 503 (the pack
/// is building), never the single-thread-DoS slow walk. A seam-less repo (GIT_DIR /
/// small) or any non-full-clone (a bounded fetch) → false → the slow walk. Pure +
/// unit-tested (the DoS-avoidance routing is proven without a live worker/R2).
fn takes_clone_cache_path(full_clone: bool, has_seam: bool) -> bool {
    full_clone && has_seam
}

fn wants_equal_all_tips(refs: &BTreeMap<String, String>, wants: &[gix_hash::ObjectId]) -> bool {
    let tips: std::collections::BTreeSet<String> = refs.values().cloned().collect();
    let want_set: std::collections::BTreeSet<String> =
        wants.iter().map(|w| w.to_string()).collect();
    want_set == tips
}

/// The CHEAP, INLINE half of a `git-upload-pack` POST: derive the read-gated ref
/// view, clone the object source, parse the want/have negotiation, and VALIDATE the
/// wants — all fast, all on the accept loop, BEFORE any worker is spawned. Returns
/// the `(source, effective_want)` a worker needs to assemble the pack, or `None`
/// (→ 404) on the SAME not-live / unsafe / not-public / malformed / unadvertised-want
/// conditions as the advertisement (no split-route bypass, no existence oracle).
///
/// Keeping the want-validation ([`wants_all_advertised`]) here — on the loop, before
/// the hand-off — is deliberate: it caps the reachability walk's ROOT to a real,
/// read-authorized tip, so a client can NEVER make a worker walk from an
/// attacker-chosen oid. The heavy walk itself runs OFF the loop
/// ([`spawn_upload_pack_worker`]).
fn prepare_upload_pack(
    state: &AppState,
    repo: &str,
    body: &[u8],
    principal: &[String],
) -> Option<(
    SharedSource,
    hugit_proto::WantHave,
    ClonePlan,
    Option<CloneCacheSeam>,
)> {
    let refs = git_refs_for(state, repo, principal)?;
    // CLONE the Arc object source (it is `Send + Sync + 'static`) so an owned handle
    // can move into the detached worker without borrowing `state`. Also snapshot the
    // repo's clone-pack cache seam (WP-BC) — `None` for a repo with no cache (GIT_DIR /
    // receive-pack off), which just means the worker slow-walks.
    let repo_state = state.repo_state_or_load(repo)?;
    let source: SharedSource = Arc::clone(&repo_state.git_source);
    let clone_cache = repo_state.clone_cache.clone();

    // Real git appends a capability list to the FIRST `want` line of a v1/v0
    // upload-pack request (`want <oid> multi_ack side-band-64k …`). The proto's
    // `WantHave::parse` treats everything after `want ` as the oid and rejects the
    // trailing caps as a bad oid — so we re-frame the body first, trimming each
    // `want`/`have` line to its leading oid token. The result is byte-clean
    // pkt-lines the proto parses faithfully.
    let cleaned = strip_want_have_caps(body);
    let request = hugit_proto::WantHave::parse(&cleaned).ok()?;

    // Resolve the effective, OWNED request to hand the worker. A clone sends wants
    // but no haves; a fetch sends both. When the client sent no wants at all (e.g.
    // an ls-refs-only probe in a v2 attempt, or an empty body) treat it as "want
    // every advertised tip" — a full clone — using the SAME ref view the advertise
    // was built from, so wants always resolve in the CAS.
    let (effective_req, full_clone) = if request.wants.is_empty() {
        let adv = hugit_proto::RefAdvertisement::from_view(&refs);
        // The full-clone wants are DERIVED from `refs` (every advertised tip), so
        // they are advertised by construction — no validation needed. An empty want
        // set IS a full clone (the whole advertised tip-set, no haves).
        (hugit_proto::WantHave::clone_all(&adv).ok()?, true)
    } else {
        // SECURITY (want-validation): every explicit `want` MUST be an advertised
        // ref tip for THIS principal (the same `refs` the advertise exposed). A
        // want for a non-advertised oid — an arbitrary interior/unreachable object,
        // or one hidden by the read-authz gate — is REJECTED (→ 404, no oracle):
        // the `uploadpack.allowReachableSHA1InWant`-off default. This closes a
        // client fishing for an unadvertised object AND caps the reachability walk
        // to a real tip's closure (defence-in-depth for the serve_fetch DoS: a
        // caller cannot force a walk from an attacker-chosen root). UNCHANGED — the
        // validation stays INLINE on the loop, BEFORE the worker is spawned.
        if !wants_all_advertised(&refs, &request.wants) {
            return None;
        }
        // A full clone iff the client wants EXACTLY the advertised tip-set with no
        // `have`s (strict set-equality — a false positive would serve a wrong pack;
        // a false negative merely slow-walks, which is safe). A fetch (haves present,
        // or a partial want-set) is NEVER a full clone → never the cached pack.
        let full = request.haves.is_empty() && wants_equal_all_tips(&refs, &request.wants);
        (request, full)
    };

    // `git clone --depth N` is a TWO-round stateless-HTTP negotiation: round 1 sends
    // `deepen N` (no `done`) and the server replies with the `shallow` boundary; round 2
    // re-sends that boundary as `shallow <oid>` lines PLUS `done` and NO `deepen`. So a
    // shallow request is detected by EITHER, and the pack is gated on `done`. Either
    // marker DISABLES the cached full-clone fast path (that pack carries the WHOLE history).
    let shallow_depth = hugit_proto::parse_deepen(body);
    let shallow_client = hugit_proto::parse_client_shallow(body);
    let is_shallow = shallow_depth.is_some() || !shallow_client.is_empty();
    let full_clone = full_clone && !is_shallow;
    let plan = ClonePlan {
        full_clone,
        refs,
        shallow_depth,
        shallow_client,
        done: effective_req.done,
    };
    Some((source, effective_req, plan, clone_cache))
}

/// Assemble the clone/fetch pack and frame the v1 upload-pack result (the `NAK`
/// pkt-line then the raw packfile). `None` (→ 404) when [`hugit_proto::serve_fetch`]
/// fails — incomplete closure, decode error, or its own internal
/// [`hugit_proto::SERVE_FETCH_BUDGET`] (300 s, the sole runaway bound now the loop
/// never waits). Every failure is fail-CLOSED to no-pack, NEVER a truncated one.
///
/// Pure + `state`-free (an `&dyn ObjectSource` + an owned request in, bytes out), so
/// it is directly unit-testable AND safe to run on a detached worker: it holds no
/// lock and mutates no shared state.
fn build_upload_pack_bytes(
    source: &dyn hugit_proto::ObjectSource,
    request: &hugit_proto::WantHave,
) -> Option<Vec<u8>> {
    let pack = hugit_proto::serve_fetch(source, request).ok()?;
    // Smart-HTTP v1 upload-pack result: the NAK pkt-line (we run no multi-ack
    // negotiation — single round, "done"), then the raw packfile bytes.
    let mut out = Vec::new();
    pkt_line(&mut out, b"NAK\n");
    out.extend_from_slice(&pack.bytes);
    Some(out)
}

/// Assemble a DEPTH-BOUNDED (shallow) upload-pack result — the response to one round
/// of a `git clone --depth N`. `None` (→ 404) on any assembly failure: fail-CLOSED to
/// no-pack, NEVER a truncated one (same contract as [`build_upload_pack_bytes`]).
///
/// TWO-round stateless-HTTP shallow protocol (both handled here):
/// * **Round 1** — the client sent `deepen N` and NO `done`. Reply with the `shallow`
///   section (the boundary computed from the depth) + a flush, and NOTHING ELSE — this
///   is pure negotiation; the pack rides round 2.
/// * **Round 2** — the client re-sent the boundary as `shallow <oid>` lines PLUS `done`.
///   Reply with the `shallow` section (echoing the client's boundary) + flush + `NAK` +
///   the pack cut at that boundary.
///
/// Every shallow round MUST begin with the `shallow` section or git dies
/// `expected shallow list` (the multi-round bug the hermetic single-round tests missed).
fn build_shallow_pack_bytes(
    source: &dyn hugit_proto::ObjectSource,
    request: &hugit_proto::WantHave,
    plan: &ClonePlan,
) -> Option<Vec<u8>> {
    // Compute the pack + the shallow boundary — from the depth (round 1 / single-round
    // clients that send `deepen`+`done`) or the client-declared boundary (round 2).
    let (pack, boundary) = if let Some(depth) = plan.shallow_depth {
        hugit_proto::serve_shallow(source, &request.wants, depth).ok()?
    } else {
        hugit_proto::serve_shallow_at(source, &request.wants, &plan.shallow_client).ok()?
    };
    let mut out = Vec::new();
    for oid in &boundary {
        pkt_line(&mut out, format!("shallow {oid}\n").as_bytes());
    }
    pkt_flush(&mut out); // ends the shallow section (empty section = bare flush)
    // Gate the pack on `done`: a shallow request WITHOUT `done` is round-1 negotiation —
    // the client wants ONLY the boundary and will send `done` in round 2 for the pack.
    // Sending the pack now makes the client die on the follow-up round.
    if plan.done {
        pkt_line(&mut out, b"NAK\n");
        out.extend_from_slice(&pack.bytes);
    }
    Some(out)
}

/// The DETACHED-worker half of a `git-upload-pack` POST: assemble the pack (the heavy
/// per-object CAS walk) and WRITE the response to `request` itself. Runs entirely on
/// the worker thread — the accept loop has already returned to accept. On any assembly
/// failure it responds a clean 404 (fail-closed, never a truncated pack). The PACK
/// write rides [`send`] → `respond_bounded`, so a stalled/zero-window drainer is
/// abandoned at `io_budget` on THIS worker (not the loop), never a thread wedged
/// forever on the socket.
fn serve_upload_pack_response(
    source: SharedSource,
    want: hugit_proto::WantHave,
    plan: ClonePlan,
    seam: Option<CloneCacheSeam>,
    request: Request,
    io_budget: Duration,
) {
    // SHALLOW (`git clone --depth N`) — a depth-bounded pack preceded by the `shallow`
    // section, over the two-round stateless protocol. NEVER uses the cached full pack
    // (`prepare_upload_pack` forces `full_clone` off for any shallow request).
    if plan.is_shallow() {
        match build_shallow_pack_bytes(source.as_ref(), &want, &plan) {
            Some(out) => {
                let body_len = out.len();
                let resp = Response::from_data(out)
                    .with_status_code(200)
                    .with_header(git_content_type("application/x-git-upload-pack-result"));
                return send(request, resp, body_len, io_budget);
            }
            None => return respond_not_found(request, io_budget),
        }
    }

    // WP-BC — the cached full-clone fast path. For a FULL clone (the strict
    // `plan.full_clone` predicate) of a repo WITH a clone-pack cache, try the
    // pre-assembled pack: ONE R2 GET vs walking the whole object closure. The
    // R2 GET runs on THIS worker (never the accept loop), so it is fine here.
    //
    // CORRECTNESS: `try_serve_cached_clone_pack` recomputes `refset_sha` from
    // `plan.refs` (the SAME read-gated snapshot the wants were validated against +
    // the advertise was built from) and refuses any cached pointer whose `refset_sha`
    // disagrees → a moved ref, an absent/garbage pointer, or a GET error all yield
    // `None` and FALL OPEN to the slow walk below. So the cache serves a byte-correct
    // pack for the advertised refs, or nothing — never a wrong/truncated pack.
    // A FULL clone of a repo that HAS a clone-pack cache seam is served from the
    // pre-assembled pack — that IS the intended serve path (one R2 GET vs walking the
    // whole closure). So for such a repo the cache lookup is authoritative on WHY it
    // missed (clone-pack legibility):
    //   • HIT    → stream the cached pack.
    //   • ABSENT → the pack is not built yet (building at boot — the ~minutes background
    //              assembly — or a transient read). Answer a RETRYABLE 503, NEVER the
    //              whole-closure slow walk, which on the single-threaded engine is a
    //              latency DoS (a full clone = thousands of sync R2 fetches → the
    //              io-deadline abandon that surfaced as a silent empty/hung clone). The
    //              client sees a clear, retryable failure; `/readyz` shows `clonepack`.
    //   • STALE  → a ref moved (pointer built for a different tip-set). FALL OPEN to the
    //              slow walk: byte-correct for the advertised refs, and it avoids
    //              wedging the repo unclonable until the next push rebuilds the pack.
    // A repo with NO seam (GIT_DIR / small) and any non-full-clone (a fetch — bounded,
    // incremental) take the slow walk below, unchanged.
    if takes_clone_cache_path(plan.full_clone, seam.is_some()) {
        let seam = seam.as_ref().expect("has_seam checked");
        match clone_pack::try_serve_cached_clone_pack(
            &seam.r2,
            &seam.tenant,
            &seam.repo_slug,
            &plan.refs,
        ) {
            clone_pack::CloneCacheLookup::Hit(pack_bytes) => {
                // Frame IDENTICALLY to `build_upload_pack_bytes`: the `NAK` pkt-line
                // (`0008NAK\n`) then the RAW cached pack bytes, same media type. The
                // cached bytes are the raw packfile the walk would produce for this
                // tip-set.
                let mut out = Vec::with_capacity(8 + pack_bytes.len());
                pkt_line(&mut out, b"NAK\n");
                out.extend_from_slice(&pack_bytes);
                let body_len = out.len();
                let resp = Response::from_data(out)
                    .with_status_code(200)
                    .with_header(git_content_type("application/x-git-upload-pack-result"));
                return send(request, resp, body_len, io_budget);
            }
            clone_pack::CloneCacheLookup::Absent => {
                return respond_clone_pack_building(request, io_budget);
            }
            // Fall open to the slow walk below (a moved-ref stale cache).
            clone_pack::CloneCacheLookup::Stale => {}
        }
    }

    // No cache seam (GIT_DIR / small repo), a non-full-clone fetch (bounded), OR a stale
    // (moved-ref) cache on a cache-backed repo → the slow walk.
    match build_upload_pack_bytes(source.as_ref(), &want) {
        Some(out) => {
            let body_len = out.len();
            let resp = Response::from_data(out)
                .with_status_code(200)
                .with_header(git_content_type("application/x-git-upload-pack-result"));
            send(request, resp, body_len, io_budget);
        }
        None => respond_not_found(request, io_budget),
    }
}

/// Hand the upload-pack RESPONSE to a DETACHED worker that OWNS `request` and writes
/// the reply itself, FREEING the accept loop immediately: it returns to accept without
/// waiting for the ~seconds-long clone, so `/readyz` + other requests stay answerable
/// while the pack builds. This is the property the plain `run_bounded` (which BLOCKS
/// the caller up to its budget) could not give.
///
/// AVAILABILITY / DoS: a spawn failure (thread exhaustion under a clone flood) SHEDS a
/// fast inline 503 — it NEVER runs the heavy walk inline (which would wedge the single
/// accept thread). So a flood ties up bounded, shed-able WORKER threads (each capped by
/// the proto's [`hugit_proto::SERVE_FETCH_BUDGET`]), and the accept loop stays free.
/// The `request` is never lost on a spawn failure: [`spawn_with_payload`] returns the
/// payload back so we can respond a real 503.
fn spawn_upload_pack_worker(
    source: SharedSource,
    want: hugit_proto::WantHave,
    plan: ClonePlan,
    seam: Option<CloneCacheSeam>,
    request: Request,
    io_budget: Duration,
) {
    let payload = (source, want, plan, seam, request, io_budget);
    if let Err((_source, _want, _plan, _seam, request, io_budget)) =
        spawn_with_payload(payload, |p| {
            let (source, want, plan, seam, request, io_budget) = p;
            serve_upload_pack_response(source, want, plan, seam, request, io_budget);
        })
    {
        // Spawn failed (thread exhaustion) — shed inline, do NOT walk inline.
        respond_git_overloaded(request, io_budget);
    }
}

/// Run `job(payload)` on a DETACHED worker thread, freeing the caller IMMEDIATELY —
/// it does NOT wait for `job`. Returns `Ok(())` when the worker was spawned (the
/// payload, including any owned `Request`, now lives on the worker), or `Err(payload)`
/// when the worker could NOT be spawned (thread exhaustion) so the caller can recover
/// — the payload is HANDED BACK, never lost/dropped, so an owned connection can be
/// shed cleanly rather than silently closed.
///
/// The payload rides a one-slot [`std::sync::mpsc::sync_channel`]: the worker `recv`s
/// it once and runs `job`. The `send` is non-blocking (capacity 1, exactly one send),
/// so the caller returns at once. `P: Send + 'static` because it crosses the thread;
/// `Request` is `Send`, so an owned connection is a valid payload.
fn spawn_with_payload<P, F>(payload: P, job: F) -> Result<(), P>
where
    P: Send + 'static,
    F: FnOnce(P) + Send + 'static,
{
    let (tx, rx) = std::sync::mpsc::sync_channel::<P>(1);
    match std::thread::Builder::new()
        .name("hugit-clone".into())
        .spawn(move || {
            if let Ok(p) = rx.recv() {
                job(p);
            }
        }) {
        Ok(_) => {
            // Capacity-1 buffer, single send, receiver alive → never blocks.
            let _ = tx.send(payload);
            Ok(())
        }
        // Spawn failed: `rx` (and the un-run `job`) drop; the payload was never moved
        // into the closure, so hand it back to the caller intact.
        Err(_) => Err(payload),
    }
}

/// A retryable `503` for a full clone whose pre-assembled clone-pack is not ready yet
/// (building at boot — the ~minutes background assembly — or mid-rebuild after a push).
/// Sent INSTEAD of the whole-closure slow walk, which on the single-threaded engine is a
/// latency DoS. Carries `Retry-After` so `git` / the client backs off + retries, and a
/// clear message so the user sees a real, actionable state instead of a silent
/// empty/hung clone. Small body → the [`send`] write stays on the inline non-blocking
/// path. Operators see which repos are building on `/readyz` (`clonepack`).
fn respond_clone_pack_building(request: Request, io_budget: Duration) {
    let body = b"clone pack is being prepared; retry shortly\n".to_vec();
    let body_len = body.len();
    let resp = Response::from_data(body)
        .with_status_code(503)
        .with_header(
            tiny_http::Header::from_bytes(&b"Content-Type"[..], &b"text/plain"[..])
                .expect("static content-type"),
        )
        .with_header(
            tiny_http::Header::from_bytes(&b"Retry-After"[..], &b"10"[..])
                .expect("static retry-after"),
        );
    send(request, resp, body_len, io_budget);
}

/// Shed a fast `503` for a git request the engine cannot service right now (a clone
/// worker could not be spawned under load). Plain text so `git` surfaces it; small, so
/// the [`send`] write stays on the inline (non-blocking) path — the loop is never
/// blocked by the shed itself.
fn respond_git_overloaded(request: Request, io_budget: Duration) {
    let body = b"engine overloaded serving clones; retry shortly\n".to_vec();
    let body_len = body.len();
    send(
        request,
        Response::from_data(body).with_status_code(503).with_header(
            tiny_http::Header::from_bytes(&b"Content-Type"[..], &b"text/plain"[..])
                .expect("static content-type"),
        ),
        body_len,
        io_budget,
    );
}

/// Shed a fast `503` for a `git push` the engine cannot service right now (a receive-pack
/// worker could not be spawned under load). The push is NEVER unpacked inline — this shed
/// is the ONLY inline work, and it is small so the [`send`] write stays on the inline
/// (non-blocking) path — the accept loop is never blocked by the shed itself. The pusher
/// (authed) retries; nothing was written (no `ok` without a durable finalize).
fn respond_push_overloaded(request: Request, io_budget: Duration) {
    let body = b"engine overloaded accepting pushes; retry shortly\n".to_vec();
    let body_len = body.len();
    send(
        request,
        Response::from_data(body).with_status_code(503).with_header(
            tiny_http::Header::from_bytes(&b"Content-Type"[..], &b"text/plain"[..])
                .expect("static content-type"),
        ),
        body_len,
        io_budget,
    );
}

/// 413 PAYLOAD_TOO_LARGE — the pushed pack body exceeds the configured pack-size cap
/// (`max_pack_bytes`) OR the durable storage quota. A clean plain-text remote message
/// (git surfaces the body), emitted BEFORE any object is durably stored. `detail`
/// names which limit tripped.
fn respond_push_payload_too_large(request: Request, detail: &str, io_budget: Duration) {
    let body = format!("{detail}\n").into_bytes();
    let body_len = body.len();
    send(
        request,
        Response::from_data(body).with_status_code(413).with_header(
            tiny_http::Header::from_bytes(&b"Content-Type"[..], &b"text/plain"[..])
                .expect("static content-type"),
        ),
        body_len,
        io_budget,
    );
}

/// 503 — the durable storage accounting could not be read/determined, so the storage
/// quota is INDETERMINATE. Fail-closed: the push is refused (never allowed on an
/// indeterminate read), the client may retry. Mirrors the provision path's fail-closed
/// `count_owned_repos` → 503.
fn respond_push_storage_unavailable(request: Request, io_budget: Duration) {
    let body =
        b"storage quota could not be determined; push refused (fail-closed), retry shortly\n"
            .to_vec();
    let body_len = body.len();
    send(
        request,
        Response::from_data(body).with_status_code(503).with_header(
            tiny_http::Header::from_bytes(&b"Content-Type"[..], &b"text/plain"[..])
                .expect("static content-type"),
        ),
        body_len,
        io_budget,
    );
}

/// Whether EVERY `want` oid is an advertised ref tip in `refs` (the map values).
///
/// The `uploadpack.allowReachableSHA1InWant`-OFF default: a client may only fetch
/// from a tip the advertisement exposed for its principal, NEVER an arbitrary
/// interior or unadvertised oid. An empty want list is vacuously `true` (a full
/// clone derives its wants from `refs` upstream, so it never reaches this check).
/// This both prevents fishing for an unadvertised object and caps the reachability
/// walk to a real tip's closure (defence-in-depth for the serve_fetch wall-clock
/// DoS — a caller cannot pick the walk's root).
fn wants_all_advertised(refs: &BTreeMap<String, String>, wants: &[gix_hash::ObjectId]) -> bool {
    let advertised: std::collections::BTreeSet<String> = refs.values().cloned().collect();
    // `ObjectId::to_string` is canonical lowercase 40-hex, matching the `for-each-ref`
    // oid strings stored as `refs` values — a byte-exact compare with no case drift.
    wants.iter().all(|w| advertised.contains(&w.to_string()))
}

/// The refs to advertise for `repo`, or `None` when git serving is not live for
/// it. Enforces, in order: git dir wired at all → repo slug safe → `principal`
/// authorized to read (the `authorize_read` gate) → non-empty ref set. Every
/// failure is `None` (the caller maps it to a uniform 404 — no existence oracle,
/// identical for absent / private-unauthorized / cross-tenant / not-live).
fn git_refs_for(
    state: &AppState,
    repo: &str,
    principal: &[String],
) -> Option<BTreeMap<String, String>> {
    if !crate::state::is_safe_repo_slug(repo) {
        return None;
    }
    // git serving is not live for this repo unless its git seam was loaded at boot.
    // A repo with no `RepoState` (un-loaded, or a different repo's content) → None
    // → a uniform 404 (no oracle for which repos are git-served).
    let repo_state = state.repo_state_or_load(repo)?;
    if repo_state.git_refs.is_empty() {
        return None;
    }
    // READ-visibility gate, identical predicate to the `/v1` reads, against the
    // clone `principal` (anonymous for an unauthed clone, a real `clerk:{org}:{user}`
    // tenant for an authed one — see `clone_principal`, which fail-closes to anon and
    // never grants the operator path). A public repo serves any principal incl. anon;
    // a PRIVATE repo serves ONLY its owning tenant (so the owner clones it; a foreign
    // tenant or anon gets the same 404 as an absent repo). A load/verify failure or a
    // denied read → None (→ 404, no oracle).
    let log = state.load_verified(repo).ok()?;
    let meta = crate::authz::project_repo_meta(&log);
    if !crate::authz::authorize_read(principal, &meta) {
        return None;
    }
    // This repo's OWN refs (the multi-repo forge resolves `{repo}` → its RepoState).
    // A snapshot of the LIVE ref map: a just-completed CAS push has already advanced
    // the pushed tip here, so the advertisement reflects it with no reboot.
    Some(repo_state.git_refs.snapshot())
}

/// Re-frame an upload-pack request body, trimming each `want`/`have` pkt-line to
/// its leading oid token (dropping the capability list real git appends to the
/// first `want`). Non-want/have lines (flush, delim, `done`, etc.) pass through
/// byte-for-byte. The output is well-formed pkt-lines the proto parser accepts.
///
/// Decodes git's pkt-line framing directly: a 4-hex big-endian length prefix
/// (covering the prefix itself); `0000` is a flush (length < 4 are the special
/// flush/delim/response-end markers, copied verbatim). A malformed/over-running
/// length stops the walk (the caller then gets whatever was parsed — an empty
/// `WantHave` at worst, which is treated as a full clone upstream).
fn strip_want_have_caps(body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(body.len());
    let mut i = 0;
    while i + 4 <= body.len() {
        let len_hex = &body[i..i + 4];
        let Ok(len_str) = std::str::from_utf8(len_hex) else {
            break;
        };
        let Ok(len) = usize::from_str_radix(len_str, 16) else {
            break;
        };
        // 0000/0001/0002 are flush/delim/response-end (no payload) — copy as-is.
        if len < 4 {
            out.extend_from_slice(len_hex);
            i += 4;
            continue;
        }
        if i + len > body.len() {
            break; // truncated — stop (don't emit a half line).
        }
        let payload = &body[i + 4..i + len];
        i += len;

        // Trim a want/have line to `<verb> <oid>\n`; everything else verbatim.
        let trimmed = trim_oid_line(payload, b"want ").or_else(|| trim_oid_line(payload, b"have "));
        match trimmed {
            Some(clean) => pkt_line(&mut out, &clean),
            None => {
                out.extend_from_slice(len_hex);
                out.extend_from_slice(payload);
            }
        }
    }
    out
}

/// If `payload` starts with `verb` (e.g. `b"want "`), return `<verb><oid>\n` with
/// only the first whitespace-delimited oid token (caps + trailing dropped); else
/// `None`.
fn trim_oid_line(payload: &[u8], verb: &[u8]) -> Option<Vec<u8>> {
    let rest = payload.strip_prefix(verb)?;
    let oid: &[u8] = rest
        .split(|b| b.is_ascii_whitespace())
        .find(|t| !t.is_empty())
        .unwrap_or(&[]);
    let mut line = Vec::with_capacity(verb.len() + oid.len() + 1);
    line.extend_from_slice(verb);
    line.extend_from_slice(oid);
    line.push(b'\n');
    Some(line)
}

/// Extract a query-param value (no percent-decoding needed for `service`, whose
/// value is a fixed ASCII token).
fn query_param<'a>(query: &'a str, key: &str) -> Option<&'a str> {
    let prefix = format!("{key}=");
    query
        .split('&')
        .find_map(|kv| kv.strip_prefix(prefix.as_str()))
}

/// Append one pkt-line: a 4-hex big-endian length prefix (length INCLUDES the 4
/// prefix bytes) followed by `data`. This is git's pkt-line framing; no library
/// needed for the trivial encode direction (the proto owns decode of want/have).
fn pkt_line(out: &mut Vec<u8>, data: &[u8]) {
    let len = data.len() + 4;
    // pkt-line length is a 16-bit field; a single git ref/line is far under 65516.
    debug_assert!(
        len <= 0xffff,
        "pkt-line payload exceeds the 65516-byte limit"
    );
    out.extend_from_slice(format!("{len:04x}").as_bytes());
    out.extend_from_slice(data);
}

/// Append a flush packet (`0000`).
fn pkt_flush(out: &mut Vec<u8>) {
    out.extend_from_slice(b"0000");
}

/// A git Content-Type header for `value`.
fn git_content_type(value: &str) -> Header {
    Header::from_bytes(&b"Content-Type"[..], value.as_bytes())
        .expect("static git content-type header is valid")
}

/// The human-readable body returned for push attempts. Plain text so `git`
/// surfaces it verbatim in the terminal ("remote: …") — the most useful UX.
const PUSH_FORBIDDEN_BODY: &str =
    "git push is not yet supported by this engine; land changes with 'hugit land'";

/// Respond 403 when a client attempts `git push` (git-receive-pack). The body
/// is plain text because `git` echoes the remote body in the terminal, giving
/// the developer an actionable message instead of a cryptic connection error.
/// The receive-pack (push) capabilities we advertise. `report-status` so the client
/// reads our result report; `delete-refs` so the git CLIENT will actually SEND a
/// zero-id (delete) command — per the protocol a client refuses to push a
/// `<old> 0{40} <ref>` delete unless the server advertised `delete-refs`, which is
/// why `git push --delete` silently sent an empty command list before this (the
/// server-side delete handler already worked — proven via a raw POST; this unlocks
/// the client path); `object-format=sha1` matches the proto's hash.
const RECEIVE_CAPS: &str = "report-status delete-refs object-format=sha1 agent=hugit-serve";

/// Serve `GET /<repo>/info/refs?service=git-receive-pack` — the push advertisement.
/// Gated identically to the push itself (flag + write seam + write-authz), so a
/// caller who couldn't push never even sees the advertisement (403/401/404, no
/// oracle). Disabled deploy → the honest 403 ("push not supported here").
fn handle_receive_advertise(state: &AppState, repo: &str, request: Request, io_budget: Duration) {
    if !state.write_path_enabled {
        return respond_push_forbidden(request, io_budget);
    }
    let headers: Vec<tiny_http::Header> = request.headers().to_vec();
    let ctx = match crate::server::two_tier_auth_ctx(state, &headers) {
        Ok(c) => c,
        Err(_) => return respond_push_unauth(request, io_budget),
    };
    // A push is a WRITE: a `repo:read`-only PAT is refused at the handshake with a
    // clear scope 403 (it can clone/fetch, never push). A session/dev credential or a
    // `repo:write` PAT passes (`write_ok`).
    if !ctx.write_ok {
        return respond_push_scope_forbidden(request, io_budget);
    }
    let principal = ctx.principal;
    // G11: resolve the bare URL slug to the pusher's user-scoped stored key (legacy
    // fallback) — gated identically to the push, so no oracle before authz.
    let repo_owned = state.resolve_repo_slug(repo, &principal);
    let repo = repo_owned.as_str();
    if !crate::state::is_safe_repo_slug(repo) {
        return respond_not_found(request, io_budget);
    }
    let Some(repo_state) = state.repo_state_or_load(repo) else {
        return respond_not_found(request, io_budget);
    };
    if !repo_state.has_write_seam() {
        return respond_not_found(request, io_budget); // no write seam (GIT_DIR or CAS) → 404
    }
    let Ok(log) = state.load_verified(repo) else {
        return respond_not_found(request, io_budget);
    };
    let meta = crate::authz::project_repo_meta(&log);
    if !crate::authz::authorize_write(&principal, &meta) {
        return respond_not_found(request, io_budget); // not the owner → 404, no oracle
    }

    let mut out = Vec::new();
    pkt_line(&mut out, b"# service=git-receive-pack\n");
    pkt_flush(&mut out);
    let mut first = true;
    // A snapshot of the LIVE refs (a prior CAS push in this lifetime is reflected).
    let refs = repo_state.git_refs.snapshot();
    if refs.is_empty() {
        // No refs yet: the zero-id capabilities line (so the client can still create).
        let line = format!("{} capabilities^{{}}\0{RECEIVE_CAPS}\n", "0".repeat(40));
        pkt_line(&mut out, line.as_bytes());
    } else {
        for (name, oid) in &refs {
            let mut line = format!("{oid} {name}");
            if first {
                line.push('\0');
                line.push_str(RECEIVE_CAPS);
                first = false;
            }
            line.push('\n');
            pkt_line(&mut out, line.as_bytes());
        }
    }
    pkt_flush(&mut out);
    let body_len = out.len();
    let resp = Response::from_data(out)
        .with_status_code(200)
        .with_header(git_content_type(
            "application/x-git-receive-pack-advertisement",
        ));
    send(request, resp, body_len, io_budget);
}

/// Handle a `POST /<repo>/git-receive-pack` (push). The WRITE side of the git wire.
///
/// Gated, fail-closed, and 404-no-oracle on any authz denial (never reveal a
/// private/absent repo). Flow: authenticate the pusher → require the deploy flag +
/// a write seam → **write-authz** (ownership, NOT the read predicate) → parse the
/// wire → `hugit_proto::receive_pack` (bound/unpack/verify/store/anchor/append) into
/// the repo's git dir → persist the mutated log (compare-and-swap) → update the
/// git-dir ref so the wire advertises the new tip → emit the `report-status`.
///
/// v0 scope: a single non-delete ref per push (`docs/plan/2026-06-22-receive-pack-wave-design.md`).
fn handle_receive_pack(
    state: &AppState,
    repo: &str,
    body: &[u8],
    request: Request,
    io_budget: Duration,
) {
    use crate::receive_wire::parse_receive_pack_body;

    // The deploy gate FIRST: receive-pack is OFF unless `HUGIT_SERVE_RECEIVE_PACK=1`.
    // Off → the honest 403 ("push not supported here"), BEFORE auth, so a stock
    // deploy answers the same clear message to any client (authed or not).
    if !state.write_path_enabled {
        return respond_push_forbidden(request, io_budget);
    }

    // Authenticate — a push is a WRITE, so it MUST carry a valid bearer (unlike an
    // anonymous clone). An invalid/absent token → 401.
    let headers: Vec<tiny_http::Header> = request.headers().to_vec();
    let ctx = match crate::server::two_tier_auth_ctx(state, &headers) {
        Ok(c) => c,
        Err(_) => return respond_push_unauth(request, io_budget),
    };
    // A push is a WRITE: a `repo:read`-only PAT is refused at the handshake with a
    // clear scope 403 (it can clone/fetch, never push). A session/dev credential or a
    // `repo:write` PAT passes (`write_ok`).
    if !ctx.write_ok {
        return respond_push_scope_forbidden(request, io_budget);
    }
    let principal = ctx.principal;
    // G11: resolve the bare URL slug to the pusher's user-scoped stored key (legacy
    // fallback). The resolved slug flows through the seam load, the write-authz, AND the
    // `ReceivePlan` handed to the worker, so the durable finalize keys the SAME stored
    // slug's manifests.
    let repo_owned = state.resolve_repo_slug(repo, &principal);
    let repo = repo_owned.as_str();

    // A write seam is required (GIT_DIR mode). No seam (CAS mode / unknown repo) →
    // 404, no oracle. Everything past here is gated behind a successful authz.
    if !crate::state::is_safe_repo_slug(repo) {
        return respond_not_found(request, io_budget);
    }
    let Some(repo_state) = state.repo_state_or_load(repo) else {
        return respond_not_found(request, io_budget);
    };
    // A write seam is required (GIT_DIR or CAS mode). None → 404, no oracle. The
    // sink itself is OPENED below (after authz + parse), so an open fault is a
    // per-ref ng rather than a pre-auth 404.
    if !repo_state.has_write_seam() {
        return respond_not_found(request, io_budget);
    }

    // Load the repo's log (the authz meta source + the append target), pinned to the
    // head we compare-and-swap against. `log` moves into the worker's `ReceivePlan`
    // (the append + persist happen off-loop), so it is not mutated here.
    let (log, token) = match state.load_verified_with_token(repo) {
        Ok(lt) => lt,
        Err(_) => return respond_not_found(request, io_budget),
    };

    // WRITE-authz: OWNERSHIP, never the read-visibility predicate (a public repo
    // opens reads, NEVER writes). Denial → 404 (no oracle).
    let meta = crate::authz::project_repo_meta(&log);
    if !crate::authz::authorize_write(&principal, &meta) {
        return respond_not_found(request, io_budget);
    }

    // Pack-size cap (G10): a DEDICATED receive-pack body ceiling
    // (`HUGIT_SERVE_MAX_PACK_BYTES`, default 64 MiB), enforced HERE — after auth/authz,
    // BEFORE the wire parse and BEFORE any worker spawn / unpack. The accept loop reads
    // the receive-pack body capped at `max_pack_bytes()+1` (every other POST keeps the 8
    // MiB JSON door), so an over-cap push arrives here truncated to cap+1 and is rejected
    // with a clean 413 — no objects unpacked, no manifest written, the accept loop freed.
    // The same number is threaded into `RecvLimits` below as the second (proto) wall.
    let max_pack = max_pack_bytes();
    if body.len() > max_pack {
        return respond_push_payload_too_large(
            request,
            &format!("pack exceeds the {max_pack}-byte limit"),
            io_budget,
        );
    }

    // Parse the wire. A framing/command error → 400 (a malformed push).
    let wire = match parse_receive_pack_body(body) {
        Ok(w) => w,
        Err(e) => return respond_push_bad_request(request, &e.to_string(), io_budget),
    };
    if let Err(e) = wire.require_pack() {
        return respond_push_bad_request(request, &e.to_string(), io_budget);
    }
    if wire.commands.len() != 1 {
        return respond_push_bad_request(
            request,
            "v0 accepts exactly one ref update per push",
            io_budget,
        );
    }
    let cmd = wire.commands[0].clone();

    // ── CHEAP INLINE HALF DONE — HAND THE HEAVY TAIL TO A DETACHED WORKER ──────
    // Everything above ran on the accept loop: the deploy-gate, the pusher auth, the
    // write-authz (OWNERSHIP), the log load + chain-verify (the authz meta + append
    // target), and the wire parse (framing + the single-command shape). What REMAINS
    // is HEAVY and I/O-bound and MUST NOT run here: opening the write sink (a CAS-mode
    // open re-reads `oid-index.json` from R2), the pure-Rust gix-pack UNPACK
    // (bound→verify→store→anchor — seconds-to-minutes on a large pack), the durable
    // finalize (objects→CAS, the log compare-and-swap, the conditional If-Match
    // refs.json/oid-index manifest PUTs), the in-memory ref hot-swap, and the
    // report-status write. Running it inline would wedge the single accept thread
    // (incl. `/readyz`) for the whole push — the same latency-DoS the clone path had.
    //
    // So HAND it to a DETACHED worker that OWNS the connection and RESPONDS itself,
    // freeing the loop IMMEDIATELY — the exact mirror of `spawn_upload_pack_worker`.
    // The durability ordering + security are UNCHANGED — only WHERE they run moves.
    // A spawn failure sheds a fast inline 503 (the `Request` is handed BACK), NEVER a
    // heavy unpack inline. See `serve_receive_pack_response` / `ReceivePlan`.
    let plan = ReceivePlan {
        state: state.clone(),
        repo: repo.to_string(),
        principal,
        log,
        token,
        cmd,
        pack: wire.pack,
        max_pack_bytes: max_pack,
    };
    spawn_receive_pack_worker(plan, request, io_budget);
}

/// The owned, `Send + 'static` payload for the heavy receive-pack tail, handed to a
/// DETACHED worker so the accept loop is freed the instant the cheap inline validation
/// passes (the exact mirror of the clone's `(SharedSource, WantHave)` hand-off).
///
/// ## Why this is `Send + 'static` AND why the in-memory hot-swap is still visible
///
/// `AppState` is `Clone + Send + Sync + 'static`. Its per-repo `RepoState`s live in a
/// `HashMap` that the clone duplicates — BUT each `RepoState`'s interior-mutable state
/// (`git_refs: LiveRefs(Arc<RwLock<…>>)` and `live_oid_index: LiveOidIndex(Arc<RwLock<…>>)`)
/// is `Arc`-shared, so the clone's maps ARE THE SAME maps the accept-loop's state reads.
/// The worker resolves `state.repo_state(&repo)` from THIS clone and applies the ref
/// hot-swap (`apply_cas_push_inmemory` / `apply_cas_delete_inmemory`) through it — the
/// mutation lands in the shared `Arc<RwLock>`, so the loop's very next advertise reflects
/// the new/removed tip with no reboot, exactly as before, only now off-loop. The
/// `RwLock` makes the now-possible concurrency (a worker writing while the loop
/// advertises, or two push workers) memory-safe; the DURABLE correctness of concurrent
/// same-ref pushes is guarded — as it already was — by the log persist compare-and-swap
/// and the conditional If-Match manifest PUTs (`conditional_manifest_write`, WP-IFMATCH),
/// NOT by the single-threaded accept loop. So moving the unpack off-loop needs no
/// `state.rs` change and introduces no race.
struct ReceivePlan {
    /// A clone of the engine state — carries `persist` (the log compare-and-swap) + the
    /// receive-pack flag gate, and (via the shared `Arc`s described above) the live
    /// ref/oid-index maps the hot-swap advances.
    state: AppState,
    repo: String,
    principal: Vec<String>,
    log: hugit_refstore::log::EventLog,
    token: crate::writes::CasToken,
    cmd: crate::receive_wire::ReceiveCommand,
    /// The uploaded pack bytes (empty for a delete, which carries no objects).
    pack: Vec<u8>,
    /// The pack-size cap that gated this push inline (`max_pack_bytes`), threaded so the
    /// worker's `RecvLimits` carries the SAME wired number (not a fresh `::default()`).
    max_pack_bytes: usize,
}

/// Hand the receive-pack RESPONSE to a DETACHED worker that OWNS `request` and writes
/// the report-status itself, FREEING the accept loop immediately — it returns to accept
/// without waiting for the ~seconds-to-minutes unpack + durable finalize, so `/readyz`
/// and other requests stay answerable throughout. The exact mirror of
/// `spawn_upload_pack_worker`.
///
/// AVAILABILITY / DoS: a spawn failure (thread exhaustion under a push flood) SHEDS a
/// fast inline 503 — it NEVER runs the heavy unpack inline (which would wedge the single
/// accept thread). `spawn_with_payload` hands the `Request` BACK on spawn-Err so the
/// connection is shed cleanly (a real 503), never silently lost. Pushes are authed +
/// ownership-gated + rare, so the worker pool is not an anon amplification surface.
fn spawn_receive_pack_worker(plan: ReceivePlan, request: Request, io_budget: Duration) {
    let payload = (plan, request, io_budget);
    if let Err((_plan, request, io_budget)) = spawn_with_payload(payload, |p| {
        let (plan, request, io_budget) = p;
        serve_receive_pack_response(plan, request, io_budget);
    }) {
        // Spawn failed (thread exhaustion) — shed inline, do NOT unpack inline.
        respond_push_overloaded(request, io_budget);
    }
}

/// The DETACHED-worker half of a `POST /<repo>/git-receive-pack`: open the write sink,
/// run the gix-pack unpack (create/update) or the durable delete, apply the in-memory
/// ref hot-swap, and WRITE the report-status — all off the accept loop, which has
/// already returned to accept. `ok` is still emitted ONLY after the durable finalize
/// succeeds (the ordering is UNCHANGED); a rejected/failed push answers a clean per-ref
/// `ng`; a panic here is isolated by the accept loop's `catch_unwind` and drops only
/// THIS connection (the worker owns nothing shared beyond the `Arc<RwLock>` maps, whose
/// locks are never held across the unpack).
fn serve_receive_pack_response(plan: ReceivePlan, request: Request, io_budget: Duration) {
    let ReceivePlan {
        state,
        repo,
        principal,
        mut log,
        token,
        cmd,
        pack,
        max_pack_bytes,
    } = plan;

    // Re-resolve the repo's live seam from the CLONED (Arc-sharing) state. Existence +
    // the write seam were already validated inline; this is a defensive re-check
    // (never an oracle) — a `None` here can only mean a concurrent teardown, → 404.
    let Some(repo_state) = state.repo_state_or_load(&repo) else {
        return respond_not_found(request, io_budget);
    };

    if cmd.is_delete() {
        // DELETE-ref path. A delete is a WRITE — it already cleared the same deploy-gate
        // + auth + write-authz (ownership) gates inline (do NOT weaken that). It carries
        // NO pack / no target / no reachability, so it never touches the proto
        // `receive_pack` unpack path. `handle_delete_ref` does the default-branch guard
        // + the stale-check against the AUTHORITATIVE live `git_refs` snapshot, the
        // durable finalize (refs.json rewrite + `ref.delete` event), and the in-memory
        // hot-swap. `ok` ONLY after a durable removal (fail-closed). Off-loop now: its
        // conditional-manifest R2 writes no longer block the accept thread either.
        return handle_delete_ref(
            &state, &repo, repo_state, &cmd, &principal, &mut log, &token, request, io_budget,
        );
    }

    finish_receive_pack(
        &state,
        &repo,
        repo_state,
        cmd,
        principal,
        log,
        token,
        pack,
        max_pack_bytes,
        request,
        io_budget,
    );
}

/// The outcome of the CAS-mode storage-quota enforcement (G10): either allow the push,
/// refuse it over-quota (→ 413), or fail closed on an indeterminate accounting read
/// (→ 503, NEVER allow-on-error).
enum StorageEnforcement {
    /// A cap is breached — the plain-text 413 message.
    Exceeded(String),
    /// The accounting could not be determined → 503 fail-closed.
    Unavailable,
}

/// Enforce the per-repo + per-owner_tenant storage-byte caps (G10) for a CAS-mode push
/// of `delta` NEW stored bytes, BEFORE any object is flushed. Enumerates the
/// owner_tenant's owned-repo slug set via the FAIL-CLOSED `owned_repo_logs` (an
/// indeterminate enumeration → 503), then runs the R2-backed [`crate::cas::check_storage_quota`]
/// (each `size.json` read fail-closed). A `delta` of 0 is always allowed. A repo with no
/// `owner_tenant` (legacy/unowned) falls back to a per-repo-only check (no rollup).
fn enforce_cas_storage_quota(
    state: &AppState,
    seam: &crate::state::CasWriteSeam,
    owner_tenant: &Option<String>,
    delta: u64,
) -> Result<(), StorageEnforcement> {
    if delta == 0 {
        return Ok(());
    }
    let caps = crate::cas::storage_caps_from_env();
    let owned_slugs: Vec<String> = match owner_tenant {
        Some(ot) => match state.owned_repo_logs(ot) {
            Some(logs) => logs.into_iter().map(|(slug, _)| slug).collect(),
            // Indeterminate ownership enumeration → fail closed (503), never allow.
            None => return Err(StorageEnforcement::Unavailable),
        },
        None => vec![seam.repo_slug.clone()],
    };
    match crate::cas::check_storage_quota(
        &seam.r2,
        &seam.tenant,
        &seam.repo_slug,
        &owned_slugs,
        delta,
        &caps,
    ) {
        Ok(()) => Ok(()),
        Err(crate::cas::StorageQuotaError::Exceeded(v)) => {
            let which = match v {
                crate::cas::QuotaVerdict::TenantExceeded => "tenant",
                _ => "repository",
            };
            Err(StorageEnforcement::Exceeded(format!(
                "STORAGE_QUOTA_EXCEEDED: this push would exceed the {which} storage quota"
            )))
        }
        Err(crate::cas::StorageQuotaError::Unavailable(_)) => Err(StorageEnforcement::Unavailable),
    }
}

/// The on-disk path of a GIT_DIR-mode repo's stored-byte counter (`hugit-size.json`,
/// inside the git dir — hugit's private space in local mode).
fn gitdir_size_path(git_dir: &std::path::Path) -> std::path::PathBuf {
    git_dir.join("hugit-size.json")
}

/// Read a GIT_DIR repo's accounted stored bytes. Absent → 0 (never pushed). Any other
/// read/parse fault is an `Err` (the caller fails closed → 503), NEVER silently 0.
fn read_gitdir_stored_bytes(git_dir: &std::path::Path) -> Result<u64, String> {
    match std::fs::read(gitdir_size_path(git_dir)) {
        Ok(bytes) => Ok(crate::cas::parse_size_manifest(&bytes)?.bytes),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(0),
        Err(e) => Err(format!("gitdir size.json read: {e}")),
    }
}

/// Write a GIT_DIR repo's stored-byte counter (best-effort accounting; the CHECK is the
/// fail-closed gate).
fn write_gitdir_stored_bytes(git_dir: &std::path::Path, bytes: u64) -> Result<(), String> {
    let body = serde_json::to_vec(&crate::cas::SizeManifest { bytes })
        .map_err(|e| format!("gitdir size.json serialize: {e}"))?;
    std::fs::write(gitdir_size_path(git_dir), body)
        .map_err(|e| format!("gitdir size.json write: {e}"))
}

/// The create/update tail of a receive-pack, run on the detached worker: build the
/// `ReceiveRequest`, open the write sink, run the gix-pack UNPACK
/// (bound→verify→store→anchor), then the mode-specific durable finalize + the in-memory
/// ref hot-swap, and answer the report-status. `ok` is emitted ONLY after the durable
/// finalize (objects → log compare-and-swap → conditional manifest PUTs) succeeds — the
/// durability ordering + every security check (bomb caps, stale-check against the live
/// `git_refs`, If-Match manifest CAS, no CAS poisoning) are BYTE-IDENTICAL to the prior
/// inline path; only the thread they run on changed.
#[allow(clippy::too_many_arguments)]
fn finish_receive_pack(
    state: &AppState,
    repo: &str,
    repo_state: &crate::state::RepoState,
    cmd: crate::receive_wire::ReceiveCommand,
    principal: Vec<String>,
    mut log: hugit_refstore::log::EventLog,
    token: crate::writes::CasToken,
    pack: Vec<u8>,
    max_pack: usize,
    request: Request,
    io_budget: Duration,
) {
    use crate::receive_wire::{RefOutcome, build_report_status};
    use crate::state::RepoWriter;
    use crate::writes::LogSink; // brings AppState::persist (the compare-and-swap) into scope
    use hugit_proto::write::receive::{ReceiveRequest, RecvLimits, RefUpdate, receive_pack};
    use hugit_proto::write::store::Cas; // the &mut dyn Cas sink coercion

    let req = ReceiveRequest {
        pack,
        update: RefUpdate {
            ref_name: cmd.ref_name.clone(),
            expected: if cmd.is_create() {
                None
            } else {
                Some(cmd.old_oid.clone())
            },
            new_oid: cmd.new_oid.clone(),
        },
        principal_chain: principal,
        recorded_at: now_ms(),
    };

    // Open the push write sink (GIT_DIR: local dir; CAS: buffer→CAS + R2 manifests).
    // A seam that fails to OPEN (e.g. an R2 read fault reading the current oid-index
    // for the CAS writer) → a transient per-ref `ng`, never a fake success.
    let mut writer = match repo_state.open_writer() {
        Ok(Some(w)) => w,
        Ok(None) => return respond_not_found(request, io_budget),
        Err(_) => {
            let report = build_report_status(
                Ok(()),
                &[RefOutcome::Ng {
                    ref_name: cmd.ref_name.clone(),
                    reason: "write-seam-unavailable".into(),
                }],
            );
            return send_report(request, report, io_budget);
        }
    };

    let gate = state.receive_flag_gate();
    // The AUTHORITATIVE current ref view for the stale-check (compare-and-append):
    // the SAME live `git_refs` projection the advertise is built from — NOT
    // `replay(log)` (a CAS-ingested branch has no `ref.update` event on the log, so
    // a log-derived view would false-reject every update of an existing branch).
    let current_refs = repo_state.git_refs.snapshot();
    // bound→unpack→verify→store→anchor→append into the mode's object sink.
    let recv = {
        let cas: &mut dyn Cas = match &mut writer {
            RepoWriter::GitDir { cas, .. } => cas,
            RepoWriter::Cas { cas, .. } => cas.as_mut(),
        };
        receive_pack(
            &gate,
            &req,
            cas,
            &mut log,
            &current_refs,
            // The proto's compressed-pack wall carries the SAME wired number the inline
            // 413 gate used (`max_pack`), so the two caps can never disagree; the other
            // bomb-caps (inflated / objects / delta-pass) keep their proto defaults.
            RecvLimits {
                max_pack_bytes: max_pack,
                ..RecvLimits::default()
            },
        )
    };
    // The push's owner_tenant (for the per-owner_tenant storage rollup), projected from
    // the loaded log. `None` for a legacy/unowned repo → the storage check falls back to
    // a per-repo-only cap (no rollup).
    let owner_tenant = crate::authz::project_repo_meta(&log).owner_tenant;
    match recv {
        Ok(_receipt) => match &mut writer {
            // GIT_DIR mode: persist the log (compare-and-swap) then move the on-disk ref
            // so the upload-pack wire advertises the new tip. Storage-byte cap (G10) is
            // enforced FIRST, before the durable advance.
            RepoWriter::GitDir { git_dir, .. } => {
                // Storage-byte cap (G10), GitDir/local mode: a PER-REPO cap enforced
                // BEFORE the log persist + ref advance. Loose objects may already be on
                // disk (dangling/unreachable/GC-able) but the ref does NOT advance over
                // quota. GitDir is the single-tenant local mode, so there is no
                // owner_tenant rollup here; the delta is the compressed pack size (a
                // conservative v0 proxy — NOTE). Fail-closed: an unreadable counter → 503.
                let caps = crate::cas::storage_caps_from_env();
                let gitdir_delta = req.pack.len() as u64;
                let gitdir_current = match read_gitdir_stored_bytes(git_dir) {
                    Ok(n) => n,
                    Err(_) => return respond_push_storage_unavailable(request, io_budget),
                };
                if gitdir_delta > 0 && gitdir_current.saturating_add(gitdir_delta) > caps.per_repo {
                    return respond_push_payload_too_large(
                        request,
                        "STORAGE_QUOTA_EXCEEDED: this push would exceed the repository storage quota",
                        io_budget,
                    );
                }
                if let Err(e) = state.persist(repo, &log, &token) {
                    let report = build_report_status(
                        Err("log-persist-failed"),
                        &[RefOutcome::Ng {
                            ref_name: cmd.ref_name.clone(),
                            reason: format!("persist:{}", e.status),
                        }],
                    );
                    return send_report(request, report, io_budget);
                }
                if let Err(reason) =
                    update_git_ref(git_dir, &cmd.ref_name, &cmd.new_oid, &cmd.old_oid)
                {
                    let report = build_report_status(
                        Ok(()),
                        &[RefOutcome::Ng {
                            ref_name: cmd.ref_name.clone(),
                            reason,
                        }],
                    );
                    return send_report(request, report, io_budget);
                }
                let report = build_report_status(Ok(()), &[RefOutcome::Ok(cmd.ref_name.clone())]);
                send_report(request, report, io_budget);
                // POST-DURABLE, POST-`ok`: bump the on-disk storage counter by this
                // push's delta (best-effort — the push already succeeded; a counter
                // write fault only under-counts, the fail-open direction on the
                // BOOKKEEPING while the CHECK above stays fail-closed).
                if gitdir_delta > 0 {
                    let _ = write_gitdir_stored_bytes(
                        git_dir,
                        gitdir_current.saturating_add(gitdir_delta),
                    );
                }
                // POST-DURABLE, POST-`ok`: best-effort KungFu merge event (WP
                // W-WEBHOOK) — see the CAS arm below for the full rationale.
                crate::merge_hook::emit_merge_event(
                    &repo_state.git_source,
                    repo,
                    &cmd.ref_name,
                    &cmd.old_oid,
                    &cmd.new_oid,
                    &req.principal_chain,
                );
            }
            // CAS mode: objects → log → manifests, each fail-closed. The ORDER is the
            // invariant — a manifest never advertises a tip whose closure was not
            // uploaded AND whose ref.update event was not recorded. `finalize_cas_push`
            // owns the flush + manifest steps; the log persist stays our closure.
            RepoWriter::Cas { cas, seam } => {
                // Storage-byte cap (G10): enforce BEFORE the flush/manifest commit. The
                // pack is unpacked in memory (bounded by the bomb caps) but NOT yet
                // stored to CAS, so a refusal here commits NO object and leaves every
                // manifest unchanged. `delta` is the NEW distinct-object stored bytes.
                let storage_delta = cas.pending_bytes();
                match enforce_cas_storage_quota(state, seam, &owner_tenant, storage_delta) {
                    Ok(()) => {}
                    Err(StorageEnforcement::Exceeded(msg)) => {
                        return respond_push_payload_too_large(request, &msg, io_budget);
                    }
                    Err(StorageEnforcement::Unavailable) => {
                        return respond_push_storage_unavailable(request, io_budget);
                    }
                }
                let res = crate::cas::finalize_cas_push(
                    cas.as_mut(),
                    &seam.r2,
                    &seam.tenant,
                    &seam.repo_slug,
                    &cmd.ref_name,
                    &cmd.new_oid,
                    // The pusher's `expected` tip (`None` for a create) — the SAME value
                    // the stale-check validated (`req.update.expected`). Threaded so the
                    // conditional refs.json write RE-VALIDATES it against the fresh base,
                    // closing the cross-instance same-ref lost-update (FIX-IFMATCH-REMERGE).
                    req.update.expected.as_deref(),
                    // The push's NEW stored-byte delta — finalize bumps size.json by this
                    // AFTER the tip is durably advertised (the quota CHECK already passed
                    // above, before the flush).
                    storage_delta,
                    || {
                        state
                            .persist(repo, &log, &token)
                            .map_err(|e| format!("persist:{}", e.status))
                    },
                );
                match res {
                    Ok(()) => {
                        // The push is DURABLE (objects → log → manifests committed).
                        // Live ref hot-swap: refresh this engine's in-memory view so
                        // the new tip serves with NO reboot — merge the pushed
                        // `oid → blake3` (reused verbatim from the CasRw, never re-read
                        // from R2) into the live oid-index, then advance the ref tip.
                        // On the (defensive) failure path the push stays durable + is
                        // reported `ok`; the prior tip keeps serving and the new tip
                        // loads on the next reboot (a read never regresses, and an
                        // unresolvable tip is never advertised).
                        if let Err(e) = repo_state.apply_cas_push_inmemory(
                            &cmd.ref_name,
                            &cmd.new_oid,
                            cas.index_additions(),
                            &cas.consumed_existing_bases(),
                        ) {
                            eprintln!(
                                "hugit-serve: CAS push durable but in-memory ref hot-swap \
                                 skipped (serves after next reboot): {e}"
                            );
                        }
                        let report =
                            build_report_status(Ok(()), &[RefOutcome::Ok(cmd.ref_name.clone())]);
                        send_report(request, report, io_budget);
                        // POST-DURABLE, POST-`ok`: best-effort KungFu merge event
                        // (WP W-WEBHOOK). The client ALREADY has its `ok`, so nothing
                        // below can regress the push. Egress is off the accept loop
                        // (a bounded queue + a worker thread); the only on-loop cost is
                        // the wall-clock-bounded touched-paths diff, which runs here
                        // AFTER the response — it delays at most the next accept, never
                        // this push, and can never wedge (bounded).
                        crate::merge_hook::emit_merge_event(
                            &repo_state.git_source,
                            repo,
                            &cmd.ref_name,
                            &cmd.old_oid,
                            &cmd.new_oid,
                            &req.principal_chain,
                        );
                        // WP-BC: the push moved the tips → the cached clone pack is now
                        // stale for the new refset. Rebuild it in the background (best-
                        // effort, off this worker's response path — the client already
                        // has its `ok`). Until it lands a full clone falls back to the
                        // slow walk (never a wrong pack: the serve side re-checks
                        // `refset_sha`). A no-op if this repo has no clone cache.
                        spawn_clone_pack_rebuild(state, repo);
                        // #70(a): the push moved HEAD → the per-path history index is now
                        // stale (a lookup HEAD-mismatch would just fall back to the live
                        // walk). Rebuild it for the new tip in the background (off this
                        // worker's response path — the client already has its `ok`).
                        crate::blob_history_index::spawn_blob_history_index_build(state, repo);
                    }
                    Err(crate::cas::CasPushError::Persist(reason)) => {
                        let report = build_report_status(
                            Err("log-persist-failed"),
                            &[RefOutcome::Ng {
                                ref_name: cmd.ref_name.clone(),
                                reason,
                            }],
                        );
                        send_report(request, report, io_budget)
                    }
                    // A flush (object upload) fault: nothing advertised, objects are
                    // idempotent — the client retries. A manifest fault: closure +
                    // event are durable, only the tip is not yet advertised.
                    Err(crate::cas::CasPushError::Flush(_)) => {
                        let report = build_report_status(
                            Ok(()),
                            &[RefOutcome::Ng {
                                ref_name: cmd.ref_name.clone(),
                                reason: "cas-upload-failed".into(),
                            }],
                        );
                        send_report(request, report, io_budget)
                    }
                    Err(crate::cas::CasPushError::Manifest(_)) => {
                        let report = build_report_status(
                            Ok(()),
                            &[RefOutcome::Ng {
                                ref_name: cmd.ref_name.clone(),
                                reason: "manifest-write-failed".into(),
                            }],
                        );
                        send_report(request, report, io_budget)
                    }
                }
            }
        },
        Err(e) => {
            // A rejected push (stale ref, tampered/unreachable target, oversized,
            // gate off, …) → HTTP 200 with a per-ref `ng <reason>` (git surfaces it).
            let report = build_report_status(
                Ok(()),
                &[RefOutcome::Ng {
                    ref_name: cmd.ref_name.clone(),
                    reason: receive_err_reason(&e),
                }],
            );
            send_report(request, report, io_budget)
        }
    }
}

/// The pure delete-ref pre-condition decision (the frozen design's §1.a + §1.b),
/// evaluated against the AUTHORITATIVE live `git_refs` snapshot `current_refs`:
///
/// * `Err("refuse-delete-default-branch")` — `ref_name` is the repo's default/HEAD
///   branch (the one [`pick_default_branch`] resolves HEAD to). Never nuke main.
/// * `Err("delete-of-absent-ref")` — the ref is not present (no fabricated success).
/// * `Err("non-fast-forward")` — the ref's current tip differs from the pusher's
///   `old_oid` (someone else moved/removed it; git sends the real current `old_oid`).
/// * `Ok(())` — the delete may proceed to the durable finalize.
fn delete_ref_decision(
    current_refs: &std::collections::BTreeMap<String, String>,
    ref_name: &str,
    old_oid: &str,
) -> Result<(), &'static str> {
    // The default-branch guard FIRST — a refusal to delete main is the strongest
    // safety, evaluated before the stale-check even reads the tip.
    if let Some(default_branch) = pick_default_branch(current_refs)
        && default_branch == ref_name
    {
        return Err("refuse-delete-default-branch");
    }
    // The stale-check against the authoritative snapshot.
    match current_refs.get(ref_name) {
        None => Err("delete-of-absent-ref"),
        Some(tip) if tip != old_oid => Err("non-fast-forward"),
        Some(_) => Ok(()),
    }
}

/// The DELETE-ref path of `handle_receive_pack` (`git push --delete <branch>`).
///
/// PRE-VALIDATED by the caller: deploy-gate ON, the pusher authenticated, a write
/// seam exists, and write-authz (OWNERSHIP) passed — a delete is a WRITE with the
/// SAME authz as any push (never the read predicate). This fn adds the delete-only
/// checks, then the durable removal:
///
/// 1. **default-branch guard** — REFUSE to delete the repo's default/HEAD branch (the
///    one the advertise resolves HEAD to via [`pick_default_branch`]) → a per-ref
///    `ng refuse-delete-default-branch`. Never let a push nuke main.
/// 2. **stale-check** against the AUTHORITATIVE live `git_refs` snapshot (NOT
///    `replay(log)` — the #2a discipline): the command's `old_oid` MUST equal the
///    current tip of `ref_name`. Absent ref → `ng delete-of-absent-ref`; tip differs
///    → `ng non-fast-forward` (someone else moved/removed it).
/// 3. **no pack / no objects / no reachability** — a delete carries no target, so the
///    proto `receive_pack` unpack path is NEVER entered.
/// 4. durable removal: [`crate::cas::finalize_cas_delete`] (refs.json rewrite +
///    `ref.delete` event), THEN [`RepoState::apply_cas_delete_inmemory`]. `ok` ONLY
///    after the durable removal (fail-closed).
#[allow(clippy::too_many_arguments)]
fn handle_delete_ref(
    state: &AppState,
    repo: &str,
    repo_state: &crate::state::RepoState,
    cmd: &crate::receive_wire::ReceiveCommand,
    principal: &[String],
    log: &mut hugit_refstore::log::EventLog,
    token: &crate::writes::CasToken,
    request: Request,
    io_budget: Duration,
) {
    use crate::receive_wire::{RefOutcome, build_report_status};
    use crate::state::RepoWriter;
    use crate::writes::LogSink;

    let ng = |request: Request, reason: &str| {
        let report = build_report_status(
            Ok(()),
            &[RefOutcome::Ng {
                ref_name: cmd.ref_name.clone(),
                reason: reason.to_string(),
            }],
        );
        send_report(request, report, io_budget);
    };

    // The AUTHORITATIVE current ref view — the SAME live `git_refs` projection the
    // advertise is built from, NOT `replay(log)` (a CAS-ingested branch has no
    // `ref.update` event on the log, so a log-derived view would mis-resolve).
    let current_refs = repo_state.git_refs.snapshot();

    // 1+2. The default-branch guard + the stale-check, as one pure decision (tested
    //       directly). A rejection → the matching per-ref `ng`, ref untouched.
    if let Err(reason) = delete_ref_decision(&current_refs, &cmd.ref_name, &cmd.old_oid) {
        return ng(request, reason);
    }

    // 3. No pack / no objects / no reachability — open the write seam ONLY to route the
    //    durable removal to the right backend. A delete never enters the unpack path.
    let writer = match repo_state.open_writer() {
        Ok(Some(w)) => w,
        Ok(None) => return respond_not_found(request, io_budget),
        Err(_) => return ng(request, "write-seam-unavailable"),
    };

    match writer {
        // GIT_DIR mode: append the `ref.delete` event (compare-and-swap persist) then
        // drop the on-disk ref so the upload-pack wire stops advertising it. Fail-closed.
        RepoWriter::GitDir { git_dir, .. } => {
            hugit_proto::write::store::record_ref_delete(
                log,
                principal.to_vec(),
                &cmd.ref_name,
                now_ms(),
            );
            if let Err(e) = state.persist(repo, log, token) {
                return ng(request, &format!("persist:{}", e.status));
            }
            if let Err(reason) = delete_git_ref(&git_dir, &cmd.ref_name, &cmd.old_oid) {
                return ng(request, &reason);
            }
            repo_state.apply_cas_delete_inmemory(&cmd.ref_name);
            let report = build_report_status(Ok(()), &[RefOutcome::Ok(cmd.ref_name.clone())]);
            send_report(request, report, io_budget);
            // POST-DURABLE, POST-`ok`: best-effort KungFu merge event (WP W-WEBHOOK).
            // A delete carries an all-zero `new_oid` → the event is a `ref-delete`
            // marker (no tree walk). Never regresses the push (`ok` already sent).
            crate::merge_hook::emit_merge_event(
                &repo_state.git_source,
                repo,
                &cmd.ref_name,
                &cmd.old_oid,
                &cmd.new_oid,
                principal,
            );
        }
        // CAS mode: log → manifest, each fail-closed (no objects to flush). `ok` ONLY
        // after the durable refs.json rewrite + the recorded `ref.delete` event.
        RepoWriter::Cas { seam, .. } => {
            let res = crate::cas::finalize_cas_delete(
                &seam.r2,
                &seam.tenant,
                &seam.repo_slug,
                &cmd.ref_name,
                // The deleter's `expected` tip — the SAME value the stale-check
                // (`delete_ref_decision`) validated against the live snapshot. Threaded so
                // the conditional refs.json write RE-VALIDATES it on the fresh base: a
                // concurrent same-ref UPDATE that landed after the stale-check fails closed
                // (StaleRef, the ref is NOT removed) rather than clobbering the update.
                &cmd.old_oid,
                || {
                    hugit_proto::write::store::record_ref_delete(
                        log,
                        principal.to_vec(),
                        &cmd.ref_name,
                        now_ms(),
                    );
                    state
                        .persist(repo, log, token)
                        .map_err(|e| format!("persist:{}", e.status))
                },
            );
            match res {
                Ok(()) => {
                    // The delete is DURABLE (event recorded + refs.json rewritten without
                    // the ref). Drop it from this engine's in-memory advertise so it stops
                    // serving with NO reboot. The oid-index is untouched (objects remain
                    // resolvable for any other ref).
                    repo_state.apply_cas_delete_inmemory(&cmd.ref_name);
                    let report =
                        build_report_status(Ok(()), &[RefOutcome::Ok(cmd.ref_name.clone())]);
                    send_report(request, report, io_budget);
                    // POST-DURABLE, POST-`ok`: best-effort KungFu merge event (WP
                    // W-WEBHOOK) — `ref-delete` marker; never regresses the push.
                    crate::merge_hook::emit_merge_event(
                        &repo_state.git_source,
                        repo,
                        &cmd.ref_name,
                        &cmd.old_oid,
                        &cmd.new_oid,
                        principal,
                    );
                    // WP-BC: the delete changed the refset → rebuild the cached clone
                    // pack in the background (best-effort; a stale cache would just
                    // fail the `refset_sha` re-check and slow-walk). No-op without a cache.
                    spawn_clone_pack_rebuild(state, repo);
                    // #70(a): the delete moved the ref set → rebuild the per-path history
                    // index for the new HEAD in the background (stale-index lookups fall
                    // back to the live walk until it lands). No-op for a refless repo.
                    crate::blob_history_index::spawn_blob_history_index_build(state, repo);
                }
                Err(crate::cas::CasPushError::Persist(reason)) => {
                    let report = build_report_status(
                        Err("log-persist-failed"),
                        &[RefOutcome::Ng {
                            ref_name: cmd.ref_name.clone(),
                            reason,
                        }],
                    );
                    send_report(request, report, io_budget)
                }
                // A manifest fault AFTER the event recorded: the removal event is durable
                // but refs.json still names the tip — `ng`, the client retries (idempotent).
                Err(crate::cas::CasPushError::Manifest(_)) => ng(request, "manifest-write-failed"),
                // A delete flushes NO objects, so the Flush variant is unreachable here;
                // map it to a generic fail-closed `ng` rather than fabricating success.
                Err(crate::cas::CasPushError::Flush(_)) => ng(request, "delete-rejected"),
            }
        }
    }
}

/// Delete the pushed ref from the git dir via `git update-ref -d`, compare-and-swap on
/// the expected old value (git refuses if the ref no longer holds `old_oid`). Returns a
/// short `ng` reason on failure.
fn delete_git_ref(git_dir: &std::path::Path, ref_name: &str, old_oid: &str) -> Result<(), String> {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(git_dir)
        .args(["update-ref", "-d", ref_name, old_oid])
        .output()
        .map_err(|e| format!("update-ref-spawn:{e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err("ref-delete-failed".to_string())
    }
}

/// Unix epoch milliseconds (the push receipt time). 0 on a clock error (the log
/// records it verbatim; a 0 is honest, never a fabricated time).
fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Releases a repo's clone-pack BUILD slot on drop — so the per-repo
/// `clone_pack_building` guard is cleared in EVERY exit path (success, error, AND a
/// panic in the build), never leaking the slot (which would wedge all future
/// rebuilds of that repo). Constructed BEFORE the thread spawn and moved INTO the
/// build closure, so even a spawn failure (the closure is dropped un-run) releases it.
struct BuildSlotGuard {
    set: Arc<std::sync::Mutex<std::collections::HashSet<String>>>,
    slug: String,
}

impl Drop for BuildSlotGuard {
    fn drop(&mut self) {
        self.set
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&self.slug);
    }
}

/// Spawn a DETACHED background thread that (re)assembles this repo's cached
/// full-clone pack (WP-BC) and flips the `current.json` pointer to it — so the next
/// anonymous full `git clone` streams ONE R2 object instead of walking the whole
/// object closure. Best-effort + fail-open: a repo with no clone cache (GIT_DIR mode
/// / receive-pack off) is a NO-OP, and ANY build/store error is logged and dropped (a
/// failed build = no cache = the slow-walk fallback; the serve side re-checks
/// `refset_sha`, so a partial/stale pack is never served).
///
/// CONCURRENCY: the per-repo `clone_pack_building` guard admits at most ONE build per
/// repo at a time (a boot bootstrap racing a post-push rebuild, or two pushes) — a
/// second call for a repo already building returns immediately. The slot is released
/// in every path via [`BuildSlotGuard`]. NEVER blocks the caller: the heavy
/// `build_clone_pack` closure walk + the R2 PUTs run on the detached thread, so this
/// is safe to call from the accept-loop bootstrap AND post-`ok` on a push worker.
fn spawn_clone_pack_rebuild(state: &AppState, repo_slug: &str) {
    let Some(repo_state) = state.repo_state(repo_slug) else {
        return;
    };
    let Some(seam) = repo_state.clone_cache.clone() else {
        return; // no cache for this repo → the clone slow-walks (no-op)
    };
    // Reserve the per-repo build slot; bail if a build for this repo is already running.
    {
        let mut building = state
            .clone_pack_building
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if !building.insert(repo_slug.to_string()) {
            return; // already building this repo — never a double build
        }
    }
    // The build inputs: the repo's object source (Arc) + the LIVE ref snapshot — the
    // SAME `git_refs` projection the advertise/serve uses, so the built pack's
    // `refset_sha` matches what a full clone recomputes. Owned/Arc handles → nothing
    // borrows `state`, so the thread is `'static`.
    let source: SharedSource = Arc::clone(&repo_state.git_source);
    let refs = repo_state.git_refs.snapshot();
    let slug = repo_slug.to_string();
    // Constructed HERE (before the spawn) so a spawn failure still releases the slot.
    let guard = BuildSlotGuard {
        set: Arc::clone(&state.clone_pack_building),
        slug: slug.clone(),
    };

    let spawned = std::thread::Builder::new()
        .name("hugit-clone-pack-build".into())
        .spawn(move || {
            // Move the guard in; it releases the slot at thread end OR on a panic
            // (unwind), so the per-repo slot can never leak.
            let _guard = guard;
            match clone_pack::build_clone_pack(source.as_ref(), &refs) {
                Ok((bytes, count)) => {
                    let sha = clone_pack::refset_sha(&refs);
                    if let Err(e) =
                        clone_pack::store_pack_and_flip(&seam, &sha, &bytes, count, now_ms())
                    {
                        eprintln!("hugit-serve: clone-pack rebuild for {slug} store failed: {e}");
                    }
                }
                Err(e) => {
                    eprintln!("hugit-serve: clone-pack rebuild for {slug} build failed: {e}");
                }
            }
        });
    if spawned.is_err() {
        // Thread exhaustion: the un-run closure (and its `guard`) was dropped, which
        // already released the slot — nothing more to do (no cache built this time).
        eprintln!("hugit-serve: clone-pack rebuild thread spawn failed for {repo_slug}");
    }
}

/// At engine start (WP-BC), ensure each CAS-backed repo with a clone cache has a
/// CURRENT cached full-clone pack. For each boot repo whose `current.json` pointer is
/// ABSENT or whose `refset_sha` != the live refs' `refset_sha`, kick a background
/// [`spawn_clone_pack_rebuild`]. Clones fall back to the slow walk until the first
/// build lands (~one background build per repo at boot — acceptable v0). Runs on the
/// serve thread BEFORE the accept loop; the only on-thread cost is one `load_current`
/// R2 GET per CAS repo (boot already does CAS reads). A repo with no cache is skipped.
pub(crate) fn bootstrap_clone_packs(state: &AppState) {
    // Only the boot repo set exists at serve start (the runtime overlay is empty).
    let slugs: Vec<String> = state.repos.keys().cloned().collect();
    for slug in slugs {
        let Some(rs) = state.repo_state(&slug) else {
            continue;
        };
        let Some(seam) = rs.clone_cache.as_ref() else {
            continue; // no cache → nothing to warm (the clone slow-walks)
        };
        let want_sha = clone_pack::refset_sha(&rs.git_refs.snapshot());
        // Absent/garbage pointer → `None` → treat as stale (build). A matching pointer
        // → already warm → skip. (A moved-ref pointer is stale → rebuild.)
        let fresh = clone_pack::load_current(&seam.r2, &seam.tenant, &seam.repo_slug)
            .is_some_and(|c| c.refset_sha == want_sha);
        if !fresh {
            eprintln!(
                "hugit-serve: clone-pack cache for {slug} absent/stale at boot — \
                 building in background"
            );
            spawn_clone_pack_rebuild(state, &slug);
        }
    }
}

/// Write the pushed ref into the git dir via `git update-ref`, compare-and-swap on
/// the expected old value (a create passes the all-zero oid, which git reads as
/// "must not exist"). Returns a short `ng` reason on failure.
fn update_git_ref(
    git_dir: &std::path::Path,
    ref_name: &str,
    new_oid: &str,
    old_oid: &str,
) -> Result<(), String> {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(git_dir)
        .args(["update-ref", ref_name, new_oid, old_oid])
        .output()
        .map_err(|e| format!("update-ref-spawn:{e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err("ref-update-failed".to_string())
    }
}

/// Map a `ReceiveError` to a short git `ng` reason token (no internal detail leak).
fn receive_err_reason(e: &hugit_proto::write::receive::ReceiveError) -> String {
    use hugit_proto::write::receive::ReceiveError as E;
    match e {
        E::WritePathDisabled => "push-disabled",
        E::MissingAttribution => "no-attribution",
        E::OversizedPack { .. } | E::DecompressionBomb { .. } => "pack-too-large",
        E::StaleRef { .. } => "non-fast-forward",
        E::RefUpdateTampered { .. } | E::UnreachableTarget { .. } => "ref-target-invalid",
        _ => "push-rejected",
    }
    .to_string()
}

/// Send a `report-status` body (HTTP 200, the git receive-pack result media type).
fn send_report(request: Request, body: Vec<u8>, io_budget: Duration) {
    let body_len = body.len();
    let resp = Response::from_data(body)
        .with_status_code(200)
        .with_header(git_content_type("application/x-git-receive-pack-result"));
    send(request, resp, body_len, io_budget);
}

/// 401 for a push with no/invalid bearer (a write must be authenticated).
fn respond_push_unauth(request: Request, io_budget: Duration) {
    let body = b"git push requires authentication\n".to_vec();
    let body_len = body.len();
    send(
        request,
        Response::from_data(body).with_status_code(401).with_header(
            tiny_http::Header::from_bytes(&b"Content-Type"[..], &b"text/plain"[..])
                .expect("static content-type"),
        ),
        body_len,
        io_budget,
    );
}

/// 400 for a malformed push body (bad pkt-line framing / command / missing pack).
fn respond_push_bad_request(request: Request, detail: &str, io_budget: Duration) {
    let body = format!("malformed receive-pack request: {detail}\n").into_bytes();
    let body_len = body.len();
    send(
        request,
        Response::from_data(body).with_status_code(400).with_header(
            tiny_http::Header::from_bytes(&b"Content-Type"[..], &b"text/plain"[..])
                .expect("static content-type"),
        ),
        body_len,
        io_budget,
    );
}

fn respond_push_forbidden(request: Request, io_budget: Duration) {
    let body = PUSH_FORBIDDEN_BODY.as_bytes().to_vec();
    let body_len = body.len();
    send(
        request,
        Response::from_data(body).with_status_code(403).with_header(
            tiny_http::Header::from_bytes(&b"Content-Type"[..], &b"text/plain"[..])
                .expect("static content-type"),
        ),
        body_len,
        io_budget,
    );
}

/// 403 for a WRITE attempted with a read-only credential (a `repo:read`-only PAT). A
/// distinct, clear message from the deploy-disabled 403 above — the caller CAN read,
/// the TOKEN just lacks `repo:write`.
fn respond_push_scope_forbidden(request: Request, io_budget: Duration) {
    let body = b"git push requires a token with repo:write scope\n".to_vec();
    let body_len = body.len();
    send(
        request,
        Response::from_data(body).with_status_code(403).with_header(
            tiny_http::Header::from_bytes(&b"Content-Type"[..], &b"text/plain"[..])
                .expect("static content-type"),
        ),
        body_len,
        io_budget,
    );
}

/// Respond 404 with no body — the uniform "git serving not live / not found"
/// answer (no existence oracle).
fn respond_not_found(request: Request, io_budget: Duration) {
    send(
        request,
        Response::from_data(Vec::new()).with_status_code(404),
        0,
        io_budget,
    );
}

/// Send a git response under the accept-loop's wall-clock I/O bound (FIX-SOCKET-TIMEOUT).
/// A small body (advertise, report-status, error, 404) is `<= RESPOND_INLINE_MAX`, so it
/// is written INLINE — byte-identical to a plain `request.respond` (it fits the kernel send
/// buffer and cannot block). A LARGE body (a clone/fetch PACK) is offloaded to a bounded
/// worker via the SAME [`crate::server::respond_bounded`] the `/v1` + SSE writes ride, so a
/// stalled / zero-window / slow-drain client cannot wedge the single-threaded accept loop
/// indefinitely — its connection is abandoned at the deadline, the loop freed. A broken
/// pipe (client hung up) is swallowed silently inside `respond_bounded`.
fn send<R: std::io::Read + Send + 'static>(
    request: Request,
    response: Response<R>,
    body_len: usize,
    io_budget: Duration,
) {
    crate::server::respond_bounded(request, response, body_len, io_budget, "git");
}

#[cfg(test)]
mod live_refs_tests {
    use super::*;
    use crate::cas::LiveOidIndex;
    use crate::state::{AppState, LiveRefs, RepoState};
    use hugit_refstore::EventLog;
    use std::collections::HashMap;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU64, Ordering};

    const TOKEN: &str = "test-dev-token";

    /// A unique scratch dir (no external tempfile dep — mirrors the integration tests).
    fn scratch_dir() -> std::path::PathBuf {
        static N: AtomicU64 = AtomicU64::new(0);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let p = std::env::temp_dir().join(format!(
            "hugit-serve-liverefs-{}-{}-{}",
            std::process::id(),
            nanos,
            N.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&p).expect("scratch dir");
        p
    }

    /// Seed a `<repo>.json` PUBLIC-meta event log on disk (so the anon clone-read
    /// authz gate `git_refs_for` applies passes) and return its dir.
    fn seed_public_log(repo: &str) -> std::path::PathBuf {
        let dir = scratch_dir();
        let mut log = EventLog::new();
        log.append_for_test(
            "repo.meta",
            vec![],
            serde_json::json!({"visibility": "public", "owner_tenant": "org-a"}).to_string(),
            0,
        );
        std::fs::write(
            dir.join(format!("{repo}.json")),
            serde_json::to_string_pretty(log.records()).unwrap(),
        )
        .unwrap();
        dir
    }

    /// A CAS-mode-shaped `RepoState`: a live ref map + a live oid-index handle. The
    /// object source is a trivial empty store (this test exercises the REF advertise
    /// path, which never reads objects — the object-source leg is proven in `cas.rs`).
    fn cas_mode_repo_state(refs: std::collections::BTreeMap<String, String>) -> RepoState {
        let src: Arc<dyn hugit_proto::ObjectSource + Send + Sync> =
            Arc::new(hugit_proto::CasObjectSource::new());
        RepoState {
            git_source: src,
            git_root_tree: gix_hash::ObjectId::from_hex(&[b'0'; 40]).unwrap(),
            git_refs: LiveRefs::new(refs),
            git_dir: None,
            cas_write: None,
            live_oid_index: Some(LiveOidIndex::new(std::collections::BTreeMap::new())),
            clone_cache: None,
        }
    }

    /// Frozen design §3: after `apply_cas_push_inmemory`, the advertise/`git_refs_for`
    /// view IMMEDIATELY shows the new tip — same process, no reload. This is the leg
    /// that makes a SECOND push see the correct base: the git client derives `old`
    /// from this advertise, so a fresh tip here ≡ the durably-reloaded log's tip,
    /// and the push is NOT falsely rejected as a non-fast-forward.
    #[test]
    fn advertise_reflects_pushed_tip_with_no_reboot() {
        let repo = "hugit";
        let old_tip = "a".repeat(40);
        let new_tip = "b".repeat(40);

        let dir = seed_public_log(repo);
        let mut state = AppState::new(dir, TOKEN.to_string());
        let mut refs = std::collections::BTreeMap::new();
        refs.insert("refs/heads/main".to_string(), old_tip.clone());
        state
            .repos
            .insert(repo.to_string(), cas_mode_repo_state(refs));

        // Before the push: the advertise shows the old tip.
        let before = git_refs_for(&state, repo, &[]).expect("public repo advertises");
        assert_eq!(before.get("refs/heads/main"), Some(&old_tip));

        // Simulate a successful CAS push finalize → in-memory hot-swap.
        state
            .repo_state(repo)
            .unwrap()
            .apply_cas_push_inmemory("refs/heads/main", &new_tip, &HashMap::new(), &[])
            .expect("hot-swap applies");

        // After the push: the SAME state (no reload) advertises the NEW tip.
        let after = git_refs_for(&state, repo, &[]).expect("still advertises");
        assert_eq!(
            after.get("refs/heads/main"),
            Some(&new_tip),
            "advertise reflects the just-pushed tip with no reboot"
        );
        // And the raw advertisement bytes carry the new oid, not the old one.
        let body = advertise_refs(&state, repo, &[]).expect("advertisement body");
        let text = String::from_utf8_lossy(&body);
        assert!(text.contains(&new_tip), "advertisement lists the new tip");
        assert!(
            !text.contains(&old_tip),
            "advertisement no longer lists the stale tip"
        );
    }

    /// A brand-new ref created by a push (a create) is advertised immediately too.
    #[test]
    fn advertise_includes_newly_created_ref_after_push() {
        let repo = "hugit";
        let main_tip = "a".repeat(40);
        let created_tip = "c".repeat(40);

        let dir = seed_public_log(repo);
        let mut state = AppState::new(dir, TOKEN.to_string());
        let mut refs = std::collections::BTreeMap::new();
        refs.insert("refs/heads/main".to_string(), main_tip.clone());
        state
            .repos
            .insert(repo.to_string(), cas_mode_repo_state(refs));

        state
            .repo_state(repo)
            .unwrap()
            .apply_cas_push_inmemory("refs/heads/feature", &created_tip, &HashMap::new(), &[])
            .expect("hot-swap applies");

        let after = git_refs_for(&state, repo, &[]).expect("advertises");
        assert_eq!(after.get("refs/heads/main"), Some(&main_tip), "main intact");
        assert_eq!(
            after.get("refs/heads/feature"),
            Some(&created_tip),
            "the just-created ref is advertised with no reboot"
        );
    }

    /// Fail-closed: a malformed pushed oid in the additions aborts the in-memory
    /// refresh WITHOUT advancing the ref — the prior tip keeps serving (a read never
    /// regresses; an unresolvable tip is never advertised).
    #[test]
    fn malformed_oid_addition_does_not_advance_ref() {
        let repo = "hugit";
        let old_tip = "a".repeat(40);
        let new_tip = "b".repeat(40);

        let dir = seed_public_log(repo);
        let mut state = AppState::new(dir, TOKEN.to_string());
        let mut refs = std::collections::BTreeMap::new();
        refs.insert("refs/heads/main".to_string(), old_tip.clone());
        state
            .repos
            .insert(repo.to_string(), cas_mode_repo_state(refs));

        let mut bad = HashMap::new();
        bad.insert("not-a-valid-oid".to_string(), "f".repeat(64));
        let err = state
            .repo_state(repo)
            .unwrap()
            .apply_cas_push_inmemory("refs/heads/main", &new_tip, &bad, &[])
            .expect_err("a malformed oid must abort the refresh");
        assert!(
            err.contains("not a valid git oid"),
            "names the fault: {err}"
        );

        let after = git_refs_for(&state, repo, &[]).expect("still advertises");
        assert_eq!(
            after.get("refs/heads/main"),
            Some(&old_tip),
            "the ref was NOT advanced (prior tip still served, fail-closed)"
        );
    }

    /// A CAS-mode `RepoState` whose live oid-index is SEEDED with the given base oids
    /// (the thin-pack re-assert tests need a non-empty read-path index to consult).
    fn cas_mode_repo_state_with_index(
        refs: std::collections::BTreeMap<String, String>,
        seed_bases: &[&str],
    ) -> RepoState {
        let src: Arc<dyn hugit_proto::ObjectSource + Send + Sync> =
            Arc::new(hugit_proto::CasObjectSource::new());
        let mut index = std::collections::BTreeMap::new();
        for hex in seed_bases {
            index.insert(
                gix_hash::ObjectId::from_hex(hex.as_bytes()).expect("valid base oid"),
                "f".repeat(64),
            );
        }
        RepoState {
            git_source: src,
            git_root_tree: gix_hash::ObjectId::from_hex(&[b'0'; 40]).unwrap(),
            git_refs: LiveRefs::new(refs),
            git_dir: None,
            cas_write: None,
            live_oid_index: Some(LiveOidIndex::new(index)),
            clone_cache: None,
        }
    }

    /// Defence-in-depth (#206 thin-pack follow-up): if a push resolved a delta against
    /// a base that is ABSENT from the read-path live oid-index, the hot-swap REFUSES to
    /// advance the ref — the prior tip keeps serving (the push stays durable; the
    /// correct tip loads on reboot). This guards a future HA / stale-index divergence
    /// from ever advertising a tip whose closure the read path can't resolve.
    #[test]
    fn hotswap_refuses_when_consumed_base_absent_from_live_index() {
        let repo = "hugit";
        let old_tip = "a".repeat(40);
        let new_tip = "b".repeat(40);
        // A consumed base oid that is NOT seeded into the live index.
        let absent_base = "d".repeat(40);

        let dir = seed_public_log(repo);
        let mut state = AppState::new(dir, TOKEN.to_string());
        let mut refs = std::collections::BTreeMap::new();
        refs.insert("refs/heads/main".to_string(), old_tip.clone());
        state.repos.insert(
            repo.to_string(),
            cas_mode_repo_state_with_index(refs, &[]), // empty live index
        );

        let err = state
            .repo_state(repo)
            .unwrap()
            .apply_cas_push_inmemory(
                "refs/heads/main",
                &new_tip,
                &HashMap::new(),
                std::slice::from_ref(&absent_base),
            )
            .expect_err("an unresolvable consumed base must abort the hot-swap");
        assert!(
            err.contains("absent from the live oid-index"),
            "names the fault: {err}"
        );

        let after = git_refs_for(&state, repo, &[]).expect("still advertises");
        assert_eq!(
            after.get("refs/heads/main"),
            Some(&old_tip),
            "the ref was NOT advanced (prior tip still serves, fail-closed)"
        );
    }

    /// The positive leg: a consumed thin-pack base that IS present in the live
    /// oid-index lets the hot-swap advance the tip (the re-assert passes).
    #[test]
    fn hotswap_advances_when_consumed_base_present_in_live_index() {
        let repo = "hugit";
        let old_tip = "a".repeat(40);
        let new_tip = "b".repeat(40);
        let present_base = "d".repeat(40);

        let dir = seed_public_log(repo);
        let mut state = AppState::new(dir, TOKEN.to_string());
        let mut refs = std::collections::BTreeMap::new();
        refs.insert("refs/heads/main".to_string(), old_tip.clone());
        state.repos.insert(
            repo.to_string(),
            cas_mode_repo_state_with_index(refs, &[&present_base]), // base seeded
        );

        state
            .repo_state(repo)
            .unwrap()
            .apply_cas_push_inmemory(
                "refs/heads/main",
                &new_tip,
                &HashMap::new(),
                std::slice::from_ref(&present_base),
            )
            .expect("a resolvable consumed base lets the hot-swap advance");

        let after = git_refs_for(&state, repo, &[]).expect("advertises");
        assert_eq!(
            after.get("refs/heads/main"),
            Some(&new_tip),
            "the tip advanced (the re-assert passed)"
        );
    }

    // ── delete-ref: the in-memory hot-swap + the pure pre-condition decision ───

    /// After `apply_cas_delete_inmemory`, the advertise IMMEDIATELY stops listing the
    /// deleted ref (same process, no reboot) — the other refs are untouched.
    #[test]
    fn advertise_drops_deleted_ref_with_no_reboot() {
        let repo = "hugit";
        let main_tip = "a".repeat(40);
        let stale_tip = "b".repeat(40);

        let dir = seed_public_log(repo);
        let mut state = AppState::new(dir, TOKEN.to_string());
        let mut refs = std::collections::BTreeMap::new();
        refs.insert("refs/heads/main".to_string(), main_tip.clone());
        refs.insert("refs/heads/stale".to_string(), stale_tip.clone());
        state
            .repos
            .insert(repo.to_string(), cas_mode_repo_state(refs));

        // Before: both refs advertised.
        let before = git_refs_for(&state, repo, &[]).expect("advertises");
        assert_eq!(before.get("refs/heads/stale"), Some(&stale_tip));

        state
            .repo_state(repo)
            .unwrap()
            .apply_cas_delete_inmemory("refs/heads/stale");

        // After: the deleted ref is GONE; main is intact.
        let after = git_refs_for(&state, repo, &[]).expect("still advertises");
        assert!(
            !after.contains_key("refs/heads/stale"),
            "the deleted ref drops out of the advertise with no reboot"
        );
        assert_eq!(after.get("refs/heads/main"), Some(&main_tip), "main intact");
    }

    /// §1.a — deleting the default/HEAD branch is REFUSED, ref untouched.
    #[test]
    fn delete_default_branch_refused() {
        let mut refs = std::collections::BTreeMap::new();
        refs.insert("refs/heads/main".to_string(), "a".repeat(40));
        refs.insert("refs/heads/feature".to_string(), "b".repeat(40));
        // `main` is the default (pick_default_branch prefers refs/heads/main).
        assert_eq!(
            delete_ref_decision(&refs, "refs/heads/main", &"a".repeat(40)),
            Err("refuse-delete-default-branch")
        );
        // The guard fires even with a matching old_oid — it precedes the stale-check.
        // A non-default branch with the right old_oid is permitted.
        assert_eq!(
            delete_ref_decision(&refs, "refs/heads/feature", &"b".repeat(40)),
            Ok(())
        );
    }

    /// §1.a — when there is no `main`/`master`, the default is the first `refs/heads/*`
    /// (the same pick the advertise uses for HEAD), and IT is refused.
    #[test]
    fn delete_default_branch_refused_when_first_branch_is_head() {
        let mut refs = std::collections::BTreeMap::new();
        refs.insert("refs/heads/alpha".to_string(), "a".repeat(40));
        refs.insert("refs/heads/beta".to_string(), "b".repeat(40));
        // pick_default_branch → first refs/heads/* alphabetically == alpha.
        assert_eq!(
            delete_ref_decision(&refs, "refs/heads/alpha", &"a".repeat(40)),
            Err("refuse-delete-default-branch")
        );
        assert_eq!(
            delete_ref_decision(&refs, "refs/heads/beta", &"b".repeat(40)),
            Ok(())
        );
    }

    /// §1.b — a delete whose `old_oid` ≠ the current tip is a non-fast-forward, refused.
    #[test]
    fn delete_with_stale_old_oid_rejected() {
        let mut refs = std::collections::BTreeMap::new();
        refs.insert("refs/heads/main".to_string(), "a".repeat(40));
        refs.insert("refs/heads/stale".to_string(), "b".repeat(40));
        assert_eq!(
            delete_ref_decision(&refs, "refs/heads/stale", &"c".repeat(40)),
            Err("non-fast-forward")
        );
        // The exact current tip is accepted.
        assert_eq!(
            delete_ref_decision(&refs, "refs/heads/stale", &"b".repeat(40)),
            Ok(())
        );
    }

    /// WP-B5 INVARIANT LOCK (clw-requested): the read-after-write refresh keeps the
    /// staleness UX-only, NEVER a lost update. After the background refresh installs another
    /// instance's advance (`feat` v1→v2 — modeled by `LiveRefs::replace`, the refresh's core
    /// action), the stale-check reads the LIVE `git_refs` the refresher just updated, so a
    /// push still carrying the STALE base (`v1`) is rejected `non-fast-forward` (the client
    /// re-fetches + retries). This is the whole reason a bounded-staleness advertise is safe:
    /// every durable ref mutation rides the stale-check + the conditional If-Match PUT.
    #[test]
    fn b5_refresh_then_stale_base_push_is_rejected_non_fast_forward() {
        let live = crate::state::LiveRefs::new(std::collections::BTreeMap::from([
            ("refs/heads/main".to_string(), "m".repeat(40)),
            ("refs/heads/feat".to_string(), "1".repeat(40)),
        ]));
        // The background refresh installs the durable manifest — another instance advanced
        // `feat` to v2.
        live.replace(std::collections::BTreeMap::from([
            ("refs/heads/main".to_string(), "m".repeat(40)),
            ("refs/heads/feat".to_string(), "2".repeat(40)),
        ]));
        // A push on the now-stale base `v1` is rejected against the refreshed live view.
        assert_eq!(
            delete_ref_decision(&live.snapshot(), "refs/heads/feat", &"1".repeat(40)),
            Err("non-fast-forward"),
            "a stale-base push is rejected post-refresh — staleness stays UX-only"
        );
        // The refreshed tip `v2` is accepted.
        assert_eq!(
            delete_ref_decision(&live.snapshot(), "refs/heads/feat", &"2".repeat(40)),
            Ok(())
        );
    }

    /// §1.b — deleting a ref that does not exist is refused (no fabricated success).
    #[test]
    fn delete_absent_branch_rejected() {
        let mut refs = std::collections::BTreeMap::new();
        refs.insert("refs/heads/main".to_string(), "a".repeat(40));
        assert_eq!(
            delete_ref_decision(&refs, "refs/heads/ghost", &"a".repeat(40)),
            Err("delete-of-absent-ref")
        );
    }

    #[test]
    fn receive_advert_advertises_delete_refs_so_the_client_sends_deletes() {
        // Per the git protocol, a client refuses to send a zero-id (delete)
        // command unless the server advertised `delete-refs` — without it
        // `git push --delete` ships an EMPTY command list and the delete never
        // reaches the (working) server handler. This regression-guards that the
        // capability stays advertised.
        assert!(
            RECEIVE_CAPS.split(' ').any(|c| c == "delete-refs"),
            "receive-pack advert must offer delete-refs; got: {RECEIVE_CAPS}"
        );
        // report-status must also stay (the client reads our ok/ng report).
        assert!(RECEIVE_CAPS.split(' ').any(|c| c == "report-status"));
    }
}

#[cfg(test)]
mod want_validation_tests {
    use super::*;

    fn oid(hex_char: char) -> gix_hash::ObjectId {
        gix_hash::ObjectId::from_hex(hex_char.to_string().repeat(40).as_bytes())
            .expect("valid 40-hex oid")
    }

    /// The clone-serve routing (WP clone-pack legibility): a full clone of a repo WITH a
    /// cache seam takes the cache path (→ cached pack on hit, retryable 503 on miss,
    /// NEVER the single-thread-DoS slow walk). A seam-less repo OR any non-full-clone
    /// (a bounded fetch) takes the slow walk.
    #[test]
    fn full_clone_with_seam_takes_the_cache_path_else_slow_walk() {
        assert!(
            takes_clone_cache_path(true, true),
            "full clone + cache seam → cache path (503 on a build-window miss, not a DoS walk)"
        );
        assert!(
            !takes_clone_cache_path(true, false),
            "full clone but NO seam (GIT_DIR/small) → slow walk"
        );
        assert!(
            !takes_clone_cache_path(false, true),
            "a fetch (not a full clone) → slow walk even with a seam (cache holds the full pack only)"
        );
        assert!(
            !takes_clone_cache_path(false, false),
            "fetch, no seam → slow walk"
        );
    }

    /// Only an advertised ref tip may be `want`ed (allowReachableSHA1InWant-off):
    /// an advertised tip passes; a non-advertised (interior/arbitrary) oid is
    /// rejected; a mixed list is rejected if ANY want is non-advertised.
    #[test]
    fn wants_must_be_advertised_tips() {
        let mut refs = BTreeMap::new();
        let tip = "a".repeat(40);
        refs.insert("refs/heads/main".to_string(), tip.clone());
        refs.insert("refs/heads/feature".to_string(), "b".repeat(40));

        let advertised_main = oid('a');
        let advertised_feature = oid('b');
        let interior = oid('c'); // reachable-but-not-a-tip / arbitrary oid

        // Advertised tips → accepted.
        assert!(wants_all_advertised(&refs, &[advertised_main]));
        assert!(wants_all_advertised(
            &refs,
            &[advertised_main, advertised_feature]
        ));

        // A non-advertised want → rejected.
        assert!(!wants_all_advertised(&refs, &[interior]));
        // ANY non-advertised want in a mixed list → the whole request is rejected.
        assert!(!wants_all_advertised(&refs, &[advertised_main, interior]));

        // Empty wants → vacuously ok (the full-clone path derives wants from refs
        // and never reaches this check).
        assert!(wants_all_advertised(&refs, &[]));
    }
}

/// The off-loop clone contract (FIX-CLONE-OFFLOOP, worker-responds shape): a real
/// (multi-object) clone ASSEMBLES byte-complete; a failing source is fail-closed to
/// no-pack (never truncated); and, crucially, the hand-off FREES the caller
/// IMMEDIATELY — a slow clone runs out-of-band on the detached worker while the
/// accept-loop side returns at once, so `/readyz` stays answerable. That last is the
/// property the earlier blocking (`run_bounded`) shape did NOT have.
#[cfg(test)]
mod offloop_tests {
    use super::*;
    use hugit_proto::{CasObjectSource, GitObject, ObjectKind, WantHave};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::{Duration, Instant};

    fn blob(src: &mut CasObjectSource, body: &str) -> gix_hash::ObjectId {
        src.insert(GitObject::new(ObjectKind::Blob, body.as_bytes().to_vec()))
    }
    fn tree(src: &mut CasObjectSource, name: &str, oid: gix_hash::ObjectId) -> gix_hash::ObjectId {
        let mut out = Vec::new();
        out.extend_from_slice(b"100644");
        out.push(b' ');
        out.extend_from_slice(name.as_bytes());
        out.push(0);
        out.extend_from_slice(oid.as_bytes());
        src.insert(GitObject::new(ObjectKind::Tree, out))
    }
    fn commit(src: &mut CasObjectSource, tree_oid: gix_hash::ObjectId) -> gix_hash::ObjectId {
        let body =
            format!("tree {tree_oid}\nauthor a <a@a> 0 +0000\ncommitter a <a@a> 0 +0000\n\nmsg\n");
        src.insert(GitObject::new(ObjectKind::Commit, body.into_bytes()))
    }

    /// A trivial commit→tree→blob graph plus the commit tip.
    fn sample() -> (CasObjectSource, gix_hash::ObjectId) {
        let mut src = CasObjectSource::new();
        let b = blob(&mut src, "hello\n");
        let t = tree(&mut src, "f.txt", b);
        let c = commit(&mut src, t);
        (src, c)
    }

    /// The multi-round stateless shallow wire (the bug the live clone caught, that the
    /// single-round hermetic tests missed): ROUND 1 (`deepen`, no `done`) must reply with
    /// the `shallow` section ONLY (no `NAK`, no pack); ROUND 2 (client `shallow` lines +
    /// `done`) must reply with the `shallow` section AGAIN, then `NAK`, then the pack.
    #[test]
    fn shallow_round1_is_negotiation_only_round2_carries_the_pack() {
        // A 2-commit chain: `root` ← `tip`. Depth 1 cuts `root`, so `tip` IS shallow
        // (a single root commit is never shallow — nothing to cut).
        let (mut src, root) = sample();
        let b2 = blob(&mut src, "world\n");
        let t2 = tree(&mut src, "g.txt", b2);
        let tip_body = format!(
            "tree {t2}\nparent {root}\nauthor a <a@a> 0 +0000\ncommitter a <a@a> 0 +0000\n\nmsg\n"
        );
        let tip = src.insert(GitObject::new(ObjectKind::Commit, tip_body.into_bytes()));
        let refs = std::collections::BTreeMap::new();

        // ROUND 1: `deepen 1`, NO done → shallow boundary + flush, and NOTHING else.
        let plan1 = ClonePlan {
            full_clone: false,
            refs: refs.clone(),
            shallow_depth: Some(1),
            shallow_client: Vec::new(),
            done: false,
        };
        let want1 = WantHave {
            wants: vec![tip],
            haves: Vec::new(),
            done: false,
        };
        let out1 = build_shallow_pack_bytes(&src, &want1, &plan1).expect("round 1 builds");
        let s1 = String::from_utf8_lossy(&out1);
        assert!(
            s1.contains(&format!("shallow {tip}")),
            "round 1 sends the shallow boundary: {s1:?}"
        );
        assert!(
            !s1.contains("NAK"),
            "round 1 is negotiation-only — NO NAK: {s1:?}"
        );
        assert!(
            !out1.windows(4).any(|w| w == b"PACK"),
            "round 1 sends NO pack (it rides round 2)"
        );

        // ROUND 2: client re-sends `shallow <tip>` + `done`, no `deepen` → shallow
        // boundary + flush + NAK + the pack cut at that boundary.
        let plan2 = ClonePlan {
            full_clone: false,
            refs,
            shallow_depth: None,
            shallow_client: vec![tip],
            done: true,
        };
        let out2 = build_shallow_pack_bytes(&src, &want(tip), &plan2).expect("round 2 builds");
        let s2 = String::from_utf8_lossy(&out2);
        assert!(
            s2.contains(&format!("shallow {tip}")),
            "round 2 RE-sends the shallow boundary (or git dies `expected shallow list`)"
        );
        assert!(s2.contains("NAK"), "round 2 sends NAK before the pack");
        assert!(
            out2.windows(4).any(|w| w == b"PACK"),
            "round 2 sends the depth-bounded pack"
        );
    }

    fn want(tip: gix_hash::ObjectId) -> WantHave {
        WantHave {
            wants: vec![tip],
            haves: Vec::new(),
            done: true,
        }
    }

    /// A real (multi-object) clone ASSEMBLES byte-complete: the v1 `NAK` pkt-line +
    /// a `PACK` stream carrying the full commit→tree→blob closure. This is the heavy
    /// step the worker runs; it has NO tight outer cutoff (the loop never waits), so
    /// a real ~50 s clone that used to 404 at the old 45 s budget now completes (only
    /// the generous 300 s proto runaway-bound applies).
    #[test]
    fn multiobject_clone_assembles_byte_complete() {
        let (src, c) = sample();
        let out = build_upload_pack_bytes(&src, &want(c))
            .expect("a multi-object clone must assemble a complete pack");
        assert!(
            out.starts_with(b"0008NAK\n"),
            "the v1 upload-pack result must open with the NAK pkt-line"
        );
        assert!(
            out.windows(4).any(|w| w == b"PACK"),
            "the framed result must carry a real PACK stream (no truncation)"
        );
    }

    /// A `want` for an object the source does not hold → `serve_fetch` errors →
    /// `None` — fail-CLOSED to no-pack (→ 404), NEVER a truncated/partial pack.
    #[test]
    fn missing_closure_is_fail_closed_no_pack() {
        let (src, _c) = sample();
        let bogus = gix_hash::ObjectId::from_hex("a".repeat(40).as_bytes()).unwrap();
        assert!(
            build_upload_pack_bytes(&src, &want(bogus)).is_none(),
            "an unresolvable want must fail closed (no pack), never a truncated one"
        );
    }

    /// THE availability property: the hand-off FREES the caller IMMEDIATELY while the
    /// job runs out-of-band on the detached worker. We spawn a job that SLEEPS ~400 ms
    /// before flipping a flag; `spawn_with_payload` must return in well under that
    /// (the caller is not blocked on the job), and the flag must still be UNSET right
    /// after it returns (the job is genuinely still running elsewhere). This is the
    /// exact accept-loop-not-wedged guarantee the clone POST now has — a slow clone
    /// no longer blocks the loop for its duration.
    #[test]
    fn handoff_frees_the_caller_immediately_job_runs_out_of_band() {
        let done = Arc::new(AtomicBool::new(false));
        let done_worker = Arc::clone(&done);

        let started = Instant::now();
        let res = spawn_with_payload(done_worker, |flag| {
            std::thread::sleep(Duration::from_millis(400));
            flag.store(true, Ordering::SeqCst);
        });
        let handoff = started.elapsed();

        assert!(res.is_ok(), "the worker must spawn");
        assert!(
            handoff < Duration::from_millis(100),
            "the caller must be FREED at hand-off, not blocked for the ~400ms job \
             (hand-off took {handoff:?})"
        );
        assert!(
            !done.load(Ordering::SeqCst),
            "the job must still be running out-of-band on the worker right after \
             hand-off — the caller did NOT wait for it"
        );

        // Sanity: the worker DOES complete the job out-of-band (the response is not
        // silently dropped). Poll rather than a fixed sleep to avoid flake.
        let deadline = Instant::now() + Duration::from_secs(5);
        while !done.load(Ordering::SeqCst) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(
            done.load(Ordering::SeqCst),
            "the detached worker must complete the job out-of-band"
        );
    }

    /// THE receive-pack (push) availability property (WP W-RECEIVE-OFFLOOP): the push
    /// unpack + durable finalize is handed to a DETACHED worker via the SAME
    /// `spawn_with_payload` primitive `spawn_receive_pack_worker` uses, so a SLOW unpack
    /// (here a ~400 ms fake) runs out-of-band while the accept-loop side returns at once
    /// — `/readyz` + other requests stay answerable during a large push. This is the
    /// exact accept-loop-not-wedged guarantee the receive-pack POST now has (previously
    /// the gix-pack unpack ran INLINE on the single accept thread, wedging it for the
    /// push's whole duration). We can't build a `tiny_http::Request` in a unit test, so
    /// we exercise the hand-off mechanism directly (the real path moves a
    /// `(ReceivePlan, Request, io_budget)` through it identically).
    #[test]
    fn receive_pack_handoff_frees_loop_during_slow_unpack() {
        let unpacked = Arc::new(AtomicBool::new(false));
        let unpacked_worker = Arc::clone(&unpacked);

        let started = Instant::now();
        // Stand-in for `serve_receive_pack_response`: a slow unpack + finalize.
        let res = spawn_with_payload(unpacked_worker, |flag| {
            std::thread::sleep(Duration::from_millis(400)); // the "unpack"
            flag.store(true, Ordering::SeqCst);
        });
        let handoff = started.elapsed();

        assert!(res.is_ok(), "the receive-pack worker must spawn");
        assert!(
            handoff < Duration::from_millis(100),
            "the accept loop must be FREED at hand-off, not blocked for the ~400ms \
             unpack (hand-off took {handoff:?})"
        );
        assert!(
            !unpacked.load(Ordering::SeqCst),
            "the unpack must still be running out-of-band on the worker right after \
             hand-off — the accept loop did NOT wait for it (so /readyz stays answerable)"
        );

        // The worker DOES complete the unpack out-of-band (the response is not dropped).
        let deadline = Instant::now() + Duration::from_secs(5);
        while !unpacked.load(Ordering::SeqCst) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(
            unpacked.load(Ordering::SeqCst),
            "the detached worker must complete the unpack out-of-band"
        );
    }

    /// The payload (an owned value the real path uses to carry the `Request`) is
    /// delivered INTACT to the worker across the hand-off channel — proving the
    /// response is served from the moved-in connection, not lost.
    #[test]
    fn handoff_delivers_the_payload_to_the_worker() {
        let seen = Arc::new(std::sync::Mutex::new(None::<u64>));
        let seen_worker = Arc::clone(&seen);
        spawn_with_payload(4242u64, move |v| {
            *seen_worker.lock().unwrap() = Some(v);
        })
        .expect("spawn");
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(v) = *seen.lock().unwrap() {
                assert_eq!(v, 4242, "the worker must receive the exact payload");
                break;
            }
            assert!(
                Instant::now() < deadline,
                "the worker never received the payload"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

#[cfg(test)]
mod pack_cap_tests {
    use super::*;

    #[test]
    fn clamp_max_pack_bytes_defaults_and_clamps() {
        // Unset / unparseable → the 64 MiB default.
        assert_eq!(clamp_max_pack_bytes(None), DEFAULT_MAX_PACK_BYTES);
        // A value inside the window is preserved.
        assert_eq!(
            clamp_max_pack_bytes(Some(100 * 1024 * 1024)),
            100 * 1024 * 1024
        );
        // Below the 1 MiB floor → clamped up.
        assert_eq!(clamp_max_pack_bytes(Some(0)), MIN_MAX_PACK_BYTES);
        assert_eq!(clamp_max_pack_bytes(Some(1)), MIN_MAX_PACK_BYTES);
        // Above the 512 MiB ceiling → clamped down (an un-bounded read is refused).
        assert_eq!(
            clamp_max_pack_bytes(Some(4 * 1024 * 1024 * 1024)),
            MAX_MAX_PACK_BYTES
        );
    }

    #[test]
    fn is_receive_pack_path_matches_only_the_push_post() {
        assert!(is_receive_pack_path("/acme/git-receive-pack"));
        assert!(is_receive_pack_path("/acme/git-receive-pack?foo=bar"));
        // The clone/fetch routes keep the 8 MiB JSON door (NOT the pack cap).
        assert!(!is_receive_pack_path("/acme/git-upload-pack"));
        assert!(!is_receive_pack_path("/acme/info/refs"));
        assert!(!is_receive_pack_path("/v1/repos/acme/blob"));
        assert!(!is_receive_pack_path("/acme/git-receive-pack/extra"));
    }
}
