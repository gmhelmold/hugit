//! `hugit-snapshot` — the one-shot engine-storage snapshot uploader (Passo 4).
//!
//! Reads a local canonical event-log file, **chain-verifies it through the engine's
//! PS-13 verified loader** (`hugit_cli::checks::load_event_log_from_bytes` →
//! `rehydrate_and_verify` → `verify_chain`), and only THEN PUTs the raw bytes to
//! `<tenant_id>/<repo>.json` in the R2 `corelink-githugr-engine` bucket. A corrupt
//! or tampered log is REFUSED before any upload — the snapshot the read path will
//! later serve is proven trustworthy at write time, not just read time.
//!
//! Config: the same `HUGIT_SERVE_R2_*` env the server reads. The standing engine
//! credential is READ-ONLY by design (a PUT 403s with a clear message); this tool
//! is run with the one-shot READ+WRITE grant from the CoreLink TL.
//!
//! Usage: `hugit-snapshot <event-log.json> <repo-slug>`
//!   e.g. `hugit-snapshot ./logs/hugit.json hugit`

use std::path::Path;
use std::process::ExitCode;

use hugit_serve::state::{R2Config, is_safe_repo_slug};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 3 {
        eprintln!("usage: hugit-snapshot <event-log.json> <repo-slug>");
        return ExitCode::from(2);
    }
    let (file, repo) = (&args[1], &args[2]);

    if !is_safe_repo_slug(repo) {
        eprintln!("hugit-snapshot: refusing unsafe repo slug {repo:?}");
        return ExitCode::from(2);
    }

    // 1. Read the local event-log bytes.
    let bytes = match std::fs::read(file) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("hugit-snapshot: cannot read {file}: {e}");
            return ExitCode::from(2);
        }
    };

    // 2. VERIFY before upload — never publish a log that would fail the read-path
    //    chain check (a tampered/corrupt snapshot must never reach the bucket).
    if let Err(e) = hugit_cli::checks::load_event_log_from_bytes(&bytes, Path::new(file)) {
        eprintln!(
            "hugit-snapshot: {file} did NOT pass chain verification ({}) — refusing to upload",
            e.kind()
        );
        return ExitCode::from(2);
    }

    // 3. Build the R2 config from env (one-shot RW grant) and PUT.
    let cfg = match R2Config::from_env() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("hugit-snapshot: cannot configure R2 — {e}");
            return ExitCode::from(2);
        }
    };
    match cfg.put(repo, &bytes) {
        Ok(key) => {
            println!(
                "hugit-snapshot: uploaded {} bytes (chain-verified) → {key}",
                bytes.len()
            );
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("hugit-snapshot: upload failed ({}) — {}", e.code, e.reason);
            ExitCode::FAILURE
        }
    }
}
