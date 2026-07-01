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

use tiny_http::{Header, Method, Request, Response};

use crate::state::AppState;

/// The `git-upload-pack` service name (clone + fetch). `git-receive-pack` (push)
/// is intentionally NOT handled here (out of scope).
const UPLOAD_PACK: &str = "git-upload-pack";

/// The v1 capabilities we advertise. Deliberately minimal + side-band-FREE: the
/// proto serves a bare packfile (no side-band-64k multiplexing), so advertising
/// side-band would make the client await framing that never arrives. `agent` is
/// informational; `object-format=sha1` matches the proto's hash.
const ADVERTISED_CAPS: &str = "object-format=sha1 agent=hugit-serve";

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

/// Handle a git smart-HTTP request inline (binary-safe), consuming `request` in
/// every branch. Mirrors `respond_sse`: it owns the whole response because a
/// packfile is a `Vec<u8>` body with a git-specific Content-Type. `body` is the
/// already-read (capped) request body — empty for the GET advertisement, the
/// want/have pkt-lines for the upload-pack POST.
pub fn respond_git(state: &AppState, method: &Method, url: &str, body: &[u8], request: Request) {
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
                return handle_receive_advertise(state, repo, request);
            }
            if svc != Some(UPLOAD_PACK) {
                return respond_not_found(request);
            }
            // Derive the clone principal from an OPTIONAL Bearer (anonymous on
            // absent/invalid/expired — fail-closed), then gate on authorize_read.
            // SAME derivation the upload-pack POST runs (no split-route bypass).
            let principal = clone_principal(state, request.headers());
            match advertise_refs(state, repo, &principal) {
                Some(body) => {
                    let resp = Response::from_data(body).with_status_code(200).with_header(
                        git_content_type("application/x-git-upload-pack-advertisement"),
                    );
                    send(request, resp);
                }
                None => respond_not_found(request),
            }
        }
        (Method::Post, [repo, "git-upload-pack"]) => {
            // IDENTICAL principal derivation + authz to the advertise above: the
            // POST must 404 for every case the advertise hides (no split-route
            // bypass where a pack is served for a repo the advertise concealed).
            let principal = clone_principal(state, request.headers());
            match upload_pack(state, repo, body, &principal) {
                Some(out) => {
                    let resp = Response::from_data(out)
                        .with_status_code(200)
                        .with_header(git_content_type("application/x-git-upload-pack-result"));
                    send(request, resp);
                }
                None => respond_not_found(request),
            }
        }
        // POST git-receive-pack (push) → the write path (gated). Other methods on
        // the receive-pack route → the clear 403 (not a silent 404).
        (Method::Post, [repo, "git-receive-pack"]) => {
            handle_receive_pack(state, repo, body, request)
        }
        (_, [_repo, "git-receive-pack"]) => respond_push_forbidden(request),
        // Any other method/shape on a git-looking path → 404, no oracle.
        _ => respond_not_found(request),
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
    // Extract `Authorization: Bearer <token>` (case-insensitive header name); absent
    // → anonymous. Mirrors `two_tier_auth`'s extraction exactly.
    let raw = headers
        .iter()
        .find(|h| {
            h.field
                .as_str()
                .as_str()
                .eq_ignore_ascii_case("Authorization")
        })
        .and_then(|h| h.value.as_str().strip_prefix("Bearer ").map(str::to_string));
    let Some(raw) = raw else {
        return Vec::new(); // no Bearer → anonymous (public-clone gate)
    };

    // Tier-1 ONLY: the engine-token store (a Clerk session exchange minted it). A
    // valid, unexpired record → the real tenant principal. Anything else (Invalid /
    // Expired / lock fault) → anonymous, NOT an error and NOT the operator path.
    match state.token_store.lookup(&raw) {
        crate::token::LookupResult::Ok(rec) => {
            vec![format!("clerk:{}:{}", rec.org, rec.user)]
        }
        _ => Vec::new(), // invalid/expired token → fail-closed to anonymous (public only)
    }
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

