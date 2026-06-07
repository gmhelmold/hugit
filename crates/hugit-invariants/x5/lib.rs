//! hugit-invariants — Squad-X namespace-law invariant (WP-X5).
//!
//! This module proves the two namespace laws hold as standing, falsifiable
//! invariants — without owning any production path. It consumes the CLI verb
//! surface (hugit-cli) and the managed-ref namespace (`refs/hugit/…`) from the
//! refstore read-only; it modifies neither.
//!
//! # The two invariants (WP-X5 owned items)
//! ① **No hugit CLI verb shadows a git verb.** The test enumerates hugit's
//!    verbs and asserts the intersection with `git help -a`'s verb set is
//!    empty. The git verb list is generated from `git help -a` at test time —
//!    never a hand-copied list (which would rot). A new shadowing verb
//!    anywhere in hugit-cli turns this red.
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

/// Parse `git help -a` output and extract the verb tokens.
///
/// The output format is lines with leading whitespace followed by the verb
/// name and a description. This function extracts only the first token from
/// each non-empty, non-header line (lines that start with whitespace and have
/// a lowercase verb token).
pub fn parse_git_verbs(git_help_output: &str) -> std::collections::HashSet<String> {
    git_help_output
        .lines()
        .filter_map(|line| {
            // Lines that list verbs start with whitespace and have a lowercase token.
            let trimmed = line.trim_start();
            if trimmed.is_empty() || !line.starts_with(' ') && !line.starts_with('\t') {
                return None;
            }
            // The verb is the first whitespace-delimited token on the line.
            let verb = trimmed.split_whitespace().next()?;
            // Only include lines that look like verb entries (lowercase start, no colon).
            if verb.starts_with(|c: char| c.is_ascii_lowercase() || c == '-') && !verb.contains(':')
            {
                Some(verb.to_string())
            } else {
                None
            }
        })
        .collect()
}
