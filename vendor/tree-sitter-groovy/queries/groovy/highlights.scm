; tree-sitter-groovy highlight queries.
;
; Capture names follow nvim-treesitter conventions:
; https://neovim.io/doc/user/treesitter.html#treesitter-highlight-groups
;
; Precedence rule: tree-sitter highlight queries are last-match-wins
; per node. Layers below are ordered from broadest to most specific:
;   1. comments / number / string / boolean / null
;   2. anonymous-token operators (also captures . , ?., etc.)
;   3. generic identifier → @variable  (the catch-all)
;   4. keywords (@keyword, @keyword.control, @keyword.operator)
;   5. field / property captures off the dot family
;   6. declared names (class / method / record / formal parameter)
;   7. call-site captures (@function.call) — must come AFTER property
;      so a method-named field beats the property capture
;   8. type uses (@type on type_identifier) — must come AFTER
;      @variable so aliased identifier-as-type wins
;   9. annotation usage (the @attribute capture)

; -- Layer 1: comments and literals --------------------------------

(line_comment) @comment
(block_comment) @comment
(groovydoc_comment) @comment.documentation

(number_literal) @number
(string_literal) @string
(string_fragment) @string
(boolean_literal) @boolean
(null_literal) @constant.builtin

; GString interpolation — `${expr}` and `$identifier` (per §5.7).
(gstring_brace_interpolation
  "${" @punctuation.special
  "}" @punctuation.special)
(gstring_dollar_interpolation
  value: (identifier) @variable)

; -- Layer 2: operators (anonymous tokens) -------------------------

[
  "+"
  "-"
  "*"
  "/"
  "%"
  "!"
  "~"
  "<<"
  ">>"
  ">>>"
  "<"
  "<="
  ">"
  ">="
  "=="
  "!="
  "&"
  "^"
  "|"
  "&&"
  "||"
  ".."
  "..<"
  "<.."
  "<..<"
  "==="
  "!=="
  "<=>"
  "=~"
  "==~"
  "?:"
  "?"
  "==>"
  "="
  "+="
  "-="
  "*="
  "/="
  "%="
  "**="
  "<<="
  ">>="
  ">>>="
  "&="
  "^="
  "|="
  "?="
  "**"
  "++"
  "--"
  "."
  "?."
  "??."
  "*."
  ".&"
  ".@"
  "::"
  "*:"
] @operator

; -- Layer 3: generic identifier fallback --------------------------

(identifier) @variable

; -- Layer 4: keywords ---------------------------------------------

[
  "new"
  "as"
  "in"
  "instanceof"
] @keyword.operator

; `!in` and `!instanceof` are composite tokens (literal + trailing
; whitespace) per SPECIFICATION.md §3.2.2 lexer rule, so they
; cannot be referenced as direct anonymous tokens in a `[ … ]`
; group. Capture them through the operator field instead.
(membership_expression operator: _ @keyword.operator)
(instanceof_expression operator: _ @keyword.operator)

[
  "if"
  "else"
  "while"
  "do"
  "for"
  "return"
  "break"
  "continue"
  "throw"
  "throws"
  "assert"
  "try"
  "catch"
  "finally"
  "switch"
  "case"
  "default"
  "yield"
] @keyword.control

[
  "class"
  "trait"
  "interface"
  "enum"
  "record"
  "def"
  "var"
  "package"
  "import"
  "pipeline"
  "sealed"
  "non-sealed"
  "permits"
  "extends"
  "implements"
] @keyword

[
  "public"
  "private"
  "protected"
  "static"
  "final"
  "abstract"
  "synchronized"
  "native"
  "transient"
  "volatile"
  "strictfp"
] @keyword.modifier

(shebang) @keyword.directive

; -- Layer 5: dot-family property captures -------------------------

(field_access                    field: [(identifier) (quoted_identifier)] @property)
(safe_navigation_expression     property: [(identifier) (quoted_identifier)] @property)
(safe_chain_dot_expression      property: [(identifier) (quoted_identifier)] @property)
(spread_dot_expression          property: [(identifier) (quoted_identifier)] @property)
(direct_field_access_expression  field: [(identifier) (quoted_identifier)] @property)
(method_pointer_expression      method: [(identifier) (quoted_identifier)] @function)
; `Foo::new` — the `new` literal is captured as @keyword.operator at
; layer 4 instead of @function here.
(method_reference_expression       name: (identifier) @function)

; -- Layer 6: declared names ---------------------------------------

(class_declaration name: (identifier) @type.definition)
(trait_declaration name: (identifier) @type.definition)
(interface_declaration name: (identifier) @type.definition)
(enum_declaration name: (identifier) @type.definition)
(record_declaration name: (identifier) @type.definition)
(annotation_type_declaration name: (identifier) @type.definition)
(enum_constant name: (identifier) @constant)
(method_declaration name: [(identifier) (quoted_identifier)] @function)
(constructor_declaration name: (identifier) @constructor)
(formal_parameter name: (identifier) @variable.parameter)
(closure_parameter name: (identifier) @variable.parameter)
(record_component name: (identifier) @variable.parameter)
(variable_declarator name: (identifier) @variable)
(field_declaration (variable_declarator name: (identifier) @property))

; Labels (declaration + use sites)
(labeled_statement label: (identifier) @label)
(break_statement label: (identifier) @label)
(continue_statement label: (identifier) @label)

; -- Layer 7: call sites (after layer 5 so method-named fields win) -

(method_invocation
  function: (identifier) @function.call)

(method_invocation
  function: (field_access
              field: (identifier) @function.call))

(command_chain
  receiver: (identifier) @function.call)

; -- Layer 8: types (after @variable so aliased identifier wins) ---

(type_identifier) @type

; Java/Groovy primitive types and `void` highlight as builtin.
((type_identifier) @type.builtin
  (#match? @type.builtin "^(boolean|byte|char|double|float|int|long|short|void)$"))

; -- Layer 9: annotation usage -------------------------------------

(annotation
  "@" @attribute
  name: (qualified_name) @attribute)