/// Handle a `git-upload-pack` POST: parse the want/have negotiation, assemble the
/// pack, and frame the v1 result (`NAK` + raw packfile). `None` (→ 404) on the
/// same not-live / unsafe / not-public / malformed conditions as the advertisement.
fn upload_pack(state: &AppState, repo: &str, body: &[u8], principal: &[String]) -> Option<Vec<u8>> {
    let refs = git_refs_for(state, repo, principal)?;
    let source = &state.repo_state(repo)?.git_source;

    // Real git appends a capability list to the FIRST `want` line of a v1/v0
    // upload-pack request (`want <oid> multi_ack side-band-64k …`). The proto's
    // `WantHave::parse` treats everything after `want ` as the oid and rejects the
    // trailing caps as a bad oid — so we re-frame the body first, trimming each
    // `want`/`have` line to its leading oid token. The result is byte-clean
    // pkt-lines the proto parses faithfully.
    let cleaned = strip_want_have_caps(body);
    let request = hugit_proto::WantHave::parse(&cleaned).ok()?;

    // A clone sends wants but no haves; a fetch sends both. When the client sent
    // no wants at all (e.g. an ls-refs-only probe in a v2 attempt, or an empty
    // body) treat it as "want every advertised tip" — a full clone — using the
    // SAME ref view the advertisement was built from, so wants always resolve in
    // the CAS.
    let pack = if request.wants.is_empty() {
        let adv = hugit_proto::RefAdvertisement::from_view(&refs);
        let clone_req = hugit_proto::WantHave::clone_all(&adv).ok()?;
        // The full-clone wants are DERIVED from `refs` (every advertised tip), so
        // they are advertised by construction — no validation needed.
        hugit_proto::serve_fetch(source.as_ref(), &clone_req).ok()?
    } else {
        // SECURITY (want-validation): every explicit `want` MUST be an advertised
        // ref tip for THIS principal (the same `refs` the advertise exposed). A
        // want for a non-advertised oid — an arbitrary interior/unreachable object,
        // or one hidden by the read-authz gate — is REJECTED (→ 404, no oracle):
        // the `uploadpack.allowReachableSHA1InWant`-off default. This closes a
        // client fishing for an unadvertised object AND caps the reachability walk
        // to a real tip's closure (defence-in-depth for the serve_fetch DoS: a
        // caller cannot force a walk from an attacker-chosen root).
        if !wants_all_advertised(&refs, &request.wants) {
            return None;
        }
        hugit_proto::serve_fetch(source.as_ref(), &request).ok()?
    };

    // Smart-HTTP v1 upload-pack result: the NAK pkt-line (we run no multi-ack
    // negotiation — single round, "done"), then the raw packfile bytes.
    let mut out = Vec::new();
    pkt_line(&mut out, b"NAK\n");
    out.extend_from_slice(&pack.bytes);
    Some(out)
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
    let repo_state = state.repo_state(repo)?;
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
fn handle_receive_advertise(state: &AppState, repo: &str, request: Request) {
    if !state.write_path_enabled {
        return respond_push_forbidden(request);
    }
    let headers: Vec<tiny_http::Header> = request.headers().to_vec();
    let principal = match crate::server::two_tier_auth(state, &headers) {
        Ok((p, _)) => p,
        Err(_) => return respond_push_unauth(request),
    };
    if !crate::state::is_safe_repo_slug(repo) {
        return respond_not_found(request);
    }
    let Some(repo_state) = state.repo_state(repo) else {
        return respond_not_found(request);
    };
    if !repo_state.has_write_seam() {
        return respond_not_found(request); // no write seam (GIT_DIR or CAS) → 404
    }
    let Ok(log) = state.load_verified(repo) else {
        return respond_not_found(request);
    };
    let meta = crate::authz::project_repo_meta(&log);
    if !crate::authz::authorize_write(&principal, &meta) {
        return respond_not_found(request); // not the owner → 404, no oracle
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
    let resp = Response::from_data(out)
        .with_status_code(200)
        .with_header(git_content_type(
            "application/x-git-receive-pack-advertisement",
        ));
    send(request, resp);
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
fn handle_receive_pack(state: &AppState, repo: &str, body: &[u8], request: Request) {
    use crate::receive_wire::{RefOutcome, build_report_status, parse_receive_pack_body};
    use crate::state::RepoWriter;
    use crate::writes::LogSink; // brings AppState::persist (the compare-and-swap) into scope
    use hugit_proto::write::receive::{ReceiveRequest, RecvLimits, RefUpdate, receive_pack};
    use hugit_proto::write::store::Cas; // the &mut dyn Cas sink coercion

    // The deploy gate FIRST: receive-pack is OFF unless `HUGIT_SERVE_RECEIVE_PACK=1`.
    // Off → the honest 403 ("push not supported here"), BEFORE auth, so a stock
    // deploy answers the same clear message to any client (authed or not).
    if !state.write_path_enabled {
        return respond_push_forbidden(request);
    }

    // Authenticate — a push is a WRITE, so it MUST carry a valid bearer (unlike an
    // anonymous clone). An invalid/absent token → 401.
    let headers: Vec<tiny_http::Header> = request.headers().to_vec();
    let principal = match crate::server::two_tier_auth(state, &headers) {
        Ok((p, _)) => p,
        Err(_) => return respond_push_unauth(request),
    };

    // A write seam is required (GIT_DIR mode). No seam (CAS mode / unknown repo) →
    // 404, no oracle. Everything past here is gated behind a successful authz.
    if !crate::state::is_safe_repo_slug(repo) {
        return respond_not_found(request);
    }
    let Some(repo_state) = state.repo_state(repo) else {
        return respond_not_found(request);
    };
    // A write seam is required (GIT_DIR or CAS mode). None → 404, no oracle. The
    // sink itself is OPENED below (after authz + parse), so an open fault is a
    // per-ref ng rather than a pre-auth 404.
    if !repo_state.has_write_seam() {
        return respond_not_found(request);
    }

    // Load the repo's log (the authz meta source + the append target), pinned to the
    // head we compare-and-swap against.
    let (mut log, token) = match state.load_verified_with_token(repo) {
        Ok(lt) => lt,
        Err(_) => return respond_not_found(request),
    };

    // WRITE-authz: OWNERSHIP, never the read-visibility predicate (a public repo
    // opens reads, NEVER writes). Denial → 404 (no oracle).
    let meta = crate::authz::project_repo_meta(&log);
    if !crate::authz::authorize_write(&principal, &meta) {
        return respond_not_found(request);
    }

    // Parse the wire. A framing/command error → 400 (a malformed push).
    let wire = match parse_receive_pack_body(body) {
        Ok(w) => w,
        Err(e) => return respond_push_bad_request(request, &e.to_string()),
    };
    if let Err(e) = wire.require_pack() {
        return respond_push_bad_request(request, &e.to_string());
    }
    if wire.commands.len() != 1 {
        return respond_push_bad_request(request, "v0 accepts exactly one ref update per push");
    }
    let cmd = wire.commands[0].clone();
    if cmd.is_delete() {
        // DELETE-ref path. A delete is a WRITE — it has already cleared the same
        // deploy-gate + auth + write-authz (ownership) gates above as any push (do NOT
        // weaken that). It carries NO pack / no target / no reachability, so it never
        // touches the proto `receive_pack` unpack path. Below: the default-branch guard
        // + the stale-check against the AUTHORITATIVE live `git_refs` snapshot, then the
        // durable finalize (refs.json rewrite + `ref.delete` event) and the in-memory
        // hot-swap. `ok` ONLY after a durable removal (fail-closed).
        return handle_delete_ref(
            state, repo, repo_state, &cmd, &principal, &mut log, &token, request,
        );
    }

    let req = ReceiveRequest {
        pack: wire.pack,
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
        Ok(None) => return respond_not_found(request),
        Err(_) => {
            let report = build_report_status(
                Ok(()),
                &[RefOutcome::Ng {
                    ref_name: cmd.ref_name.clone(),
                    reason: "write-seam-unavailable".into(),
                }],
            );
            return send_report(request, report);
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
            RecvLimits::default(),
        )
    };
    match recv {
        Ok(_receipt) => match &mut writer {
            // GIT_DIR mode (UNCHANGED): persist the log (compare-and-swap) then move
            // the on-disk ref so the upload-pack wire advertises the new tip.
            RepoWriter::GitDir { git_dir, .. } => {
                if let Err(e) = state.persist(repo, &log, &token) {
                    let report = build_report_status(
                        Err("log-persist-failed"),
                        &[RefOutcome::Ng {
                            ref_name: cmd.ref_name.clone(),
                            reason: format!("persist:{}", e.status),
                        }],
                    );
                    return send_report(request, report);
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
                    return send_report(request, report);
                }
                let report = build_report_status(Ok(()), &[RefOutcome::Ok(cmd.ref_name.clone())]);
                send_report(request, report);
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
                let res = crate::cas::finalize_cas_push(
                    cas.as_mut(),
                    &seam.r2,
                    &seam.tenant,
                    &seam.repo_slug,
                    &cmd.ref_name,
                    &cmd.new_oid,
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
                        send_report(request, report);
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
                    }
                    Err(crate::cas::CasPushError::Persist(reason)) => {
                        let report = build_report_status(
                            Err("log-persist-failed"),
                            &[RefOutcome::Ng {
                                ref_name: cmd.ref_name.clone(),
                                reason,
                            }],
                        );
                        send_report(request, report)
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
                        send_report(request, report)
                    }
                    Err(crate::cas::CasPushError::Manifest(_)) => {
                        let report = build_report_status(
                            Ok(()),
                            &[RefOutcome::Ng {
                                ref_name: cmd.ref_name.clone(),
                                reason: "manifest-write-failed".into(),
                            }],
                        );
                        send_report(request, report)
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
            send_report(request, report)
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
        send_report(request, report);
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
        Ok(None) => return respond_not_found(request),
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
            send_report(request, report);
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
                    send_report(request, report);
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
                }
                Err(crate::cas::CasPushError::Persist(reason)) => {
                    let report = build_report_status(
                        Err("log-persist-failed"),
                        &[RefOutcome::Ng {
                            ref_name: cmd.ref_name.clone(),
                            reason,
                        }],
                    );
                    send_report(request, report)
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
fn send_report(request: Request, body: Vec<u8>) {
    let resp = Response::from_data(body)
        .with_status_code(200)
        .with_header(git_content_type("application/x-git-receive-pack-result"));
    send(request, resp);
}

/// 401 for a push with no/invalid bearer (a write must be authenticated).
fn respond_push_unauth(request: Request) {
    send(
        request,
        Response::from_data(b"git push requires authentication\n".to_vec())
            .with_status_code(401)
            .with_header(
                tiny_http::Header::from_bytes(&b"Content-Type"[..], &b"text/plain"[..])
                    .expect("static content-type"),
            ),
    );
}

/// 400 for a malformed push body (bad pkt-line framing / command / missing pack).
fn respond_push_bad_request(request: Request, detail: &str) {
    send(
        request,
        Response::from_data(format!("malformed receive-pack request: {detail}\n").into_bytes())
            .with_status_code(400)
            .with_header(
                tiny_http::Header::from_bytes(&b"Content-Type"[..], &b"text/plain"[..])
                    .expect("static content-type"),
            ),
    );
}

fn respond_push_forbidden(request: Request) {
    send(
        request,
        Response::from_data(PUSH_FORBIDDEN_BODY.as_bytes().to_vec())
            .with_status_code(403)
            .with_header(
                tiny_http::Header::from_bytes(&b"Content-Type"[..], &b"text/plain"[..])
                    .expect("static content-type"),
            ),
    );
}

/// Respond 404 with no body — the uniform "git serving not live / not found"
/// answer (no existence oracle).
fn respond_not_found(request: Request) {
    send(
        request,
        Response::from_data(Vec::new()).with_status_code(404),
    );
}

/// Send a response, swallowing a broken-pipe (client hung up) like the rest of
/// the server loop; log other faults.
fn send<R: std::io::Read>(request: Request, response: Response<R>) {
    if let Err(e) = request.respond(response)
        && e.kind() != std::io::ErrorKind::BrokenPipe
    {
        eprintln!("hugit-serve: git respond error: {e}");
    }
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
