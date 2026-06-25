//! `hugit-symbols` — the local symbol-outline producer (whitepaper "semantic
//! index", audit's W6).
//!
//! Turns a single blob's bytes into a deterministic, flat outline of its
//! top-level AND nested declarations (functions, types, classes, methods,
//! modules, consts, macros, type aliases, …) across the supported source
//! languages. Pure + headless: no I/O, no network, no serde — it returns a
//! plain domain type the cli/serve layers map onto the frozen
//! `hugit_http_contracts::blob::OutlineItemVm` wire shape.
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
//!
//! ## Architecture
//! Every language flows through one shared engine, [`outline_with`]: parse with
//! the grammar's `LANGUAGE`, run that language's `.scm` query, and for each match
//! hand the matched `@decl.<kind>` capture tag + the decl node + the optional
//! `@name` node to a per-language [`Classifier`] closure that returns the FINAL
//! `(SymbolKind, name)` (or `None` to drop the match). The query-can't-express-it
//! refinements — C++ callable disambiguation, Python/Ruby method-vs-function by
//! enclosing scope, Go `type:`-child struct/interface/type, JS arrow-vs-variable,
//! C name-by-declarator-descent — all live in those closures.

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
///
/// The first block is the Rust-era vocabulary (frozen); the second block adds
/// the cross-language tokens (TypeScript / JavaScript / Python / Go / Java / C /
/// C++ / Ruby). `Fn`→`fn` and `Function`→`function` are deliberately distinct
/// tokens (Rust emits `fn`, the curly-brace family emits `function`); likewise
/// `Mod`→`mod` (Rust) vs `Module`→`module` (Ruby).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SymbolKind {
    // Rust-era vocabulary (frozen — do not change a token, it is a wire break).
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
    // Cross-language vocabulary (frozen). Each maps onto a distinct wire token
    // agreed with the githugr renderer.
    Function,
    Method,
    Constructor,
    Class,
    Interface,
    Namespace,
    Field,
    Variable,
    Constant,
    Module,
}

impl SymbolKind {
    /// The FROZEN wire `kind` token for `OutlineItemVm.kind` (master plan C1).
    ///
    /// Agreed with the githugr renderer so icons/grouping match across every
    /// language. Changing a token is a wire break.
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
            SymbolKind::Function => "function",
            SymbolKind::Method => "method",
            SymbolKind::Constructor => "constructor",
            SymbolKind::Class => "class",
            SymbolKind::Interface => "interface",
            SymbolKind::Namespace => "namespace",
            SymbolKind::Field => "field",
            SymbolKind::Variable => "variable",
            SymbolKind::Constant => "constant",
            SymbolKind::Module => "module",
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

// The queries, embedded so the crate is self-contained (no runtime file read).
const RUST_QUERY: &str = include_str!("../queries/rust.scm");
const TYPESCRIPT_QUERY: &str = include_str!("../queries/typescript.scm");
const JAVASCRIPT_QUERY: &str = include_str!("../queries/javascript.scm");
const PYTHON_QUERY: &str = include_str!("../queries/python.scm");
const GO_QUERY: &str = include_str!("../queries/go.scm");
const JAVA_QUERY: &str = include_str!("../queries/java.scm");
const C_QUERY: &str = include_str!("../queries/c.scm");
const CPP_QUERY: &str = include_str!("../queries/cpp.scm");
const RUBY_QUERY: &str = include_str!("../queries/ruby.scm");

/// The per-language decision the query cannot express on its own: given the
/// matched `@decl.<tag>` capture name, the declaration node, the optional
/// `@name` node, and the source bytes, return the FINAL `(kind, name)` to emit —
/// or `None` to drop the match. Every language-specific refinement lives in one
/// of these closures; the shared [`outline_with`] engine is otherwise identical.
type Classifier<'a> = dyn Fn(&str, Node, Option<Node>, &[u8]) -> Option<(SymbolKind, String)> + 'a;

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

    // Honor the totality contract (this fn's doc says "never panics"): the
    // tree-sitter parse / query / classifier path can panic on a pathological
    // input. Catch it and degrade to an empty outline (the honest default),
    // never letting a parser panic escape to the caller.
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        outline_for_lang(lang, text)
    }))
    .unwrap_or_default()
}

