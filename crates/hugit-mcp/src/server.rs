//! The MCP method dispatcher + tool registry.
//!
//! Handles the three MCP methods the four tools need over JSON-RPC 2.0:
//! `initialize`, `tools/list`, `tools/call`. A `tools/call` returns the MCP
//! content envelope (`{ content: [{type:"text", text}], isError }`) — a tool
//! FAILURE is a successful JSON-RPC response with `isError: true`, so the model
//! sees it (distinct from a JSON-RPC protocol error like method-not-found).

use serde_json::{Value, json};

use crate::rpc::{Request, Response, RpcError, codes};
use crate::tools::{self, ToolOutcome};

/// The MCP protocol revision this server speaks.
const PROTOCOL_VERSION: &str = "2024-11-05";
const SERVER_NAME: &str = "hugit-mcp";
const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Handle one parsed request, returning the response to write (or `None` for a
/// notification, which gets no reply).
pub fn handle(req: Request) -> Option<Response> {
    // Validate the JSON-RPC version envelope.
    if req.jsonrpc != "2.0" {
        if req.is_notification() {
            return None;
        }
        return Some(Response::err(
            req.id.clone().unwrap_or(Value::Null),
            RpcError::new(codes::INVALID_REQUEST, "jsonrpc must be exactly \"2.0\""),
        ));
    }

    // Notifications (no id) get no reply. `notifications/initialized` is the
    // standard post-initialize handshake notification — accept silently.
    if req.is_notification() {
        return None;
    }
    let id = req.id.clone().unwrap_or(Value::Null);

    match req.method.as_str() {
        "initialize" => Some(Response::ok(id, initialize_result())),
        "tools/list" => Some(Response::ok(id, json!({ "tools": tool_specs() }))),
        "tools/call" => Some(handle_tools_call(id, &req.params)),
        // `ping` — a common MCP liveness method; reply with an empty result.
        "ping" => Some(Response::ok(id, json!({}))),
        other => Some(Response::err(
            id,
            RpcError::new(
                codes::METHOD_NOT_FOUND,
                format!("method not found: {other}"),
            ),
        )),
    }
}

fn initialize_result() -> Value {
    json!({
        "protocolVersion": PROTOCOL_VERSION,
        "capabilities": { "tools": { "listChanged": false } },
        "serverInfo": { "name": SERVER_NAME, "version": SERVER_VERSION },
        "instructions": "hugit's LLM-native tools. claim-disjointness wraps the real union-engine \
            disjointness primitive (v1 path-approximation, stated). land-status shells the hugit \
            queue-show porcelain. cost-attest reads only the engine's attested figures (no \
            hand-stamp; per-PR cost honest-null until the runner fabric). liveness-probe checks \
            /readyz + a bounded authed probe with a git UA and REFUSES heavy reads against the \
            single-thread engine. capture records git activity that fires no hook (e.g. jj) via \
            the same `hugit capture` seam the silent hooks use. Capture returns only \
            `status: dispatched`: no receipt or invocation id exists before WP3, so MCP cannot \
            confirm that a specific capture landed."
    })
}

/// Dispatch a `tools/call`: validate `name` + `arguments`, route to the tool,
/// wrap the outcome into the MCP content envelope.
fn handle_tools_call(id: Value, params: &Value) -> Response {
    let name = match params.get("name").and_then(Value::as_str) {
        Some(n) => n,
        None => {
            return Response::err(
                id,
                RpcError::new(codes::INVALID_PARAMS, "tools/call requires a string `name`"),
            );
        }
    };
    // `arguments` defaults to `{}` when omitted.
    let args = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));

    let outcome = match name {
        "claim-disjointness" => tools::claim_disjointness::run(&args),
        "land-status" => tools::land_status::run(&args),
        "cost-attest" => tools::cost_attest::run(&args),
        "liveness-probe" => tools::liveness_probe::run(&args),
        "capture" => tools::capture::run(&args),
        other => {
            return Response::err(
                id,
                RpcError::new(codes::METHOD_NOT_FOUND, format!("unknown tool: {other}")),
            );
        }
    };

    Response::ok(id, tool_envelope(outcome))
}

/// Wrap a [`ToolOutcome`] into the MCP `tools/call` result envelope. A tool
/// error is `isError: true` with the message as text content — a normal,
/// successful JSON-RPC response the model can read and react to.
fn tool_envelope(outcome: ToolOutcome) -> Value {
    match outcome {
        ToolOutcome::Ok(value) => {
            let text = serde_json::to_string_pretty(&value).unwrap_or_else(|_| value.to_string());
            json!({
                "content": [{ "type": "text", "text": text }],
                "isError": false,
            })
        }
        ToolOutcome::Err(message) => json!({
            "content": [{ "type": "text", "text": message }],
            "isError": true,
        }),
    }
}

