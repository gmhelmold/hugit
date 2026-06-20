//! Hermetic fixture tests for the C++ outliner: assert exact kind+name+line for
//! a sample source covering top-level AND nested declarations + every captured
//! construct (namespace, class/struct/union, enum, type alias, free function,
//! in-class method, static method, constructor, destructor, out-of-line member
//! definitions, data fields, templates).

use hugit_symbols::{Lang, SymbolItem, SymbolKind, lang_for_ext, outline_blob};

/// Sample C++ source exercising every captured kind, top-level and nested.
const FIXTURE: &str = r#"namespace geo {

const double PI = 3.14159;

template <typename T>
struct Box {
    T width;
    T height;
};

union Variant {
    int i;
    float f;
};

enum class Color { Red, Green };

typedef unsigned long ulong;
using Real = double;

int proto(int);

class Point {
public:
    Point(int x, int y);
    ~Point();
    int magnitude() const;
    static Point origin();
    int x_;
    int y_;
};

Point::Point(int x, int y) : x_(x), y_(y) {}

Point::~Point() {}

int Point::magnitude() const {
    return 0;
}

template <typename T>
T identity(T v) {
    return v;
}

int area(int w, int h) {
    return w * h;
}

double* make() {
    return nullptr;
}

namespace inner {
    void deep() {}
}

}  // namespace geo

void freefn() {}
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
    let items = outline_blob(Lang::Cpp, FIXTURE.as_bytes());

    // namespace (top-level and nested)
    assert_eq!(find(&items, SymbolKind::Namespace, "geo").line, 1);
    assert_eq!(find(&items, SymbolKind::Namespace, "inner").line, 54);

    // class / struct / union (union maps onto the frozen `struct` token)
    let cls = find(&items, SymbolKind::Class, "Point");
    assert_eq!(cls.line, 23);
    assert_eq!(cls.kind.as_wire_str(), "class");
    let bx = find(&items, SymbolKind::Struct, "Box"); // template-wrapped struct
    assert_eq!(bx.line, 6);
    let un = find(&items, SymbolKind::Struct, "Variant"); // union → struct
    assert_eq!(un.line, 11);
    assert_eq!(un.kind.as_wire_str(), "struct");

    // enum (scoped `enum class`)
    let en = find(&items, SymbolKind::Enum, "Color");
    assert_eq!(en.line, 16);
    assert_eq!(en.kind.as_wire_str(), "enum");

    // type aliases: typedef + using
    assert_eq!(find(&items, SymbolKind::TypeAlias, "ulong").line, 18);
    let real = find(&items, SymbolKind::TypeAlias, "Real");
    assert_eq!(real.line, 19);
    assert_eq!(real.kind.as_wire_str(), "type");

    // free functions (prototype, template, plain, pointer-return) + global fn
    assert_eq!(find(&items, SymbolKind::Function, "proto").line, 21);
    assert_eq!(find(&items, SymbolKind::Function, "identity").line, 42);
    assert_eq!(find(&items, SymbolKind::Function, "area").line, 46);
    let mk = find(&items, SymbolKind::Function, "make"); // pointer return type
    assert_eq!(mk.line, 50);
    assert_eq!(mk.kind.as_wire_str(), "function");
    assert_eq!(find(&items, SymbolKind::Function, "freefn").line, 60);

    // NESTED: data fields inside a class/struct
    assert_eq!(find(&items, SymbolKind::Field, "width").line, 7);
    assert_eq!(find(&items, SymbolKind::Field, "height").line, 8);
    assert_eq!(find(&items, SymbolKind::Field, "i").line, 12);
    let xf = find(&items, SymbolKind::Field, "x_");
    assert_eq!(xf.line, 29);
    assert_eq!(xf.kind.as_wire_str(), "field");
    assert_eq!(find(&items, SymbolKind::Field, "y_").line, 30);

    // NESTED: in-class method declarations (incl. static)
    let mag = find(&items, SymbolKind::Method, "magnitude");
    assert_eq!(mag.line, 27); // the in-class prototype
    assert_eq!(mag.kind.as_wire_str(), "method");
    assert_eq!(find(&items, SymbolKind::Method, "origin").line, 28);

    // NESTED: in-class constructor + destructor declarations
    let ctors: Vec<&SymbolItem> = items
        .iter()
        .filter(|i| i.kind == SymbolKind::Constructor && i.name == "Point")
        .collect();
    assert!(
        ctors.iter().any(|c| c.line == 25),
        "in-class ctor decl at L25 missing: {ctors:#?}"
    );
    let dtors: Vec<&SymbolItem> = items
        .iter()
        .filter(|i| i.kind == SymbolKind::Constructor && i.name == "~Point")
        .collect();
    assert!(
        dtors.iter().any(|d| d.line == 26),
        "in-class dtor decl at L26 missing: {dtors:#?}"
    );

    // NESTED: deep function inside nested namespace
    assert_eq!(find(&items, SymbolKind::Function, "deep").line, 55);

    // out-of-line member definitions: ctor / dtor / method
    assert!(
        ctors.iter().any(|c| c.line == 33),
        "out-of-line ctor def `Point::Point` at L33 missing"
    );
    assert!(
        dtors.iter().any(|d| d.line == 35),
        "out-of-line dtor def `Point::~Point` at L35 missing"
    );
    // out-of-line `Point::magnitude` definition is a method too
    assert!(
        items
            .iter()
            .any(|i| i.kind == SymbolKind::Method && i.name == "magnitude" && i.line == 37),
        "out-of-line method def `Point::magnitude` at L37 missing"
    );

    // const at namespace scope is a `declaration` with an init_declarator — it is
    // NOT a callable, so it is intentionally absent from the outline (the C++
    // outline focuses on type/callable/member structure, like the Rust one's set).
}

