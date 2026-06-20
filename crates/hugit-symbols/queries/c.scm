; Symbol-outline capture set for C (tree-sitter-c grammar).
;
; Each `@decl.<kind>` capture pins the construct + its kind; `outline_blob` reads
; the decl node's start row (0-based) and +1's it to a 1-based line. Names are
; NOT captured via a `name:` field for every kind — a C function's identifier is
; buried inside (possibly pointer-wrapped) declarators, so the producer derives
; the function name by descending the `declarator` field to the innermost
; identifier (see `c_declarator_name`). For the named type specifiers the grammar
; DOES expose a `name:` field, captured as `@name`.
;
; Top-level AND nested declarations are captured uniformly — tree-sitter matches
; the pattern at any depth, so a function defined inside a (GNU statement-expr or
; nested) scope, or a struct declared inside another struct's body, both surface.
;
; FROZEN wire `kind` vocabulary (master plan C1):
;   fn function method constructor struct class enum interface trait impl mod
;   module namespace field property const constant static variable macro type

; A function definition: `int f(void) { ... }`. The name lives inside the
; `declarator` subtree (function_declarator → … → identifier), derived by the
; producer; the whole definition is the decl node (its start row = the line).
(function_definition) @decl.function

; Tagged aggregate / enum specifiers. Only the *named* forms surface (an
; anonymous `struct { … }` inside a typedef is named by the typedef instead).
; A union is the most accurate available frozen kind: `struct` (a tagged record).
(struct_specifier name: (type_identifier) @name) @decl.struct
(union_specifier  name: (type_identifier) @name) @decl.struct
(enum_specifier   name: (type_identifier) @name) @decl.enum

; `typedef … NAME;` — the new type alias. The grammar puts the introduced name in
; the `declarator` field (a type_identifier for the simple case); the producer
; derives it by descending the declarator to the innermost identifier so that
; `typedef int (*Fn)(void);` still yields `Fn`.
(type_definition) @decl.type

; `#define NAME …` — both object-like (preproc_def) and function-like
; (preproc_function_def) macros. The grammar exposes the macro name as `name:`.
(preproc_def          name: (identifier) @name) @decl.macro
(preproc_function_def name: (identifier) @name) @decl.macro