/// The four tool specifications (name + description + JSON-Schema input).
fn tool_specs() -> Vec<Value> {
    vec![
        json!({
            "name": "claim-disjointness",
            "description": "Decide whether two claims (each a set of touched file paths) are \
                disjoint and may land in parallel lanes without a shared union re-test. Wraps the \
                real union-engine disjointness primitive. HONEST CAVEAT: v1 approximates the \
                affected set by file paths, not the true memoized-check blast radius — `disjoint: \
                true` is necessary but not yet sufficient; the union test remains the authority.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "claim_a": claim_schema(),
                    "claim_b": claim_schema(),
                },
                "required": ["claim_a", "claim_b"],
            }
        }),
        json!({
            "name": "land-status",
            "description": "Show the current landing-queue state (entries in queue order, batch \
                composition by campaign, per-batch union verdict + failing pair) by shelling the \
                real `hugit queue show --log <path>` porcelain and parsing its JSON. A null \
                verdict is the honest 'no verdict recorded yet', never a faked pass/fail.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "top_level": { "type": "string", "description": "Repository top-level directory. Required only when log and $HUGIT_LOG are both absent." },
                    "log": { "type": "string", "description": "Explicit event log path. Legacy {log} calls remain supported without top_level." },
                    "campaign": { "type": "string", "description": "Optional: scope to one campaign's batch." },
                    "hugit_bin": { "type": "string", "description": "Optional hugit binary path (else $HUGIT_BIN, else `hugit` on PATH)." },
                },
                "required": [],
            }
        }),
        json!({
            "name": "cost-attest",
            "description": "Read the attested cost figures for a repo from the live engine's \
                GET /v1/repos/{repo}/insights (integer micro-USD). Reads ONLY the engine's \
                attested values — a caller-supplied cost is structurally refused (no hand-stamp). \
                Per-PR raw cost is honest-null until the runner fabric supplies a provider-billed \
                figure.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "engine_base": { "type": "string", "description": "Engine base URL, e.g. https://engine.githugr.com" },
                    "repo": { "type": "string", "description": "Repo name, e.g. hugit" },
                    "token": { "type": "string", "description": "Session Bearer token (insights is auth-gated; audience = repo's tenant)." },
                },
                "required": ["engine_base", "repo", "token"],
            }
        }),
        json!({
            "name": "liveness-probe",
            "description": "Probe engine liveness: GET /readyz (unauthenticated) plus an optional \
                bounded authed probe of a LIGHT endpoint (with a git UA) to disambiguate \
                401-auth-gate / 403-bot / 404-denied-or-missing. REFUSES heavy reads (search, \
                diff, insights, blob history) — they wedge the single-thread lazy-CAS engine.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "engine_base": { "type": "string", "description": "Engine base URL." },
                    "repo": { "type": "string", "description": "Optional: enables the authed disambiguation probe." },
                    "token": { "type": "string", "description": "Optional Bearer for the authed probe." },
                    "probe_path": { "type": "string", "description": "Optional probe path override; REFUSED if it matches a heavy-read class." },
                },
                "required": ["engine_base"],
            }
        }),
        json!({
            "name": "capture",
            "description": "Record a git event (commit / checkout / push-attempt / merge) on the \
                canonical event log by shelling the REAL `hugit capture` seam — the SAME one the \
                silent hooks use. Use when the LLM did a git action whose path fires NO hook \
                (e.g. `jj describe` + `jj git export` write refs directly): call this tool \
                IN PLACE of the raw action. Returns `status: dispatched` only: no receipt or \
                invocation id exists before WP3, so silent exit-0 cannot prove capture landed.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "kind": { "type": "string", "description": "commit | checkout | push-attempt | merge." },
                    "top_level": { "type": "string", "description": "Repo top-level dir (git rev-parse --show-toplevel)." },
                    "log": { "type": "string", "description": "Optional explicit event log path; omitted delegates to CLI default resolution." },
                    "oid": { "type": "string", "description": "commit target / checkout-to / merge tip." },
                    "branch": { "type": "string", "description": "Branch name." },
                    "from": { "type": "string", "description": "checkout/merge from oid." },
                    "recorded_at": { "type": "string", "description": "Unix seconds (committer date preferred)." },
                    "refspecs": { "type": "string", "description": "push stdin refspec lines." },
                    "shas": { "type": "string", "description": "local shas being pushed (whitespace separated)." },
                    "files": { "type": "array", "items": { "type": "string" }, "description": "files the commit touched (for hugit why --path)." },
                    "hook_log": { "type": "string", "description": "Optional .hugit/hooks.log path for capture trace." },
                    "hugit_bin": { "type": "string", "description": "Optional hugit binary path (else $HUGIT_BIN, else `hugit` on PATH)." },
                },
                "required": ["kind", "top_level"],
            }
        }),
    ]
}

