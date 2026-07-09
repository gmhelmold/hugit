//! Checks-API write-back client (WP-B1 item ③ / WP-E3 item ①).
//!
//! Surfaces hugit's verdict **on GitHub** ("landing rides ON GitHub"): given a
//! landed intent / merge event, POST a GitHub **Check-Run** and/or a **commit
//! status** back to the repo, authenticated with a per-installation token minted
//! by the GitHub App.
//!
//! # Live vs fail-closed
//!
//! The single network seam is [`ChecksTransport`]; the real implementation is
//! [`UreqChecksTransport`] (bounded timeout, Bearer auth). Everything around it —
//! request shaping, response decode, error mapping — is proven hermetically
//! against a fake transport with ZERO network. FAIL-CLOSED at every step:
//! - absent installation token → [`ChecksClientError::TokenRevoked`] (never a
//!   fabricated post);
//! - non-2xx / transport fault / undecodable body → [`ChecksClientError::Api`];
//! - `local_mode` returns a synthetic response (unit-test convenience) and never
//!   touches the network.
//!
//! # Secret hygiene
//!
//! The installation token is passed by borrow, forwarded only into the
//! `Authorization: Bearer` header inside the transport, and is NEVER logged nor
//! embedded in any error (errors carry only the HTTP status / a generic reason).

use std::time::Duration;

use hugit_contracts::{ChecksWriteRequest, ChecksWriteResponse};

/// Errors from the Checks API client.
#[derive(Debug, thiserror::Error)]
pub enum ChecksClientError {
    /// The request payload could not be serialised.
    #[error("serialise checks request: {0}")]
    Serialise(String),

    /// The API call failed (HTTP non-2xx or transport error). Carries a short,
    /// SECRET-FREE reason (never the token, never a response body verbatim).
    #[error("GitHub Checks API error: {0}")]
    Api(String),

    /// Installation token is absent or revoked — cannot write checks
    /// (fail-closed; this is the honest "not configured / not installed" path).
    #[error("installation token absent or revoked for repo {repo}")]
    TokenRevoked {
        /// The repo the write was attempted against.
        repo: String,
    },
}

/// A single authenticated HTTP exchange against the GitHub REST API — the ONLY
/// network seam. Behind a trait so request shaping, decode, and error mapping are
/// proven hermetically with a fake transport (mirrors `auth::TokenTransport`).
pub trait ChecksTransport {
    /// `POST {url}` with `Authorization: Bearer {token}`, the GitHub API headers,
    /// and `body` (JSON) as the request body. Returns the decoded
    /// `(status, body_bytes)`. A transport-level fault (no response) must map to
    /// `Err(ChecksClientError::Api(..))` with a generic, secret-free reason.
    fn post_json(
        &self,
        url: &str,
        token: &str,
        body: &[u8],
    ) -> Result<(u16, Vec<u8>), ChecksClientError>;
}

/// The real `ureq`-backed transport — the thin network seam with a BOUNDED
/// timeout. The ONLY code here that opens a socket; exercised in production /
/// the dogfood soak, never in a unit test.
#[derive(Debug, Clone, Copy, Default)]
pub struct UreqChecksTransport;

impl ChecksTransport for UreqChecksTransport {
    fn post_json(
        &self,
        url: &str,
        token: &str,
        body: &[u8],
    ) -> Result<(u16, Vec<u8>), ChecksClientError> {
        let agent = ureq::AgentBuilder::new()
            .timeout(Duration::from_secs(20))
            .build();
        let resp = agent
            .post(url)
            .set("Authorization", &format!("Bearer {token}"))
            .set("Accept", "application/vnd.github+json")
            .set("X-GitHub-Api-Version", "2022-11-28")
            .set("User-Agent", "hugit-app")
            .set("Content-Type", "application/json")
            .send_bytes(body);
        match resp {
            Ok(r) => {
                let status = r.status();
                let mut buf = Vec::new();
                use std::io::Read;
                r.into_reader()
                    .read_to_end(&mut buf)
                    .map_err(|_| ChecksClientError::Api("response read failed".to_string()))?;
                Ok((status, buf))
            }
            Err(ureq::Error::Status(code, resp)) => {
                let mut buf = Vec::new();
                use std::io::Read;
                let _ = resp.into_reader().read_to_end(&mut buf);
                Ok((code, buf))
            }
            Err(_) => Err(ChecksClientError::Api("transport fault".to_string())),
        }
    }
}

