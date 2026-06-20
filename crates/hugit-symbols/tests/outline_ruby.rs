//! Hermetic fixture tests for the Ruby outliner: assert exact kind+name+line for
//! a sample source covering top-level AND nested declarations + every kind.

use hugit_symbols::{Lang, SymbolItem, SymbolKind, lang_for_ext, outline_blob};

/// Sample source exercising every captured kind, top-level and nested:
/// - a top-level `def` → `function`
/// - a `module` with a nested `class`, an instance `def` (`method`), and a
///   class `def self.x` (`singleton_method` → `method`)
/// - a deeply nested `module` inside a `class`
/// - a method-name that is a setter (`name=`) and an operator (`==`)
const FIXTURE: &str = r#"# frozen_string_literal: true

CONFIG = { timeout: 30 }

def standalone_helper(x)
  x * 2
end

module Billing
  TAX_RATE = 0.2

  class Invoice
    def initialize(total)
      @total = total
    end

    def total
      @total
    end

    def total=(value)
      @total = value
    end

    def ==(other)
      total == other.total
    end

    def self.from_cents(cents)
      new(cents / 100.0)
    end
  end

  module Reports
    def summary
      "report"
    end
  end
end

class Ledger
  module Internal
    def reconcile
      true
    end
  end

  def self.open
    new
  end
end
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
    let items = outline_blob(Lang::Ruby, FIXTURE.as_bytes());

    // top-level def → function
    let f = find(&items, SymbolKind::Function, "standalone_helper");
    assert_eq!(f.line, 5);
    assert_eq!(f.kind.as_wire_str(), "function");

    // module (top-level) and nested module
    assert_eq!(find(&items, SymbolKind::Module, "Billing").line, 9);
    assert_eq!(find(&items, SymbolKind::Module, "Reports").line, 34);

    // class (nested in module) and a top-level class
    assert_eq!(find(&items, SymbolKind::Class, "Invoice").line, 12);
    assert_eq!(find(&items, SymbolKind::Class, "Ledger").line, 41);

    // instance methods (def inside a class) → method
    assert_eq!(find(&items, SymbolKind::Method, "initialize").line, 13);
    assert_eq!(find(&items, SymbolKind::Method, "total").line, 17);
    // a setter method-name renders verbatim (`total=`)
    assert_eq!(find(&items, SymbolKind::Method, "total=").line, 21);
    // an operator method-name renders verbatim (`==`)
    assert_eq!(find(&items, SymbolKind::Method, "==").line, 25);

    // class method `def self.from_cents` (singleton_method) → method, name `self.x`
    let cm = find(&items, SymbolKind::Method, "self.from_cents");
    assert_eq!(cm.line, 29);
    assert_eq!(cm.kind.as_wire_str(), "method");

    // a method inside a module → method
    assert_eq!(find(&items, SymbolKind::Method, "summary").line, 35);

    // DEEP NESTING: a module nested inside a class, and its method.
    assert_eq!(find(&items, SymbolKind::Module, "Internal").line, 42);
    assert_eq!(find(&items, SymbolKind::Method, "reconcile").line, 43);

    // class method on a top-level class.
    assert_eq!(find(&items, SymbolKind::Method, "self.open").line, 48);
}

#[test]
fn outline_is_ordered_by_source_position() {
    let items = outline_blob(Lang::Ruby, FIXTURE.as_bytes());
    let lines: Vec<u32> = items.iter().map(|i| i.line).collect();
    let mut sorted = lines.clone();
    sorted.sort_unstable();
    assert_eq!(lines, sorted, "outline must be in source order");
}

#[test]
fn ruby_extension_maps() {
    assert_eq!(lang_for_ext("rb"), Some(Lang::Ruby));
    assert_eq!(lang_for_ext("md"), None);
}

#[test]
fn empty_source_is_empty_outline() {
    assert!(outline_blob(Lang::Ruby, b"").is_empty());
    assert!(outline_blob(Lang::Ruby, b"# just a comment\n").is_empty());
    assert!(outline_blob(Lang::Ruby, b"x = 1\nputs x\n").is_empty());
}

#[test]
fn determinism() {
    let a = outline_blob(Lang::Ruby, FIXTURE.as_bytes());
    let b = outline_blob(Lang::Ruby, FIXTURE.as_bytes());
    assert_eq!(a, b);
}

#[test]
fn empty_on_non_utf8_and_oversized() {
    let bytes = [0x64u8, 0x65, 0x66, 0xff, 0xfe, 0x00]; // "def" + invalid bytes
    assert!(outline_blob(Lang::Ruby, &bytes).is_empty());
    let big = vec![b'a'; 2 * 1024 * 1024 + 1];
    assert!(outline_blob(Lang::Ruby, &big).is_empty());
}

#[test]
fn ruby_wire_vocabulary_is_frozen() {
    assert_eq!(SymbolKind::Class.as_wire_str(), "class");
    assert_eq!(SymbolKind::Module.as_wire_str(), "module");
    assert_eq!(SymbolKind::Method.as_wire_str(), "method");
    assert_eq!(SymbolKind::Function.as_wire_str(), "function");
}
