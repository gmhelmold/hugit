//! Hermetic fixture tests for the TypeScript / TSX outliner: assert exact
//! kind+name+line for sample sources covering top-level AND nested declarations
//! and every captured kind.

use hugit_symbols::{Lang, SymbolItem, SymbolKind, lang_for_ext, outline_blob};

/// Sample TypeScript exercising every captured kind, top-level and nested.
const FIXTURE: &str = r#"export const MAX = 10;

export function area(w: number, h: number): number {
  return w * h;
}

export const scale = (n: number): number => n * 2;

export class Point {
  x: number;
  constructor(x: number) {
    this.x = x;
  }
  norm(): number {
    return this.x;
  }
}

export abstract class Shape {
  abstract area(): number;
}

export interface Drawable {
  draw(): void;
}

export enum Color {
  Red,
  Green,
}

export type Pair = [number, number];

export namespace Geo {
  export const PI = 3.14;
  export function dist(a: number, b: number): number {
    return a - b;
  }
  export class Vec {
    len(): number {
      return 0;
    }
  }
}
"#;

/// TSX-specific fixture: a component arrow function returning JSX must parse
/// under the JSX-aware grammar (and surface as a `function`).
const TSX_FIXTURE: &str = r#"export const Button = (props: { label: string }) => {
  return <button>{props.label}</button>;
};

export function Panel(): JSX.Element {
  return <div><Button label="ok" /></div>;
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
    let items = outline_blob(Lang::TypeScript, FIXTURE.as_bytes());

    // const arrow function -> function (idiomatic TS)
    let scale = find(&items, SymbolKind::Function, "scale");
    assert_eq!(scale.line, 7);
    assert_eq!(scale.kind.as_wire_str(), "function");

    // top-level function declaration
    assert_eq!(find(&items, SymbolKind::Function, "area").line, 3);

    // class + abstract class
    assert_eq!(find(&items, SymbolKind::Class, "Point").line, 9);
    let shape = find(&items, SymbolKind::Class, "Shape");
    assert_eq!(shape.line, 19);
    assert_eq!(shape.kind.as_wire_str(), "class");

    // interface / enum / type alias
    assert_eq!(find(&items, SymbolKind::Interface, "Drawable").line, 23);
    assert_eq!(find(&items, SymbolKind::Enum, "Color").line, 27);
    let pair = find(&items, SymbolKind::TypeAlias, "Pair");
    assert_eq!(pair.line, 32);
    assert_eq!(pair.kind.as_wire_str(), "type");

    // namespace
    let geo = find(&items, SymbolKind::Namespace, "Geo");
    assert_eq!(geo.line, 34);
    assert_eq!(geo.kind.as_wire_str(), "namespace");

    // NESTED: class field, constructor + method, namespace-nested fn + class
    assert_eq!(find(&items, SymbolKind::Field, "x").line, 10);
    assert_eq!(find(&items, SymbolKind::Method, "constructor").line, 11);
    assert_eq!(find(&items, SymbolKind::Method, "norm").line, 14);
    // abstract method signature inside the abstract class
    assert_eq!(find(&items, SymbolKind::Method, "area").line, 20);
    // interface method signature
    assert_eq!(find(&items, SymbolKind::Method, "draw").line, 24);
    // namespace-nested declarations surface at depth
    assert_eq!(find(&items, SymbolKind::Function, "dist").line, 36);
    assert_eq!(find(&items, SymbolKind::Class, "Vec").line, 39);
    assert_eq!(find(&items, SymbolKind::Method, "len").line, 40);
}

#[test]
fn outline_is_ordered_by_source_position() {
    let items = outline_blob(Lang::TypeScript, FIXTURE.as_bytes());
    let lines: Vec<u32> = items.iter().map(|i| i.line).collect();
    let mut sorted = lines.clone();
    sorted.sort_unstable();
    assert_eq!(lines, sorted, "outline must be in source order");
}

#[test]
fn tsx_component_parses_under_jsx_grammar() {
    let items = outline_blob(Lang::Tsx, TSX_FIXTURE.as_bytes());
    // The JSX-returning arrow component and the JSX-returning function both
    // surface as `function`.
    assert_eq!(find(&items, SymbolKind::Function, "Button").line, 1);
    assert_eq!(find(&items, SymbolKind::Function, "Panel").line, 5);
}

#[test]
fn extensions_map_to_lang() {
    assert_eq!(lang_for_ext("ts"), Some(Lang::TypeScript));
    assert_eq!(lang_for_ext("mts"), Some(Lang::TypeScript));
    assert_eq!(lang_for_ext("cts"), Some(Lang::TypeScript));
    assert_eq!(lang_for_ext("tsx"), Some(Lang::Tsx));
    assert_eq!(lang_for_ext("md"), None);
}

#[test]
fn empty_and_garbage_yield_empty_outline() {
    assert!(outline_blob(Lang::TypeScript, b"").is_empty());
    assert!(outline_blob(Lang::TypeScript, b"// just a comment\n").is_empty());
    // non-UTF-8 bytes -> empty, never a panic
    assert!(outline_blob(Lang::TypeScript, &[0x66, 0xff, 0xfe, 0x00]).is_empty());
}

#[test]
fn deterministic() {
    let a = outline_blob(Lang::TypeScript, FIXTURE.as_bytes());
    let b = outline_blob(Lang::TypeScript, FIXTURE.as_bytes());
    assert_eq!(a, b);
}
