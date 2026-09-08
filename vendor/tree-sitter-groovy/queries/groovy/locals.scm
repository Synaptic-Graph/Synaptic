; Local-scope captures per SPECIFICATION.md §9.3.
;
; nvim-treesitter / Neovim local-scope query language:
;   @local.scope      — node introduces a fresh scope
;   @local.definition — captures a binding (formal, declarator, …)
;   @local.reference  — captures a use site of an identifier

; Scopes — anywhere a new symbol table is opened.
[
  (class_body)
  (block)
  (closure)
  (method_declaration)
  (constructor_declaration)
  (static_initializer)
  (for_statement)
  (for_in_statement)
  (catch_clause)
] @local.scope

; Definitions — bound names introduced inside their enclosing scope.
(formal_parameter name: (identifier) @local.definition.parameter)
(closure_parameter name: (identifier) @local.definition.parameter)
(variable_declarator name: (identifier) @local.definition.var)
(field_declaration
  (variable_declarator name: (identifier) @local.definition.field))
(for_in_statement variable: (identifier) @local.definition.var)
(catch_formal_parameter name: (identifier) @local.definition.var)
(record_component name: (identifier) @local.definition.parameter)

; References — bare identifiers used as values.
(identifier) @local.reference
