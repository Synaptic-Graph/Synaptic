# tree-sitter-groovy — Grammar Specification

Status: Draft v2 — 2026-05-17
Author: Elijah Zupancic.

This specification was informed by independent review of two prior
community Groovy grammars — `amaanq/tree-sitter-groovy@master` and
`murtaza64/tree-sitter-groovy@main` (both at default branch on
2026-05-17) — as inspiration only; no code from either project is
incorporated here. The authoritative references are the official
Apache Groovy language documentation (`https://groovy-lang.org/operators.html`,
`syntax.html`, `semantics.html`, `objectorientation.html`, `dsls.html`)
and the Apache Groovy reference grammar
(`https://github.com/apache/groovy/blob/master/src/antlr/GroovyParser.g4`,
`GroovyLexer.g4`) — both files retrieved 2026-05-17.

Revision history:

- v1 (initial draft) — section structure, all operators from issue
  #247, all murtaza64 open issues, external-scanner sketch.
- v2 (this revision) — corrected precedence levels to match Apache
  reference grammar exactly (range = shift, ternary = Elvis);
  added safe-chain-dot `??.` (Groovy 4); added contextual keywords
  `async` / `await` / `defer` / `module` / `goto` / `val` /
  `threadsafe`; documented `!instanceof` and `!in` trailing-context
  lexer rule from Apache; clarified subscript-chained-with-call
  composition; added triple-quoted edge cases from murtaza64
  string corpus.

This document is a **specification only** — no grammar code is generated
from it directly. It is the contract for the implementation work that
follows.

---

## 1. Goals and non-goals

### 1.1 Goals

