//! WP0 — the shared CAS commit-metadata helper.
//!
//! Decode a commit object's machine-altitude fields DIRECTLY from the CoreLink CAS
//! via `gix_object::CommitRef` — the SAME canonical decoder `hugit_proto` uses
//! (`read::history::load_commit`, `commit_root_tree`); the commit byte format is
//! never reimplemented. This is the ONE seam both the `/commits` list (WP2) and the
//! `/commit/{sha}` detail (WP3) read a git-pushed-but-never-landed repo through, so
//! a repo whose event log is EMPTY still renders REAL git data.
//!
//! ## Honesty (the whole point)
//! NOTHING is written to the event log here — this is a pure READ. Every field is
//! REAL (read from the commit bytes) or the whole decode is `None` (a missing /
//! non-commit / undecodable object → honest-partial: the caller drops the row or
//! 404s). It NEVER errors, NEVER panics, and NEVER fabricates a landing / cost /
//! check / synthetic intent (the forbidden design-(a) trap).

use gix_hash::ObjectId;
use hugit_proto::{ObjectKind, ObjectSource};

/// The commit-altitude fields a git-only projection needs, decoded once per commit.
/// Every field is REAL git data or the decode returns `None`.
pub(crate) struct CommitMeta {
    /// `name <email>` VERBATIM from the commit's author signature. Free text → the
    /// read boundary MUST scrub it (an author string could carry a secret-shaped
    /// value); this layer does not redact.
    pub author: String,
    /// The FULL commit message VERBATIM. Free text → the read boundary scrubs it;
    /// callers take the first line for a row subject / detail title.
    pub message: String,
    /// The author date as a Unix-epoch millisecond timestamp (the REAL commit time
    /// — NEVER the log `recorded_at`). A malformed time header → `0` (honest
    /// "unknown time", never a fabricated date). Mirrors
    /// `hugit_proto::read::history`'s decode exactly.
    pub commit_time_ms: u64,
    /// The parent oids in order; `parents[0]` is the first parent (`^1`) — the
    /// spine of the bounded first-parent walk.
    pub parents: Vec<ObjectId>,
}

/// Decode a commit object's [`CommitMeta`] straight from the CAS `src`.
///
/// `None` when the object is absent, is not a commit, or is undecodable — the
/// honest-partial signal the callers treat as "drop this row" / "404, no oracle".
/// NEVER an error, NEVER a fabrication.
pub(crate) fn commit_meta_from_cas(src: &dyn ObjectSource, oid: &ObjectId) -> Option<CommitMeta> {
    let object = src.get(oid).ok().flatten()?;
    if object.kind != ObjectKind::Commit {
        return None;
    }
    // The canonical decoder — mirrors `hugit_proto::read::history::load_commit`.
    let commit = gix_object::CommitRef::from_bytes(&object.data).ok()?;

    // The author signature (`name`, `email`, `time`). A malformed author header →
    // `None` (stop / 404; never fabricate an identity).
    let author = commit.author().ok()?;
    let author_str = format!("{} <{}>", author.name, author.email);
    // Author time as Unix-epoch ms; a malformed time → 0. `seconds` is i64 epoch
    // seconds — clamp a negative (pre-1970) time to 0 rather than underflow.
    let commit_time_ms = author
        .time()
        .map(|t| {
            u64::try_from(t.seconds.max(0))
                .unwrap_or(0)
                .saturating_mul(1_000)
        })
        .unwrap_or(0);

    let message = String::from_utf8_lossy(commit.message.as_ref()).into_owned();
    let parents = commit.parents().collect();

    Some(CommitMeta {
        author: author_str,
        message,
        commit_time_ms,
        parents,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use hugit_proto::{CasObjectSource, ObjectKind};

    /// A commit object built from raw bytes (mirrors the diff/history test doubles).
    fn insert_commit(
        src: &mut CasObjectSource,
        parent: Option<ObjectId>,
        author: &str,
        time_secs: i64,
        message: &str,
    ) -> ObjectId {
        // The tree oid is irrelevant to the metadata decode; use the empty tree.
        let tree = ObjectId::empty_tree(gix_hash::Kind::Sha1);
        let mut body = format!("tree {tree}\n");
        if let Some(p) = parent {
            body.push_str(&format!("parent {p}\n"));
        }
        body.push_str(&format!(
            "author {author} {time_secs} +0000\ncommitter {author} {time_secs} +0000\n\n{message}\n"
        ));
        src.insert_raw(ObjectKind::Commit, body.into_bytes())
    }

    #[test]
    fn decodes_real_author_message_time_parents() {
        let mut src = CasObjectSource::new();
        let root = insert_commit(&mut src, None, "Ana <a@x>", 1000, "root");
        let head = insert_commit(&mut src, Some(root), "Bob <b@x>", 2000, "second\n\nbody");

        let meta = commit_meta_from_cas(&src, &head).expect("commit decodes");
        assert_eq!(meta.author, "Bob <b@x>");
        // FULL message (subject + body), never truncated by the helper.
        assert_eq!(meta.message, "second\n\nbody\n");
        assert_eq!(meta.commit_time_ms, 2_000_000);
        assert_eq!(meta.parents, vec![root]);
    }

    #[test]
    fn missing_object_is_none_never_error() {
        let src = CasObjectSource::new();
        let phantom = ObjectId::from_hex(b"0123456789012345678901234567890123456789").unwrap();
        assert!(commit_meta_from_cas(&src, &phantom).is_none());
    }

    #[test]
    fn non_commit_object_is_none() {
        let mut src = CasObjectSource::new();
        let blob = src.insert_raw(ObjectKind::Blob, b"not a commit".to_vec());
        assert!(commit_meta_from_cas(&src, &blob).is_none());
    }
}
