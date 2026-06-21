//! hugit-invariants — namespace-law invariant (WP-X5).
//!
//! This module proves the two namespace laws hold as standing, falsifiable
//! invariants — without owning any production path. It consumes the CLI verb
//! surface (hugit-cli) and the managed-ref namespace (`refs/hugit/…`) from the
//! refstore read-only; it modifies neither.
//!
//! # The two invariants (WP-X5 owned items)
//! ① **No hugit CLI verb shadows a git verb.** The test enumerates hugit's
//!    verbs and asserts the intersection with git's command set is empty. The
//!    git verb list is generated at test time from `git --list-cmds=builtins,main`
//!    (git's OWN compiled-in + porcelain commands) — never `git help -a`, which
//!    also lists ambient external `git-*` binaries on `PATH` and so varies by
//!    machine (a CI runner's third-party `git-repo` once tripped this), and
//!    never a hand-copied list (which would rot). A new shadowing verb anywhere
//!    in hugit-cli turns this red.
//! ② **Managed refs (`refs/hugit/…`) never collide with user branches/tags.**
//!    A property test generates arbitrary user branch/tag names and asserts
//!    none can collide with the `refs/hugit/…` reserved namespace, AND that
//!    managed-ref creation never lands a ref in user space.
//!
//! Everything here is verification logic over the *consumed* surfaces — there
//! is no production behavior to ship from this module.

/// The canonical hugit CLI verb set — **re-exported from the single source of
/// truth** [`hugit_cli::HUGIT_VERBS`], never a hand-copied list.
///
/// This is the load-bearing fix for the X5 defect (brutal review R3): a local
/// hardcoded copy rots silently relative to the real CLI surface, so the
/// no-shadow oracle could pass while the binary actually dispatches a
/// git-shadowing verb the local copy never saw. By deriving the verb set from
/// the real registry the `hugit` binary dispatches on, "the oracle tests the
/// real surface" is structurally true: any git-shadowing verb added to
/// hugit-cli turns the X5① oracle red automatically — no manual sync required.
///
/// Verbs are the top-level subcommand tokens (before any sub-subcommands like
/// `ws spawn` or `ctx snap`).
pub use hugit_cli::HUGIT_VERBS;

/// The reserved managed-ref prefix. All hugit-internal refs live under this
/// path and ONLY under this path. User branches and tags MUST NOT start with
/// this prefix; hugit-managed refs MUST start with it.
pub const HUGIT_REF_PREFIX: &str = "refs/hugit/";

/// Returns `true` if `ref_name` is in the hugit-managed namespace.
///
/// A managed ref begins with [`HUGIT_REF_PREFIX`]. Any ref that does NOT begin
/// with this prefix is a user ref (branch, tag, note, etc.).
pub fn is_managed_ref(ref_name: &str) -> bool {
    ref_name.starts_with(HUGIT_REF_PREFIX)
}

/// Returns `true` if `ref_name` is a valid user ref that cannot collide with
/// the hugit-managed namespace.
///
/// A user ref is anything that does NOT start with [`HUGIT_REF_PREFIX`].
/// This covers `refs/heads/…`, `refs/tags/…`, `refs/remotes/…`, and any
/// other non-hugit-reserved ref path.
pub fn is_user_ref(ref_name: &str) -> bool {
    !ref_name.starts_with(HUGIT_REF_PREFIX)
}

/// Parse `git --list-cmds=<categories>` output into a verb set.
///
/// Unlike `git help -a` (whose output is sectioned/indented AND includes ambient
/// external `git-*` binaries found in `PATH` — e.g. a CI runner's third-party
/// `git-repo` tool, which made the namespace oracle environment-dependent),
/// `git --list-cmds=builtins,main` prints git's OWN command surface — one bare
/// command per line, deterministic across machines. Each non-empty line IS a
/// command token.
pub fn parse_git_cmd_list(list_cmds_output: &str) -> std::collections::HashSet<String> {
    list_cmds_output
        .lines()
        .map(str::trim)
        .filter(|l| {
            !l.is_empty()
                && l.starts_with(|c: char| c.is_ascii_lowercase() || c == '-')
                && !l.contains(char::is_whitespace)
                && !l.contains(':')
        })
        .map(str::to_string)
        .collect()
}
