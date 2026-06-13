//! The three-axis memo key (item ①②⑥⑦, client side).
//!
//! `key = H(tree_root ‖ check_def_digest ‖ toolchain_digest)` (whitepaper §6.2).
//! The canonical key formula is single-sourced in
//! [`hugit_refstore::compute_memo_key`] — we NEVER re-transcribe it here; we only
//! derive the three axes it consumes and call it.
//!
//! Axes:
//!   - **tree_root** — a Merkle hash over the check's INPUT subtree (the files
//!     whose path matches the `CheckDef::glob_set`). An edit inside the glob
//!     changes this; an edit outside does not (item ②).
//!   - **def_digest** — a canonical digest over the normalized definition body
//!     (`command + inputs + toolchain_ref + glob_set`). A changed definition →
//!     different digest → MISS + re-execute (item ⑦).
//!   - **toolchain_digest** — a content-addressed digest of the toolchain; a
//!     different toolchain → different digest → MISS, never a false hit (item ⑥).
//!
//! Every axis is necessary AND sufficient: changing ANY axis must change the
//! key. A false hit across any axis is a correctness fault — these functions are
//! built fail-closed and proven exhaustively in `tests/acceptance_b2a.rs`.

use std::collections::BTreeMap;

use hugit_contracts::CheckDef;
use sha2::{Digest, Sha256};

use super::glob;

/// A single file in the workspace tree: a `/`-separated relative path mapped to
/// the bytes the tree axis hashes for that file. The materializer / B3
/// affected-set produces this; B2a consumes it to derive the scoped `tree_root`.
///
/// # Mode-bit folding (N-1 stale-green close)
///
/// The tree axis MUST hash every result-affecting input of a file, and on POSIX
/// the executable / permission bits are result-affecting: a check `--cmd
/// './gate.sh'` over a `gate.sh` that is `chmod -x`'d (SAME content) flips from
/// `exit 0` to `exit 126` (permission denied). Content alone is therefore an
/// INCOMPLETE input set — dropping the mode is a STALE-GREEN hole (a warm HIT
/// served `exit 0` where the real run now fails).
///
/// To close it WITHOUT changing this type's shape (it is consumed by callers
/// across crates as a `Vec<u8>`), the snapshotter folds the POSIX mode INTO this
/// byte field via [`frame_file_with_mode`] before inserting it: the value the
/// tree hash sees is `MODE_TAG ‖ LP(mode_le) ‖ LP(content)`. Because EVERY
/// snapshot entry is framed identically, a mode change (same content) and a
/// content change both change the framed bytes → a different `tree_root` → a
/// MISS; an unchanged (same mode + content) file frames identically → a HIT
/// (hit-rate preserved). The framing is canonical (fixed tag, fixed-width
/// little-endian mode, length-prefixed), so it can never collide with a raw
/// file's bytes the way a naïve `mode ‖ bytes` concatenation could.
pub type FileContent = Vec<u8>;

/// Magic tag prefixed to every mode-framed snapshot entry so the framing is
/// self-describing and can never be confused with raw file content. Fixed bytes
/// `\0hugit-fmode\0` — a NUL-bracketed marker no shell script begins with.
const MODE_TAG: &[u8] = b"\0hugit-fmode\0";

/// Fold a file's POSIX `mode` and `content` into the single canonical byte field
/// the tree axis hashes (N-1). Layout: `MODE_TAG ‖ LP(u32_le(mode)) ‖ LP(content)`.
///
/// The full `mode` (the `st_mode` low bits, in practice the `0o7777`
/// permission/setuid/setgid/sticky bits the snapshot passes) is folded — not
/// merely the executable bit — so ANY permission change busts the memo key. On
/// non-unix the caller passes a fixed sentinel mode (the bits are not
/// meaningful there), so the framing is stable cross-platform and the Windows
/// build is unaffected.
///
/// This is the ONE place the mode↔content framing lives, so the snapshotter and
/// any future producer fold identically (no drift between producers).
pub fn frame_file_with_mode(mode: u32, content: &[u8]) -> FileContent {
    let mut framed = Vec::with_capacity(MODE_TAG.len() + 8 + content.len());
    framed.extend_from_slice(MODE_TAG);
    push_lp(&mut framed, &mode.to_le_bytes());
    push_lp(&mut framed, content);
    framed
}

