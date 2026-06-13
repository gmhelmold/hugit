//! `hugit-serve` binary — the `/v1` HTTP engine port.
//!
//! Reads config from env (fail-closed): `HUGIT_SERVE_LOG_DIR` (required, holds
//! `<repo>.json` event logs), `HUGIT_ENGINE_DEV_TOKEN` (required Bearer token —
//! the Wave-1 stub for ADR-0002 Clerk-JWKS, P2), `HUGIT_SERVE_ADDR` (optional,
//! default `127.0.0.1:8787`). Serves the 5 Wave-1 reads + `/readyz`.

use std::process::ExitCode;

use hugit_serve::server;
use hugit_serve::state::AppState;

fn main() -> ExitCode {
    let state = match AppState::from_env() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("hugit-serve: cannot start — {e}");
            return ExitCode::from(2);
        }
    };
    let addr = std::env::var("HUGIT_SERVE_ADDR").unwrap_or_else(|_| "127.0.0.1:8787".to_string());
    match server::serve(state, &addr) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("hugit-serve: server error — {e}");
            ExitCode::FAILURE
        }
    }
}
