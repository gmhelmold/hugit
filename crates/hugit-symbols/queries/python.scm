; Symbol-outline capture set for Python (tree-sitter-python grammar).
;
; Each capture binds the *named identifier* of a declaration; the surrounding
; `@decl.<kind>` capture pins the kind + the declaration's start byte. Top-level
; AND nested declarations are captured uniformly — tree-sitter matches the
; pattern at any depth, so a method inside a class body and a function nested in
; another function both surface.
;
; `function_definition` is captured generically as `@decl.fn`; the producer
; re-classifies it to `method` when its nearest enclosing definition is a
; `class_definition` (a query cannot reliably express "the closest ancestor is a
; class" across `async def` / decorated wrappers, so the distinction is decided
; in Rust by ancestry — see `python_reclass`). An `assignment` to an identifier
; is captured generically as `@decl.const`; the producer keeps only the ones
; whose name is SCREAMING_SNAKE_CASE *and* that sit directly in a module or class
; body (never a local inside a function), so a local `max = 1` or `MAX = 1` in a
; function body never pollutes the outline.
;
; FROZEN wire `kind` vocabulary (master plan C1):
;   fn method class const  (the subset Python maps onto)

(class_definition    name: (identifier) @name) @decl.class
(function_definition name: (identifier) @name) @decl.fn
(assignment          left: (identifier) @name) @decl.const