1. Produce a tree-sitter grammar named `groovy` that parses idiomatic
   Groovy 2.x–4.x source files without `ERROR` nodes for the constructs
   listed in §3 ("Operator coverage matrix") and §4 ("Statement and
   declaration coverage").
2. Emit **distinct named nodes** for every Groovy-specific operator
   (Elvis, safe navigation, spread, identity, spaceship, regex find /
   match, exclusive ranges, `!instanceof`, `!in`, `as`, method pointer,
   direct field, safe index, Elvis-assign, logical implication, method
   reference `::`). Downstream tooling must be able to identify each
   operator by node kind alone — no `MISSING`-child inspection, no
   tree-shape heuristics. This is the contract that closes
   `dekobon/big-code-analysis#246` and `#247`.
3. Be a **standalone grammar** (no `grammar(Java, …)` inheritance). The
   primary failure mode of amaanq's grammar is that Java's grammar
   shapes the parse tree even where Groovy's syntax has diverged
   (e.g. `x as String` → two `local_variable_declaration` nodes,
   `trait T {}` → `juxt_function_call` + closure). Inheritance is too
   blunt a tool for a language with this many surface-syntax
   differences.
4. Preserve every strength documented in the murtaza64 grammar:
   purpose-built operator table, multi-flavor strings with
   interpolation, slashy and dollar-slashy strings, closures with `->`
   parameters, switch / try / for / while / for-in, Jenkins `pipeline`
   keyword, Groovydoc comments with `@param` / `@throws` tags, quoted
   identifiers, command-chain (`juxt_function_call`) support.
5. Fix the bugs catalogued in murtaza64's open issues (#5, #16, #22,
   #36, #37, #38, #39) — see §10 ("Issue resolution map") for the
   per-issue plan.
6. Ship a Cargo crate suitable for use by
   `dekobon/big-code-analysis` (`tree-sitter-groovy` ≥ 0.2.0,
   semver-compatible replacement for the existing `=0.1.2` pin), with
   the same `tree-sitter` minor-version requirement as the rest of the
   crate's grammar dependencies.

### 1.2 Non-goals

1. **No GroovyDoc semantics.** Block comments matching `/** … */` are
   recognised as `groovydoc_comment` so editors can theme them, but
   `@param` / `@return` parsing is not in scope for v1 (the murtaza64
   approach over-tokenises and is the root cause of #16 — empty
   `/***/` / `/**/` killing highlighting).
2. **No semantic checks.** "`!instanceof` requires a type on the
   right" is a runtime/compiler rule. The grammar accepts any
   expression and lets downstream tools check.
3. **No AST transformations / macros.** Groovy ASTs at runtime are
   compiler-internal.
4. **No Jenkinsfile-only mode.** `pipeline { … }` is supported as a
   regular declaration (resolving #37), but Jenkins-specific step
   methods (`sh`, `agent`, `stage`) are ordinary method calls — no
   special node kinds.
5. **No tracking of Groovy 5+ syntax** until released. We target the
   Groovy 4.x grammar; Groovy 3.x and 2.x are subsets and parse with
   the same productions.

---

## 2. Architecture

### 2.1 High-level shape

```text
tree-sitter-groovy/
├── grammar.js              # the entire grammar, hand-authored
├── src/
│   ├── parser.c            # generated
│   ├── scanner.c           # external scanner (see §6)
│   ├── tree_sitter/
│   │   └── parser.h
│   ├── node-types.json     # generated
│   └── grammar.json        # generated
├── queries/
│   ├── highlights.scm
│   ├── locals.scm
│   ├── injections.scm     # regex bodies, GString embedded expressions
│   ├── indents.scm
│   └── tags.scm
├── bindings/
│   ├── node/
│   ├── python/
│   ├── rust/
│   ├── go/
│   └── swift/
├── test/corpus/            # tree-sitter test files, see §8
├── Cargo.toml
├── package.json
├── pyproject.toml
├── binding.gyp
├── Package.swift
├── go.mod
├── tree-sitter.json
├── Makefile
├── README.md
└── SPECIFICATION.md        # this file
```

Layout matches the sibling grammars in this checkout
(`tree-sitter-tcl/`, `tree-sitter-irules/`), which is the integration
target for `dekobon/big-code-analysis` and several other consumers.

### 2.2 Why a standalone grammar (not `grammar(Java, …)`)

amaanq's choice to extend `tree-sitter-java` was pragmatic — Groovy
*claims* Java-syntax compatibility — but the cost is that every Groovy
feature absent from the Java parent has to fight the Java production
that *also* matches the same tokens. The dekobon issue #247 table is
that cost made concrete:

- `x as String` matches Java's variable declaration before Groovy's
  cast — emits two `local_variable_declaration`s.
- `trait Greeter { … }` matches Java's command-style call before the
  Groovy keyword — emits `juxt_function_call` + `closure`.
- `x !instanceof T` matches `! (x instanceof T)` because `!instanceof`
  is not a Java token.

The fixes for *each* of these would mean overriding the parent
production, at which point the parent rule is dead code in the child
grammar. The murtaza64 grammar's purpose-built operator table proves
the standalone approach scales: a single `binary_op` rule covers 30+
operators without any inherited noise.

We **do** keep a Java-shaped *layout* for class bodies, method
declarations, formal parameters, and annotation syntax — Groovy's
class-body syntax really is a Java superset there. We just refuse to
inherit from `tree-sitter-java` to get it.

### 2.3 Inputs the grammar borrows from each parent

| Feature | Source | Rationale |
|---|---|---|
| Operator precedence table (15-level) | **Apache reference grammar `expression` rule** | Authoritative source; we follow Apache's level ordering exactly (range = shift same level, ternary = Elvis same level). amaanq and murtaza64 each deviate in different ways. |
| Distinct `range_expression` node | amaanq | Keeps ranges out of `binary_expression`; eases metric impls |
| `juxt_function_call` rule | both | murtaza64's is more complete; we adopt its argument list, fix #5 with the precedence strategy in §5.5 |
| Multiline string fragments | amaanq | Cleaner; murtaza64's regex-heavy approach is the cause of #16 |
| `closure` with `->` parameters | murtaza64 | amaanq's closure has no parameter form |
| Slashy / dollar-slashy strings | murtaza64, scanner-promoted | murtaza64 has them as regular rules; we promote to external scanner to fix the division-vs-slashy ambiguity (see §6.2) |
| Groovydoc as a single token | murtaza64 (idea) + amaanq (impl) | murtaza64's per-tag parsing is the proximate cause of #16; we tokenise the whole block |
| `pipeline` statement | murtaza64 | we keep the rule but remove the `optional($.pipeline)`-at-end-of-file constraint (fix for #37) |
| Switch with arrow form | new | neither grammar has it; fix for #36 |
| Multi-catch | new | neither grammar has it; fix for #39 |
| Multi-assignment | new | neither grammar has it; fix for #22 |
| Trait, enum, record, sealed | new | fix for amaanq #247 trait line and murtaza64 #36 enum line |
| Named arguments | murtaza64 + spec | `map_item` in argument position; we tighten the rule |
| Method reference `::` | new | Groovy 3+ |
| Annotation `@interface` | murtaza64 | keep |
| Spread `*x`, `*.method()`, `*: m` | new | murtaza64 has spread as a prefix `access_op`; we split it into three productions for clarity and metric correctness |

---

## 3. Operator coverage matrix

This is the single source of truth for which operators MUST be
recognised and what node kind each emits. The precedence column is
the level in the table in §3.1; higher number = higher precedence
in the grammar's PREC table (tree-sitter convention is opposite of
the Groovy spec's "1 = highest", but we keep the spec's relative
order).

### 3.1 Precedence table — matches Apache reference grammar

This is the precedence ordering encoded in
`apache/groovy:src/antlr/GroovyParser.g4` (rule `expression`, lines
797–908). The numbering follows Apache's "level" comments
(1 = highest, 15 = lowest); the tree-sitter PREC constants we use
are simply 21 minus this number so higher = tighter — but the
*relative* order is what matters and it matches Apache exactly.

| Apache level | Name | Operators | Associativity | Notes |
|---:|---|---|---|---|
| 1 | UNARY_NOT | `~` `!` | right | |
| 2 | POWER | `**` | right | `1 ** 2 ** 3` → `1 ** (2 ** 3)` |
| 3 | UNARY_ADD | prefix `++` `--`, unary `+` `-` | right | |
| 4 | MUL | `*` `/` `%` | left | |
| 5 | ADD | `+` `-` (binary) | left | |
| 6 | SHIFT_OR_RANGE | `<<` `>>` `>>>` `..` `..<` `<..` `<..<` | left | **Apache puts ranges and shifts at the same level**, with left-associative resolution. `0..n<<1` parses as `(0..n)<<1`. |
| 7 | REL | `<` `<=` `>` `>=` `in` `!in` `instanceof` `!instanceof` `as` | left | `as` and `!instanceof` take a *type* on the right (not an expression); `in` / `!in` / `<` etc. take an expression. |
| 8 | EQ | `==` `!=` `<=>` `===` `!==` `=~` `==~` | left | Apache's comment calls regex "level 8.5" but the rule places them in the same alternative as `==` family. We treat them as one level. |
| 9 | BAND | `&` | left | |
| 10 | BXOR | `^` | left | |
| 11 | BOR | `\|` | left | |
| 12 | LAND | `&&` | left | |
| 13 | LOR | `\|\|` | left | |
| 13.5 | IMPL | `==>` | right | Apache uses `<assoc=right>`. |
| 14 | TERNARY_OR_ELVIS | `? :` and `?:` | right | **Apache puts ternary and Elvis at the same level**, in the same rule alternative (`conditionalExprAlt`). `a ?: b ?: c` parses as `a ?: (b ?: c)` by right-associativity. `a ? b ?: c : d` parses as `a ? (b ?: c) : d` because the recursive descent on `tb=expression` between `?` and `:` re-enters `expression` at this same level. |
| 15 | ASSIGN | `=` `**=` `*=` `/=` `%=` `+=` `-=` `<<=` `>>=` `>>>=` `&=` `^=` `\|=` `?=` | right | |

And primary / postfix / member-access levels, which Apache models
inside `postfixExpression` rather than the `expression` alternatives:

| Tier | Constructs | Associativity |
|---|---|---|
| POSTFIX | postfix `++` `--`, subscript `[]`, safe-index `?[]`, call `()` | left |
| ACCESS | `.` `?.` `??.` (safe chain dot) `*.` `.&` `.@` `::` | left |
| PRIMARY | literal, identifier, `()`, `new`, closure, list, map | n/a |

Notes on the corrected ordering versus v1 of this spec:

- **Range and shift are the same level.** v1 split them (claiming
  Apache treats ranges as a separate production). That was wrong:
  the Apache `shiftExprAlt` rule explicitly alternates `(LT LT |
  GT GT GT | GT GT)` with `rangeOp=(RANGE_INCLUSIVE | …)`. amaanq's
  grammar puts ranges above shifts, which is a *deviation* from
  Apache, not the Apache rule. We follow Apache.
- **Ternary and Elvis are the same level.** v1 placed Elvis one
  level below ternary so `a ? b ?: c : d` would parse as
  `a ? (b ?: c) : d`. That parse is correct — but Apache achieves
  it differently. In Apache, both `? :` and `?:` are in
  `conditionalExprAlt` as the *same* alternative; the parse
  `a ? (b ?: c) : d` falls out because the middle slot recursively
  re-enters `expression`, and within that recursion the same
  conditional rule matches again on `b ?: c`. We replicate this
  with a single `conditional_expression` rule that internally
  chooses between the ternary and Elvis forms, then aliases the
  result to the appropriate distinct node kind (`ternary_expression`
  or `elvis_expression`) — so downstream tools still see the
  distinct kinds promised in §3.2.
- **Implication `==>` is right-associative** per Apache
  `<assoc=right>`. v1 was correct on associativity but worth
  emphasising: `a ==> b ==> c` parses as `a ==> (b ==> c)`.
- **`as` and `!instanceof` take a type, not an expression.** Apache
  has `op=(AS | NOT_INSTANCEOF) nls coercionType` versus
  `op=INSTANCEOF nls matchingType` versus
  `op=(LE | GE | …) nls right=expression`. The grammar must
  thread this through — the RHS of `as` is `_type`, RHS of `<` is
  `_expression`. See §3.2 cast_expression and
  instanceof_expression fields.

### 3.2 Operator → node kind map

Every cell maps to a distinct named node so that
`get_op_type()` in big-code-analysis can do exact node-kind matching
instead of fishing through tree shape. This is the explicit ask in
issue #246's "Suggested fix" path 1.

| Symbol | Node kind | Field shape | Notes |
|---|---|---|---|
| `+` `-` `*` `/` `%` `<<` `>>` `>>>` `&` `^` `\|` `&&` `\|\|` `<` `<=` `>` `>=` `==` `!=` | `binary_expression` | `left`, `operator`, `right` | Java-shaped; precedence per §3.1. |
| `**` | `power_expression` | `left`, `right` | Right-associative; separate node so metric counts power-ops distinctly. |
| `?:` | **`elvis_expression`** | `value`, `default` | Resolves #246. No `MISSING` child anywhere. |
| `? :` | `ternary_expression` | `condition`, `consequence`, `alternative` | Standard. |
| `==>` | `logical_implication_expression` | `left`, `right` | Rare; emit distinct kind. |
| `===` | `identity_expression` | `left`, `operator: "==="`, `right` | Distinct kind avoids being lumped with `==`. |
| `!==` | `identity_expression` | `left`, `operator: "!=="`, `right` | Same kind as `===`; distinguished by operator field. |
| `<=>` | `spaceship_expression` | `left`, `right` | Distinct kind. |
| `=~` | `regex_find_expression` | `subject`, `pattern` | `pattern` is an expression (commonly slashy string). |
| `==~` | `regex_match_expression` | `subject`, `pattern` | As above. |
| `as` | `cast_expression` | `value`, `type` | `type` is a `_type`. NOT `binary_expression` — `x as String` is a coercion, not an arithmetic op. (Distinguishes from C-style cast `(String) x` which is `parenthesized_type_cast`.) |
| `in` | `membership_expression` | `element`, `operator: "in"`, `collection` | Distinct from generic binary so the `in` of `for (x in xs)` doesn't conflict. |
| `!in` | `membership_expression` | `element`, `operator: "!in"`, `collection` | Same kind, different operator. |
| `instanceof` | `instanceof_expression` | `value`, `operator: "instanceof"`, `type` | `type` slot is a `_type`. |
| `!instanceof` | `instanceof_expression` | `value`, `operator: "!instanceof"`, `type` | Same kind, different operator. |
| `..` | `range_expression` | `start`, `operator: ".."`, `end` | |
| `..<` | `range_expression` | `start`, `operator: "..<"`, `end` | |
| `<..` | `range_expression` | `start`, `operator: "<.."`, `end` | |
| `<..<` | `range_expression` | `start`, `operator: "<..<"`, `end` | |
| `?.` | `safe_navigation_expression` | `object`, `property` | `property` is `identifier` or `quoted_identifier`. |
| `??.` | `safe_chain_dot_expression` | `object`, `property` | **Groovy 4+.** Per `apache/groovy:GroovyLexer.g4:849` (`SAFE_CHAIN_DOT`), distinct from `?.`. Where `?.` only short-circuits when the *immediate* receiver is null, `??.` propagates null through the whole chain (Apache's own docstring: "safer than `?.`"). v1 of this spec omitted it; restored in v2. |
| `.` | `field_access` | `object`, `field` | The plain dot. |
| `.&` | `method_pointer_expression` | `object`, `method` | Distinct kind. |
| `.@` | `direct_field_access_expression` | `object`, `field` | Distinct kind. |
| `*.` | `spread_dot_expression` | `object`, `property` | Property/method access. |
| `*` (prefix in arg) | `spread_arguments` | `value` | Only valid inside argument lists / list literals. |
| `*:` (prefix in map) | `spread_map_entry` | `value` | Only valid inside map literals. |
| `?[]` | `safe_subscript_expression` | `object`, `index` | Distinct from regular `subscript`. |
| `[]` | `subscript_expression` | `object`, `index` | |
| `::` | `method_reference_expression` | `target`, `name` | Groovy 3+. |
| `++` / `--` (prefix) | `unary_update_expression` | `operator`, `operand` | |
| `++` / `--` (postfix) | `update_expression` | `operand`, `operator` | |
| unary `+` `-` `!` `~` | `unary_expression` | `operator`, `operand` | |
| `new` | `object_creation_expression` | `type`, `arguments` | |
| `=` and all augmented assignments | `assignment_expression` | `left`, `operator`, `right` | One node kind, operator field distinguishes. Includes `?=` (Elvis assign). |
| `?=` | `assignment_expression` (operator `"?="`) | as above | Distinct via operator field; semantically "assign if currently null/falsy." |

The full list of *symbols* required by issue #247 and resolved here:
`?:` ✓ `?.` ✓ `as` ✓ `..<` ✓ `*args` (spread args) ✓ `*.` ✓ `*:` ✓
`=~` ✓ `==~` ✓ slashy `/…/` ✓ `===` ✓ `!==` ✓ `<=>` ✓ `!instanceof` ✓
`!in` ✓ `?=` ✓ `?[]` ✓ `.&` ✓ `.@` ✓ `$/…/$` ✓ `==>` ✓.

Additional Groovy 4 operators not in issue #247 but recognised by
this spec: `??.` (safe chain dot), `::` (method reference, also
Groovy 3+).

Issue #247 also lists **`trait`** as a parse failure; covered in §4.

#### 3.2.1 Composability — subscript chained with call

`stepImplementations[Platform.ALL](ctx)` appears in murtaza64's
`realcode.test`. The construct is "subscript that yields a callable,
then invoked." Grammar implication: `subscript_expression` and
`safe_subscript_expression` are postfix forms that compose with the
following `argument_list` to produce a method invocation. The parse
tree shape:

```text
(method_invocation
  function: (subscript_expression
              object: (identifier "stepImplementations")
              index:  (field_access object: … field: …))
  arguments: (argument_list (identifier "ctx")))
```

This works automatically if `method_invocation`'s `function` field
accepts any `postfix_expression` (which includes subscripts), rather
than only an `identifier`. The amaanq grammar's `method_invocation`
uses `primary_expression` for the callee slot, which suffices. The
spec inherits that breadth and explicitly tests this case.

Field name is `function:` (not `name:` or `object:`) because the
callee may be any `_expression` — identifier, field-access,
subscript, or spread-dot — and `name:` / `object:` each describe
only a subset of those shapes. All §8 corpus snippets and the
`queries/groovy/highlights.scm` patterns use this field name.

#### 3.2.2 `!instanceof` and `!in` — lexer trailing-context rule

Apache `GroovyLexer.g4` defines these tokens with a positive
trailing-context predicate:

```antlr
NOT_INSTANCEOF : '!instanceof'
   { isFollowedBy(_input, ' ', '\t', '\r', '\n') }?;
NOT_IN         : '!in'
   { isFollowedBy(_input, ' ', '\t', '\r', '\n', '[', '(', '{') }?;
```

…i.e., the lexer only emits `!instanceof` if the next character is
whitespace, and `!in` if followed by whitespace or an opening
bracket. Without this rule, `!instanceofX` would be a single
identifier-looking token (or the parse would be ambiguous between
`!instanceof X` and `! instanceofX`).

The tree-sitter external scanner replicates this: `!instanceof` and
`!in` are scanner-emitted tokens that only fire when the trailing
character is one of the permitted ones. The grammar rule for the
unary `!` operator otherwise consumes the `!` and parses the rest
as an expression.

---

## 4. Statement and declaration coverage

Productions that must be supported. Every row that says "new" is a
fix relative to *both* parent grammars.

| Construct | Status | Notes |
|---|---|---|
| `package` declaration | port from amaanq | both grammars have it; trim trailing-`;` requirement to optional |
| `import` (including static and aliased) | port from amaanq | `import static a.b.C.*`, `import a.b.C as D` |
| `class` / `interface` / `@interface` declaration | port from murtaza64 | |
| `trait` declaration | **new** | identical body to `class`; distinct node kind `trait_declaration`. Closes the `trait` row in #247. |
| `enum` declaration | **new** | constants list followed by optional `;` then class body; arrow switch is the typical use site (#36). |
| `record` declaration | **new** | `record Name(Type a, Type b) { … }`. Groovy 4+. |
| `sealed` / `non-sealed` / `permits` modifiers | **new** | parsed as modifier tokens that take a `permits` clause on class/interface. Groovy 4+. |
| Method declaration | port from murtaza64 | `def` or type, optional throws clause |
| Method definition (with body) | port from murtaza64 | |
| Multi-assignment | **new** | `def (a, b) = expr` and `(a, b) = expr`. Closes murtaza64 #22. |
| Local variable declaration | port from both | including `def` and explicit types |
| Closure expression | port from murtaza64 | `{ a, b -> … }` and `{ … }`. |
| `if` / `else` | port | |
| `while` / `do-while` | port | |
| `for(;;)` | port | `init` accepts a single expression (`i = 0`) or a `local_variable_declaration` (`int i = 0`, `def i = 0`, incl. multiple declarators). Apache `forInit: localVariableDeclaration \| expressionList` also allows a comma expression list (`i = 0, j = 1`) — not yet supported. |
| `for (x in xs)` | port | |
| `switch` with `case … :` | port | |
| `switch` with `case … -> …` | **new** | arrow form. Closes murtaza64 #36 (second half). |
| `try` / `catch` / `finally` | port | |
| Multi-catch | **new** | `catch (Foo \| Bar e)`. Closes murtaza64 #39. |
| Try-with-resources | **new** | `try (Foo f = …) { … }`. |
| `return`, `break`, `continue`, `yield` | port | break/continue accept an optional label. |
| `throw` | port | |
| `assert e` and `assert e : msg` | port from murtaza64 | power-assertion shape. |
| Labeled statement | port from murtaza64 | `name: statement`. Disambiguation vs. map literal in §5.3. |
| Command chain (`juxt_function_call`) | port from murtaza64 + §5.5 | |
| `pipeline { … }` followed by anything | port + fix | Promote `pipeline` to a regular statement, drop `optional($.pipeline)`-at-EOF. Closes #37. |
| Annotation usage `@Foo(args)` | port | including `@Foo` and `@Foo.Bar(x = 1)`. |
| Shebang line | port from amaanq | `#! …\n` at start of file. |
| Line comment `// …` | port | |
| Block comment `/* … */` | **fixed** | Single `block_comment` token. Must accept `/**/` and `/***/`. Closes murtaza64 #16. |
| Groovydoc `/** … */` | port from murtaza64 (but tokenised) | Single `groovydoc_comment` token; tag parsing is out of scope per §1.2(1). |

---

## 5. Lexical and syntactic edge cases

This section is the "sequential thinking" output: each tricky bit of
Groovy gets a discrete decision rather than being hand-waved.

### 5.1 Numeric literals

```text
integer_literal  = bin | oct | hex | dec [type_suffix]
float_literal    = digits "." digits [exponent] [type_suffix]
                 | "." digits [exponent] [type_suffix]
                 | digits exponent [type_suffix]
type_suffix      = one-of  G g L l I i D d F f
```

- `_` separators allowed between digits per Java rules.
- Leading sign is **not** part of the literal — `-3` is `unary_expression`
  over `3`. (murtaza64's grammar bakes the sign into the literal regex,
  which breaks `a-3` parsing in column position; we don't repeat that.)
- The `G` suffix promotes to `BigInteger`/`BigDecimal` per the Groovy
  spec — this affects the type system but the grammar treats it as a
  pure suffix.

### 5.2 Identifiers and reserved-but-contextual words

Groovy's keyword set, derived from `apache/groovy:GroovyLexer.g4`
lines 400–501, splits into three groups:

**Hard reserved keywords** (always keywords, can never be
identifiers): `abstract`, `assert`, `boolean`, `break`, `byte`,
`case`, `catch`, `char`, `class`, `const`, `continue`, `default`,
`do`, `double`, `else`, `enum`, `extends`, `final`, `finally`,
`float`, `for`, `if`, `goto`, `implements`, `import`, `instanceof`,
`interface`, `int`, `long`, `native`, `new`, `package`, `private`,
`protected`, `public`, `return`, `short`, `static`, `strictfp`,
`super`, `switch`, `synchronized`, `this`, `throw`, `throws`,
`transient`, `try`, `void`, `volatile`, `while`. Also the
Groovy-specific hard keywords `def`, `trait`, `module`,
`threadsafe`.

**Contextual keywords** (keywords only in specific syntactic
positions, identifiers elsewhere): `as`, `in`, `var`, `record`,
`sealed`, `non-sealed`, `permits`, `yield`, `async`, `await`,
`defer`, `val` (the last four added in v2 of this spec from the
Apache lexer; `val` is `'val' {isValEnabled()}?` — conditionally
enabled). Each appears in only one position-specific rule:

- `as` — RHS operator of `cast_expression`; also `import X as Y`.
- `in` — RHS operator of `membership_expression`; also `for (x in xs)`.
- `var` — replaces `def` as a declaration type token.
- `record`, `sealed`, `non-sealed`, `permits` — class-declaration
  prefixes/clauses.
- `yield` — switch-expression branch.
- `async` — closure/lambda prefix (`async closureOrLambdaExpression`).
- `await` — unary prefix on an expression or expression list.
- `defer` — block prefix.

The grammar encodes contextual keywords as ordinary tokens with
explicit fallbacks where they may also be identifiers:

```js
identifier_or_contextual: $ => choice($.identifier, 'var', 'record',
                                     'sealed', 'non-sealed',
                                     'permits', 'yield', 'async',
                                     'await', 'defer', 'val')
```

…and uses `identifier_or_contextual` everywhere the lexer might
otherwise hide a usable identifier behind a keyword (variable names,
property accesses, named arguments).

Quoted identifiers (`map."key with spaces"`, `def "test name"() { … }`)
are supported by allowing a string-literal in identifier position
inside method declarations and after `.`. Per the Groovy syntax
docs, this is how methods with keyword names work too:
`def "abstract"() { true }`.

### 5.3 Label vs. map literal vs. ternary

Three syntactic positions where `:` is ambiguous:

1. `LABEL: stmt` — statement label.
2. `[ k : v ]` — map entry.
3. `c ? t : e` — ternary.

Disambiguation strategy:

- **Labels** are only matched at statement-start position; the
  statement rule sequences `optional($.label)` *before* the
  statement choice. The label rule itself is
  `seq(field('name', $.identifier), ':', /[ \t]*/, NEWLINE_OR_NONE)`
  — but tree-sitter cannot peek arbitrarily, so we use a positive
  lookahead via the external scanner (token `_label_colon`) that
  succeeds only when the colon is at statement-start and the next
  non-whitespace token is not a keyword that starts a sub-expression.
- **Map entries** sit inside `[ … ]` or argument lists. The map rule
  pre-empts the list rule via tree-sitter precedence.
- **Ternary colons** are an internal `:` already nested inside the
  ternary production; no ambiguity once `?` has been seen.

murtaza64's grammar gets the label/map/ternary distinction working
*mostly* by precedence (`assignment` at `-1`, `parenthesized_expression`
at `PREC.PRIORITY`); we make the rule explicit with the external scanner
token because the precedence-only approach failed on the test cases in
murtaza64 #5 (`a b, c ? d : e` and `f y:b ? … : …`).

### 5.4 Slashy strings vs. division

`/foo/` is a slashy string regex literal. `a / b` is division. The
disambiguator is a context-sensitive lexer token:

- A `/` *can* start a slashy string only if the preceding non-trivia
  token is one of:
  - statement boundary (start-of-file, `;`, `\n` at statement-end,
    `{`, `(`, `[`, `,`, `:`, `=`, `=>`, `->`),
  - any operator that takes an expression on its right (`+`, `-`,
    `*`, `/` itself if a previous `/` was already eaten as division,
    `==`, `!=`, etc.),
  - the keywords `return`, `yield`, `throw`, `in`, `as`,
    `instanceof`, `case`, `when`.
- Otherwise the `/` is division (or `/=`).

This decision lives in the external scanner (§6.2). The corresponding
test (`def s = /pat/`, `a = b / c`, `[1, /pat/, 2]`) is in §8.

Dollar-slashy `$/…/$` does not have this ambiguity — `$` is not a
binary operator — so it is a simple grammar rule.

### 5.5 Command chains (`juxt_function_call`)

The murtaza64 grammar's `juxt_function_call` has open issue #5 —
optional parentheses produce ambiguities with ternaries and map items.
The resolution we adopt:

1. **Command chains are statement-level only.** Per the Groovy spec
   and the Apache grammar, `a b c d` is a `commandExpression`, which
   in `GroovyParser.g4` appears under `statement → expressionStatement
   → commandExpression`. It is **not** a general expression. This
   removes most of murtaza64's nested-juxt-call conflicts.
2. **First token is an identifier or property-access chain**, not
   any expression — so `(x + y) z` is a syntax error, not a juxt-call
   on a parenthesised expression.
3. **Argument forms allowed**: positional args (literals, identifiers,
   dotted paths, closures), named args (`k: v` where `k` is an
   identifier or string), and trailing closure. Disallowed: nested
   command chains, ternaries with unparenthesised conditions inside
   the argument list (forces `f (a ? b : c)`).
4. **Chain continuation**: after the first call, subsequent `name`
   tokens become property/method dispatches. The Apache grammar uses
   `commandArgument: pathElement+ argumentList?` for the chain
   continuation — we mirror that with a `command_chain_segment` rule.
5. **Conflicts** with `binary_expression` (where `a - b` could be
   "call `a` with `-b`") are resolved by giving `binary_expression`
   higher precedence: command chains *only* match when there is a
   space and the next token is an identifier or a literal that is
   not a binary operator.

This is meaningfully more conservative than murtaza64's rule, which
attempts to allow command chains in argument lists. We trade DSL
expressiveness for parse-tree reliability — the metric tooling and
syntax-highlighter targets are better served by reliable parses on
the 95% case than ambitious parses on the 5%.

### 5.6 Optional parentheses

Groovy lets you write `println foo` for `println(foo)`. That **is** a
juxt-function call (§5.5) and is the dominant form. Method calls
*nested in expressions* always require parentheses
(`return f(x)` not `return f x`). The grammar does not need a
separate rule.

### 5.7 GString interpolation

The double-quoted-string body alternates between literal fragments,
escape sequences, and interpolation segments. Two interpolation
forms:

- `$identifier` — followed by an immediate identifier (with optional
  dotted property access, no method call).
- `${ expression }` — full expression, recursively parsed.

Grammar:

```js
interpolation: $ => choice(
  seq('$', alias($.identifier, $.interpolation_identifier),
        repeat(seq(token.immediate('.'), $.identifier))),
  seq('${', $.expression, '}'),
)
```

Triple-quoted strings allow interpolation only in `"""…"""` form.
Single-quoted `'…'` and `'''…'''` are *plain* — no interpolation.

Slashy strings allow GString interpolation. Dollar-slashy strings
allow interpolation with `$identifier` and `${expr}` — and treat
`$/` and `$$` as escapes.

The lexer state machine for interpolation lives in the external
scanner (§6.3). This avoids tree-sitter's "ambiguity over the entire
file" trap when a stray `"` mid-source could otherwise re-anchor the
string lexer.

**Triple-quoted edge cases** (from murtaza64's `string.test`):

1. **Empty triple-quoted string**: `""""""` is valid — opener `"""`,
   empty body, closer `"""`. The scanner must not greedily consume
   five quotes as `"""""`+`"` looking for a longer match.
2. **Embedded one-or-two quotes before interpolation**:
   `print("""hello world ""${0}""")` — the body contains literal
   `""` immediately before `${0}`, then literal `""` before the
   closing `"""`. The scanner emits body fragments that may
   *contain* one or two consecutive `"` characters, only stopping
   on the third consecutive `"` (or on `$` introducing interpolation).
3. **Embedded `"` inside body**: `"""this has "quotes" inside"""`.
4. **Backslash before interpolation in triple-quoted**:
   `"""text \$ ${expr}"""` — the `\$` escapes the dollar, the
   subsequent `${expr}` is normal interpolation.

The scanner contract for `"""` bodies:

```text
Consumes characters until:
  - three consecutive unescaped `"` → emit BODY (without the closing
    `"""`), return; the grammar then matches the literal `"""`.
  - `$` followed by `{` or identifier-start → emit BODY, return for
    interpolation handling.
  - `\` consumes the next character as escape.
  - one or two consecutive `"` followed by anything other than `"`
    are included in the body fragment, not treated as a close.
```

**Dollar-slashy edge cases** (from murtaza64's `string.test`):

```groovy
x = $/
hello
world
$$ dollar          // $$ is literal $
/                  // bare slash is literal
\                  // bare backslash is literal
$interp            // GString interpolation
$dotted.interp     // dotted property interpolation
$/ escaped slash   // $/ is literal /
/$
```

Escape rules:

- `$$` → literal `$`.
- `$/` → literal `/`.
- `\` (anywhere except before the closer) → literal `\`.
- `$identifier` → interpolation.
- `${expr}` → interpolation.
- `/$` (only when followed by no further content within the literal)
  → terminator.

### 5.8 Closure parameters and the `it` shorthand

`{ a, b -> … }` declares parameters; `{ … }` with no `->` implicitly
binds `it`. The grammar rule:

```js
closure: $ => seq('{',
  optional(seq($._closure_parameters, '->')),
  repeat($.statement),
  '}')
```

We do **not** emit a synthetic `it` parameter. Downstream tools that
care can match `closure` nodes with no parameter list.

### 5.9 Annotations

`@Foo`, `@Foo(arg)`, `@Foo(k1=v1, k2=v2)`, `@a.b.Foo(…)`, `@Foo @Bar`
(stacked). Annotation `@interface` declaration (Java annotation type
syntax) is a separate class-declaration form (§4).

### 5.10 Statement terminators

Groovy treats both newline and `;` as statement terminators *most of
the time*. The grammar handles the two cases differently, both
in-grammar (no external `_terminator` token — see §6 on why dormant
scanner branches are avoided):

- **Newline** is whitespace (`/\s/` is in `extras`), so statements are
  delimited purely structurally: a statement list is `repeat(...)` and
  two adjacent complete statements (`x()` then `y()`) need no explicit
  separator. Inside `(`/`[`/`{` a newline is likewise just whitespace,
  matching Groovy's rule.
- **Semicolon** `;` is an optional, anonymous element of every
  statement list (`repeat(choice($._statement, ';'))`) and member list
  (`repeat(choice($._class_member, ';'))`). It never appears as a named
  node in the AST, so wherever it attaches the named-node tree is
  identical. This covers terminators (`x();`), separators (`x(); y()`),
  empty statements (`;`, `;;`), and empty / terminated members
  (`class C { ; }`, `void f();`, `int x = 1;`). The list contexts are
  `source_file`, `block`, `closure`, the classic `switch`
  `case`/`default` bodies, and the class / interface / trait / enum
  member bodies.
- Three single-statement (non-list) bodies take an explicit
  `optional(';')` so their terminator is not claimed by the *enclosing*
  list or rejected before a required follow-on token: the braceless
  `if` consequence (without it the `;` in `if (x) a(); else b()` would
  be eaten by the outer list and the `else` severed into a stray
  declaration), the braceless `do`-`while` body (`do a(); while (c)`,
  where `while` must follow), and the `switch` arrow-case body
  (`case 1 -> foo();`, matching the Java/Groovy `case L -> expr;`
  form). For `if`, the `;` sits *after* the consequence and *before*
  the optional `else`, so the no-`else` form (`if (x) a(); b()`) does
  not error.

An earlier draft proposed a `_terminator` external scanner token
matching `;`-or-meaningful-newline; it was never wired, because the
in-grammar split above is simpler and avoids a dormant scanner branch
(§6). amaanq's grammar uses `DELIMITER = choice(';', /\n/, '\0')`
literally in every statement rule — this works for tree-sitter but
produces ambiguity at line continuations (`a +\n b` would split into
two statements), which the structural-newline approach sidesteps.

### 5.11 Spread in three positions

| Position | Symbol | Production |
|---|---|---|
| Argument list | `*expr` | `spread_arguments` |
| Member access | `recv*.prop` / `recv*.method(args)` | `spread_dot_expression` |
| Map literal entry | `*: expr` | `spread_map_entry` |

murtaza64 collapses these under `access_op` which mis-shapes the
spread-args case (`*x` in a call) and the spread-map case (`*:`).
We split them.

Apache `GroovyParser.g4` rule `argumentList` confirms the split:
`expressionListElement: MUL? expression` — the optional asterisk
at expression-element start is the spread, not a multiplication
operator. The tree-sitter rule mirrors this with an explicit
alternative for `seq('*', $.expression)` inside `argument_list`.

### 5.12 `def` in different positions

`def` can prefix:

- a local variable declaration (`def x = …`),
- a method definition (`def f() { … }`),
- a closure parameter (`{ def x -> … }`),
- a multi-assignment (`def (a, b) = …`).

The grammar uses `def` as a token in each rule rather than as a
shared `_optional_def` non-terminal, because the conflicts each rule
allows are different (a method definition allows a return-type-like
slot before `def`; a variable declaration does not).

### 5.13 Trailing commas

Lists, maps, argument lists, parameter lists, enum constants — all
accept an optional trailing comma. Use `commaSep1` + `optional(',')`
in every list rule.

### 5.14 Method reference `::`

Groovy 3+: `String::length`, `obj::method`, `Foo::new`. Grammar:

```js
method_reference_expression: $ => prec.left(PREC.ACCESS, seq(
  field('target', choice($.identifier, $._type, $.expression)),
  '::',
  field('name', choice($.identifier, 'new'))))
```

Conflicts with C++ `::`-scoped names are not relevant in Groovy.

### 5.15 Parenthesized type cast

`(String) x` — C-style cast. Distinct from `x as String`. Grammar:

```js
parenthesized_type_cast: $ => prec.right(PREC.UNARY, seq(
  '(', $._type, ')', $.expression))
```

Ambiguity with parenthesized expression: `(a) b` could be a cast
(if `a` is a type) or a juxt-call. Resolved by precedence and
GLR: if `a` matches `_type`, prefer the cast; otherwise it's a
parenthesized expression followed by a juxt-call (which is a
syntax error in expression position, by §5.5).

---

## 6. External scanner design

The external scanner is the C file `src/scanner.c`. It exists to
handle the four context-sensitive lexing situations that pure
tree-sitter grammar rules cannot disambiguate. Each token below has a
single emit/no-emit decision per call.

### 6.1 Tokens exported

```c
enum TokenType {
  AUTOMATIC_SEMICOLON,    // §5.10 — newline-as-terminator
  SLASHY_STRING_START,    // §5.4 — opening `/` of slashy regex
  SLASHY_STRING_BODY,     // §5.4 — body (until unescaped `/`)
  SLASHY_STRING_END,      // §5.4 — closing `/`
  DOLLAR_SLASHY_STRING,   // §5.7 — single-token `$/.../$` when no interpolation
  GSTRING_BODY,           // §5.7 — literal fragment of a double-quoted string
  GSTRING_INTERPOLATION_START, // `${` or `$` followed by identifier
  LABEL_COLON,            // §5.3 — colon that closes a statement label
  BLOCK_COMMENT,          // single-token block comment incl. /**/ and /***/
  GROOVYDOC_COMMENT,      // single-token /** ... */ comment
};
```

### 6.2 SLASHY_STRING — implementation contract

```text
Inputs visible to scanner:
  - previous_non_trivia_token_kind  (tracked in scanner state)
  - current lookahead char

State (scanner state struct):
  - last_token: one of OPERATOR, OPEN_BRACKET, COMMA, COLON,
    KEYWORD_RHS, IDENTIFIER, LITERAL, CLOSE_BRACKET, NONE
  - paren depth, bracket depth, brace depth (for GString containment)

SLASHY_STRING_START emits iff:
  - lookahead is `/`
  - AND NOT immediately followed by `=` (rule out `/=` augmented assign)
  - AND NOT immediately followed by another `/` (rule out `//` comment)
  - AND NOT immediately followed by `*` (rule out `/*` comment)
  - AND last_token ∈ { OPERATOR (any binary except postfix),
                       OPEN_BRACKET, COMMA, COLON, KEYWORD_RHS,
                       NONE (start-of-file or right after newline-as-terminator) }

SLASHY_STRING_BODY consumes chars until:
  - unescaped `/` → emit BODY (possibly empty), do NOT consume `/`, return
  - `\\/` → consumed as escape, continue
  - `\\` followed by any other char → consumed literally, continue
  - `$` followed by `{` or identifier → emit BODY, return (caller
    matches GSTRING_INTERPOLATION_START)
  - `\n` is allowed (slashy strings are multi-line per spec)

SLASHY_STRING_END emits iff lookahead is `/` AND state says we are
mid-slashy.
```

### 6.3 GSTRING — implementation contract

Double-quoted strings, triple-double-quoted strings, slashy, and
dollar-slashy all share the GString machinery (interpolation rules,
escape rules). Differences encoded by per-flavor state.

```text
After a `"` or `"""` is consumed in the grammar:
  GSTRING_BODY consumes chars until:
    - matching `"` (or `"""`) → emit BODY (possibly empty), return
    - `\\` followed by any char → consumed as escape
    - `$` followed by `{` → emit BODY, return (caller matches `${`)
    - `$` followed by `[A-Za-z_]` → emit BODY, return
    - `\n` is allowed in triple-quoted variants; not in single
      double-quoted (emit ERROR, let grammar handle).
```

Brace depth tracking inside `${ … }` is essential: a `}` only ends
the interpolation when the interpolation's own brace counter is at
zero. The scanner increments the counter on `{` and decrements on `}`
within an active interpolation context. When the counter hits zero,
the `}` is emitted as the interpolation terminator and the scanner
returns to GSTRING_BODY mode.

### 6.4 AUTOMATIC_SEMICOLON — implementation contract

```text
Emits iff:
  - lookahead is `\n` (or sequence of `\n`s)
  - previous non-trivia token can end a statement
    (identifier, literal, closing bracket, `++`, `--`, etc.)
  - we are NOT inside `(` or `[` (brace depth doesn't suppress)
  - the next non-trivia token would NOT continue the expression
    (i.e., it is not a binary operator, not a dot, not `?`, not `:`
    inside a known ternary)

Else: consume newline as whitespace.
```

This resolves the line-continuation problem and is the underlying
reason murtaza64's grammar can't parse `pipeline {}\ndef foo = 5`
(issue #37). The pipeline rule there ends at `}`; the newline becomes
a statement terminator; the next statement begins. The murtaza64
grammar instead requires `pipeline` to be the *final* statement
because it does not have AUTOMATIC_SEMICOLON.

### 6.5 LABEL_COLON — implementation contract

```text
Emits iff:
  - the immediately preceding tokens form an `identifier` at
    statement-start position (we have not consumed any non-newline
    non-comment tokens since the last statement terminator)
  - lookahead is `:`
  - the token after `:` is NOT a sub-expression-starting token (a
    digit, a quote, a `(`, a `[`, an operator, `?`, `:`)

Else: do not emit; the `:` is consumed by whatever rule is in
flight (ternary, map item, switch case).
```

### 6.6 Block comment scanner

A single token `BLOCK_COMMENT` (or `GROOVYDOC_COMMENT` when the open
sequence is `/**`) matches the entire comment. This fixes
murtaza64 #16: the regex `seq('/**', token.immediate(/[*\n\s]+/), …)`
in murtaza64's grammar requires at least one whitespace character
after `/**`, so `/**/` (empty groovydoc) and `/***/` (immediate `*/`)
both fail. A scanner-driven token simply consumes from the opener to
the matching `*/` regardless of intermediate content.

---

## 7. Tree-sitter conflict declarations

The expected `conflicts` list, in order of root cause:

```js
conflicts: $ => [
  // Standard Groovy/Java ambiguity: label vs. expression-statement
  [$.label, $.identifier],
  // Map entries vs. labeled statements (resolved by external scanner)
  [$.map_entry, $.label],
  // Generic method call vs. less-than expression (`f<X>()` vs. `f < X > ()`)
  [$._type_arguments, $.binary_expression],
  // Type cast vs. parenthesized expression: (Foo) x  vs  (Foo)
  [$.parenthesized_type_cast, $.parenthesized_expression],
  // Closure as map literal: `{ k: v }` is a closure with a label,
  // `[k: v]` is a map. Inside a closure-only position (lambda body)
  // we need to prefer closure.
  [$.closure, $.map_literal],
  // Command chain vs. expression statement: `a b c` vs `a; b; c`.
  [$.command_chain, $.expression_statement],
  // Method reference vs. ternary fallback path
  [$.method_reference_expression, $.ternary_expression],
  // Trait body shares closure rule until we hit a method declaration
  [$.trait_declaration, $.class_declaration],
];
```

The amaanq grammar's 14 inherited conflicts collapse to roughly half
this set once we stop inheriting Java's `_unannotated_type`
ambiguities.

---

## 8. Test corpus plan

Tests live in `test/corpus/`. Each file is a tree-sitter test corpus
(name on top, source between `=` lines, S-expression after `---`).

Per issue #247, "regression tests anchored per AGENTS.md and lesson
1" — each operator gets a test pinned to its known-good S-expression.

### 8.1 Per-operator corpus

`test/corpus/operators.txt`:

```text
================================
Elvis at top level
================================

def x = a ?: b

---

(source_file
  (local_variable_declaration
    (variable_declarator
      name: (identifier)
      value: (elvis_expression
        value: (identifier)
        default: (identifier)))))

================================
Elvis chain (issue #246 reproducer)
================================

def pick(a, b, c) {
    return a ?: b ?: c
}

---

(source_file
  (method_declaration
    name: (identifier)
    parameters: (formal_parameters
      (formal_parameter name: (identifier))
      (formal_parameter name: (identifier))
      (formal_parameter name: (identifier)))
    body: (block
      (return_statement
        (elvis_expression
          value: (identifier)
          default: (elvis_expression
            value: (identifier)
            default: (identifier)))))))
```

`test/corpus/operators.txt`:

```text
================================
Safe navigation single
================================

obj?.field

---

(source_file
  (expression_statement
    (safe_navigation_expression
      object: (identifier)
      property: (identifier))))

================================
Safe navigation chained
================================

obj?.field?.other

---

(source_file
  (expression_statement
    (safe_navigation_expression
      object: (safe_navigation_expression
        object: (identifier)
        property: (identifier))
      property: (identifier))))
```

`test/corpus/operators.txt` (covers spread args, spread-dot,
spread-map):

```text
================================
Spread arguments
================================

f(*args)

---

(source_file
  (expression_statement
    (method_invocation
      function: (identifier)
      arguments: (argument_list
        (spread_arguments value: (identifier))))))

================================
Spread dot
================================

items*.size()

---

(source_file
  (expression_statement
    (method_invocation
      function: (spread_dot_expression
        object: (identifier)
        property: (identifier))
      arguments: (argument_list))))

================================
Spread map
================================

def m = [*: other, x: 1]

---

(source_file
  (local_variable_declaration
    (variable_declarator
      name: (identifier)
      value: (map_literal
        (spread_map_entry value: (identifier))
        (map_entry key: (identifier) value: (number_literal))))))
```

Other coverage required in `test/corpus/operators.txt` (original upstream file names):

- `operators-regex.txt` (=~, ==~ with slashy and double-quoted RHS)
- `operators-identity.txt` (===, !==)
- `operators-spaceship.txt` (<=>)
- `operators-range.txt` (.., ..<, <.., <..<)
- `operators-membership.txt` (in, !in)
- `operators-instanceof.txt` (instanceof, !instanceof)
- `operators-coercion.txt` (`x as String`, `def y = list as Set` —
  paired with `def y = (Set) list` to keep cast vs. coercion distinct)
- `operators-method-pointer.txt` (.&)
- `operators-direct-field.txt` (.@)
- `operators-safe-index.txt` (?[])
- `operators-method-reference.txt` (::)
- `operators-elvis-assign.txt` (?=)
- `operators-implication.txt` (==>)
- `operators-power.txt` (** including `2 ** 3 ** 2` right-assoc test)
- `operators-safe-chain-dot.txt` (??.) — Groovy 4+; v2 addition
- `operators-precedence-tiebreaks.txt` — explicit tests that pin
  the Apache precedence ordering on the tie-break cases:
  - `0..n<<1` → `(0..n)<<1` (range/shift same level, left-assoc)
  - `a ? b ?: c : d` → `a ? (b ?: c) : d`
    (Elvis nests inside ternary's middle slot)
  - `a ?: b ?: c` → `a ?: (b ?: c)` (right-assoc)
  - `a ==> b ==> c` → `a ==> (b ==> c)` (right-assoc)
  - `1 ** 2 ** 3` → `1 ** (2 ** 3)` (right-assoc)

### 8.2 Per-issue regression corpus

`test/corpus/regressions.txt` — one example per closed issue, with a
header comment naming the issue:

```text
=========================================================
murtaza64 #5 — juxt_function_call ternary disambiguation
=========================================================

def x = foo "bar"
print foo y, z

---

(source_file
  (local_variable_declaration
    (variable_declarator
      name: (identifier)
      value: (command_chain
        receiver: (identifier)
        argument: (string_literal))))
  (command_chain
    receiver: (identifier)
    argument: (command_chain
      receiver: (identifier)
      argument: (identifier)
      argument: (identifier))))

=========================================================
murtaza64 #16 — empty groovydoc / block comment
=========================================================

/**/
def a = 1
/***/
def b = 2
/* */
def c = 3

---

(source_file
  (groovydoc_comment)
  (local_variable_declaration
    (variable_declarator name: (identifier) value: (number_literal)))
  (groovydoc_comment)
  (local_variable_declaration
    (variable_declarator name: (identifier) value: (number_literal)))
  (block_comment)
  (local_variable_declaration
    (variable_declarator name: (identifier) value: (number_literal))))

=========================================================
murtaza64 #22 — multiple assignment
=========================================================

def (a, b) = [1, 2]

---

(source_file
  (multi_assignment_declaration
    (variable_declarator name: (identifier))
    (variable_declarator name: (identifier))
    value: (list_literal
      (number_literal)
      (number_literal))))

=========================================================
murtaza64 #36 — enum + switch arrow
=========================================================

enum Color { R, G, B }

def toS(c) {
    switch (c) {
        case Color.R -> 'r'
        case Color.G -> 'g'
        case Color.B -> 'b'
    }
}

---

(source_file
  (enum_declaration
    name: (identifier)
    body: (enum_body
      (enum_constant name: (identifier))
      (enum_constant name: (identifier))
      (enum_constant name: (identifier))))
  (method_declaration
    name: (identifier)
    parameters: (formal_parameters (formal_parameter name: (identifier)))
    body: (block
      (switch_expression
        value: (identifier)
        body: (switch_block
          (switch_arrow_case
            value: (field_access
              object: (identifier)
              field: (identifier))
            body: (string_literal))
          (switch_arrow_case
            value: (field_access
              object: (identifier)
              field: (identifier))
            body: (string_literal))
          (switch_arrow_case
            value: (field_access
              object: (identifier)
              field: (identifier))
            body: (string_literal)))))))

=========================================================
murtaza64 #37 — pipeline followed by definition
=========================================================

pipeline {}
def foo = 5

---

(source_file
  (pipeline_statement body: (closure))
  (local_variable_declaration
    (variable_declarator name: (identifier) value: (number_literal))))

=========================================================
murtaza64 #39 — multi-catch
=========================================================

try {
    risky()
} catch (IOException | RuntimeException e) {
    handle(e)
}

---

(source_file
  (try_statement
    body: (block (expression_statement (method_invocation function: (identifier) arguments: (argument_list))))
    (catch_clause
      parameter: (catch_formal_parameter
        type: (multi_type
          (type_identifier)
          (type_identifier))
        name: (identifier))
      body: (block (expression_statement (method_invocation function: (identifier) arguments: (argument_list (identifier))))))))

=========================================================
dekobon #246 — Elvis as elvis_expression, not ternary
=========================================================

def pick(a, b, c) {
    return a ?: b ?: c
}

---

(source_file
  (method_declaration
    name: (identifier)
    parameters: (formal_parameters
      (formal_parameter name: (identifier))
      (formal_parameter name: (identifier))
      (formal_parameter name: (identifier)))
    body: (block
      (return_statement
        (elvis_expression
          value: (identifier)
          default: (elvis_expression
            value: (identifier)
            default: (identifier)))))))

=========================================================
dekobon #247 — trait declaration
=========================================================

trait Greeter {
    def greet() {
        "hello"
    }
}

---

(source_file
  (trait_declaration
    name: (identifier)
    body: (class_body
      (method_declaration
        name: (identifier)
        parameters: (formal_parameters)
        body: (block (string_literal (string_fragment)))))))

=========================================================
dekobon #247 — exclusive range
=========================================================

for (i in 0..<n) { f(i) }

---

(source_file
  (for_in_statement
    variable: (identifier)
    value: (range_expression
      start: (number_literal)
      end: (identifier))
    body: (block (expression_statement (method_invocation function: (identifier) arguments: (argument_list (identifier)))))))
```

### 8.3 Stress / real-world corpus

Nine synthetic Groovy snippets or adaptations of public-domain examples cover
Jenkins pipelines, Gradle scripts, declarations, generics, control flow, and
operators. They are embedded in `bindings/rust/tests/parse_stress.rs` to avoid
network dependencies. Acceptance bar: zero `ERROR` and zero `MISSING` nodes.
See `README.codegraph.md` for source attribution and coverage.

### 8.4 Test execution

Run these checks from the grammar directory:

1. `tree-sitter test` over `test/corpus/` and `test/highlight/`.
2. `cargo test --test parse_stress` runs `bindings/rust/tests/parse_stress.rs`,
   parses each inline case independently, and asserts no
   `ERROR` nodes and no `MISSING` nodes are present in the parse
   tree. (The MISSING-node assertion is the *direct* anti-regression
   for issue #246's failure mode.)

---

## 9. Highlight, locals, injection queries

### 9.1 highlights.scm

Standard tree-sitter highlighting captures:

- `@keyword` — `def`, `class`, `trait`, `enum`, `record`, `interface`,
  `if`, `else`, `for`, `while`, `do`, `switch`, `case`, `default`,
  `return`, `throw`, `try`, `catch`, `finally`, `break`, `continue`,
  `import`, `package`, `new`, `as`, `in`, `instanceof`, `assert`,
  `yield`, `pipeline`, `sealed`, `non-sealed`, `permits`.
  Fixes murtaza64 #36 (enum unhighlighted).
- `@operator` — every operator node in §3.2.
- `@string`, `@string.regex` (slashy strings), `@string.escape`.
- `@type` — class names in declaration position and after `:`.
- `@function` — identifier in method declaration and at call site.
- `@variable` — bare identifiers in expression position.
- `@comment` — `block_comment`, `line_comment`, `groovydoc_comment`.

### 9.2 injections.scm

- Slashy-string and dollar-slashy bodies inject as `regex`. Solves
  the "slashy is a regex" expectation in Jenkins users' editors.
- Triple-quoted strings inside `sql"""…"""` or `xml"""…"""` named-call
  positions could inject the respective language; we don't ship those
  by default but the query file is extensible.

### 9.3 locals.scm

Standard Groovy scope rules:

- `method_declaration.parameters`, `closure.parameters`, `for_loop`,
  `for_in_statement`, `catch_clause.parameter` are scopes.
- `formal_parameter`, `variable_declarator`, `multi_assignment_declaration`
  define identifiers.
- `identifier` references resolve to nearest enclosing definition.

---

## 10. Issue resolution map

How each filed bug closes against this spec.

| Issue | Where this spec addresses it |
|---|---|
| dekobon/big-code-analysis **#246** (Elvis short-circuit) | §3.2 — `elvis_expression` as a distinct named node. §8.1 — pinned corpus test reproducing the issue's `def pick(a, b, c) { … }` snippet with the expected S-expression. The downstream `get_op_type()` impl can now `matches!(id.into(), AMPAMP \| PIPEPIPE \| ELVIS_EXPRESSION)`. No `MISSING` child anywhere. |
| dekobon/big-code-analysis **#247** umbrella (all operator gaps) | §3.2 row-by-row: every symbol in the issue's three tables maps to a distinct node kind. The "critical" rows (`?:`, `?.`, `as`, `trait`, `..<`) all have dedicated nodes. The "important" rows (`=~`, `==~`, slashy, `===`, `!==`, `<=>`, `!instanceof`, `!in`) likewise. The "niche" rows (`?=`, `?[]`, `.&`, `.@`, `$/…/$`, `==>`) likewise. §8 anchors each with a pinned test. §3.2.1 explicitly handles the subscript-yielding-callable composition (`arr[i](args)`). §3.2.2 documents the lexer trailing-context rule for `!instanceof` / `!in` to avoid mis-tokenisation when followed by an identifier character. |
| amaanq #2 / #3 (maintenance / license) | The prior grammar is treated as inspiration only — no code is incorporated. This grammar is an independent project, dual-licensed under `Apache-2.0 OR MIT`, with no dependency on the upstream project's release cadence. |
| murtaza64 **#5** (juxt_function_call ambiguity) | §5.5 — command chains restricted to statement-level only, first token must be identifier-or-property-access, conflicts with `binary_expression` resolved by precedence. §8.2 — pinned test on `f y:b ? … : …` resolving to `f(y:b)` then ternary. |
| murtaza64 **#16** (empty block comments) | §6.6 — block comments and groovydoc comments are external-scanner tokens that consume the entire `/* … */` or `/** … */` regardless of content. §8.2 — pinned test with `/**/`, `/***/`, `/* */`. |
| murtaza64 **#22** (multiple assignment) | §4 — `multi_assignment_declaration` rule. §8.2 — pinned test on `def (a, b) = [1, 2]`. |
| murtaza64 **#27** (publish to npm) | Out of grammar scope. The package layout in §2.1 includes `package.json` and `bindings/node/`; npm publishing is part of release tooling, not the grammar. |
| murtaza64 **#36** (enum + switch arrow) | §4 — `enum_declaration` with constants. `switch_arrow_case` rule. §9.1 — `enum` as `@keyword`. §8.2 — pinned regression. |
| murtaza64 **#37** (pipeline followed by def) | §4 — `pipeline_statement` is a normal statement, not a file-trailer-only production. §6.4 — AUTOMATIC_SEMICOLON ensures the newline between `pipeline {}` and `def foo = 5` is a statement terminator. §8.2 — pinned regression. |
| murtaza64 **#38** (TS typing on node bindings) | Out of grammar scope; lives in `bindings/node/index.d.ts`. The fix is to declare `language: Language` to match the upstream `tree-sitter` types. Documented in §11. |
| murtaza64 **#39** (multi-catch) | §4 — `catch_formal_parameter` accepts a `multi_type` (`Type1 \| Type2`). §8.2 — pinned regression. |

---

## 11. Bindings and packaging

Each binding file is generated from `grammar.js` via `tree-sitter
generate` and committed; the C scanner is hand-authored.

- `bindings/node/`: includes `binding.gyp`, `index.js`, `index.d.ts`.
  The `index.d.ts` declares `language: Language` to match the
  upstream `tree-sitter` types (fixes murtaza64 #38).
- `bindings/python/`: `pyproject.toml` + `setup.py` for
  `tree-sitter-groovy` PyPI package.
- `bindings/rust/`: a `lib.rs` exposing `language()` and a
  `tree-sitter-groovy` crate published on crates.io. Cargo features:
  default builds with cc; `prebuilt` feature for binary distribution.
  **This is the integration target for `dekobon/big-code-analysis`.**
- `bindings/go/`: a Go module exposing `GetLanguage()`.
- `bindings/swift/`: `Package.swift` for SwiftPM consumers.

`tree-sitter.json` declares grammar metadata for editor integration
per the tree-sitter CLI's expected schema (file types `.groovy`,
`.gradle`, `.gvy`, `.gy`, `Jenkinsfile`).

---

## 12. Versioning and release plan

- v0.1.0 — Initial grammar with all of §3 (operator coverage) and §4
  (statement/declaration coverage). Resolves dekobon #246, #247 in
  full and all murtaza64 open issues except #27 / #38 / #36's
  "enum highlighted" sub-bug (latter is a query-file change in v0.1.1).
- v0.1.1 — Highlight query refinements; node-binding TS typing fix.
- v0.2.0 — Groovy 4 record/sealed support (already in §4; promoted
  to GA when Groovy 4's spec stabilises for both).

semver: tree-sitter grammars are still finding their semver
conventions; we follow `tree-sitter-java`'s: MINOR for grammar shape
changes (different S-expressions for existing source), PATCH for
additive node-types and bug fixes that don't change existing trees.

The `dekobon/big-code-analysis` pin (currently
`tree-sitter-groovy = "=0.1.2"` from amaanq) will be moved to this
crate at v0.1.0 publication. The dekobon-side bump is tracked by
**this spec** acting as the published artifact; the actual bump PR
on dekobon's side is a follow-up.

---

## 13. Out-of-scope but worth noting

- **Tree-sitter incremental reparsing** correctness for slashy
  strings: editing inside a slashy can change whether subsequent
  `/` is a string or division. This is a known tree-sitter wart; we
  mitigate via the external scanner's strict state model but cannot
  fully fix it without parser-API changes upstream.
- **Groovy `as` for closure coercion** (`{ it } as Comparator`) is
  the same `cast_expression` as `x as String` — the RHS type can be
  any `_type`, including a single type identifier or a generic type.
- **Stub generation / IDE support beyond editor highlighting** —
  language servers can consume this grammar but are not in scope.
- **Performance budget**: parsing the Apache Groovy codebase
  (~250k LOC of `.groovy` test files) under 5 seconds on a modern
  laptop is the target. Tree-sitter's incremental nature makes
  steady-state editing free.

---

## 14. Sources

Primary references used in writing this spec:

- Apache Groovy language docs:
  - `https://groovy-lang.org/operators.html`
  - `https://groovy-lang.org/syntax.html`
  - `https://groovy-lang.org/semantics.html`
  - `https://groovy-lang.org/objectorientation.html`
  - `https://groovy-lang.org/dsls.html`
  - `https://groovy-lang.org/style-guide.html`
- Apache Groovy reference grammar (ANTLR4):
  - `https://github.com/apache/groovy/blob/master/src/antlr/GroovyParser.g4`
  - `https://github.com/apache/groovy/blob/master/src/antlr/GroovyLexer.g4`
- Source grammars analysed:
  - `https://github.com/amaanq/tree-sitter-groovy/blob/master/grammar.js`
  - `https://github.com/murtaza64/tree-sitter-groovy/blob/main/grammar.js`
- Bug reports being closed:
  - `https://github.com/dekobon/big-code-analysis/issues/246`
  - `https://github.com/dekobon/big-code-analysis/issues/247`
- Sibling tree-sitter grammars in this checkout providing the layout
  template: `tree-sitter-tcl/`, `tree-sitter-irules/`.
