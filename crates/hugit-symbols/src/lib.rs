//! `hugit-symbols` — the local symbol-outline producer (whitepaper "semantic
//! index", audit's W6).
//!
//! Turns a single blob's bytes into a deterministic, flat outline of its
//! top-level AND nested declarations (functions, types, traits, impls, modules,
//! consts/statics, macros, type aliases). Pure + headless: no I/O, no network,
//! no serde — it returns a plain domain type the cli/serve layers map onto the
//! frozen `hugit_http_contracts::blob::OutlineItemVm` wire shape.
//!
//! The whitepaper's grander vision — a content-addressed semantic index keyed by
//! tree-hash, parsed once per subtree across all tenants — is the AC-memoized
//! wrapper that wraps THIS pure function later (exactly as the check executor is
//! wrapped). This crate is strictly the local parse-blob-to-outline core.
//!
//! ## Safety / determinism contract (master plan C1)
//! - Deterministic: the same bytes always yield the same outline.
//! - Bounded: input above [`MAX_INPUT_BYTES`] returns an empty outline (never an
//!   unbounded multi-MB parse / OOM).
//! - Total: non-UTF-8, binary, unknown-language, or unparseable input returns an
//!   empty outline — never a panic.

mod lang;

pub use lang::{Lang, lang_for_ext};

use streaming_iterator::StreamingIterator;
use tree_sitter::{Node, Parser, Query, QueryCursor};

/// Largest blob the outliner will attempt to parse (2 MiB). Anything larger
/// returns an empty outline — the honest-default discipline (`outline: []`)
/// already used by the serve blob handler, and a hard bound against a hostile
/// multi-GB blob OOMing the parser.
pub const MAX_INPUT_BYTES: usize = 2 * 1024 * 1024;

/// The kind of a declaration. Domain enum (never a raw string) so the wire
/// vocabulary is produced in exactly one place — [`SymbolKind::as_wire_str`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SymbolKind {
    Fn,
    Struct,
    Enum,
    Trait,
    Impl,
    Mod,
    Const,
    Static,
    Macro,
    TypeAlias,
}

impl SymbolKind {
    /// The FROZEN wire `kind` token for `OutlineItemVm.kind` (master plan C1).
    ///
    /// `fn struct enum trait impl mod const static macro type` — agreed with the
    /// githugr renderer so icons/grouping match. Changing a token is a wire break.
    pub fn as_wire_str(&self) -> &'static str {
        match self {
            SymbolKind::Fn => "fn",
            SymbolKind::Struct => "struct",
            SymbolKind::Enum => "enum",
            SymbolKind::Trait => "trait",
            SymbolKind::Impl => "impl",
            SymbolKind::Mod => "mod",
            SymbolKind::Const => "const",
            SymbolKind::Static => "static",
            SymbolKind::Macro => "macro",
            SymbolKind::TypeAlias => "type",
        }
    }
}

/// One declaration of the outline. Maps 1:1 onto the four wire fields the serve
/// layer fills (`kind` via [`SymbolKind::as_wire_str`], `name`, `line`; the
/// VM's `active` cursor flag is set by the server, not the parser).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SymbolItem {
    pub kind: SymbolKind,
    pub name: String,
    /// 1-based line of the declaration's start.
    pub line: u32,
}

/// The query, embedded so the crate is self-contained (no runtime file read).
const RUST_QUERY: &str = include_str!("../queries/rust.scm");

/// Parse `source` as `lang` and return its symbol outline, ordered by source
/// appearance (declaration start byte). Deterministic; empty on
/// binary / non-UTF-8 / oversized / unparseable input (never panics).
pub fn outline_blob(lang: Lang, source: &[u8]) -> Vec<SymbolItem> {
    // Bound the input — a hostile or accidental multi-GB blob must not OOM.
    if source.len() > MAX_INPUT_BYTES {
        return Vec::new();
    }
    // tree-sitter parses UTF-8; reject non-UTF-8 (binary) up front. The check is
    // also what makes byte-offset → str slicing for names infallible below.
    let Ok(text) = std::str::from_utf8(source) else {
        return Vec::new();
    };

    match lang {
        Lang::Rust => outline_rust(text),
    }
}

