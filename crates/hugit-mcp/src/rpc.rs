//! Minimal MCP / JSON-RPC 2.0 envelope types.
//!
//! The Model Context Protocol rides JSON-RPC 2.0 over a stdio transport: one
//! JSON object per line on stdin (a request or a notification), one JSON object
//! per line on stdout (a response). We implement exactly the slice the four
//! hugit tools need — `initialize`, `tools/list`, `tools/call` — plus the
//! standard error envelope. No batch requests, no SSE, no server-initiated
//! requests: a user-hosted, single-client, line-delimited server.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A JSON-RPC 2.0 request (or notification, when `id` is absent).
#[derive(Debug, Deserialize)]
pub struct Request {
    /// MUST be exactly `"2.0"`. We validate and reject anything else.
    #[serde(default)]
    pub jsonrpc: String,
    /// Present for a request, absent for a notification (no response is sent
    /// for a notification). May be a string or a number per the spec.
    #[serde(default)]
    pub id: Option<Value>,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

impl Request {
    /// A notification has no `id` — the server MUST NOT reply to it.
    pub fn is_notification(&self) -> bool {
        self.id.is_none()
    }
}

/// A JSON-RPC 2.0 response — exactly one of `result` / `error` is `Some`.
#[derive(Debug, Serialize)]
pub struct Response {
    pub jsonrpc: &'static str,
    pub id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcError>,
}

impl Response {
    pub fn ok(id: Value, result: Value) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            result: Some(result),
            error: None,
        }
    }

    pub fn err(id: Value, error: RpcError) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            result: None,
            error: Some(error),
        }
    }
}

/// A JSON-RPC 2.0 error object.
#[derive(Debug, Serialize)]
pub struct RpcError {
    pub code: i64,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl RpcError {
    pub fn new(code: i64, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            data: None,
        }
    }
}

/// Standard JSON-RPC 2.0 error codes (the slice we emit).
pub mod codes {
    /// Invalid JSON was received (the line did not parse).
    pub const PARSE_ERROR: i64 = -32700;
    /// The JSON sent is not a valid Request object.
    pub const INVALID_REQUEST: i64 = -32600;
    /// The method does not exist.
    pub const METHOD_NOT_FOUND: i64 = -32601;
    /// Invalid method parameters.
    pub const INVALID_PARAMS: i64 = -32602;
    /// Internal JSON-RPC error.
    pub const INTERNAL_ERROR: i64 = -32603;
}
