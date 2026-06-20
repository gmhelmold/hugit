; Symbol-outline capture set for TypeScript / TSX (tree-sitter-typescript).
;
; Each capture binds the *named identifier* of a declaration; the surrounding
; `@decl.<kind>` capture pins the kind. `outline_blob` reads the decl node's
; start row (0-based) and +1's it to a 1-based line. Top-level AND nested
; declarations are captured uniformly — tree-sitter matches the pattern at any
; depth, so a class-nested method or a namespace-nested function both surface.
;
; FROZEN wire `kind` vocabulary (master plan C1) used here:
;   function method class interface enum type namespace field

; --- Functions -------------------------------------------------------------
; Top-level / nested declared functions, plus ambient/overload signatures.
(function_declaration name: (identifier) @name) @decl.function
(function_signature   name: (identifier) @name) @decl.function

; Exported (or plain) const/let/var bound to an arrow function or a function
; expression — the idiomatic TS "function". The declarator's identifier names
; it; the arrow/function value distinguishes it from a plain variable.
(lexical_declaration
  (variable_declarator
    name: (identifier) @name
    value: [(arrow_function) (function_expression)])) @decl.function
(variable_declaration
  (variable_declarator
    name: (identifier) @name
    value: [(arrow_function) (function_expression)])) @decl.function

; --- Classes & members -----------------------------------------------------
(class_declaration          name: (type_identifier) @name) @decl.class
(abstract_class_declaration name: (type_identifier) @name) @decl.class

; Methods (incl. constructors, getters/setters) inside a class body.
(method_definition name: (property_identifier) @name) @decl.method
; Abstract methods (`abstract foo(): T;`) and interface/type method members
; (`foo(): void;`) are bodyless signatures, not method_definition.
(abstract_method_signature name: (property_identifier) @name) @decl.method
(method_signature          name: (property_identifier) @name) @decl.method

; Class fields (public/private/static).
(public_field_definition name: (property_identifier) @name) @decl.field

; --- Interfaces, enums, type aliases --------------------------------------
(interface_declaration  name: (type_identifier) @name) @decl.interface
(enum_declaration       name: (identifier)      @name) @decl.enum
(type_alias_declaration name: (type_identifier) @name) @decl.type

; --- Namespaces / modules --------------------------------------------------
; `namespace Foo {}` and `module Foo {}` parse to internal_module / module.
(internal_module name: (identifier) @name) @decl.namespace
(module          name: (identifier) @name) @decl.namespace