/// Dispatch `text` to the per-language [`outline_with`] engine. Split out from
/// [`outline_blob`] so the totality `catch_unwind` wraps exactly the parse path.
fn outline_for_lang(lang: Lang, text: &str) -> Vec<SymbolItem> {
    match lang {
        Lang::Rust => outline_with(
            text,
            tree_sitter_rust::LANGUAGE.into(),
            RUST_QUERY,
            &classify_rust,
        ),
        Lang::TypeScript => outline_with(
            text,
            tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            TYPESCRIPT_QUERY,
            &classify_simple,
        ),
        Lang::Tsx => outline_with(
            text,
            tree_sitter_typescript::LANGUAGE_TSX.into(),
            TYPESCRIPT_QUERY,
            &classify_simple,
        ),
        Lang::JavaScript => outline_with(
            text,
            tree_sitter_javascript::LANGUAGE.into(),
            JAVASCRIPT_QUERY,
            &classify_javascript,
        ),
        Lang::Python => outline_with(
            text,
            tree_sitter_python::LANGUAGE.into(),
            PYTHON_QUERY,
            &classify_python,
        ),
        Lang::Go => outline_with(
            text,
            tree_sitter_go::LANGUAGE.into(),
            GO_QUERY,
            &classify_go,
        ),
        Lang::Java => outline_with(
            text,
            tree_sitter_java::LANGUAGE.into(),
            JAVA_QUERY,
            &classify_simple,
        ),
        Lang::C => outline_with(text, tree_sitter_c::LANGUAGE.into(), C_QUERY, &classify_c),
        Lang::Cpp => outline_with(
            text,
            tree_sitter_cpp::LANGUAGE.into(),
            CPP_QUERY,
            &classify_cpp,
        ),
        Lang::Ruby => outline_with(
            text,
            tree_sitter_ruby::LANGUAGE.into(),
            RUBY_QUERY,
            &classify_ruby,
        ),
    }
}

