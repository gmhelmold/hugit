//! Git smart-HTTP wire serving (`git clone`/`git fetch` — `git-upload-pack`).
//!
//! This is the FIRST live git wire surface on `hugit-serve`. Scope: the READ
//! side only — `git-upload-pack` (clone + fetch). `git-receive-pack` (push) is
//! explicitly OUT of scope (a later piece): the POST `git-receive-pack` route is
//! not served here.
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
//! The [`hugit_proto::RefView`] for the advertisement is the `git_refs` map on
//! [`AppState`](crate::state::AppState) — populated by `for-each-ref` over the
//! exact `HUGIT_SERVE_GIT_DIR` the objects were enumerated from. Refs and objects
//! must be consistent; reading refs from the event log (a different projection)
//! could advertise a tip whose closure is not in the CAS → a clone that 404s
//! mid-stream. They are co-loaded, fail-closed, at boot.
//!
//! ## Auth
//!
//! A `git` client sends no `Bearer`. A clone is gated on the repo's **READ
//! visibility** — the SAME `authz::authorize_read` predicate every `/v1` read
//! uses, against an unauthenticated principal. Only a **publicly readable** repo
//! is served; anything else is a 404 (no existence oracle — never reveal a
//! private/absent repo). When no git dir is wired (`git_source`/`git_refs`
//! empty) every git route is a 404 (git serving is not live — honest, no fake).

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
                return respond_push_forbidden(request);
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
        // POST git-receive-pack (push) → 403 with a clear human message. Not a
        // silent 404: the client explicitly asked to push and deserves to know why
        // it is refused, rather than seeing a cryptic "repository not found" error.
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
    let source = state.git_source.as_ref()?;

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
    // git serving is not live unless a git dir was loaded at boot.
    if state.git_source.is_none() || state.git_refs.is_empty() {
        return None;
    }
    if !crate::state::is_safe_repo_slug(repo) {
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
    // The git dir is single-repo (one `HUGIT_SERVE_GIT_DIR`); `git_refs` are its
    // refs. Serve them for the gated repo. (A multi-repo git dir map is a later
    // seam — see the honest-delivery audit; today one dir backs the launch repo.)
    Some(state.git_refs.clone())
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
