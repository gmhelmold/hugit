; Symbol-outline capture set for C++ (tree-sitter-cpp grammar).
;
; Each pattern binds the *named identifier* of a declaration via @name; the
; surrounding `@decl.<kind>` capture pins the kind + the declaration's start
; byte. tree-sitter matches at any depth, so a method inside a class, a class
; inside a namespace, and a function inside a namespace all surface uniformly
; (top-level AND nested). Where the C++ grammar makes a callable's *kind*
; (function vs method vs constructor) impossible to tell from a single pattern,
; the pattern carries the SUPERSET kind (`function`) and the producer refines it
; in Rust by inspecting the declarator (see `refine_cpp_callable` in lib.rs).
;
; FROZEN wire `kind` vocabulary mapped here (master plan C1):
;   namespace class struct enum type field function method constructor const
; (a C++ `union` aggregate → `struct`, the closest frozen token; a
;  `typedef`/`using` alias → `type`.)

; ── namespaces ──────────────────────────────────────────────────────────────
(namespace_definition
  name: (namespace_identifier) @name) @decl.namespace

; ── class / struct / union DEFINITIONS (body present; forward decls skipped) ──
(class_specifier
  name: (type_identifier) @name
  body: (_)) @decl.class
(struct_specifier
  name: (type_identifier) @name
  body: (_)) @decl.struct
(union_specifier
  name: (type_identifier) @name
  body: (_)) @decl.struct

; ── enum (scoped `enum class` and plain `enum`) ─────────────────────────────
(enum_specifier
  name: (type_identifier) @name) @decl.enum

; ── type aliases: `typedef … Name;` and `using Name = …;` ───────────────────
(type_definition
  declarator: (type_identifier) @name) @decl.type
(alias_declaration
  name: (type_identifier) @name) @decl.type

; ── callables: function DEFINITIONS (with a body) ───────────────────────────
; The name lives at the leaf of the declarator chain. The Rust producer reads
; the captured function_declarator to decide function / method / constructor.
;   declarator name nodes seen: identifier (free fn / ctor),
;   field_identifier (in-class method/ctor), qualified_identifier (out-of-line),
;   destructor_name (always a destructor → constructor kind).
; The chain may be wrapped by a pointer_/reference_declarator for a returned
; pointer/reference; the descendant match below pierces that wrapper.
(function_definition
  declarator: (function_declarator) @decl.cpp_callable)
; …with a pointer / reference return type the function_declarator is wrapped.
(function_definition
  declarator: (pointer_declarator
    declarator: (function_declarator) @decl.cpp_callable))
(function_definition
  declarator: (reference_declarator
    (function_declarator) @decl.cpp_callable))

; ── callables: in-class member DECLARATIONS (prototypes; no body) ────────────
; `int magnitude() const;`, `static Point origin();` are field_declarations.
(field_declaration
  declarator: (function_declarator) @decl.cpp_callable)
(field_declaration
  declarator: (pointer_declarator
    declarator: (function_declarator) @decl.cpp_callable))
(field_declaration
  declarator: (reference_declarator
    (function_declarator) @decl.cpp_callable))

; ── callables: free/constructor DECLARATIONS (no body, not in a class) ───────
; `int foo();` (prototype) and an in-class `Point(int,int);` are both
; `declaration`s; the producer's refiner separates function from constructor.
(declaration
  declarator: (function_declarator) @decl.cpp_callable)
(declaration
  declarator: (pointer_declarator
    declarator: (function_declarator) @decl.cpp_callable))
(declaration
  declarator: (reference_declarator
    (function_declarator) @decl.cpp_callable))

; ── data members (in-class fields) ──────────────────────────────────────────
(field_declaration
  declarator: (field_identifier) @name) @decl.field