/// Checks API client.
///
/// The production client (`new_production`) issues authenticated HTTPS calls to
/// the GitHub REST API via [`UreqChecksTransport`]. `new_local` returns synthetic
/// responses (no network) for unit tests. Both paths share one fail-closed core
/// ([`ChecksClient::write_check_run_with_transport`]).
pub struct ChecksClient {
    /// GitHub API base URL (overridable for tests).
    api_base: String,
    /// In-process mode: bypass real HTTP (synthetic responses).
    local_mode: bool,
    /// Running counter for synthetic check_run_id (local mode only).
    next_id: std::sync::atomic::AtomicU64,
}

impl ChecksClient {
    /// Create a production client against the real GitHub API.
    pub fn new_production() -> Self {
        Self {
            api_base: "https://api.github.com".to_string(),
            local_mode: false,
            next_id: std::sync::atomic::AtomicU64::new(1),
        }
    }

    /// Create a local/test client (no real HTTP, returns synthetic responses).
    pub fn new_local() -> Self {
        Self {
            api_base: "https://api.github.com".to_string(),
            local_mode: true,
            next_id: std::sync::atomic::AtomicU64::new(100_001),
        }
    }

    /// Override the API base (tests point it at a fake host).
    pub fn with_api_base(mut self, base: impl Into<String>) -> Self {
        self.api_base = base.into();
        self
    }

    /// The check-runs endpoint URL for `owner/repo`.
    fn check_runs_url(&self, repo: &str) -> String {
        format!("{}/repos/{repo}/check-runs", self.api_base)
    }

    /// The commit-status endpoint URL for `owner/repo` @ `sha`.
    fn statuses_url(&self, repo: &str, sha: &str) -> String {
        format!("{}/repos/{repo}/statuses/{sha}", self.api_base)
    }

    /// Write a check-run to GitHub (item ③) using the production transport.
    ///
    /// `installation_token` must be `Some(&str)` — a valid installation access
    /// token minted by the GitHub App. `None` → [`ChecksClientError::TokenRevoked`]
    /// (fail-closed). In `local_mode` returns a synthetic response; in production
    /// POSTs over [`UreqChecksTransport`].
    pub fn write_check_run(
        &self,
        request: &ChecksWriteRequest,
        installation_token: Option<&str>,
    ) -> Result<ChecksWriteResponse, ChecksClientError> {
        self.write_check_run_with_transport(request, installation_token, &UreqChecksTransport)
    }

    /// The transport-injectable core of [`Self::write_check_run`] — the
    /// hermetically-testable seam (a fake [`ChecksTransport`] proves the whole
    /// path with ZERO network).
    ///
    /// FAIL-CLOSED: token absent → `TokenRevoked`; empty repo/head_sha → `Api`;
    /// non-2xx / decode failure → `Api`; never a fabricated response.
    pub fn write_check_run_with_transport<T: ChecksTransport>(
        &self,
        request: &ChecksWriteRequest,
        installation_token: Option<&str>,
        transport: &T,
    ) -> Result<ChecksWriteResponse, ChecksClientError> {
        // Fail-closed: token must be present before any work.
        let token = installation_token.ok_or_else(|| ChecksClientError::TokenRevoked {
            repo: request.repo.clone(),
        })?;

        // Validate the request is well-formed.
        if request.repo.is_empty() {
            return Err(ChecksClientError::Api("repo is empty".to_string()));
        }
        if request.head_sha.is_empty() {
            return Err(ChecksClientError::Api("head_sha is empty".to_string()));
        }
        if request.check_name.is_empty() {
            return Err(ChecksClientError::Api("check_name is empty".to_string()));
        }

        if self.local_mode {
            let id = self
                .next_id
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            return Ok(ChecksWriteResponse {
                check_run_id: id,
                html_url: format!("{}/repos/{}/check-runs/{}", self.api_base, request.repo, id),
            });
        }

        // Build the GitHub check-runs POST body (subset of the Checks API).
        let body = serde_json::json!({
            "name": request.check_name,
            "head_sha": request.head_sha,
            "status": request.status,
            "conclusion": request.conclusion,
            "output": {
                "title": request.check_name,
                "summary": request.summary,
            },
        });
        let body =
            serde_json::to_vec(&body).map_err(|e| ChecksClientError::Serialise(e.to_string()))?;

        let url = self.check_runs_url(&request.repo);
        let (status, resp_body) = transport.post_json(&url, token, &body)?;
        Self::decode_check_run_response(status, &resp_body)
    }

