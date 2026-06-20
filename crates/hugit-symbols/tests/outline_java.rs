//! Hermetic fixture tests for the Java outliner: assert exact kind+name+line for
//! a sample source covering top-level AND nested declarations + every kind.

use hugit_symbols::{Lang, SymbolItem, SymbolKind, lang_for_ext, outline_blob};

/// Sample source exercising every captured kind, top-level and nested.
const FIXTURE: &str = r#"package com.example.app;

import java.util.List;

public class Account {
    private final long id;
    public static final int MAX_RETRIES = 3;

    public Account(long id) {
        this.id = id;
    }

    public long getId() {
        return id;
    }

    private void touch() {
    }

    // Nested (inner) class — must surface.
    static class Snapshot {
        int version;

        Snapshot() {
        }

        int version() {
            return version;
        }
    }

    // Nested interface — must surface.
    interface Listener {
        void onChange();
    }

    // Nested enum with constants — must surface.
    enum State {
        OPEN,
        CLOSED;

        boolean isOpen() {
            return this == OPEN;
        }
    }

    // Nested record — maps to `class`.
    record Pair(int a, int b) {
    }
}

interface Repository {
    Account find(long id);
}

enum Color {
    RED,
    GREEN;
}

@interface Audited {
    String value();
}

record Point(int x, int y) {
    int sum() {
        return x + y;
    }
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
    let items = outline_blob(Lang::Java, FIXTURE.as_bytes());

    // top-level class
    let acct = find(&items, SymbolKind::Class, "Account");
    assert_eq!(acct.line, 5);
    assert_eq!(acct.kind.as_wire_str(), "class");

    // fields (one per declarator) + a constant-looking static-final field
    // (still a `field` — Java has no distinct const declaration node).
    assert_eq!(find(&items, SymbolKind::Field, "id").line, 6);
    let max = find(&items, SymbolKind::Field, "MAX_RETRIES");
    assert_eq!(max.line, 7);
    assert_eq!(max.kind.as_wire_str(), "field");

    // constructor + methods (top-level-of-class)
    assert_eq!(find(&items, SymbolKind::Constructor, "Account").line, 9);
    assert_eq!(find(&items, SymbolKind::Method, "getId").line, 13);
    assert_eq!(find(&items, SymbolKind::Method, "touch").line, 17);

    // NESTED: inner class + its field/constructor/method
    let snap = find(&items, SymbolKind::Class, "Snapshot");
    assert_eq!(snap.line, 21);
    assert_eq!(find(&items, SymbolKind::Field, "version").line, 22);
    assert_eq!(find(&items, SymbolKind::Constructor, "Snapshot").line, 24);
    assert_eq!(find(&items, SymbolKind::Method, "version").line, 27);

    // NESTED interface + its method
    assert_eq!(find(&items, SymbolKind::Interface, "Listener").line, 33);
    assert_eq!(find(&items, SymbolKind::Method, "onChange").line, 34);

    // NESTED enum, its constants, and a body method
    let state = find(&items, SymbolKind::Enum, "State");
    assert_eq!(state.line, 38);
    assert_eq!(state.kind.as_wire_str(), "enum");
    let open = find(&items, SymbolKind::Constant, "OPEN");
    assert_eq!(open.line, 39);
    assert_eq!(open.kind.as_wire_str(), "constant");
    assert_eq!(find(&items, SymbolKind::Constant, "CLOSED").line, 40);
    assert_eq!(find(&items, SymbolKind::Method, "isOpen").line, 42);

    // NESTED record -> class
    let pair = find(&items, SymbolKind::Class, "Pair");
    assert_eq!(pair.line, 48);
    assert_eq!(pair.kind.as_wire_str(), "class");

    // top-level interface
    assert_eq!(find(&items, SymbolKind::Interface, "Repository").line, 52);
    assert_eq!(find(&items, SymbolKind::Method, "find").line, 53);

    // top-level enum + its constants
    assert_eq!(find(&items, SymbolKind::Enum, "Color").line, 56);
    assert_eq!(find(&items, SymbolKind::Constant, "RED").line, 57);
    assert_eq!(find(&items, SymbolKind::Constant, "GREEN").line, 58);

    // annotation type -> interface
    let audited = find(&items, SymbolKind::Interface, "Audited");
    assert_eq!(audited.line, 61);
    assert_eq!(audited.kind.as_wire_str(), "interface");
    assert_eq!(find(&items, SymbolKind::Method, "value").line, 62);

    // top-level record -> class + its method
    let point = find(&items, SymbolKind::Class, "Point");
    assert_eq!(point.line, 65);
    assert_eq!(find(&items, SymbolKind::Method, "sum").line, 66);
}

#[test]
fn outline_is_ordered_by_source_position() {
    let items = outline_blob(Lang::Java, FIXTURE.as_bytes());
    let lines: Vec<u32> = items.iter().map(|i| i.line).collect();
    let mut sorted = lines.clone();
    sorted.sort_unstable();
    assert_eq!(lines, sorted, "outline must be in source order");
}

#[test]
fn java_extension_maps() {
    assert_eq!(lang_for_ext("java"), Some(Lang::Java));
    assert_eq!(lang_for_ext("JAVA"), Some(Lang::Java));
    assert_eq!(lang_for_ext("class"), None); // compiled artifact, not source
}

#[test]
fn empty_source_is_empty_outline() {
    assert!(outline_blob(Lang::Java, b"").is_empty());
    assert!(outline_blob(Lang::Java, b"// just a comment\n").is_empty());
}

#[test]
fn deterministic() {
    let a = outline_blob(Lang::Java, FIXTURE.as_bytes());
    let b = outline_blob(Lang::Java, FIXTURE.as_bytes());
    assert_eq!(a, b);
}

#[test]
fn wire_vocabulary_for_java_kinds_is_frozen() {
    assert_eq!(SymbolKind::Class.as_wire_str(), "class");
    assert_eq!(SymbolKind::Interface.as_wire_str(), "interface");
    assert_eq!(SymbolKind::Enum.as_wire_str(), "enum");
    assert_eq!(SymbolKind::Method.as_wire_str(), "method");
    assert_eq!(SymbolKind::Constructor.as_wire_str(), "constructor");
    assert_eq!(SymbolKind::Field.as_wire_str(), "field");
    assert_eq!(SymbolKind::Constant.as_wire_str(), "constant");
}