/// Rust outline via tree-sitter-rust. Errors degrade to an empty outline.
fn outline_rust(text: &str) -> Vec<SymbolItem> {
    let language = tree_sitter_rust::LANGUAGE.into();

    let mut parser = Parser::new();
    if parser.set_language(&language).is_err() {
        return Vec::new();
    }
    let Some(tree) = parser.parse(text, None) else {
        return Vec::new();
    };

    let Ok(query) = Query::new(&language, RUST_QUERY) else {
        // A malformed query is a build-time bug, not a runtime input fault; still
        // degrade rather than panic to honor the totality contract.
        return Vec::new();
    };

    // Resolve the `@decl.<kind>` capture index → SymbolKind once, by name.
    let kind_of_capture: Vec<Option<SymbolKind>> = query
        .capture_names()
        .iter()
        .map(|n| kind_for_capture(n))
        .collect();
    let name_capture = query.capture_names().iter().position(|n| *n == "name");

    let src = text.as_bytes();
    let mut items: Vec<(usize, SymbolItem)> = Vec::new();

    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(&query, tree.root_node(), src);
    while let Some(m) = matches.next() {
        // The decl capture pins both the kind and the declaration's start byte;
        // the optional `name` capture pins the rendered identifier.
        let mut kind: Option<SymbolKind> = None;
        let mut decl_node: Option<Node> = None;
        let mut name_node: Option<Node> = None;

        for cap in m.captures {
            let idx = cap.index as usize;
            if Some(idx) == name_capture {
                name_node = Some(cap.node);
            } else if let Some(Some(k)) = kind_of_capture.get(idx) {
                kind = Some(*k);
                decl_node = Some(cap.node);
            }
        }

        let (Some(kind), Some(decl)) = (kind, decl_node) else {
            continue;
        };

        let name = match kind {
            // `impl` has no name field — derive "Type" or "Trait for Type".
            SymbolKind::Impl => impl_name(decl, src),
            _ => name_node
                .and_then(|n| node_text(n, src))
                .map(str::to_owned)
                .unwrap_or_default(),
        };
        if name.is_empty() {
            continue;
        }

        let line = decl.start_position().row as u32 + 1; // 1-based
        items.push((decl.start_byte(), SymbolItem { kind, name, line }));
    }

    // Deterministic order: by source position, then kind+name to break ties
    // (e.g. an impl and its first method can share a start byte across grammars).
    items.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then_with(|| a.1.kind.as_wire_str().cmp(b.1.kind.as_wire_str()))
            .then_with(|| a.1.name.cmp(&b.1.name))
    });
    items.into_iter().map(|(_, item)| item).collect()
}

/// Map a `@decl.<kind>` capture name to its [`SymbolKind`]. Non-decl captures
/// (e.g. `@name`) return `None`.
fn kind_for_capture(capture_name: &str) -> Option<SymbolKind> {
    match capture_name {
        "decl.fn" => Some(SymbolKind::Fn),
        "decl.struct" => Some(SymbolKind::Struct),
        "decl.enum" => Some(SymbolKind::Enum),
        "decl.trait" => Some(SymbolKind::Trait),
        "decl.impl" => Some(SymbolKind::Impl),
        "decl.mod" => Some(SymbolKind::Mod),
        "decl.const" => Some(SymbolKind::Const),
        "decl.static" => Some(SymbolKind::Static),
        "decl.macro" => Some(SymbolKind::Macro),
        "decl.type" => Some(SymbolKind::TypeAlias),
        _ => None,
    }
}

/// Render an `impl_item`'s name as `Type` (inherent) or `Trait for Type`.
/// Returns empty on a shape the grammar didn't give a `type:` field for.
fn impl_name(impl_node: Node, src: &[u8]) -> String {
    let ty = impl_node
        .child_by_field_name("type")
        .and_then(|n| node_text(n, src))
        .unwrap_or("");
    if ty.is_empty() {
        return String::new();
    }
    match impl_node
        .child_by_field_name("trait")
        .and_then(|n| node_text(n, src))
    {
        Some(tr) if !tr.is_empty() => format!("{tr} for {ty}"),
        _ => ty.to_owned(),
    }
}

/// UTF-8 text of a node's byte span. `outline_blob` already verified the whole
/// source is UTF-8, so the slice is always on a char boundary — but we still go
/// through `from_utf8` (returning `None` on the impossible) to never panic.
fn node_text<'a>(node: Node, src: &'a [u8]) -> Option<&'a str> {
    src.get(node.start_byte()..node.end_byte())
        .and_then(|b| std::str::from_utf8(b).ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_on_oversized_input() {
        let big = vec![b'a'; MAX_INPUT_BYTES + 1];
        assert!(outline_blob(Lang::Rust, &big).is_empty());
    }

    #[test]
    fn empty_on_non_utf8() {
        // 0xFF is never valid UTF-8.
        let bytes = [0x66u8, 0x6e, 0x20, 0xff, 0xfe, 0x00];
        assert!(outline_blob(Lang::Rust, &bytes).is_empty());
    }

    #[test]
    fn empty_on_binary_garbage() {
        let bytes: Vec<u8> = (0u8..=255).cycle().take(4096).collect();
        // Either non-UTF-8 (returns empty) or UTF-8 noise with no decls.
        assert!(outline_blob(Lang::Rust, &bytes).is_empty());
    }

    #[test]
    fn deterministic() {
        let src = b"fn a() {}\nstruct B;\nfn c() {}\n";
        assert_eq!(outline_blob(Lang::Rust, src), outline_blob(Lang::Rust, src));
    }
}
