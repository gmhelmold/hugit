//! `hugit journal` — session journal.
//!
//! Ships `note` (`hugit note --log <path> --note <text> [--principal <p>]
//! [--workspace <id>] [--intent <id>]`), which appends a `journal.note` record
//! onto the CANONICAL `--log` (the same `[EventRecord, …]` hash chain every
//! porcelain verb writes through) — NOT a separate journal file. One source of
//! truth: a session note is a hash-chained, scrubbed, attributable event like any
//! other. The D11 in-memory `Journal` model becomes a projection over these
//! records.

use std::process::ExitCode;

use clap::Subcommand;

pub mod note;

pub use note::NoteArgs;

/// `hugit journal <subcommand>`.
#[derive(clap::Args, Debug)]
pub struct JournalArgs {
    #[command(subcommand)]
    pub command: JournalCommand,
}

/// Journal subcommand surface.
#[derive(Subcommand, Debug)]
pub enum JournalCommand {
    /// Append a session note onto the canonical log.
    Note(NoteArgs),
}

/// Dispatch a `hugit journal` subcommand.
pub fn run(args: JournalArgs) -> ExitCode {
    match args.command {
        JournalCommand::Note(a) => note::run(a),
    }
}
