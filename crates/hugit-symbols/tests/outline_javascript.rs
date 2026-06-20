//! Hermetic fixture tests for the JavaScript outliner: assert exact
//! kind+name+line for a sample source covering top-level AND nested
//! declarations + every captured construct.

use hugit_symbols::{Lang, SymbolItem, SymbolKind, lang_for_ext, outline_blob};

/// Sample source exercising every captured kind, top-level and nested.
const FIXTURE: &str = r#"// a leading comment
const MAX = 10;
let counter = 0;
var legacy = "x";

const add = (a, b) => a + b;
let multiply = function (a, b) { return a * b; };

function greet(name) {
  return "hi " + name;
}

function* generate() {
  yield 1;
}

class Point {
  constructor(x, y) {
    this.x = x;
    this.y = y;
  }

  count = 0;

  distance() {
    return Math.sqrt(this.x * this.x + this.y * this.y);
  }

  static origin() {
    return new Point(0, 0);
  }

  get magnitude() {
    return this.distance();
  }
}

const Shape = class Circle {
  area() {
    return 3.14;
  }
};

function outer() {
  function inner() {
    return 42;
  }
  return inner;
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
    let items = outline_blob(Lang::JavaScript, FIXTURE.as_bytes());

    // const / let / var bindings with non-function values.
    let c = find(&items, SymbolKind::Const, "MAX");
    assert_eq!(c.line, 2);
    assert_eq!(c.kind.as_wire_str(), "const");
    let counter = find(&items, SymbolKind::Variable, "counter");
    assert_eq!(counter.line, 3);
    assert_eq!(counter.kind.as_wire_str(), "variable");
    let legacy = find(&items, SymbolKind::Variable, "legacy");
    assert_eq!(legacy.line, 4);

    // const-bound arrow / function-expression bindings remap to `function`.
    let add = find(&items, SymbolKind::Function, "add");
    assert_eq!(add.line, 6);
    assert_eq!(add.kind.as_wire_str(), "function");
    let multiply = find(&items, SymbolKind::Function, "multiply");
    assert_eq!(multiply.line, 7);

    // function declaration + generator function declaration.
    assert_eq!(find(&items, SymbolKind::Function, "greet").line, 9);
    assert_eq!(find(&items, SymbolKind::Function, "generate").line, 13);

    // class declaration.
    let pt = find(&items, SymbolKind::Class, "Point");
    assert_eq!(pt.line, 17);
    assert_eq!(pt.kind.as_wire_str(), "class");

    // NESTED class members: constructor, field, method, static method, getter.
    let ctor = find(&items, SymbolKind::Constructor, "constructor");
    assert_eq!(ctor.line, 18);
    assert_eq!(ctor.kind.as_wire_str(), "constructor");
    assert_eq!(find(&items, SymbolKind::Field, "count").line, 23);
    let dist = find(&items, SymbolKind::Method, "distance");
    assert_eq!(dist.line, 25);
    assert_eq!(dist.kind.as_wire_str(), "method");
    assert_eq!(find(&items, SymbolKind::Method, "origin").line, 29);
    assert_eq!(find(&items, SymbolKind::Method, "magnitude").line, 33);

    // class EXPRESSION (`const Shape = class Circle {}`): both the const-bound
    // class name and its nested method must surface.
    assert_eq!(find(&items, SymbolKind::Class, "Circle").line, 38);
    assert_eq!(find(&items, SymbolKind::Method, "area").line, 39);

    // NESTED function inside a function.
    assert_eq!(find(&items, SymbolKind::Function, "outer").line, 44);
    assert_eq!(find(&items, SymbolKind::Function, "inner").line, 45);
}

#[test]
fn outline_is_ordered_by_source_position() {
    let items = outline_blob(Lang::JavaScript, FIXTURE.as_bytes());
    let lines: Vec<u32> = items.iter().map(|i| i.line).collect();
    let mut sorted = lines.clone();
    sorted.sort_unstable();
    assert_eq!(lines, sorted, "outline must be in source order");
}

#[test]
fn javascript_extensions_map() {
    assert_eq!(lang_for_ext("js"), Some(Lang::JavaScript));
    assert_eq!(lang_for_ext("jsx"), Some(Lang::JavaScript));
    assert_eq!(lang_for_ext("mjs"), Some(Lang::JavaScript));
    assert_eq!(lang_for_ext("cjs"), Some(Lang::JavaScript));
    assert_eq!(lang_for_ext("JS"), Some(Lang::JavaScript));
    assert_eq!(lang_for_ext("ts"), Some(Lang::TypeScript)); // `.ts` is TypeScript in the consolidated map
}

#[test]
fn display_label_is_stable() {
    assert_eq!(Lang::JavaScript.as_str(), "javascript");
}

#[test]
fn empty_source_is_empty_outline() {
    assert!(outline_blob(Lang::JavaScript, b"").is_empty());
    assert!(outline_blob(Lang::JavaScript, b"// just a comment\n").is_empty());
}

#[test]
fn empty_on_non_utf8() {
    let bytes = [0x66u8, 0x6e, 0x20, 0xff, 0xfe, 0x00];
    assert!(outline_blob(Lang::JavaScript, &bytes).is_empty());
}

#[test]
fn deterministic() {
    let a = outline_blob(Lang::JavaScript, FIXTURE.as_bytes());
    let b = outline_blob(Lang::JavaScript, FIXTURE.as_bytes());
    assert_eq!(a, b);
}
