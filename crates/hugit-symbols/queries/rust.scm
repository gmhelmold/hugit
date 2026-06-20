; Symbol-outline capture set for Rust (tree-sitter-rust grammar).
;
; Each capture binds the *named identifier* of a declaration; the surrounding
; `@decl.<kind>` capture pins the kind. `outline_blob` reads the name node's
; start row (0-based) and +1's it to a 1-based line. Top-level AND nested
; declarations are captured uniformly — tree-sitter matches the pattern at any
; depth, so an `impl`-nested `fn` or a `mod`-nested `struct` both surface.
;
; FROZEN wire `kind` vocabulary (master plan C1):
;   fn struct enum trait impl mod const static macro type

(function_item    name: (identifier)      @name) @decl.fn
(struct_item      name: (type_identifier) @name) @decl.struct
(enum_item        name: (type_identifier) @name) @decl.enum
(trait_item       name: (type_identifier) @name) @decl.trait
(mod_item         name: (identifier)      @name) @decl.mod
(const_item       name: (identifier)      @name) @decl.const
(static_item      name: (identifier)      @name) @decl.static
(macro_definition name: (identifier)      @name) @decl.macro
(type_item        name: (type_identifier) @name) @decl.type

; `impl` blocks have no `name:` field — the rendered name is the implementing
; type (and, for trait impls, "Trait for Type"). Captured whole; the producer
; derives the display name from the `type:`/`trait:` children.
(impl_item) @decl.impl
