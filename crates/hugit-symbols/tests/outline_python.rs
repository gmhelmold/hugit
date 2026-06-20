//! Hermetic fixture tests for the Python outliner: assert exact kind+name+line
//! for a sample source covering top-level AND nested declarations + every
//! Python-mapped kind (class / function / method / const), plus the
//! reclassification rules (fn→method by class ancestry; const = module/class-
//! level SCREAMING_SNAKE only).

use hugit_symbols::{Lang, SymbolItem, SymbolKind, lang_for_ext, outline_blob};

/// Sample source exercising every captured kind, top-level and nested.
/// Line numbers are 1-based and asserted exactly below. The literal opens with a
/// leading newline so the `"""docstring"""` is not adjacent to the raw-string
/// opener (`r#"`) — line 1 is intentionally blank, real content starts at line 2.
const FIXTURE: &str = r#"
"""module docstring"""
import os

MAX_RETRIES = 5
DEFAULT_NAME = "hi"
lowercase_var = 1


def top_level_fn(a, b):
    local_const = 2
    LOCAL_CAP = 3
    return a + b


async def async_fn():
    return None


@decorator
def decorated_fn():
    pass


class Animal:
    LEGS = 4

    def __init__(self, name):
        self.name = name

    def speak(self):
        return "..."

    @property
    def display(self):
        return self.name

    @staticmethod
    def kingdom():
        return "Animalia"

    class Inner:
        def inner_method(self):
            return 1


def outer():
    def nested_fn():
        return 0
    return nested_fn
"#;

fn find<'a>(items: &'a [SymbolItem], kind: SymbolKind, name: &str) -> &'a SymbolItem {
    items
        .iter()
        .find(|i| i.kind == kind && i.name == name)
        .unwrap_or_else(|| {
            panic!(
                "missing {} {name} in outline: {items:#?}",
                kind.as_wire_str()
            )
        })
}

fn absent(items: &[SymbolItem], kind: SymbolKind, name: &str) {
    assert!(
        !items.iter().any(|i| i.kind == kind && i.name == name),
        "unexpected {} {name} in outline: {items:#?}",
        kind.as_wire_str()
    );
}

#[test]
fn outline_captures_kind_name_line_for_each_decl() {
    let items = outline_blob(Lang::Python, FIXTURE.as_bytes());

    // module-level SCREAMING_SNAKE constants
    let c = find(&items, SymbolKind::Const, "MAX_RETRIES");
    assert_eq!(c.line, 5);
    assert_eq!(c.kind.as_wire_str(), "const");
    assert_eq!(find(&items, SymbolKind::Const, "DEFAULT_NAME").line, 6);

    // top-level functions (plain, async, decorated)
    let f = find(&items, SymbolKind::Fn, "top_level_fn");
    assert_eq!(f.line, 10);
    assert_eq!(f.kind.as_wire_str(), "fn");
    assert_eq!(find(&items, SymbolKind::Fn, "async_fn").line, 16);
    assert_eq!(find(&items, SymbolKind::Fn, "decorated_fn").line, 21);

    // class
    let cls = find(&items, SymbolKind::Class, "Animal");
    assert_eq!(cls.line, 25);
    assert_eq!(cls.kind.as_wire_str(), "class");

    // class-level constant
    assert_eq!(find(&items, SymbolKind::Const, "LEGS").line, 26);

    // methods (plain, property, staticmethod) — fn reclassified to method
    let m = find(&items, SymbolKind::Method, "__init__");
    assert_eq!(m.line, 28);
    assert_eq!(m.kind.as_wire_str(), "method");
    assert_eq!(find(&items, SymbolKind::Method, "speak").line, 31);
    assert_eq!(find(&items, SymbolKind::Method, "display").line, 35);
    assert_eq!(find(&items, SymbolKind::Method, "kingdom").line, 39);

    // NESTED class + its method
    assert_eq!(find(&items, SymbolKind::Class, "Inner").line, 42);
    assert_eq!(find(&items, SymbolKind::Method, "inner_method").line, 43);

    // NESTED function inside a function is a free fn, NOT a method
    assert_eq!(find(&items, SymbolKind::Fn, "outer").line, 47);
    let nested = find(&items, SymbolKind::Fn, "nested_fn");
    assert_eq!(nested.line, 48);
    assert_eq!(nested.kind.as_wire_str(), "fn");
}

#[test]
fn lowercase_and_local_bindings_are_not_constants() {
    let items = outline_blob(Lang::Python, FIXTURE.as_bytes());
    // a module-level lowercase variable is NOT a constant
    absent(&items, SymbolKind::Const, "lowercase_var");
    // function-local bindings (even SCREAMING_SNAKE) are NOT constants
    absent(&items, SymbolKind::Const, "local_const");
    absent(&items, SymbolKind::Const, "LOCAL_CAP");
}

#[test]
fn methods_are_not_double_counted_as_functions() {
    let items = outline_blob(Lang::Python, FIXTURE.as_bytes());
    // a method must surface ONLY as `method`, never also as `fn`
    absent(&items, SymbolKind::Fn, "__init__");
    absent(&items, SymbolKind::Fn, "speak");
    absent(&items, SymbolKind::Fn, "inner_method");
    // and a top-level fn is never a method
    absent(&items, SymbolKind::Method, "top_level_fn");
}

#[test]
fn outline_is_ordered_by_source_position() {
    let items = outline_blob(Lang::Python, FIXTURE.as_bytes());
    let lines: Vec<u32> = items.iter().map(|i| i.line).collect();
    let mut sorted = lines.clone();
    sorted.sort_unstable();
    assert_eq!(lines, sorted, "outline must be in source order");
}

#[test]
fn outline_is_deterministic() {
    let a = outline_blob(Lang::Python, FIXTURE.as_bytes());
    let b = outline_blob(Lang::Python, FIXTURE.as_bytes());
    assert_eq!(a, b);
}

#[test]
fn extension_maps_to_python() {
    assert_eq!(lang_for_ext("py"), Some(Lang::Python));
    assert_eq!(lang_for_ext("pyi"), Some(Lang::Python));
    assert_eq!(lang_for_ext("md"), None);
}

#[test]
fn empty_and_comment_only_sources_are_empty() {
    assert!(outline_blob(Lang::Python, b"").is_empty());
    assert!(outline_blob(Lang::Python, b"# just a comment\n").is_empty());
}

#[test]
fn non_utf8_and_oversized_are_empty() {
    // 0xFF is never valid UTF-8 → empty (totality).
    let bytes = [0x64u8, 0x65, 0x66, 0x20, 0xff, 0xfe, 0x00];
    assert!(outline_blob(Lang::Python, &bytes).is_empty());
    // oversized → empty (bounded).
    let big = vec![b'a'; 2 * 1024 * 1024 + 1];
    assert!(outline_blob(Lang::Python, &big).is_empty());
}

#[test]
fn wire_vocabulary_for_python_is_frozen() {
    assert_eq!(SymbolKind::Fn.as_wire_str(), "fn");
    assert_eq!(SymbolKind::Method.as_wire_str(), "method");
    assert_eq!(SymbolKind::Class.as_wire_str(), "class");
    assert_eq!(SymbolKind::Const.as_wire_str(), "const");
}