/// JSON-Schema for one claim object in `claim-disjointness`.
fn claim_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "id": { "type": "string", "description": "Optional human label for the claim." },
            "paths": {
                "type": "array",
                "items": { "type": "string" },
                "description": "The file paths this claim touches (its v1 affected set).",
            },
        },
        "required": ["paths"],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(method: &str, params: Value, id: Value) -> Request {
        // Build via JSON so we exercise the same deserialize path as the loop.
        serde_json::from_value(json!({
            "jsonrpc": "2.0", "method": method, "params": params, "id": id
        }))
        .unwrap()
    }

    #[test]
    fn initialize_reports_the_tools_via_list() {
        let resp = handle(req("tools/list", json!({}), json!(1))).unwrap();
        let result = resp.result.unwrap();
        let names: Vec<&str> = result["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        assert_eq!(
            names,
            vec![
                "claim-disjointness",
                "land-status",
                "cost-attest",
                "liveness-probe",
                "capture",
            ]
        );
    }

    #[test]
    fn initialize_returns_protocol_and_server_info() {
        let resp = handle(req("initialize", json!({}), json!(1))).unwrap();
        let r = resp.result.unwrap();
        assert_eq!(r["protocolVersion"], json!(PROTOCOL_VERSION));
        assert_eq!(r["serverInfo"]["name"], json!(SERVER_NAME));
    }

    #[test]
    fn capture_documentation_promises_dispatch_only() {
        let initialize = initialize_result();
        let instructions = initialize["instructions"].as_str().unwrap();
        assert!(instructions.contains("Capture returns only `status: dispatched`"));
        assert!(instructions.contains("no receipt or invocation id exists before WP3"));

        let capture = tool_specs()
            .into_iter()
            .find(|tool| tool["name"] == "capture")
            .unwrap();
        assert!(
            capture["description"]
                .as_str()
                .unwrap()
                .contains("Returns `status: dispatched` only")
        );
        assert!(
            capture["description"]
                .as_str()
                .unwrap()
                .contains("no receipt or invocation id exists before WP3")
        );
        assert!(capture["inputSchema"]["properties"].get("verify").is_none());
    }

    #[test]
    fn a_notification_gets_no_response() {
        let n: Request = serde_json::from_value(json!({
            "jsonrpc": "2.0", "method": "notifications/initialized"
        }))
        .unwrap();
        assert!(handle(n).is_none());
    }

    #[test]
    fn unknown_method_is_method_not_found() {
        let resp = handle(req("does/not/exist", json!({}), json!(7))).unwrap();
        let err = resp.error.unwrap();
        assert_eq!(err.code, codes::METHOD_NOT_FOUND);
    }

    #[test]
    fn tools_call_routes_to_claim_disjointness_and_wraps_content() {
        let params = json!({
            "name": "claim-disjointness",
            "arguments": {
                "claim_a": { "paths": ["a.rs"] },
                "claim_b": { "paths": ["b.rs"] }
            }
        });
        let resp = handle(req("tools/call", params, json!(2))).unwrap();
        let r = resp.result.unwrap();
        assert_eq!(r["isError"], json!(false));
        let text = r["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("\"disjoint\": true"));
    }

    #[test]
    fn tools_call_tool_error_is_iserror_true_not_a_protocol_error() {
        // Missing required arg → a TOOL error (isError:true), not a JSON-RPC error.
        let params = json!({ "name": "land-status", "arguments": {} });
        let resp = handle(req("tools/call", params, json!(3))).unwrap();
        assert!(
            resp.error.is_none(),
            "must be a successful JSON-RPC response"
        );
        let r = resp.result.unwrap();
        assert_eq!(r["isError"], json!(true));
    }

    #[test]
    fn tools_call_unknown_tool_is_method_not_found() {
        let params = json!({ "name": "no-such-tool", "arguments": {} });
        let resp = handle(req("tools/call", params, json!(4))).unwrap();
        assert_eq!(resp.error.unwrap().code, codes::METHOD_NOT_FOUND);
    }

    #[test]
    fn tools_call_without_name_is_invalid_params() {
        let resp = handle(req("tools/call", json!({}), json!(5))).unwrap();
        assert_eq!(resp.error.unwrap().code, codes::INVALID_PARAMS);
    }
}