    /// Decode a GitHub check-runs response, fail-closed. 2xx with a `{id, html_url}`
    /// body → [`ChecksWriteResponse`]; anything else → [`ChecksClientError::Api`]
    /// (the status only; never the body verbatim).
    fn decode_check_run_response(
        status: u16,
        body: &[u8],
    ) -> Result<ChecksWriteResponse, ChecksClientError> {
        if !(200..300).contains(&status) {
            return Err(ChecksClientError::Api(format!(
                "check-run POST returned HTTP {status}"
            )));
        }
        #[derive(serde::Deserialize)]
        struct CheckRunResp {
            id: u64,
            #[serde(default)]
            html_url: String,
        }
        let parsed: CheckRunResp = serde_json::from_slice(body).map_err(|_| {
            ChecksClientError::Api("undecodable check-run response body".to_string())
        })?;
        Ok(ChecksWriteResponse {
            check_run_id: parsed.id,
            html_url: parsed.html_url,
        })
    }

    /// Write a **commit status** to GitHub (the lighter-weight "landing rides ON
    /// GitHub" surface) using the production transport.
    ///
    /// `state` is a GitHub commit-status state (`success` | `failure` |
    /// `pending` | `error`). Fail-closed identically to [`Self::write_check_run`].
    pub fn write_commit_status(
        &self,
        repo: &str,
        sha: &str,
        state: &str,
        context: &str,
        description: &str,
        installation_token: Option<&str>,
    ) -> Result<(), ChecksClientError> {
        self.write_commit_status_with_transport(
            repo,
            sha,
            state,
            context,
            description,
            installation_token,
            &UreqChecksTransport,
        )
    }

    /// Transport-injectable core of [`Self::write_commit_status`] (fake-testable).
    #[allow(clippy::too_many_arguments)]
    pub fn write_commit_status_with_transport<T: ChecksTransport>(
        &self,
        repo: &str,
        sha: &str,
        state: &str,
        context: &str,
        description: &str,
        installation_token: Option<&str>,
        transport: &T,
    ) -> Result<(), ChecksClientError> {
        let token = installation_token.ok_or_else(|| ChecksClientError::TokenRevoked {
            repo: repo.to_string(),
        })?;
        if repo.is_empty() || sha.is_empty() {
            return Err(ChecksClientError::Api("repo/sha is empty".to_string()));
        }
        if !matches!(state, "success" | "failure" | "pending" | "error") {
            return Err(ChecksClientError::Api(format!(
                "invalid commit-status state {state:?}"
            )));
        }
        if self.local_mode {
            return Ok(());
        }
        let body = serde_json::json!({
            "state": state,
            "context": context,
            "description": description,
        });
        let body =
            serde_json::to_vec(&body).map_err(|e| ChecksClientError::Serialise(e.to_string()))?;
        let url = self.statuses_url(repo, sha);
        let (status, _resp) = transport.post_json(&url, token, &body)?;
        if !(200..300).contains(&status) {
            return Err(ChecksClientError::Api(format!(
                "commit-status POST returned HTTP {status}"
            )));
        }
        Ok(())
    }
}

