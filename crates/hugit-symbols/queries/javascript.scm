; Symbol-outline capture set for JavaScript (tree-sitter-javascript grammar).
;
; Each capture binds the *named identifier* of a declaration; the surrounding
; `@decl.<kind>` capture pins the kind. `outline_blob` reads the decl node's
; start row (0-based) and +1's it to a 1-based line. Top-level AND nested
; declarations are captured uniformly — tree-sitter matches the pattern at any
; depth, so a class-nested `method_definition` or a namespace-nested
; `function_declaration` both surface.
;
; FROZEN wire `kind` vocabulary (master plan C1) — JS uses this subset:
;   function method class const variable

; --- functions -------------------------------------------------------------
; `function foo() {}` and `function* gen() {}`.
(function_declaration
  name: (identifier) @name) @decl.function
(generator_function_declaration
  name: (identifier) @name) @decl.function

; --- classes + their members ----------------------------------------------
; `class Foo {}` (and the expression form `const Foo = class Bar {}`).
(class_declaration
  name: (identifier) @name) @decl.class
(class
  name: (identifier) @name) @decl.class

; A class member `foo() {}` / `get x() {}` / `static bar() {}` /
; `*gen() {}` — the grammar models them all as `method_definition`. The
; constructor is a `method_definition` whose name is `constructor`; the
; producer remaps that name to the `constructor` kind.
(method_definition
  name: (property_identifier) @name) @decl.method

; A class field `count = 0;` / `#private = 1;`.
(field_definition
  property: (property_identifier) @name) @decl.field
(field_definition
  property: (private_property_identifier) @name) @decl.field

; --- top-level / nested binding declarations -------------------------------
; `const x = …`, `let y = …`, `var z = …`. A `const`-bound arrow function
; (`const f = () => {}`) is an extremely common way to declare a JS function;
; the producer remaps such bindings to the `function` kind when the value is an
; arrow / function expression, otherwise keeps `const`/`variable`.
(lexical_declaration
  (variable_declarator
    name: (identifier) @name)) @decl.lexical
(variable_declaration
  (variable_declarator
    name: (identifier) @name)) @decl.var
