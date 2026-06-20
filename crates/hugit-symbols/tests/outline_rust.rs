//! Hermetic fixture tests for the Rust outliner: assert exact kind+name+line for
//! a sample source covering top-level AND nested declarations + every kind.

use hugit_symbols::{Lang, SymbolItem, SymbolKind, lang_for_ext, outline_blob};

/// Sample source exercising every captured kind, top-level and nested.
const FIXTURE: &str = r#"//! a module doc comment
use std::fmt;

pub const MAX: u32 = 10;
static GREETING: &str = "hi";

pub struct Point {
    x: i32,
    y: i32,
}

pub enum Shape {
    Circle,
    Square,
}

pub trait Draw {
    fn draw(&self);
}

impl Point {
    pub fn origin() -> Self {
        Point { x: 0, y: 0 }
    }
}

impl Draw for Point {
    fn draw(&self) {}
}

pub type Pair = (i32, i32);

macro_rules! shout {
    () => {};
}

pub fn area(s: &Shape) -> u32 {
    0
}

pub mod inner {
    pub fn helper() {}
    pub struct Nested;
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
    let items = outline_blob(Lang::Rust, FIXTURE.as_bytes());

    // const / static
    let c = find(&items, SymbolKind::Const, "MAX");
    assert_eq!(c.line, 4);
    assert_eq!(c.kind.as_wire_str(), "const");
    let s = find(&items, SymbolKind::Static, "GREETING");
    assert_eq!(s.line, 5);

    // struct / enum / trait
    assert_eq!(find(&items, SymbolKind::Struct, "Point").line, 7);
    assert_eq!(find(&items, SymbolKind::Enum, "Shape").line, 12);
    assert_eq!(find(&items, SymbolKind::Trait, "Draw").line, 17);

    // impl (inherent + trait impl) — derived display name
    assert_eq!(find(&items, SymbolKind::Impl, "Point").line, 21);
    let trait_impl = find(&items, SymbolKind::Impl, "Draw for Point");
    assert_eq!(trait_impl.line, 27);
    assert_eq!(trait_impl.kind.as_wire_str(), "impl");

    // type alias
    let ta = find(&items, SymbolKind::TypeAlias, "Pair");
    assert_eq!(ta.line, 31);
    assert_eq!(ta.kind.as_wire_str(), "type");

    // macro_rules!
    let m = find(&items, SymbolKind::Macro, "shout");
    assert_eq!(m.line, 33);
    assert_eq!(m.kind.as_wire_str(), "macro");

    // top-level fn + mod
    assert_eq!(find(&items, SymbolKind::Fn, "area").line, 37);
    assert_eq!(find(&items, SymbolKind::Mod, "inner").line, 41);

    // NESTED: an impl-method fn and a mod-nested struct must surface too.
    assert_eq!(find(&items, SymbolKind::Fn, "origin").line, 22);
    assert_eq!(find(&items, SymbolKind::Fn, "draw").line, 28);
    assert_eq!(find(&items, SymbolKind::Fn, "helper").line, 42);
    assert_eq!(find(&items, SymbolKind::Struct, "Nested").line, 43);
}

#[test]
fn outline_is_ordered_by_source_position() {
    let items = outline_blob(Lang::Rust, FIXTURE.as_bytes());
    let lines: Vec<u32> = items.iter().map(|i| i.line).collect();
    let mut sorted = lines.clone();
    sorted.sort_unstable();
    assert_eq!(lines, sorted, "outline must be in source order");
}

#[test]
fn unknown_language_extension_yields_no_lang() {
    assert_eq!(lang_for_ext("rs"), Some(Lang::Rust));
    assert_eq!(lang_for_ext("md"), None);
}

#[test]
fn empty_source_is_empty_outline() {
    assert!(outline_blob(Lang::Rust, b"").is_empty());
    assert!(outline_blob(Lang::Rust, b"// just a comment\n").is_empty());
}

#[test]
fn wire_vocabulary_is_frozen() {
    // The exact FROZEN tokens (master plan C1). A change here is a wire break.
    assert_eq!(SymbolKind::Fn.as_wire_str(), "fn");
    assert_eq!(SymbolKind::Struct.as_wire_str(), "struct");
    assert_eq!(SymbolKind::Enum.as_wire_str(), "enum");
    assert_eq!(SymbolKind::Trait.as_wire_str(), "trait");
    assert_eq!(SymbolKind::Impl.as_wire_str(), "impl");
    assert_eq!(SymbolKind::Mod.as_wire_str(), "mod");
    assert_eq!(SymbolKind::Const.as_wire_str(), "const");
    assert_eq!(SymbolKind::Static.as_wire_str(), "static");
    assert_eq!(SymbolKind::Macro.as_wire_str(), "macro");
    assert_eq!(SymbolKind::TypeAlias.as_wire_str(), "type");
}
