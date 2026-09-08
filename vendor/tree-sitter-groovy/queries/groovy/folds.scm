; Fold regions for Groovy. nvim-treesitter / Neovim folding ranges
; are taken from any node captured as @fold; the node's start and
; end positions become the fold's bounds.
;
; See SPECIFICATION.md §9 — folds cover class / method / closure /
; control-flow bodies plus large literal collections.

(class_body) @fold
(enum_body) @fold
(switch_block) @fold
(block) @fold
(closure) @fold
(list_literal) @fold
(map_literal) @fold
(argument_list) @fold

(groovydoc_comment) @fold
(block_comment) @fold
