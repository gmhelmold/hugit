//! A small, deterministic glob matcher for scoping a check's input subtree.
//!
//! B2a needs to decide, for a given file path, whether it falls inside a
//! [`CheckDef`](hugit_contracts::CheckDef)'s `glob_set` — that decision scopes
//! the `tree_root` axis of the memo key (item ②: an edit INSIDE the glob must
//! change the key → rerun; an edit OUTSIDE must leave it unchanged → hit).
//!
//! Rather than pull a glob crate (no new workspace dep), we implement the
//! minimal, fully-tested subset the check format needs:
//!   - `*`  matches any run of characters **except** `/` (single path segment)
//!   - `**` matches any run of characters **including** `/` (any depth)
//!   - `?`  matches exactly one character except `/`
//!   - everything else is a literal (including `.`)
//!
//! Paths are matched as `/`-separated UTF-8 strings with no leading `/`.
//! Matching is anchored at both ends (the whole path must match the pattern).

/// Returns true iff `path` matches `pattern` under the glob subset above.
pub fn glob_match(pattern: &str, path: &str) -> bool {
    matches_at(pattern.as_bytes(), path.as_bytes())
}

/// Returns true iff `path` matches ANY pattern in `glob_set`. An empty
/// `glob_set` matches NOTHING (a check that declares no inputs scopes an empty
/// subtree — its tree axis is the hash of "no files", which is still stable).
pub fn matches_any(glob_set: &[String], path: &str) -> bool {
    glob_set.iter().any(|p| glob_match(p, path))
}

/// Recursive matcher over byte slices. `**` is the only construct that may
/// consume `/`; `*` and `?` never cross a segment boundary.
fn matches_at(pat: &[u8], text: &[u8]) -> bool {
    // Fast exit on exhausted pattern.
    if pat.is_empty() {
        return text.is_empty();
    }

    match pat[0] {
        b'*' => {
            // Distinguish `**` (cross-segment) from `*` (within-segment).
            if pat.len() >= 2 && pat[1] == b'*' {
                // `**/` is the canonical "zero or more directories" construct:
                // it must ALSO match the empty prefix (so `**/*.rs` matches a
                // top-level `x.rs`). Try skipping the trailing `/` first.
                if pat.len() >= 3 && pat[2] == b'/' && matches_at(&pat[3..], text) {
                    return true;
                }
                let rest = &pat[2..];
                // `**` can consume zero or more characters of ANY kind.
                // Try the shortest first, then extend one byte at a time.
                let mut i = 0;
                loop {
                    if matches_at(rest, &text[i..]) {
                        return true;
                    }
                    if i >= text.len() {
                        return false;
                    }
                    i += 1;
                }
            } else {
                let rest = &pat[1..];
                // `*` consumes zero or more NON-`/` characters.
                let mut i = 0;
                loop {
                    if matches_at(rest, &text[i..]) {
                        return true;
                    }
                    if i >= text.len() || text[i] == b'/' {
                        return false;
                    }
                    i += 1;
                }
            }
        }
        b'?' => {
            if let Some((&c, tail)) = text.split_first()
                && c != b'/'
            {
                return matches_at(&pat[1..], tail);
            }
            false
        }
        lit => {
            if let Some((&c, tail)) = text.split_first()
                && c == lit
            {
                return matches_at(&pat[1..], tail);
            }
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn literal_and_star() {
        assert!(glob_match("src/main.rs", "src/main.rs"));
        assert!(!glob_match("src/main.rs", "src/lib.rs"));
        assert!(glob_match("src/*.rs", "src/main.rs"));
        assert!(!glob_match("src/*.rs", "src/sub/main.rs")); // `*` no `/`
    }

    #[test]
    fn double_star_crosses_segments() {
        assert!(glob_match("src/**/*.rs", "src/a/b/c.rs"));
        assert!(glob_match("**/*.rs", "x.rs"));
        assert!(glob_match("**", "anything/at/all.txt"));
        assert!(!glob_match("docs/**", "src/x.rs"));
    }

    #[test]
    fn question_mark() {
        assert!(glob_match("a?c", "abc"));
        assert!(!glob_match("a?c", "a/c"));
        assert!(!glob_match("a?c", "ac"));
    }

    #[test]
    fn empty_set_matches_nothing() {
        assert!(!matches_any(&[], "anything"));
        assert!(matches_any(&["*.rs".to_string()], "x.rs"));
    }
}
