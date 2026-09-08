; Language injections per SPECIFICATION.md §9.2.
;
; Doc comments inject as a documentation language. Slashy and
; dollar-slashy string literals inject as `regex` so editors
; highlight the pattern body. Named-call DSL strings
; (`sql.execute """..."""`, `xmlSlurper.parseText """..."""`)
; inject the embedded language when the receiver name matches.

((groovydoc_comment) @injection.content
 (#set! injection.language "javadoc"))

; Slashy regex `/.../` — the leading `/` is what distinguishes it
; from the plain-string flavours at the query layer (single-,
; triple-single-, double-, triple-double-quoted strings all
; share the `string_literal` node kind).
((string_literal) @injection.content
  (#match? @injection.content "^/[^/]")
  (#set! injection.language "regex"))

; Dollar-slashy regex `$/.../$`.
((string_literal) @injection.content
  (#match? @injection.content "^\\$/")
  (#set! injection.language "regex"))

; `sql.execute """SELECT ..."""` and similar SQL DSL idioms.
; Matches a method invocation whose receiver chain ends in `sql`
; or `Sql` and whose first argument is a triple-double-quoted
; string. The injection content is the body's literal-fragment
; range, so `${param}` interpolations stay tree-sitter-parsable
; rather than being swallowed by the SQL highlighter.
((method_invocation
   function: (field_access
     object: (identifier) @_recv
     field: (identifier))
   arguments: (argument_list
     (string_literal (string_fragment) @injection.content)))
 (#match? @_recv "^[Ss]ql$")
 (#set! injection.language "sql"))
