//! hugit-mcp — a user-hosted MCP (Model Context Protocol) server exposing the
//! four hugit LLM-native tools over stdio JSON-RPC 2.0.
//!
//! Ships with hugit, free/open. The server is intentionally minimal and HONEST:
//! every tool states its real scope (the v1 path-approximation in
//! `claim-disjointness`, the honest-null per-PR cost in `cost-attest`, the
//! heavy-read refusal in `liveness-probe`) and never fabricates a figure or a
//! verdict.
//!
//! ## Tools
//!
//! | tool | seam | honesty |
//! |------|------|---------|
//! | `claim-disjointness` | `hugit_queue::core::AffectedSet::is_disjoint` (in-process) | v1 = file-path approximation, not the real check blast-radius (B3) |
//! | `land-status` | shells `hugit queue show --log <path>` | passes the porcelain's null-verdict honesty through |
//! | `cost-attest` | `GET /v1/repos/{repo}/insights` | reads only attested figures; no hand-stamp; per-PR honest-null |
//! | `liveness-probe` | `GET /readyz` + a bounded authed light read | disambiguates 401/403/404; refuses heavy reads |
//!
//! ## Transport
//!
//! Line-delimited JSON-RPC 2.0 over stdio: one JSON object per line on stdin
//! (request/notification), one per line on stdout (response). See
//! [`run_stdio`].

pub mod http;
pub mod rpc;
pub mod server;
pub mod tools;

use std::io::{BufRead, Write};

/// Run the MCP server over the given line-delimited reader/writer until EOF.
///
/// Extracted from `main` so it is testable with in-memory buffers. Each input
/// line is parsed as a JSON-RPC request; a parse failure yields a JSON-RPC
/// `PARSE_ERROR` response (id null). A notification (no `id`) yields no output.
/// Blank lines are skipped. Returns the first I/O write error, else `Ok(())` at
/// EOF.
pub fn run_stdio<R: BufRead, W: Write>(reader: R, mut writer: W) -> std::io::Result<()> {
    for line in reader.lines() {
        let line = line?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let response = match serde_json::from_str::<rpc::Request>(trimmed) {
            Ok(req) => server::handle(req),
            Err(e) => Some(rpc::Response::err(
                serde_json::Value::Null,
                rpc::RpcError::new(rpc::codes::PARSE_ERROR, format!("parse error: {e}")),
            )),
        };

        if let Some(resp) = response {
            let encoded = serde_json::to_string(&resp).unwrap_or_else(|_| {
                // Serializing our own Response is infallible in practice; keep a
                // valid JSON-RPC fallback rather than panicking.
                r#"{"jsonrpc":"2.0","id":null,"error":{"code":-32603,"message":"internal serialize error"}}"#.to_string()
            });
            writeln!(writer, "{encoded}")?;
            writer.flush()?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    /// Drive the loop with a sequence of input lines, return the output lines.
    fn drive(input: &str) -> Vec<serde_json::Value> {
        let mut out = Vec::new();
        run_stdio(Cursor::new(input), &mut out).unwrap();
        String::from_utf8(out)
            .unwrap()
            .lines()
            .filter(|l| !l.is_empty())
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }

    #[test]
    fn initialize_then_list_then_call_round_trips_over_stdio() {
        let input = concat!(
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize"}"#,
            "\n",
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
            "\n",
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#,
            "\n",
            r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"claim-disjointness","arguments":{"claim_a":{"paths":["a"]},"claim_b":{"paths":["b"]}}}}"#,
            "\n"
        );
        let out = drive(input);
        // initialize, tools/list, tools/call — the notification produced no line.
        assert_eq!(out.len(), 3);
        assert_eq!(out[0]["id"], serde_json::json!(1));
        assert_eq!(out[0]["result"]["serverInfo"]["name"], "hugit-mcp");
        assert_eq!(out[1]["result"]["tools"].as_array().unwrap().len(), 5);
        assert_eq!(out[2]["result"]["isError"], serde_json::json!(false));
    }

    #[test]
    fn a_malformed_line_yields_a_parse_error_response() {
        let out = drive("this is not json\n");
        assert_eq!(out.len(), 1);
        assert_eq!(
            out[0]["error"]["code"],
            serde_json::json!(rpc::codes::PARSE_ERROR)
        );
        assert_eq!(out[0]["id"], serde_json::Value::Null);
    }

    #[test]
    fn blank_lines_are_skipped() {
        let out = drive("\n\n");
        assert!(out.is_empty());
    }

    #[test]
    fn a_notification_produces_no_output_line() {
        let out = drive(r#"{"jsonrpc":"2.0","method":"ping"}"#);
        // ping as a notification (no id) → no reply.
        assert!(out.is_empty());
    }
}
