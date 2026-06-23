//! Git smart-HTTP wire serving — `git clone`/`git fetch` (`git-upload-pack`, the
//! READ side) AND `git push` (`git-receive-pack`, the WRITE side).
//!
//! The READ side serves any repo whose git seam is loaded. The WRITE side
//! (`git-receive-pack`) is gated, fail-closed: OFF unless `HUGIT_SERVE_RECEIVE_PACK=1`
//! AND the repo has an on-disk `git_dir` write seam AND the pusher passes
//! `authorize_write` (ownership). It wires the git client straight into
//! [`hugit_proto::write::receive::receive_pack`] (bound→unpack→verify→store→anchor→
//! append) and answers the `report-status`. v0 scope: a single non-delete ref per
//! push, self-contained pack (incremental/thin-pack reachability that consults the
//! CAS for server-side bases, and the live in-process ref hot-swap, are follow-ups
//! — see `docs/plan/2026-06-22-receive-pack-wave-design.md`).
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
//! A `git` client sends no `Bearer`. A clone is gated on the repo's **READ
//! visibility** — the SAME `authz::authorize_read` predicate every `/v1` read
//! uses, against an unauthenticated principal. Only a **publicly readable** repo
//! is served; anything else is a 404 (no existence oracle — never reveal a
//! private/absent repo). When the requested repo has no git seam loaded (not in
//! the [`AppState`](crate::state::AppState) repo map) every git route is a 404
//! (git serving is not live for it — honest, no fake; no oracle for which repos
//! are git-served).

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
            match advertise_refs(state, repo) {
                Some(body) => {
                    let resp = Response::from_data(body).with_status_code(200).with_header(
                        git_content_type("application/x-git-upload-pack-advertisement"),
                    );
                    send(request, resp);
                }
                None => respond_not_found(request),
            }
        }
        (Method::Post, [repo, "git-upload-pack"]) => match upload_pack(state, repo, body) {
            Some(out) => {
                let resp = Response::from_data(out)
                    .with_status_code(200)
                    .with_header(git_content_type("application/x-git-upload-pack-result"));
                send(request, resp);
            }
            None => respond_not_found(request),
        },
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

/// Build the v1 `info/refs` advertisement body for `repo`, or `None` (→ 404) when
/// git serving is not live, the repo is unsafe/absent, or it is not publicly
/// readable. The auth gate is the SAME `authorize_read` predicate as the `/v1`
/// reads, evaluated against an unauthenticated principal.
fn advertise_refs(state: &AppState, repo: &str) -> Option<Vec<u8>> {
    let refs = git_refs_for(state, repo)?;

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
fn pick_default_branch(refs: &BTreeMap<String, String>) -> Option<String> {
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
fn upload_pack(state: &AppState, repo: &str, body: &[u8]) -> Option<Vec<u8>> {
    let refs = git_refs_for(state, repo)?;
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
        hugit_proto::serve_fetch(source.as_ref(), &clone_req).ok()?
    } else {
        hugit_proto::serve_fetch(source.as_ref(), &request).ok()?
    };

    // Smart-HTTP v1 upload-pack result: the NAK pkt-line (we run no multi-ack
    // negotiation — single round, "done"), then the raw packfile bytes.
    let mut out = Vec::new();
    pkt_line(&mut out, b"NAK\n");
    out.extend_from_slice(&pack.bytes);
    Some(out)
}

/// The refs to advertise for `repo`, or `None` when git serving is not live for
/// it. Enforces, in order: git dir wired at all → repo slug safe → repo publicly
/// readable (the `authorize_read` gate against an unauthenticated principal) →
/// non-empty ref set. Every failure is `None` (the caller maps it to a uniform
/// 404 — no existence oracle, identical for absent/private/not-live).
fn git_refs_for(state: &AppState, repo: &str) -> Option<BTreeMap<String, String>> {
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
    // READ-visibility gate, identical predicate to the `/v1` reads, against an
    // UNAUTHENTICATED principal (git sends no Bearer): only a publicly readable
    // repo is cloneable. A load/verify failure or a private/absent repo → None
    // (→ 404, no oracle). The operator bypass does not apply: there is no operator
    // credential on the git wire.
    let log = state.load_verified(repo).ok()?;
    let meta = crate::authz::project_repo_meta(&log);
    if !crate::authz::authorize_read(&[], &meta) {
        return None;
    }
    // This repo's OWN refs (the multi-repo forge resolves `{repo}` → its RepoState).
    Some(repo_state.git_refs.clone())
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
/// reads our result report; `object-format=sha1` matches the proto's hash.
const RECEIVE_CAPS: &str = "report-status object-format=sha1 agent=hugit-serve";

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
    if repo_state.git_dir.is_none() {
        return respond_not_found(request); // no write seam (CAS mode) → 404
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
    if repo_state.git_refs.is_empty() {
        // No refs yet: the zero-id capabilities line (so the client can still create).
        let line = format!("{} capabilities^{{}}\0{RECEIVE_CAPS}\n", "0".repeat(40));
        pkt_line(&mut out, line.as_bytes());
    } else {
        for (name, oid) in &repo_state.git_refs {
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
    use crate::writes::LogSink; // brings AppState::persist (the compare-and-swap) into scope
    use hugit_proto::write::receive::{ReceiveRequest, RecvLimits, RefUpdate, receive_pack};

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
    let (Some(mut cas), Some(git_dir)) = (repo_state.write_cas(), repo_state.git_dir.clone())
    else {
        return respond_not_found(request); // no on-disk write seam → 404
    };

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
        // delete-ref is out of v0 scope — a clean per-ref `ng`, not a hard error.
        let report = build_report_status(
            Ok(()),
            &[RefOutcome::Ng {
                ref_name: cmd.ref_name.clone(),
                reason: "delete-not-supported".into(),
            }],
        );
        return send_report(request, report);
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

    let gate = state.receive_flag_gate();
    match receive_pack(&gate, &req, &mut cas, &mut log, RecvLimits::default()) {
        Ok(_receipt) => {
            // Persist the appended ref.update event (compare-and-swap against the
            // head we read). A conflict/transport fault → report it, store nothing
            // half-applied (the objects are content-addressed + idempotent).
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
            // Update the git-dir ref so the upload-pack wire advertises the new tip
            // (after the next load). Compare-and-swap against the expected old value
            // (git's own ref CAS). In CAS-mode prod this path is unreachable (no
            // git_dir); the live ref write-back is the W3 follow-up.
            if let Err(reason) = update_git_ref(&git_dir, &cmd.ref_name, &cmd.new_oid, &cmd.old_oid)
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
            send_report(request, report)
        }
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
