//! Hermetic fixture tests for the Go outliner: assert exact kind+name+line for a
//! sample source covering top-level AND nested declarations + every Go kind the
//! capture set maps onto the frozen wire vocabulary.

use hugit_symbols::{Lang, SymbolItem, SymbolKind, lang_for_ext, outline_blob};

/// Sample Go source exercising every captured kind, top-level and nested.
/// Line numbers below are 1-based and asserted exactly.
const FIXTURE: &str = r#"package main

import "fmt"

const Pi = 3.14

const (
	MaxRetries = 3
	MinRetries = 1
)

var Greeting = "hi"

var (
	counter int
	enabled bool
)

type Point struct {
	X int
	Y int
}

type Drawer interface {
	Draw() error
}

type Celsius float64

type Pair = [2]int

func Area(p Point) int {
	return p.X * p.Y
}

func (p Point) Origin() Point {
	// a nested const inside a method body must surface too
	const Zero = 0
	return Point{X: Zero, Y: Zero}
}

func (d Celsius) String() string {
	return fmt.Sprintf("%g", d)
}

func Outer() {
	// a nested function-typed var + a nested type inside a function body
	var local = 1
	type inner struct {
		z int
	}
	_ = local
	_ = inner{}
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
    let items = outline_blob(Lang::Go, FIXTURE.as_bytes());

    // const — single + grouped
    let pi = find(&items, SymbolKind::Constant, "Pi");
    assert_eq!(pi.line, 5);
    assert_eq!(pi.kind.as_wire_str(), "constant");
    assert_eq!(find(&items, SymbolKind::Constant, "MaxRetries").line, 8);
    assert_eq!(find(&items, SymbolKind::Constant, "MinRetries").line, 9);

    // var — single + grouped
    let g = find(&items, SymbolKind::Variable, "Greeting");
    assert_eq!(g.line, 12);
    assert_eq!(g.kind.as_wire_str(), "variable");
    assert_eq!(find(&items, SymbolKind::Variable, "counter").line, 15);
    assert_eq!(find(&items, SymbolKind::Variable, "enabled").line, 16);

    // struct / interface
    assert_eq!(find(&items, SymbolKind::Struct, "Point").line, 19);
    let drawer = find(&items, SymbolKind::Interface, "Drawer");
    assert_eq!(drawer.line, 24);
    assert_eq!(drawer.kind.as_wire_str(), "interface");

    // named type + type alias → both the "type" wire token
    let cel = find(&items, SymbolKind::TypeAlias, "Celsius");
    assert_eq!(cel.line, 28);
    assert_eq!(cel.kind.as_wire_str(), "type");
    assert_eq!(find(&items, SymbolKind::TypeAlias, "Pair").line, 30);

    // top-level function
    let area = find(&items, SymbolKind::Function, "Area");
    assert_eq!(area.line, 32);
    assert_eq!(area.kind.as_wire_str(), "function");

    // methods (have a receiver) → the "method" wire token
    let origin = find(&items, SymbolKind::Method, "Origin");
    assert_eq!(origin.line, 36);
    assert_eq!(origin.kind.as_wire_str(), "method");
    assert_eq!(find(&items, SymbolKind::Method, "String").line, 42);

    // NESTED: a const inside a method body, a var + a struct type inside a
    // function body. A struct type_spec is a "struct" at any depth — the nested
    // `type inner struct {…}` surfaces as Struct, not the generic type token.
    assert_eq!(find(&items, SymbolKind::Constant, "Zero").line, 38);
    assert_eq!(find(&items, SymbolKind::Variable, "local").line, 48);
    assert_eq!(find(&items, SymbolKind::Struct, "inner").line, 49);

    // the enclosing top-level function must surface as well
    assert_eq!(find(&items, SymbolKind::Function, "Outer").line, 46);
}

#[test]
fn outline_is_ordered_by_source_position() {
    let items = outline_blob(Lang::Go, FIXTURE.as_bytes());
    let lines: Vec<u32> = items.iter().map(|i| i.line).collect();
    let mut sorted = lines.clone();
    sorted.sort_unstable();
    assert_eq!(lines, sorted, "outline must be in source order");
}

#[test]
fn go_extension_maps() {
    assert_eq!(lang_for_ext("go"), Some(Lang::Go));
    assert_eq!(lang_for_ext("md"), None);
}

#[test]
fn empty_source_is_empty_outline() {
    assert!(outline_blob(Lang::Go, b"").is_empty());
    assert!(outline_blob(Lang::Go, b"package main\n// just a comment\n").is_empty());
}

#[test]
fn deterministic() {
    let src = FIXTURE.as_bytes();
    assert_eq!(outline_blob(Lang::Go, src), outline_blob(Lang::Go, src));
}

#[test]
fn non_utf8_and_oversized_are_empty() {
    let bytes = [0x66u8, 0x75, 0x6e, 0x63, 0x20, 0xff, 0xfe, 0x00];
    assert!(outline_blob(Lang::Go, &bytes).is_empty());
    let big = vec![b'a'; 2 * 1024 * 1024 + 1];
    assert!(outline_blob(Lang::Go, &big).is_empty());
}

#[test]
fn wire_vocabulary_is_frozen() {
    assert_eq!(SymbolKind::Function.as_wire_str(), "function");
    assert_eq!(SymbolKind::Method.as_wire_str(), "method");
    assert_eq!(SymbolKind::Struct.as_wire_str(), "struct");
    assert_eq!(SymbolKind::Interface.as_wire_str(), "interface");
    assert_eq!(SymbolKind::TypeAlias.as_wire_str(), "type");
    assert_eq!(SymbolKind::Constant.as_wire_str(), "constant");
    assert_eq!(SymbolKind::Variable.as_wire_str(), "variable");
}