#[test]
fn outline_is_ordered_by_source_position() {
    let items = outline_blob(Lang::Cpp, FIXTURE.as_bytes());
    let lines: Vec<u32> = items.iter().map(|i| i.line).collect();
    let mut sorted = lines.clone();
    sorted.sort_unstable();
    assert_eq!(lines, sorted, "outline must be in source order");
}

#[test]
fn cpp_extension_maps_and_labels() {
    assert_eq!(lang_for_ext("cpp"), Some(Lang::Cpp));
    assert_eq!(lang_for_ext("cc"), Some(Lang::Cpp));
    assert_eq!(lang_for_ext("hpp"), Some(Lang::Cpp));
    assert_eq!(lang_for_ext("CXX"), Some(Lang::Cpp));
    assert_eq!(lang_for_ext("h"), Some(Lang::C)); // `.h` is owned by C in the consolidated map
    assert_eq!(Lang::Cpp.as_str(), "cpp");
}

#[test]
fn deterministic_same_bytes_same_outline() {
    assert_eq!(
        outline_blob(Lang::Cpp, FIXTURE.as_bytes()),
        outline_blob(Lang::Cpp, FIXTURE.as_bytes())
    );
}

#[test]
fn empty_and_total_on_degenerate_input() {
    assert!(outline_blob(Lang::Cpp, b"").is_empty());
    assert!(outline_blob(Lang::Cpp, b"// just a comment\n").is_empty());
    // non-UTF-8 / binary → empty, never a panic
    assert!(outline_blob(Lang::Cpp, &[0xff, 0xfe, 0x00, 0x01]).is_empty());
    let garbage: Vec<u8> = (0u8..=255).cycle().take(8192).collect();
    let _ = outline_blob(Lang::Cpp, &garbage); // must not panic
}

#[test]
fn forward_declaration_without_body_is_skipped() {
    // A bare `class Foo;` forward declaration has no body → not an outline entry.
    let items = outline_blob(Lang::Cpp, b"class Foo;\nclass Bar { int z; };\n");
    assert!(
        !items
            .iter()
            .any(|i| i.kind == SymbolKind::Class && i.name == "Foo"),
        "forward declaration must not surface: {items:#?}"
    );
    assert_eq!(find(&items, SymbolKind::Class, "Bar").line, 2);
}
