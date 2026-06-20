; Symbol-outline capture set for Go (tree-sitter-go grammar).
;
; Each capture binds the *named identifier* of a declaration; the surrounding
; `@decl.<kind>` capture pins the declaration's start byte. `outline_blob` reads
; the decl node's start row (0-based) and +1's it to a 1-based line. Top-level
; AND nested declarations are captured uniformly — tree-sitter matches the
; pattern at any depth, so a function-body type_spec or var_spec both surface.
;
; FROZEN wire `kind` vocabulary (master plan C1) used here:
;   function method struct interface type constant variable
;
; A `type_spec` (`type T struct{…}` / `interface{…}` / a named type) and a
; `type_alias` (`type T = U`) are captured under ONE `@decl.go_type` token; the
; producer inspects the decl's `type:` child to pick struct / interface / type —
; exactly like the Rust impl-name derivation reads a node's children. This keeps
; the query robust against the open-ended set of Go body-type node kinds.

; Plain top-level / nested function: `func name(...) {...}`.
(function_declaration name: (identifier) @name) @decl.function

; Method: a function WITH a receiver — `func (r T) name(...) {...}`. The
; method_declaration node only exists for methods, so the receiver is implied.
(method_declaration name: (field_identifier) @name) @decl.method

; Both type-declaration spellings → one capture; the producer refines the kind.
(type_spec  name: (type_identifier) @name) @decl.go_type
(type_alias name: (type_identifier) @name) @decl.go_type

; const / var specs. Each spec binds one-or-more names on the left of `=`; we
; capture the FIRST identifier as the display name (the common single-name case;
; a grouped `const ( A = …; B = … )` yields one spec per line, each surfacing).
(const_spec name: (identifier) @name) @decl.constant
(var_spec   name: (identifier) @name) @decl.variable