/// Map a check-run `conclusion` (as carried in [`ChecksWriteRequest::conclusion`])
/// to a GitHub **commit-status** state. This is the projection that lets one
/// verdict light BOTH surfaces consistently.
pub fn conclusion_to_status_state(status: &str, conclusion: Option<&str>) -> &'static str {
    if status != "completed" {
        return "pending";
    }
    match conclusion {
        Some("success") => "success",
        Some("failure") | Some("timed_out") | Some("cancelled") => "failure",
        Some("action_required") | None => "error",
        // neutral / skipped / stale → treated as a non-blocking success surface.
        Some(_) => "success",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// An in-memory transport: returns a queued `(status, body)`, records what was
    /// posted, and NEVER opens a socket.
    #[derive(Default)]
    struct FakeChecksTransport {
        responses: Mutex<std::collections::VecDeque<(u16, Vec<u8>)>>,
        last: Mutex<Option<(String, String, Vec<u8>)>>, // (url, token, body)
    }
    impl FakeChecksTransport {
        fn with(responses: Vec<(u16, Vec<u8>)>) -> Self {
            Self {
                responses: Mutex::new(responses.into()),
                last: Mutex::new(None),
            }
        }
        fn last_url(&self) -> String {
            self.last.lock().unwrap().as_ref().unwrap().0.clone()
        }
        fn last_token(&self) -> String {
            self.last.lock().unwrap().as_ref().unwrap().1.clone()
        }
        fn last_body(&self) -> String {
            String::from_utf8(self.last.lock().unwrap().as_ref().unwrap().2.clone()).unwrap()
        }
    }
    impl ChecksTransport for FakeChecksTransport {
        fn post_json(
            &self,
            url: &str,
            token: &str,
            body: &[u8],
        ) -> Result<(u16, Vec<u8>), ChecksClientError> {
            *self.last.lock().unwrap() = Some((url.to_string(), token.to_string(), body.to_vec()));
            self.responses
                .lock()
                .unwrap()
                .pop_front()
                .ok_or_else(|| ChecksClientError::Api("no queued response".to_string()))
        }
    }

    fn req() -> ChecksWriteRequest {
        ChecksWriteRequest {
            repo: "acme/widgets".to_string(),
            head_sha: "abc123".to_string(),
            check_name: "hugit/land".to_string(),
            status: "completed".to_string(),
            conclusion: Some("success".to_string()),
            summary: "landed via KungFu union merge".to_string(),
            output_ref: "cas://out".to_string(),
        }
    }

    #[test]
    fn posts_a_check_run_against_a_fake_transport() {
        let client = ChecksClient::new_production().with_api_base("https://gh.test");
        let transport = FakeChecksTransport::with(vec![(
            201,
            br#"{"id":998877,"html_url":"https://gh.test/repos/acme/widgets/check-runs/998877"}"#
                .to_vec(),
        )]);
        let resp = client
            .write_check_run_with_transport(&req(), Some("ghs_tok"), &transport)
            .unwrap();
        assert_eq!(resp.check_run_id, 998877);
        assert!(resp.html_url.contains("998877"));
        // The right endpoint, the token forwarded as Bearer, and the verdict in
        // the body.
        assert_eq!(
            transport.last_url(),
            "https://gh.test/repos/acme/widgets/check-runs"
        );
        assert_eq!(transport.last_token(), "ghs_tok");
        let body = transport.last_body();
        assert!(body.contains("\"head_sha\":\"abc123\""));
        assert!(body.contains("\"conclusion\":\"success\""));
        assert!(body.contains("hugit/land"));
    }

    #[test]
    fn absent_token_is_fail_closed_never_a_post() {
        let client = ChecksClient::new_production();
        let transport = FakeChecksTransport::with(vec![(201, b"{}".to_vec())]);
        let err = client
            .write_check_run_with_transport(&req(), None, &transport)
            .unwrap_err();
        assert!(matches!(err, ChecksClientError::TokenRevoked { .. }));
        // The transport was NEVER called (no fabricated post).
        assert!(transport.last.lock().unwrap().is_none());
    }

    #[test]
    fn non_2xx_is_fail_closed() {
        let client = ChecksClient::new_production();
        let transport = FakeChecksTransport::with(vec![(422, br#"{"message":"bad"}"#.to_vec())]);
        let err = client
            .write_check_run_with_transport(&req(), Some("t"), &transport)
            .unwrap_err();
        match err {
            ChecksClientError::Api(msg) => assert!(msg.contains("422")),
            other => panic!("expected Api(422), got {other:?}"),
        }
    }

    #[test]
    fn commit_status_posts_to_statuses_endpoint() {
        let client = ChecksClient::new_production().with_api_base("https://gh.test");
        let transport = FakeChecksTransport::with(vec![(201, b"{}".to_vec())]);
        client
            .write_commit_status_with_transport(
                "acme/widgets",
                "abc123",
                "success",
                "hugit/land",
                "landed",
                Some("t"),
                &transport,
            )
            .unwrap();
        assert_eq!(
            transport.last_url(),
            "https://gh.test/repos/acme/widgets/statuses/abc123"
        );
        assert!(transport.last_body().contains("\"state\":\"success\""));
    }

    #[test]
    fn commit_status_rejects_an_invalid_state() {
        let client = ChecksClient::new_production();
        let transport = FakeChecksTransport::with(vec![(201, b"{}".to_vec())]);
        let err = client
            .write_commit_status_with_transport(
                "acme/widgets",
                "abc123",
                "green", // not a GitHub state
                "ctx",
                "desc",
                Some("t"),
                &transport,
            )
            .unwrap_err();
        assert!(matches!(err, ChecksClientError::Api(_)));
    }

    #[test]
    fn conclusion_projection_maps_the_states() {
        assert_eq!(
            conclusion_to_status_state("completed", Some("success")),
            "success"
        );
        assert_eq!(
            conclusion_to_status_state("completed", Some("failure")),
            "failure"
        );
        assert_eq!(conclusion_to_status_state("completed", None), "error");
        assert_eq!(
            conclusion_to_status_state("in_progress", Some("success")),
            "pending"
        );
    }

    #[test]
    fn local_mode_returns_synthetic_without_network() {
        let client = ChecksClient::new_local();
        // The fake transport is never consulted in local mode (synthetic path).
        let transport = FakeChecksTransport::default();
        let resp = client
            .write_check_run_with_transport(&req(), Some("t"), &transport)
            .unwrap();
        assert!(resp.check_run_id >= 100_001);
        assert!(transport.last.lock().unwrap().is_none());
    }
}
