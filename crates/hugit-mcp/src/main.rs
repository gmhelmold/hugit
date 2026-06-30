//! `hugit-mcp` — the MCP server binary.
//!
//! Reads JSON-RPC 2.0 requests line-by-line from stdin, writes responses
//! line-by-line to stdout, until EOF. Intended to be launched by an MCP host
//! (an LLM agent runtime) as a child process over stdio.

use std::io::{self, BufReader};

fn main() -> io::Result<()> {
    let stdin = io::stdin();
    let reader = BufReader::new(stdin.lock());
    let stdout = io::stdout();
    let writer = stdout.lock();
    hugit_mcp::run_stdio(reader, writer)
}