/// The shared, language-agnostic outline engine: parse `text` with `language`,
/// run `query_src`, and for every match resolve the `@decl.<tag>` capture (the
/// kind tag + the declaration node, which pins the start byte) plus the optional
/// `@name` capture, then delegate the FINAL `(kind, name)` to `classify`. Honors
/// the totality contract — any parse / query failure degrades to an empty
/// outline. Output is deterministic, ordered by source position.
fn outline_with(
    text: &str,
    language: tree_sitter::Language,
    query_src: &str,
    classify: &Classifier<'_>,
) -> Vec<SymbolItem> {
    let mut parser = Parser::new();
    if parser.set_language(&language).is_err() {
        return Vec::new();
    }
    let Some(tree) = parser.parse(text, None) else {
        return Vec::new();
    };

    let Ok(query) = Query::new(&language, query_src) else {
        // A malformed query is a build-time bug, not a runtime input fault; still
        // degrade rather than panic to honor the totality contract.
        return Vec::new();
    };

    let capture_names = query.capture_names();
    let name_capture = capture_names.iter().position(|n| *n == "name");

    let src = text.as_bytes();
    let mut items: Vec<(usize, SymbolItem)> = Vec::new();

    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(&query, tree.root_node(), src);
    while let Some(m) = matches.next() {
        // The `@decl.<tag>` capture pins both the raw kind tag and the
        // declaration's start byte; the optional `@name` capture pins the
        // identifier text.
        let mut decl_tag: Option<&str> = None;
        let mut decl_node: Option<Node> = None;
        let mut name_node: Option<Node> = None;

        for cap in m.captures {
            let idx = cap.index as usize;
            if Some(idx) == name_capture {
                name_node = Some(cap.node);
            } else if let Some(tag) = capture_names.get(idx).copied() {
                // Any non-`name` capture is a `@decl.*` tag; the query never
                // declares another capture class.
                decl_tag = Some(tag);
                decl_node = Some(cap.node);
            }
        }

        let (Some(tag), Some(decl)) = (decl_tag, decl_node) else {
            continue;
        };

        let Some((kind, name)) = classify(tag, decl, name_node, src) else {
            continue;
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

// ─────────────────────────── shared helpers ────────────────────────────────

/// UTF-8 text of a node's byte span. `outline_blob` already verified the whole
/// source is UTF-8, so the slice is always on a char boundary — but we still go
/// through `from_utf8` (returning `None` on the impossible) to never panic.
fn node_text<'a>(node: Node, src: &'a [u8]) -> Option<&'a str> {
    src.get(node.start_byte()..node.end_byte())
        .and_then(|b| std::str::from_utf8(b).ok())
}

/// The `@name` capture's text, or an empty string if absent.
fn name_text(name_node: Option<Node>, src: &[u8]) -> String {
    name_node
        .and_then(|n| node_text(n, src))
        .unwrap_or("")
        .to_owned()
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

// ───────────────────────── per-language classifiers ─────────────────────────

/// The default classifier: map a `@decl.<kind>` tag straight to its
/// [`SymbolKind`] and take the name from the `@name` capture. Used by languages
/// whose query already pins the final kind (TypeScript / TSX, Java).
fn classify_simple(
    tag: &str,
    _decl: Node,
    name_node: Option<Node>,
    src: &[u8],
) -> Option<(SymbolKind, String)> {
    let kind = simple_kind(tag)?;
    Some((kind, name_text(name_node, src)))
}

/// The shared `@decl.<kind>` → [`SymbolKind`] table for the languages whose
/// query pins the final kind directly (every tag that is not a refinement
/// placeholder). Unknown / refinement tags return `None`.
fn simple_kind(tag: &str) -> Option<SymbolKind> {
    match tag {
        // Rust.
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
        // Cross-language.
        "decl.function" => Some(SymbolKind::Function),
        "decl.method" => Some(SymbolKind::Method),
        "decl.constructor" => Some(SymbolKind::Constructor),
        "decl.class" => Some(SymbolKind::Class),
        "decl.interface" => Some(SymbolKind::Interface),
        "decl.namespace" => Some(SymbolKind::Namespace),
        "decl.field" => Some(SymbolKind::Field),
        "decl.variable" => Some(SymbolKind::Variable),
        "decl.constant" => Some(SymbolKind::Constant),
        "decl.module" => Some(SymbolKind::Module),
        _ => None,
    }
}

/// Rust: the simple table, except `impl` has no name field — derive
/// "Type" / "Trait for Type" from the decl node's children.
fn classify_rust(
    tag: &str,
    decl: Node,
    name_node: Option<Node>,
    src: &[u8],
) -> Option<(SymbolKind, String)> {
    let kind = simple_kind(tag)?;
    let name = match kind {
        SymbolKind::Impl => impl_name(decl, src),
        _ => name_text(name_node, src),
    };
    Some((kind, name))
}

// ── JavaScript ──────────────────────────────────────────────────────────────

/// JavaScript. Beyond the simple kinds:
/// - `decl.method` named `constructor` → [`SymbolKind::Constructor`];
/// - `decl.lexical` (`const`/`let`) and `decl.var` (`var`): a binding whose
///   value is an arrow / function expression → [`SymbolKind::Function`];
///   otherwise a `const` binding → [`SymbolKind::Const`] and a `let`/`var`
///   binding → [`SymbolKind::Variable`].
fn classify_javascript(
    tag: &str,
    decl: Node,
    name_node: Option<Node>,
    src: &[u8],
) -> Option<(SymbolKind, String)> {
    let name = name_text(name_node, src);
    let kind = match tag {
        "decl.function" => SymbolKind::Function,
        "decl.class" => SymbolKind::Class,
        "decl.field" => SymbolKind::Field,
        "decl.method" => {
            if name == "constructor" {
                SymbolKind::Constructor
            } else {
                SymbolKind::Method
            }
        }
        "decl.lexical" | "decl.var" => {
            if binding_is_function(decl) {
                SymbolKind::Function
            } else if tag == "decl.lexical" && lexical_is_const(decl) {
                SymbolKind::Const
            } else {
                SymbolKind::Variable
            }
        }
        _ => return None,
    };
    Some((kind, name))
}

/// True if a `lexical_declaration`/`variable_declaration` binds an arrow or
/// classic function expression (so it reads as a `function` to a human).
fn binding_is_function(decl: Node) -> bool {
    let mut c = decl.walk();
    for child in decl.named_children(&mut c) {
        if child.kind() == "variable_declarator"
            && let Some(value) = child.child_by_field_name("value")
        {
            let vk = value.kind();
            if vk == "arrow_function" || vk == "function_expression" || vk == "function" {
                return true;
            }
        }
    }
    false
}

/// True if a `lexical_declaration`'s keyword is `const` (vs `let`). The keyword
/// is the declaration's first child (an anonymous `const`/`let` token).
fn lexical_is_const(decl: Node) -> bool {
    let mut c = decl.walk();
    for child in decl.children(&mut c) {
        if child.kind() == "const" {
            return true;
        }
        if child.kind() == "let" {
            return false;
        }
    }
    false
}

// ── Python ──────────────────────────────────────────────────────────────────

/// Python. The query captures `@decl.fn` (every `function_definition`) and
/// `@decl.const` (every identifier `assignment`) generically; refine here:
/// - a `@decl.fn` whose nearest enclosing *definition* is a `class_definition`
///   is a `method`, not a free function;
/// - a `@decl.const` is kept only when its name is SCREAMING_SNAKE_CASE *and* it
///   is a direct statement of a module or class body (never a local binding).
fn classify_python(
    tag: &str,
    decl: Node,
    name_node: Option<Node>,
    src: &[u8],
) -> Option<(SymbolKind, String)> {
    let name = name_text(name_node, src);
    if name.is_empty() {
        return None;
    }
    match tag {
        "decl.class" => Some((SymbolKind::Class, name)),
        "decl.fn" => {
            let kind = if py_enclosed_by_class(decl) {
                SymbolKind::Method
            } else {
                SymbolKind::Fn
            };
            Some((kind, name))
        }
        "decl.const" => {
            if py_is_screaming_snake(&name) && py_assignment_is_module_or_class_level(decl) {
                Some((SymbolKind::Const, name))
            } else {
                None
            }
        }
        _ => None,
    }
}

/// True if the nearest enclosing *definition* of `node` (a `function_definition`)
/// is a `class_definition` — i.e. the function is a method, not a free function
/// (and not a closure/local nested in another `def`). Walks the ancestor chain,
/// stopping at the first `class_definition` (method) or `function_definition`
/// (a nested function — NOT a method even if a class is further up).
fn py_enclosed_by_class(node: Node) -> bool {
    let mut cur = node.parent();
    while let Some(n) = cur {
        match n.kind() {
            "class_definition" => return true,
            "function_definition" => return false,
            _ => {}
        }
        cur = n.parent();
    }
    false
}

/// True if `name` is SCREAMING_SNAKE_CASE (`^[A-Z_][A-Z0-9_]*$`) — the Python
/// module-constant convention. Hand-rolled (no regex dep) and ASCII-only by
/// design: a constant name with non-ASCII is treated as a plain variable.
fn py_is_screaming_snake(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c.is_ascii_uppercase() || c == '_' => {}
        _ => return false,
    }
    name.chars()
        .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}

/// True if the `assignment` node sits as a direct statement of a module or class
/// body — i.e. a module-level or class-level binding, never a local inside a
/// function.
fn py_assignment_is_module_or_class_level(assignment: Node) -> bool {
    // assignment -> expression_statement -> (module | block)
    let Some(stmt) = assignment.parent() else {
        return false;
    };
    if stmt.kind() != "expression_statement" {
        return false;
    }
    match stmt.parent() {
        None => false,
        Some(p) => match p.kind() {
            "module" => true,
            // A `block` is a class body iff its parent is a class_definition;
            // a function body block's parent is a function_definition (reject).
            "block" => p
                .parent()
                .map(|gp| gp.kind() == "class_definition")
                .unwrap_or(false),
            _ => false,
        },
    }
}

// ── Go ──────────────────────────────────────────────────────────────────────

/// Go. A `type_spec`/`type_alias` is captured generically as `@decl.go_type`;
/// refine struct / interface / type by inspecting the decl's `type:` child (a
/// `type_alias` — `type T = U` — has no such struct/interface body, so it stays
/// the generic `type`). Every other tag maps via the simple table.
fn classify_go(
    tag: &str,
    decl: Node,
    name_node: Option<Node>,
    src: &[u8],
) -> Option<(SymbolKind, String)> {
    if tag == "decl.go_type" {
        let kind = match decl.child_by_field_name("type").map(|b| b.kind()) {
            Some("struct_type") => SymbolKind::Struct,
            Some("interface_type") => SymbolKind::Interface,
            _ => SymbolKind::TypeAlias,
        };
        return Some((kind, name_text(name_node, src)));
    }
    let kind = simple_kind(tag)?;
    Some((kind, name_text(name_node, src)))
}

// ── C ───────────────────────────────────────────────────────────────────────

/// C. A function/typedef name is buried inside the `declarator` subtree (there
/// is no `name:` field), so derive it by descent; tagged struct/union/enum +
/// macros carry a `@name` capture and use the simple table.
fn classify_c(
    tag: &str,
    decl: Node,
    name_node: Option<Node>,
    src: &[u8],
) -> Option<(SymbolKind, String)> {
    let kind = simple_kind(tag)?;
    let name = match kind {
        SymbolKind::Function | SymbolKind::TypeAlias => c_declarator_name(decl, src),
        _ => name_text(name_node, src),
    };
    Some((kind, name))
}

/// Derive a C declaration's name by descending its `declarator` field to the
/// innermost identifier. C buries the introduced name under pointer/array/
/// function declarators. We walk the `declarator` field at each level (bounded
/// to avoid a pathological deep tree) and, failing that, the first
/// identifier-like descendant. Returns empty on a shape with no derivable name.
fn c_declarator_name(node: Node, src: &[u8]) -> String {
    let mut cur = node;
    for _ in 0..64 {
        match cur.kind() {
            "identifier" | "field_identifier" | "type_identifier" => {
                return node_text(cur, src).unwrap_or("").to_owned();
            }
            _ => {}
        }
        if let Some(next) = cur.child_by_field_name("declarator") {
            cur = next;
            continue;
        }
        break;
    }
    first_identifier(node, src, 0).unwrap_or_default()
}

/// First `identifier`/`field_identifier`/`type_identifier` in a bounded pre-order
/// walk of `node`. Bounded depth keeps the totality guarantee against a hostile
/// deeply-nested tree.
fn first_identifier(node: Node, src: &[u8], depth: u32) -> Option<String> {
    if depth > 64 {
        return None;
    }
    match node.kind() {
        "identifier" | "field_identifier" | "type_identifier" => {
            return Some(node_text(node, src).unwrap_or("").to_owned());
        }
        _ => {}
    }
    let mut walker = node.walk();
    for child in node.named_children(&mut walker) {
        if let Some(found) = first_identifier(child, src, depth + 1) {
            return Some(found);
        }
    }
    None
}

// ── C++ ───────────────────────────────────────────────────────────────────────

/// C++. The `@decl.cpp_callable` capture binds a `function_declarator` whose
/// kind (function / method / constructor) AND name are both derived from the
/// declarator shape — see [`refine_cpp_callable`]. Every other tag maps via the
/// simple table (a `union` aggregate is captured as `@decl.struct`, an alias as
/// `@decl.type`).
fn classify_cpp(
    tag: &str,
    decl: Node,
    name_node: Option<Node>,
    src: &[u8],
) -> Option<(SymbolKind, String)> {
    if tag == "decl.cpp_callable" {
        return refine_cpp_callable(decl, src);
    }
    let kind = simple_kind(tag)?;
    Some((kind, name_text(name_node, src)))
}

/// Resolve a C++ `function_declarator`'s `(SymbolKind, display name)` from the
/// declarator shape — the one place that distinguishes function / method /
/// constructor, which the grammar cannot in a single query pattern.
///
/// Name-node cases (the function_declarator's `declarator:` field):
/// - `identifier` → free function, unless the enclosing declaration has NO
///   return-type field (a C++ constructor has none) → constructor.
/// - `field_identifier` → in-class method.
/// - `destructor_name` → destructor → `constructor` kind, name `~Type`.
/// - `qualified_identifier` (out-of-line `Scope::name`) → method, unless
///   `Scope == name` (a constructor) or the name is a `destructor_name` (a
///   destructor) → constructor.
///
/// Returns `None` (skipped, never panics) for a shape with no resolvable name.
fn refine_cpp_callable(fdecl: Node, src: &[u8]) -> Option<(SymbolKind, String)> {
    let name_node = fdecl.child_by_field_name("declarator")?;
    match name_node.kind() {
        "identifier" => {
            let name = node_text(name_node, src)?.to_owned();
            // A constructor declaration/definition carries no return `type` field
            // on its enclosing declaration; a free function (proto or def) always
            // does. Walk up past pointer_/reference_declarator wrappers to reach
            // that declaration node.
            let decl = enclosing_cpp_declaration(fdecl);
            let has_type = decl.and_then(|p| p.child_by_field_name("type")).is_some();
            let kind = if has_type {
                SymbolKind::Function
            } else {
                SymbolKind::Constructor
            };
            Some((kind, name))
        }
        "field_identifier" => {
            let name = node_text(name_node, src)?.to_owned();
            Some((SymbolKind::Method, name))
        }
        "destructor_name" => {
            // `~Type` — a destructor, rendered with the leading `~`.
            let name = node_text(name_node, src)?.to_owned();
            Some((SymbolKind::Constructor, name))
        }
        "qualified_identifier" => {
            let inner = name_node.child_by_field_name("name")?;
            let scope = name_node
                .child_by_field_name("scope")
                .and_then(|n| node_text(n, src));
            match inner.kind() {
                "destructor_name" => {
                    let name = node_text(inner, src)?.to_owned();
                    Some((SymbolKind::Constructor, name))
                }
                _ => {
                    let name = node_text(inner, src)?.to_owned();
                    // `Scope::name` where the unqualified name equals the scope
                    // is an out-of-line constructor (`Point::Point`).
                    let kind = if scope == Some(name.as_str()) {
                        SymbolKind::Constructor
                    } else {
                        SymbolKind::Method
                    };
                    Some((kind, name))
                }
            }
        }
        _ => None,
    }
}

/// Walk up from a `function_declarator` past any `pointer_declarator` /
/// `reference_declarator` wrappers (a pointer/reference return type) to the
/// enclosing `function_definition` / `declaration` / `field_declaration` that
/// carries the return-`type` field. Returns `None` if no such ancestor exists.
fn enclosing_cpp_declaration(fdecl: Node) -> Option<Node> {
    let mut node = fdecl.parent()?;
    while matches!(node.kind(), "pointer_declarator" | "reference_declarator") {
        node = node.parent()?;
    }
    Some(node)
}

// ── Ruby ──────────────────────────────────────────────────────────────────────

/// Ruby. `class` → class, `module` → module; a plain `method` (`def x`) → method
/// when lexically inside a class / module / singleton_class, else a top-level
/// `function`; a `singleton_method` (`def self.x` / `def Recv.x`) → method
/// rendered as `<object>.<name>`.
fn classify_ruby(
    tag: &str,
    decl: Node,
    name_node: Option<Node>,
    src: &[u8],
) -> Option<(SymbolKind, String)> {
    match tag {
        "decl.class" => Some((SymbolKind::Class, name_text(name_node, src))),
        "decl.module" => Some((SymbolKind::Module, name_text(name_node, src))),
        "decl.method" => {
            let kind = if ruby_in_class_scope(decl) {
                SymbolKind::Method
            } else {
                SymbolKind::Function
            };
            Some((kind, name_text(name_node, src)))
        }
        "decl.singleton_method" => Some((SymbolKind::Method, ruby_singleton_name(decl, src))),
        _ => None,
    }
}

/// True if `def_node` is lexically inside a `class` / `module` / `singleton_class`
/// body — i.e. it should render as a `method` rather than a top-level `function`.
fn ruby_in_class_scope(def_node: Node) -> bool {
    let mut cur = def_node.parent();
    while let Some(n) = cur {
        match n.kind() {
            "class" | "module" | "singleton_class" => return true,
            _ => {}
        }
        cur = n.parent();
    }
    false
}

/// Render a `singleton_method`'s name as `<object>.<name>` (e.g. `self.foo`,
/// `Config.load`). Empty if the `name:` field is absent (degrade, never panic).
fn ruby_singleton_name(node: Node, src: &[u8]) -> String {
    let object = node
        .child_by_field_name("object")
        .and_then(|n| node_text(n, src))
        .unwrap_or("");
    let name = node
        .child_by_field_name("name")
        .and_then(|n| node_text(n, src))
        .unwrap_or("");
    if name.is_empty() {
        return String::new();
    }
    if object.is_empty() {
        name.to_owned()
    } else {
        format!("{object}.{name}")
    }
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

    /// Every [`Lang`] the outliner supports — keep in lock-step with the enum.
    const ALL_LANGS: &[Lang] = &[
        Lang::Rust,
        Lang::TypeScript,
        Lang::Tsx,
        Lang::JavaScript,
        Lang::Python,
        Lang::Go,
        Lang::Java,
        Lang::C,
        Lang::Cpp,
        Lang::Ruby,
    ];

    /// A focused adversarial corpus — one representative per failure-mode class
    /// the prod 503 could have come from. Kept compact ON PURPOSE: `outline_blob`
    /// compiles its tree-sitter `Query` on every call (notably the large C++
    /// grammar costs tens of ms per call), so totality is proven by HITTING each
    /// parse path once per language, not by huge inputs or a huge corpus. Sizes
    /// are bounded for the same reason: tree-sitter error recovery is super-linear
    /// in the count of malformed tokens, so a large wrong-grammar blob is a
    /// multi-second parse for no extra coverage.
    fn adversarial_corpus() -> Vec<Vec<u8>> {
        // The fixed leading cases use a `vec![]` initializer (idiomatic — avoids
        // the clippy `vec_init_then_push` lint); the dynamically-built cases
        // below stay as `push` since they need intermediate statements.
        let mut c: Vec<Vec<u8>> = vec![
            // Empty + whitespace-only.
            Vec::new(),
            b" \t\r\n".to_vec(),
            // Non-UTF-8 / truncated-at-multibyte-boundary / lone-surrogate bytes.
            // `outline_blob` rejects these up front, but they pin the early-return.
            b"fn \xf0\x9f".to_vec(),      // truncated 4-byte emoji
            b"\xc3".to_vec(),             // lone 2-byte lead
            vec![0xed, 0xa0, 0x80],       // U+D800 surrogate as bytes (invalid)
            vec![0xff, 0xfe, 0x00, 0x01], // never-valid UTF-8 + NUL
            // A small valid source in one grammar (cross-parsed by all the others —
            // i.e. mostly error-recovery for the other nine).
            b"pub fn f(){}\nstruct S;\n".to_vec(),
            // Deeply nested / unbalanced grouping tokens — recursive-descent + error
            // recovery (depths in the low hundreds; see the fn-doc on why bounded).
            b"{".repeat(300), // opener flood, no closers
            b"}".repeat(300), // closer flood, no openers
        ];
        {
            let mut nested = Vec::new();
            nested.extend(b"fn f() ".iter());
            nested.extend(b"{".repeat(200));
            nested.extend(b"}".repeat(200));
            c.push(nested);
        }
        // Nested generics / templates — the C++/TS template-argument recovery
        // (the classic super-linear GLR case), bounded to a shallow depth.
        c.push(b"Vec<".repeat(64));

        // One giant token (a 64 KiB identifier — cheap: a single lexeme, not an
        // error flood) and a short high-decl-count blob (wide, not deep).
        c.push(vec![b'a'; 64 * 1024]);
        c.push(b"fn a(){}\n".repeat(64));

        // Valid-UTF-8 pseudo-random ASCII noise (the parse path actually runs).
        let mut rng: u64 = 0x9e37_79b9_7f4a_7c15;
        let mut noise = Vec::with_capacity(4_096);
        for _ in 0..4_096 {
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            noise.push((0x20 + (rng % 0x5f) as u8).min(0x7e)); // printable ASCII
        }
        c.push(noise);

        // NUL bytes interleaved with source-shaped text, and a NUL block.
        c.push(b"fn\0a\0(\0)\0{\0}".to_vec());
        c.push(vec![0u8; 1_024]);

        // Unterminated string / block-comment openers (lexer / error-recovery
        // edge), bounded.
        c.push(b"\"".repeat(128));
        c.push(b"/*".repeat(128));

        // Size-boundary: just under the cap (full parse — a single-token fill so
        // it is cheap), exactly at the cap, and one over (early oversized return).
        c.push(vec![b'a'; MAX_INPUT_BYTES - 1]);
        c.push(vec![b'b'; MAX_INPUT_BYTES]);
        c.push(vec![b'c'; MAX_INPUT_BYTES + 1]);

        c
    }

    /// TOTALITY CONTRACT (the doc on `outline_blob` says "never panics"): feed the
    /// adversarial corpus to EVERY language and assert the call NEVER panics — the
    /// regression guard for the `/v1/.../blob` 503 (a parser panic on a prod blob
    /// must degrade to an empty outline, never unwind).
    #[test]
    fn outline_blob_is_total_never_panics() {
        let corpus = adversarial_corpus();
        for lang in ALL_LANGS {
            for input in &corpus {
                // The assertion is simply that the call RETURNS — any panic would
                // unwind out of the test and fail it. We do not constrain the
                // OUTPUT (the contract is totality, not a specific outline). The
                // catch_unwind inside `outline_blob` is what makes this hold.
                let _ = outline_blob(*lang, input);
            }
        }
    }
}
