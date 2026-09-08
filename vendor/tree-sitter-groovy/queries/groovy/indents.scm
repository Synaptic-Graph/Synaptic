; Indent rules for Groovy.
;
; nvim-treesitter conventions:
;   @indent.begin  — node whose contents are indented one level
;   @indent.end    — node that closes an indent block
;   @indent.dedent — node that dedents by one level
;   @indent.branch — node that aligns with its enclosing block
;
; See SPECIFICATION.md §9.

[
  (class_body)
  (enum_body)
  (switch_block)
  (block)
  (closure)
  (list_literal)
  (map_literal)
  (argument_list)
  (formal_parameters)
  (record_components)
] @indent.begin

[
  "}"
  "]"
  ")"
] @indent.end

; Branch labels — case / default lines align with the switch_block.
[
  (switch_case)
  (switch_arrow_case)
  (switch_default)
] @indent.branch
