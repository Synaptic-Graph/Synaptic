; Tag captures per SPECIFICATION.md §2.1.
;
; Tag-query convention (matches tree-sitter-tags / nvim-treesitter):
;   @definition.<kind>  — declaration site of a top-level symbol
;   @reference.<kind>   — call / instantiation reference
;   @name               — the identifier inside the tag, used to
;                         resolve the symbol name when emitting tags
;
; Captures are kept conservative: only declarations whose name is a
; bare `identifier` are tagged. Quoted-identifier method names
; (`def "abstract"() { … }`) are skipped — `ctags` consumers expect
; plain symbol names.

; --- Definitions ----------------------------------------------------

(class_declaration
  name: (identifier) @name) @definition.class

(interface_declaration
  name: (identifier) @name) @definition.interface

(trait_declaration
  name: (identifier) @name) @definition.trait

(annotation_type_declaration
  name: (identifier) @name) @definition.interface

(enum_declaration
  name: (identifier) @name) @definition.enum

(record_declaration
  name: (identifier) @name) @definition.class

(method_declaration
  name: (identifier) @name) @definition.method

(constructor_declaration
  name: (identifier) @name) @definition.constructor

(field_declaration
  (variable_declarator
    name: (identifier) @name)) @definition.field

(enum_constant
  name: (identifier) @name) @definition.constant

; --- References -----------------------------------------------------

(method_invocation
  function: (identifier) @name) @reference.call

(method_invocation
  function: (field_access
    field: (identifier) @name)) @reference.call

(object_creation_expression
  type: (type_identifier) @name) @reference.class

(object_creation_expression
  type: (generic_type
    base: (type_identifier) @name)) @reference.class

(superclass (type_identifier) @name) @reference.class

(super_interfaces (type_identifier) @name) @reference.implementation

(extends_interfaces (type_identifier) @name) @reference.implementation