/// Length-prefix a byte field exactly as the canonical format does:
/// `u32_be(len) ‖ bytes`. Mirrors `hugit_refstore`'s framing so tree hashing is
/// unambiguous (no field-boundary collisions).
fn push_lp(buf: &mut Vec<u8>, bytes: &[u8]) {
    buf.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
    buf.extend_from_slice(bytes);
}

/// Compute the **scoped tree root**: a SHA-256 Merkle hash over the subset of
/// `files` whose path matches `glob_set`, in sorted-path order.
///
/// Pre-image (per matched file, sorted by path): `LP(path) ‖ LP(content)`,
/// prefixed by `u32_be(count)`. Sorting + length-prefixing make the digest
/// canonical and collision-resistant across path/content boundaries.
///
/// Files OUTSIDE the glob are excluded by construction — so editing them cannot
/// change this digest (item ②, out → hit). Files INSIDE the glob contribute
/// both path and content — so any edit changes it (item ②, in → rerun).
pub fn scoped_tree_root<'a, I>(glob_set: &[String], files: I) -> String
where
    I: IntoIterator<Item = (&'a str, &'a FileContent)>,
{
    // Collect matched files into a sorted map for deterministic ordering.
    let matched: BTreeMap<&str, &FileContent> = files
        .into_iter()
        .filter(|(path, _)| glob::matches_any(glob_set, path))
        .collect();

    let mut buf: Vec<u8> = Vec::new();
    buf.extend_from_slice(&(matched.len() as u32).to_be_bytes());
    for (path, content) in matched {
        push_lp(&mut buf, path.as_bytes());
        push_lp(&mut buf, content.as_slice());
    }
    hex::encode(Sha256::digest(&buf))
}

/// Compute the canonical `def_digest` for a [`CheckDef`] body.
///
/// The digest covers the LOAD-BEARING definition fields in a fixed order:
/// `command`, `inputs` (as a vector), `toolchain_ref`, `glob_set` (as a vector).
/// `env_manifest` is included too (it changes the execution environment, so it
/// is part of the definition's identity). The existing `def_digest` field on the
/// incoming `CheckDef` is NOT an input (it is the output we are computing).
///
/// Framing: each scalar is `LP`-framed; each vector is `u32_be(count)` then each
/// element `LP`-framed — identical to `hugit_refstore`'s vector framing, so a
/// reordering or boundary shift cannot collide.
pub fn compute_def_digest(def: &CheckDef) -> String {
    let mut buf: Vec<u8> = Vec::new();
    push_lp(&mut buf, def.command.as_bytes());
    push_vec(&mut buf, &def.inputs);
    push_lp(&mut buf, def.toolchain_ref.as_bytes());
    push_lp(&mut buf, def.env_manifest.as_bytes());
    push_vec(&mut buf, &def.glob_set);
    hex::encode(Sha256::digest(&buf))
}

/// Length-prefixed vector framing: `u32_be(count) ‖ LP(e0) ‖ LP(e1) ‖ …`.
fn push_vec(buf: &mut Vec<u8>, items: &[String]) {
    buf.extend_from_slice(&(items.len() as u32).to_be_bytes());
    for item in items {
        push_lp(buf, item.as_bytes());
    }
}

/// Return a [`CheckDef`] with its `def_digest` field recomputed canonically.
/// Use this to normalize a parsed/authored def so its self-described digest
/// matches the body (a def whose `def_digest` disagrees with its body is
/// invalid — see [`super::parser`]).
pub fn with_canonical_def_digest(mut def: CheckDef) -> CheckDef {
    def.def_digest = compute_def_digest(&def);
    def
}

/// Derive the full three-axis memo key for a check over a given workspace tree
/// and toolchain. This is the single entry point the client uses before an AC
/// lookup.
///
/// - `def` supplies axis 2 (its body → `def_digest`, recomputed canonically so
///   a stale self-reported digest can never cause a false hit).
/// - `files` + `def.glob_set` supply axis 1 (the scoped `tree_root`).
/// - `toolchain_digest` supplies axis 3 (passed through, content-addressed).
///
/// Delegates the final key to [`hugit_refstore::compute_memo_key`] — the only
/// place the key formula lives.
pub fn derive_memo_key<'a, I>(def: &CheckDef, files: I, toolchain_digest: &str) -> String
where
    I: IntoIterator<Item = (&'a str, &'a FileContent)>,
{
    let tree_root = scoped_tree_root(&def.glob_set, files);
    let def_digest = compute_def_digest(def);
    hugit_refstore::compute_memo_key(&tree_root, &def_digest, toolchain_digest)
}
