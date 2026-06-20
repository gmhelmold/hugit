; Symbol-outline capture set for Java (tree-sitter-java grammar).
;
; Each capture binds the *named identifier* of a declaration; the surrounding
; `@decl.<kind>` capture pins the kind. `outline_blob` reads the decl node's
; start row (0-based) and +1's it to a 1-based line. Top-level AND nested
; declarations are captured uniformly — tree-sitter matches the pattern at any
; depth, so a class-nested method, an inner class, an interface-nested type, or
; a method-local class all surface.
;
; FROZEN wire `kind` vocabulary (master plan C1) — Java uses this subset:
;   class interface enum method constructor field constant

; Type declarations. Records map to `class` (a special final class) and
; annotation types map to `interface` (a special interface) — the most accurate
; frozen kind for each.
(class_declaration            name: (identifier) @name) @decl.class
(record_declaration           name: (identifier) @name) @decl.class
(interface_declaration        name: (identifier) @name) @decl.interface
(annotation_type_declaration  name: (identifier) @name) @decl.interface
(enum_declaration             name: (identifier) @name) @decl.enum

; Callables. An annotation type's element (`String value();` inside an
; `@interface`) is an `annotation_type_element_declaration` — surfaced as a
; method, the most accurate frozen kind for a named callable member.
(method_declaration                  name: (identifier) @name) @decl.method
(annotation_type_element_declaration name: (identifier) @name) @decl.method
(constructor_declaration             name: (identifier) @name) @decl.constructor

; Fields — `field_declaration` has no `name:` field; the name lives on its
; `variable_declarator`. A single declaration can declare several variables
; (`int a, b;`); each declarator surfaces as its own field row.
(field_declaration
  declarator: (variable_declarator name: (identifier) @name)) @decl.field

; Enum constants (e.g. `RED, GREEN` inside an enum body) — the closest frozen
; kind is `constant`.
(enum_constant name: (identifier) @name) @decl.constant

