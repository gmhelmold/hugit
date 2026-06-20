//! Hermetic fixture tests for the C outliner: assert exact kind+name+line for a
//! sample translation unit covering top-level AND nested declarations + every
//! captured construct (function_definition, struct/union/enum specifiers,
//! typedef, object- and function-like #define).

use hugit_symbols::{Lang, SymbolItem, SymbolKind, lang_for_ext, outline_blob};

/// Sample C source exercising every captured kind, top-level and nested.
const FIXTURE: &str = r#"#include <stddef.h>

#define MAX_LEN 256
#define SQUARE(x) ((x) * (x))

struct Point {
    int x;
    int y;
};

union Value {
    int i;
    float f;
};

enum Color {
    RED,
    GREEN,
    BLUE,
};

typedef struct Point Point;
typedef int (*Comparator)(int, int);

int add(int a, int b) {
    return a + b;
}

static char *greeting(void) {
    return "hi";
}

void outer(void) {
    struct Inner {
        int z;
    };
    enum Local { A, B };
}
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

#[test]
fn outline_captures_kind_name_line_for_each_decl() {
    let items = outline_blob(Lang::C, FIXTURE.as_bytes());

    // object-like and function-like macros (#define)
    let m1 = find(&items, SymbolKind::Macro, "MAX_LEN");
    assert_eq!(m1.line, 3);
    assert_eq!(m1.kind.as_wire_str(), "macro");
    assert_eq!(find(&items, SymbolKind::Macro, "SQUARE").line, 4);

    // tagged struct / union (union → struct kind) / enum
    assert_eq!(find(&items, SymbolKind::Struct, "Point").line, 6);
    assert_eq!(find(&items, SymbolKind::Struct, "Value").line, 11);
    let col = find(&items, SymbolKind::Enum, "Color");
    assert_eq!(col.line, 16);
    assert_eq!(col.kind.as_wire_str(), "enum");

    // typedefs (simple + function-pointer) → type kind, name derived from the
    // declarator (NOT the wrapped struct tag).
    let td = find(&items, SymbolKind::TypeAlias, "Point");
    assert_eq!(td.line, 22);
    assert_eq!(td.kind.as_wire_str(), "type");
    let cmp = find(&items, SymbolKind::TypeAlias, "Comparator");
    assert_eq!(cmp.line, 23);

    // function definitions → function kind (distinct from Rust's `fn`); name
    // derived from the declarator (incl. through a pointer-return declarator).
    let add = find(&items, SymbolKind::Function, "add");
    assert_eq!(add.line, 25);
    assert_eq!(add.kind.as_wire_str(), "function");
    assert_eq!(find(&items, SymbolKind::Function, "greeting").line, 29);
    assert_eq!(find(&items, SymbolKind::Function, "outer").line, 33);

    // NESTED: a struct + enum declared inside a function body must surface too.
    assert_eq!(find(&items, SymbolKind::Struct, "Inner").line, 34);
    assert_eq!(find(&items, SymbolKind::Enum, "Local").line, 37);
}

#[test]
fn outline_is_ordered_by_source_position() {
    let items = outline_blob(Lang::C, FIXTURE.as_bytes());
    let lines: Vec<u32> = items.iter().map(|i| i.line).collect();
    let mut sorted = lines.clone();
    sorted.sort_unstable();
    assert_eq!(lines, sorted, "outline must be in source order");
}

#[test]
fn c_extensions_map_to_c() {
    assert_eq!(lang_for_ext("c"), Some(Lang::C));
    assert_eq!(lang_for_ext("h"), Some(Lang::C));
    assert_eq!(lang_for_ext("C"), Some(Lang::C));
    assert_eq!(lang_for_ext("md"), None);
}

#[test]
fn empty_source_is_empty_outline() {
    assert!(outline_blob(Lang::C, b"").is_empty());
    assert!(outline_blob(Lang::C, b"/* just a comment */\n").is_empty());
}

#[test]
fn deterministic() {
    let a = outline_blob(Lang::C, FIXTURE.as_bytes());
    let b = outline_blob(Lang::C, FIXTURE.as_bytes());
    assert_eq!(a, b, "same bytes → same outline");
}

#[test]
fn empty_on_non_utf8() {
    let bytes = [0x69u8, 0x6e, 0x74, 0x20, 0xff, 0xfe, 0x00];
    assert!(outline_blob(Lang::C, &bytes).is_empty());
}
