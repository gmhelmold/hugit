; Symbol-outline capture set for Ruby (tree-sitter-ruby grammar).
;
; Each capture binds the *named identifier* of a declaration; the surrounding
; `@decl.<kind>` capture pins the declaration node (its start row → 1-based line)
; and selects the kind. Top-level AND nested declarations are captured uniformly —
; tree-sitter matches the pattern at any depth, so a class-nested `def`, a
; module-nested class, and a nested module all surface.
;
; FROZEN wire `kind` vocabulary (master plan C1) — the producer maps onto:
;   class module method function
; A `def` (method node) renders as `method` when it is lexically inside a
; class / module / singleton_class, and as `function` at the top level. That
; class-vs-top-level decision is made in the producer (`outline_ruby`) by walking
; the declaration's ancestor chain — tree-sitter patterns cannot express depth.
; A `singleton_method` (`def self.x` / `def Recv.x`) is always a `method`; its
; rendered name is `self.x` / `Recv.x`.

; A class: name is a `constant` (Foo) or a `scope_resolution` (Foo::Bar).
(class  name: (_) @name) @decl.class

; A module: name is a `constant` or a `scope_resolution`.
(module name: (_) @name) @decl.module

; An instance/class method `def`. The producer decides method vs function from
; the enclosing scope. `name:` is a `_method_name` (identifier / constant /
; setter / operator / symbol).
(method name: (_) @name) @decl.method

; A singleton method `def self.foo` / `def Recv.foo` — always a `method`. Captured
; whole; the producer renders `<object>.<name>` from the `object:`/`name:` fields.
(singleton_method) @decl.singleton_method
