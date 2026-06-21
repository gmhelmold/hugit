//! `hugit note --log <path> --note <text> …` — append a session note onto the
//! canonical event log.
//!
//! Git-proximity cleanup: this was `hugit journal note`. A session note is a
//! single, frequent action — it does not need a `journal` namespace wrapper, so
//! it graduated to the top-level `note` verb. The on-wire event kind stays
//! `journal.note` (a frozen contract the serve surface and projections read), and
//! the append logic is unchanged — this is purely the CLI token.

use std::process::ExitCode;

// The note logic lives next to its `journal.note` producer; the verb is just a
// re-export so the implementation (scrub-on-append, D14 guard, atomic persist)
// is single-sourced.
pub use crate::journal::note::{JOURNAL_NOTE_KIND, NoteArgs};

/// Run `hugit note` — append a `journal.note` record onto the canonical log.
pub fn run(args: NoteArgs) -> ExitCode {
    crate::journal::note::run(args)
}
