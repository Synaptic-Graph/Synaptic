/**
 * @file Apache Groovy grammar for tree-sitter
 * @author Elijah Zupancic <elijah@zupancic.name>
 * @license MIT
 *
 * The authoritative design document for this grammar is
 * SPECIFICATION.md at the repository root. Every rule below maps to
 * a section there. Read the spec before making non-trivial changes.
 *
 * The external scanner (src/scanner.c) handles four context-sensitive
 * lexing situations that pure grammar rules cannot disambiguate:
 *   - Slashy-string-start vs. division
 *   - Automatic-semicolon (newline-as-terminator)
 *   - Labelled-statement colon
 *   - Block-comment / Groovydoc single-token emit
 * See SPECIFICATION.md §6.
 */

/// <reference types="tree-sitter-cli/dsl" />
// @ts-check

/* eslint-disable no-multi-spaces --
 * The aligned trailing comments make the Apache-mirror precedence
 * ladder readable at a glance.
 */
const PREC = {
  // Higher number = tighter binding in tree-sitter.
  // Mirrors apache/groovy:GroovyParser.g4 `expression` rule.
  // See SPECIFICATION.md §3.1.
  ASSIGN: 1,                  // = += -= *= /= %= **= <<= >>= >>>= &= ^= |= ?=
  CONDITIONAL: 2,             // ?: (Elvis) and ? : (ternary) — same level
  IMPLICATION: 3,             // ==>
  LOGICAL_OR: 4,              // ||
  LOGICAL_AND: 5,             // &&
  BITWISE_OR: 6,              // |
  BITWISE_XOR: 7,             // ^
  BITWISE_AND: 8,             // &
  EQUALITY: 9,                // == != <=> === !== =~ ==~
  RELATIONAL: 10,             // < <= > >= in !in instanceof !instanceof as
  SHIFT_OR_RANGE: 11,         // << >> >>> .. ..< <.. <..<
  ADDITIVE: 12,               // + -
  MULTIPLICATIVE: 13,         // * / %
  UNARY: 14,                  // prefix ++ -- + -          (Apache level 3 UNARY_ADD)
  POWER: 15,                  // **                         (Apache level 2)
  UNARY_NOT: 16,              // ! ~                        (Apache level 1, tighter than POWER)
  POSTFIX: 17,                // postfix ++ --, []
  ACCESS: 18,                 // . ?. ??. *. .& .@ ::
  PRIMARY: 19,                // literals, identifiers, ()
  // Type-context only — never competes with the expression ladder
  // above. Shares CONDITIONAL's numeric value by accident; the two
  // do not collide because `_type` and `_expression` reach the
  // resolver through disjoint parse stacks.
  ARRAY_TYPE: 2,
};
/* eslint-enable no-multi-spaces */

/**
 * GString body segment choice shared between `"..."` and `"""..."""`.
 * The text rule differs (single-quoted bans newlines, triple-quoted
 * allows them) but every other alternative is identical.
 *
 * @param {GrammarSymbols<string>} $ Grammar reference symbols.
 * @param {RuleOrLiteral} textRule The flavour-specific literal-text rule.
 * @returns {ChoiceRule} The shared body-segment choice.
 */
function gstringPart($, textRule) {
  return choice(
    alias(textRule, $.string_fragment),
    $.gstring_dollar_interpolation,
    $.gstring_brace_interpolation,
    alias($._gstring_literal_dollar, $.string_fragment),
  );
}

module.exports = grammar({
  name: 'groovy',

  // Placeholder for the real implementation. SPECIFICATION.md §3-§7
  // is the contract this rule set will fulfil.
  word: $ => $.identifier,

  extras: $ => [
    /\s/,
    $.line_comment,
    $.block_comment,
    $.groovydoc_comment,
  ],

  // Externals listed here must have a matching branch in
  // `src/scanner.c` AND be referenced by at least one rule below.
  // Newlines remain whitespace inside expressions. At statement boundaries,
  // the scanner emits _statement_break so the next line cannot become a
  // subscript or argument list belonging to the preceding statement.
  externals: $ => [
    $._slashy_string_start,
    $.line_comment,
    $.block_comment,
    $.groovydoc_comment,
    $._statement_break,
  ],

  conflicts: $ => [
    [$._initialized_declarator, $.closure_parameter],
    [$._expression, $.closure_parameter],
    [$.command_chain],
    [$._modifier, $.local_variable_declaration],
    [$.closure],
    [$.enum_body],
    [$.command_chain, $._type],
    [$.command_chain, $._expression, $._type],
    [$.command_chain, $._expression],
    [$.source_file],
    [$.enum_constant, $._type],
    // See SPECIFICATION.md §7. Populated as rules accumulate.
    // (list_literal vs map_literal — tree-sitter resolves without
    //  an explicit conflict because `:` after the first element
    //  always disambiguates within one token of lookahead.)
    //
    // §5.15 — inside `( … )` an identifier could be either an
    // _expression (parenthesized_expression) or a _type
    // (parenthesized_type_cast). Tree-sitter explores both via
    // this conflict; the `)` plus the next token disambiguates.
    [$._expression, $._type],

    // §4 — `a.b.c` could be a field_access chain (expression
    // context) or a qualified_type (type context). Same family
    // as the _expression / _type conflict above but the implicit
    // qualified_type_repeat1 aux symbol forces a separate
    // declaration.
    [$._expression, $.qualified_type],

    // §3.2.2 — `_cast_type` overlaps `_type` on identifier / array /
    // generic alternatives; only the dotted path diverges. The
    // companion three-way conflict pulls in `_dotted_type` so the
    // dotted shape can win the trailing `.identifier` from the
    // surrounding `field_access` reduction.
    [$._cast_type, $._type],
    [$._cast_type, $._dotted_type, $.qualified_type],

    // §4 — `for ( identifier …` could continue as a for_statement
    // (identifier is part of the init expression) or as a
    // for_in_statement (identifier is the loop variable). The
    // token after the identifier (`;` / operator vs `in`)
    // disambiguates.
    [$.for_in_statement, $._expression],

    // §5.2 — `def identifier` could either be a
    // local_variable_declaration (variable_declarator with no
    // initializer) or the start of a method_declaration
    // (_property_name + parameters). The `(` after the name
    // decides; the conflict keeps both alternatives alive until
    // then.
    [$.variable_declarator, $._property_name],
    [$.variable_declarator, $._type],
    [$.formal_parameter, $._expression, $._type],
    [$.formal_parameter, $._expression],
  ],

  rules: {
    // Top-level. Real rule set lands incrementally per
    // SPECIFICATION.md §4 (statement and declaration coverage).
    source_file: $ => seq(
      repeat($._statement_break),
      optional($.shebang),
      repeat(choice($._statement, ';', $._statement_break)),
    ),

    // §4 — `#!/usr/bin/env groovy` line at file start. Single
    // token consumed by the in-grammar lexer.
    shebang: _ => token(seq('#!', /[^\n]*/)),

    // §4 — statement-level alternatives. Declarations and the rest
    // of control flow fill out this choice in subsequent iterations.
    // `block` is intentionally NOT here — a top-level `{ ... }`
    // continues to parse as an expression_statement-wrapped closure
    // (the historically intended form for Groovy scripts). Blocks
    // appear explicitly in the body slot of `if` / `while` / etc.
    _statement: $ => choice(
      $.expression_statement,
      $.if_statement,
      $.while_statement,
      $.do_while_statement,
      $.for_statement,
      $.for_in_statement,
      $.return_statement,
      $.break_statement,
      $.continue_statement,
      $.throw_statement,
      $.assert_statement,
      $.yield_statement,
      $.try_statement,
      $.class_declaration,
      $.trait_declaration,
      $.interface_declaration,
      $.annotation_type_declaration,
      $.enum_declaration,
      $.record_declaration,
      $.package_declaration,
      $.import_declaration,
      $.local_variable_declaration,
      $.multi_assignment_declaration,
      $.pipeline_statement,
      $.method_declaration,
      $.labeled_statement,
      $.command_chain,
      $.synchronized_statement,
    ),

    // Parenthesis-free calls with literal and closure arguments.
    command_chain: $ => choice(
      prec(2, seq(field('receiver', $.identifier), field('argument', $.closure))),
      prec(2, seq(
        field('receiver', choice($.identifier, $.field_access)),
        field('argument', choice($.string_literal, $.number_literal, $.boolean_literal, $.null_literal)),
        repeat(seq(',', field('argument', choice($._expression, $.map_entry)))),
      )),
      prec.dynamic(-1, prec(0, seq(
        field('receiver', choice($.identifier, $.field_access)),
        field('argument', choice($.identifier, $.field_access, $.map_entry, $.list_literal, $.method_pointer_expression, $.method_invocation, $.subscript_expression, $.cast_expression)),
        repeat(seq(',', field('argument', choice($._expression, $.map_entry)))),
      ))),
    ),

    labeled_statement: $ => prec(1, seq(
      field('label', $.identifier),
      ':',
      field('statement', $._statement),
    )),

    // §4 — block of statements, used in control-flow bodies and
    // (eventually) method bodies. Shares `{ ... }` syntax with
    // closure (§5.8). A `prec(1, …)` keeps block preferred at use
    // sites where both `block` and a closure-bearing `_statement`
    // are valid alternatives (e.g. the body slot of `if` / `while`).
    block: $ => prec(1, seq(
      '{',
      repeat(choice($._statement, ';', $._statement_break)),
      '}',
    )),

    // §4 — if / else. `prec.right` resolves the dangling-else
    // ambiguity in favour of the closest `if`. Body accepts either
    // an explicit block (preferred) or a single statement. The
    // `optional(';')` after the consequence keeps the else attached
    // when a braceless consequence is terminated by a semicolon
    // (`if (x) a(); else b()`); without it the `;` is claimed by the
    // enclosing statement list (§5.10) and the `else` is severed. It
    // sits outside the else clause so the no-else form (`if (x) a();
    // b()`) does not require an `else` after eating the `;`.
    if_statement: $ => prec.right(seq(
      'if',
      '(',
      field('condition', $._expression),
      ')',
      field('consequence', choice($.block, $._statement)),
      optional(';'),
      optional(seq(
        'else',
        field('alternative', choice($.block, $._statement)),
      )),
    )),

    // §4 — while loop. do-while lands separately.
    while_statement: $ => seq(
      'while',
      '(',
      field('condition', $._expression),
      ')',
      field('body', choice($.block, $._statement)),
    ),

    // §4 — C-style for. init / condition / update slots all
    // optional; bare `for (;;)` is the infinite-loop idiom. The init
    // slot accepts a single expression (`i = 0`) or a variable
    // declaration (`int i = 0`, `def i = 0`, incl. multiple
    // declarators). Apache `GroovyParser.g4` `forInit:
    // localVariableDeclaration | expressionList` also allows a
    // comma-separated expression list (`i = 0, j = 1`); that form is
    // not yet supported.
    for_statement: $ => seq(
      'for',
      '(',
      optional(field('init', choice($._expression, $.local_variable_declaration))),
      ';',
      optional(field('condition', $._expression)),
      ';',
      optional(field('update', $._expression)),
      ')',
      field('body', choice($.block, $._statement)),
    ),

    // §4 — do-while. Body is required; condition follows the body.
    // `optional(';')` lets a braceless body be terminated before
    // `while` (`do a(); while (c)`, the Java/Groovy `do S while (e);`
    // form); without it the `;` errors since `while` is required next.
    do_while_statement: $ => seq(
      'do',
      field('body', choice($.block, $._statement)),
      optional(';'),
      'while',
      '(',
      field('condition', $._expression),
      ')',
    ),

    // §4 — return / break / continue / throw / assert. Until
    // AUTOMATIC_SEMICOLON lands the parser will greedily consume
    // a trailing expression onto `return` / `throw` / `assert`;
    // explicit `;` or statement boundary is recommended in source.
    return_statement: $ => prec.right(seq(
      'return',
      optional($._expression),
    )),

    break_statement: $ => prec.right(seq(
      'break',
      optional(field('label', $.identifier)),
    )),

    continue_statement: $ => prec.right(seq(
      'continue',
      optional(field('label', $.identifier)),
    )),

    throw_statement: $ => seq('throw', $._expression),

    // §4 — `yield expr` in a Groovy 4 switch expression's case
    // body. `yield` is a contextual keyword (§5.2) so it remains
    // a usable identifier elsewhere — tree-sitter's word:
    // directive resolves the ambiguity.
    yield_statement: $ => prec.right(seq(
      'yield',
      $._expression,
    )),

    // §4 — power-assert: `assert e` or `assert e : msg`. The colon
    // separator may collide with map_entry in later iterations, but
    // assert is a statement-only construct so the context resolves
    // it.
    assert_statement: $ => prec.right(seq(
      'assert',
      $._expression,
      optional(seq(choice(':', ','), field('message', $._expression))),
    )),

    // §4 — try / catch / finally. At least one of catch/finally is
    // required per Java/Groovy semantics; the grammar relaxes that
    // to allow a bare `try { … }` and lets downstream tooling
    // surface the error if needed. Try-with-resources lands later
    // with local variable declarations.
    try_statement: $ => seq(
      'try',
      optional(field('resources', $.resource_specification)),
      field('body', $.block),
      repeat($.catch_clause),
      optional($.finally_clause),
    ),

    // §4 — `try (Foo f = expr; Bar b = expr) { … }`. Resources
    // are typed or `def`-style declarators with required `=`
    // initializer (per Java-style try-with-resources). Multiple
    // resources separated by `;`.
    resource_specification: $ => seq(
      '(',
      $.resource,
      repeat(seq(';', $.resource)),
      optional(';'),
      ')',
    ),

    resource: $ => seq(
      optional(field('type', $._type)),
      field('name', $.identifier),
      '=',
      field('value', $._expression),
    ),

    catch_clause: $ => seq(
      'catch',
      '(',
      field('parameter', $.catch_formal_parameter),
      ')',
      field('body', $.block),
    ),

    // §4 — `catch_formal_parameter` accepts either a single _type
    // or a multi_type for Java 7+ / Groovy multi-catch. Closes
    // murtaza64 #39.
    catch_formal_parameter: $ => seq(
      field('type', choice($._type, $.multi_type)),
      optional(field('name', $.identifier)),
    ),

    multi_type: $ => seq(
      $._type,
      repeat1(seq('|', $._type)),
    ),

    synchronized_statement: $ => seq(
      'synchronized', '(', field('value', $._expression), ')', field('body', $.block),
    ),

    finally_clause: $ => seq(
      'finally',
      field('body', $.block),
    ),

    // §4 — class / trait / interface declarations. v1 supports the
    // shape `[keyword] Name { body }`. extends, implements, generics,
    // modifiers, and sealed / permits clauses land in later
    // iterations. trait closes the dekobon #247 trait row.
    // §4 — class declaration with optional Groovy 4 sealed modifier
    // and `permits` clause. The `non-sealed` form is a single
    // anonymous token (token('non-sealed')) so the dash doesn't
    // get lexed as subtraction.
    class_declaration: $ => seq(
      repeat(choice($.annotation, $._modifier)),
      optional(choice('sealed', token('non-sealed'))),
      'class',
      field('name', $.identifier),
      optional(field('type_parameters', $.type_parameters)),
      optional($.superclass),
      optional($.super_interfaces),
      optional($.permits_clause),
      field('body', $.class_body),
    ),

    trait_declaration: $ => seq(
      repeat(choice($.annotation, $._modifier)),
      optional(choice('sealed', token('non-sealed'))),
      'trait',
      field('name', $.identifier),
      optional(field('type_parameters', $.type_parameters)),
      optional($.extends_interfaces),
      optional($.permits_clause),
      field('body', $.class_body),
    ),

    interface_declaration: $ => seq(
      repeat(choice($.annotation, $._modifier)),
      optional(choice('sealed', token('non-sealed'))),
      'interface',
      field('name', $.identifier),
      optional(field('type_parameters', $.type_parameters)),
      optional($.extends_interfaces),
      optional($.permits_clause),
      field('body', $.class_body),
    ),

    // §4 / §5.14 — generic type parameter list on a class /
    // interface / trait / method declaration. The `<` is
    // `token.immediate` so the parser only treats it as a type
    // parameter opener when it follows the declaration name with
    // no intervening whitespace.
    type_parameters: $ => seq(
      token.immediate('<'),
      $.type_parameter,
      repeat(seq(',', $.type_parameter)),
      '>',
    ),

    type_parameter: $ => seq(
      field('name', alias($.identifier, $.type_identifier)),
      optional(seq(
        'extends',
        $._type,
        repeat(seq('&', $._type)),
      )),
    ),

    // §4 — class has a single super and zero-or-more interfaces.
    superclass: $ => seq('extends', $._type),

    super_interfaces: $ => seq(
      'implements',
      $._type,
      repeat(seq(',', $._type)),
    ),

    // §4 — trait / interface extend multiple interfaces (no
    // `implements` keyword in that position).
    extends_interfaces: $ => seq(
      'extends',
      $._type,
      repeat(seq(',', $._type)),
    ),

    // §4 — Java-style modifiers. Each one is a hard reserved
    // keyword (§5.2) so the `word: $.identifier` directive keeps
    // them from accidentally being treated as identifiers.
    _modifier: _ => choice(
      'public',
      'private',
      'protected',
      'static',
      'final',
      'abstract',
      'synchronized',
      'native',
      'transient',
      'volatile',
      'strictfp',
    ),

    permits_clause: $ => seq(
      'permits',
      $._type,
      repeat(seq(',', $._type)),
    ),

    // §4 — `@interface Foo { … }` declares an annotation type.
    // `@` followed by the `interface` keyword is unambiguous —
    // `interface` is a hard reserved keyword (§5.2) so the
    // alternative parse `annotation` (which needs identifier)
    // cannot fire here.
    annotation_type_declaration: $ => seq(
      repeat(choice($.annotation, $._modifier)),
      '@',
      'interface',
      field('name', $.identifier),
      field('body', $.class_body),
    ),

    // §4 — `{ method* }` for v1. Field declarations, static
    // initialisers, and inner classes land later.
    // The body accepts `;` as an empty member / terminator the same
    // way statement lists do (§5.10): a field or abstract method is
    // commonly written `int x = 1;` / `void f();`, and a stray `;` is
    // a no-op member. `;` is anonymous, so it never appears in the AST.
    class_body: $ => seq(
      '{',
      repeat(choice($._class_member, ';', $._statement_break)),
      '}',
    ),

    _class_member: $ => choice(
      $.class_declaration,
      $.trait_declaration,
      $.interface_declaration,
      $.enum_declaration,
      $.annotation_type_declaration,
      $.method_declaration,
      $.field_declaration,
      $.constructor_declaration,
      $.static_initializer,
    ),

    // §4 — constructors share the method_declaration shape but have
    // no return type and the name matches the enclosing class. We
    // don't enforce the name match (downstream tooling can), but we
    // emit a distinct node kind so IDE features (rename, jump) can
    // tell constructors apart from methods. `prec(3, …)` wins over
    // both field and method when the identifier is followed by `(`
    // and no return type precedes it.
    constructor_declaration: $ => prec.right(3, seq(
      repeat(choice($.annotation, $._modifier)),
      field('name', $.identifier),
      field('parameters', $.formal_parameters),
      optional($.throws_clause),
      field('body', $.block),
    )),

    // §4 — `static { … }` class-body initializer block. Runs once
    // when the class is loaded. Distinct node kind so downstream
    // tools can index it.
    static_initializer: $ => seq(
      'static',
      field('body', $.block),
    ),

    // §4 — class / interface / trait field. Java-style shape: optional
    // annotations, optional modifiers, `def` or a `_type`, then one or
    // more declarators (each with optional initializer). Method
    // declarations share the same prefix; `prec(2, …)` on
    // `method_declaration` and the trailing `(` discriminate.
    field_declaration: $ => prec.right(1, seq(
      choice(
        prec.dynamic(3, seq(repeat(choice($.annotation, $._modifier)), choice('def', field('type', $._type)))),
        prec.dynamic(-1, repeat1(choice($.annotation, $._modifier))),
      ),
      $.variable_declarator,
      repeat(seq(',', $.variable_declarator)),
    )),

    // §4 — `def name(params) body`. Typed return type (`String foo()
    // { … }`) lands once we can disambiguate from local variable
    // declaration. Modifiers and throws clause similarly defer.
    // prec(1) wins over local_variable_declaration when `(` follows
    // the name, so `def f() { … }` at script-top-level parses as a
    // method rather than a `def f` variable followed by `(…) { … }`.
    method_declaration: $ => prec.dynamic(2, prec.right(2, seq(
      choice(
        seq(
          repeat(choice($.annotation, $._modifier)),
          choice(
            seq(optional(field('type_parameters', $.method_type_parameters)), choice('def', field('return_type', $._type))),
            seq('def', field('type_parameters', $.method_type_parameters), field('return_type', $._type)),
          ),
        ),
        repeat1(choice($.annotation, $._modifier)),
      ),
      field('name', $._property_name),
      field('parameters', $.formal_parameters),
      optional($.throws_clause),
      optional(seq('default', field('value', $._expression))),
      optional(field('body', $.block)),
    ))),

    // §4 / §5.14 — method-level generic type parameters
    // (`<T> T identity(T x) { x }`). Distinct from `type_parameters`
    // (used on class/trait/interface) because here the `<` cannot be
    // `token.immediate` — it appears at the start of the declaration
    // with optional whitespace from preceding modifiers / annotations.
    method_type_parameters: $ => seq(
      '<',
      $.type_parameter,
      repeat(seq(',', $.type_parameter)),
      '>',
    ),

    // §4 — `throws E1, E2, …` after a method signature.
    throws_clause: $ => prec.left(seq(
      'throws',
      $._type,
      repeat(seq(',', $._type)),
    )),

    formal_parameters: $ => seq(
      '(',
      optional(seq(
        $.formal_parameter,
        repeat(seq(',', $.formal_parameter)),
      )),
      ')',
    ),

    // §5.13 — formal parameter with optional type, optional
    // varargs marker (`Type... name`), and optional default value
    // (`name = expr`).
    formal_parameter: $ => seq(
      repeat(choice($.annotation, $._modifier)),
      optional(field('type', choice($._type, $.varargs_type))),
      field('name', $.identifier),
      optional(seq('=', field('default', $._expression))),
    ),

    varargs_type: $ => seq($._type, '...'),

    // §4 — enum. Closes murtaza64 #36 first half. Constants list
    // (optionally trailing-comma) optionally followed by `;` and
    // further class members. Constants may carry constructor
    // arguments (`RED(0xff0000)`); v1 supports that via the
    // optional argument_list.
    enum_declaration: $ => seq(
      repeat(choice($.annotation, $._modifier)),
      'enum',
      field('name', $.identifier),
      field('body', $.enum_body),
    ),

    enum_body: $ => seq(
      '{',
      repeat($._statement_break),
      optional(seq(
        $.enum_constant,
        repeat(seq(',', repeat($._statement_break), $.enum_constant)),
        optional(','),
      )),
      repeat(choice($._class_member, ';', $._statement_break)),
      '}',
    ),

    enum_constant: $ => seq(
      field('name', $.identifier),
      optional(field('arguments', $.argument_list)),
    ),

    // §4 — record (Groovy 4+). `record Name(Type a, Type b) [{ body }]`.
    // `record` is a contextual keyword — tree-sitter's word: directive
    // keeps `record` usable as a plain identifier elsewhere.
    record_declaration: $ => prec.right(seq(
      repeat(choice($.annotation, $._modifier)),
      'record',
      field('name', $.identifier),
      field('components', $.record_components),
      optional(field('body', $.class_body)),
    )),

    record_components: $ => seq(
      '(',
      optional(seq(
        $.record_component,
        repeat(seq(',', $.record_component)),
      )),
      ')',
    ),

    record_component: $ => seq(
      field('type', $._type),
      field('name', $.identifier),
    ),

    // §4 — package and import. Trailing `;` is optional in Groovy;
    // we accept both forms by not requiring it.
    package_declaration: $ => seq(
      'package',
      field('name', $.qualified_name),
    ),

    // §4 — `import [static] qualified.Name [.*] [as Alias]`.
    // `.*` is a single anonymous token so it lexes distinctly from
    // a continuation of qualified_name (`.identifier`). Wildcard
    // imports only appear in this position, so the lexer-wide
    // longest-match is safe.
    import_declaration: $ => seq(
      'import',
      optional('static'),
      field('name', $.qualified_name),
      optional(token('.*')),
      optional(seq('as', field('alias', $.identifier))),
    ),

    qualified_name: $ => seq(
      $.identifier,
      repeat(seq('.', $.identifier)),
    ),

    // §4 / §5.12 — `def x = …`, `var x = …`, and `Type x = …`.
    // The typed form requires every declarator to carry an
    // `= value` initializer; bare `Type x` without initializer
    // would conflict with method_invocation `Type(x)` and is
    // disallowed in v1. Multiple declarators per statement
    // (`def a = 1, b = 2`) are supported in all three forms.
    local_variable_declaration: $ => prec.dynamic(1, prec.right(choice(
      seq(
        choice('def', 'var', 'final'),
        $.variable_declarator,
        repeat(seq(',', $.variable_declarator)),
      ),
      seq(
        field('type', $._type),
        $._initialized_declarator,
        repeat(seq(',', $._initialized_declarator)),
      ),
    ))),

    _initialized_declarator: $ => seq(
      field('name', $.identifier),
      '=',
      field('value', choice($._expression, $.command_chain)),
    ),

    variable_declarator: $ => seq(
      field('name', $.identifier),
      optional(seq('=', field('value', choice($._expression, $.command_chain)))),
    ),

    // §4 — `def (a, b) = expr`. Closes murtaza64 #22. v1 requires
    // the leading `def`; the no-`def` form `(a, b) = expr` lands
    // later (it shares a prefix with parenthesized_expression and
    // needs a conflict declaration).
    multi_assignment_declaration: $ => seq(
      'def',
      '(',
      alias($.closure_parameter, $.variable_declarator),
      repeat(seq(',', alias($.closure_parameter, $.variable_declarator))),
      ')',
      '=',
      field('value', $._expression),
    ),

    // §4 / §10 row #37 — Jenkins-style `pipeline { … }` block.
    // Closes murtaza64 #37 (pipeline as a regular statement, not
    // a file-trailer-only construct). The body is a closure, since
    // that's how Jenkins's DSL is shaped — `pipeline` is followed
    // by a closure literal with nested stages, steps, etc.
    pipeline_statement: $ => seq(
      'pipeline',
      field('body', $.closure),
    ),

    // §5.9 — annotation usage. Lands on its own here; class /
    // method / record / enum declarations gain an optional
    // leading `repeat(choice($.annotation, $._modifier))` so a `@Foo @Bar class X { }`
    // attaches the annotations to the class node.
    annotation: $ => seq(
      '@',
      field('name', $.qualified_name),
      optional(field('arguments', $.argument_list)),
    ),

    // §4 — `for (x in xs)`. Untyped variable for v1. Typed
    // (`for (Type x in xs)`) lands later. The `in` keyword here
    // is a structural delimiter, not the membership_expression
    // operator — context disambiguates because the for-header is
    // not an expression position.
    for_in_statement: $ => seq(
      'for',
      '(',
      optional(field('type', $._type)),
      field('variable', $.identifier),
      choice('in', ':'),
      field('value', $._expression),
      ')',
      field('body', choice($.block, $._statement)),
    ),

    expression_statement: $ => $._expression,

    // §3.2 PRIMARY — minimal alternative set for now. Every primary
    // expression form from §3.2 / §5 will land in this choice over
    // subsequent iterations.
    _expression: $ => choice(
      $.identifier,
      $.number_literal,
      $.string_literal,
      $.boolean_literal,
      $.null_literal,
      $.parenthesized_expression,
      $.tuple_expression,
      $.unary_expression,
      $.list_literal,
      $.map_literal,
      $.closure,
      $.binary_expression,
      $.lambda_expression,
      $.range_expression,
      $.identity_expression,
      $.spaceship_expression,
      $.regex_find_expression,
      $.regex_match_expression,
      $.ternary_expression,
      $.elvis_expression,
      $.logical_implication_expression,
      $.assignment_expression,
      $.power_expression,
      $.subscript_expression,
      $.safe_subscript_expression,
      $.method_invocation,
      $.update_expression,
      $.unary_update_expression,
      $.field_access,
      $.safe_navigation_expression,
      $.safe_chain_dot_expression,
      $.spread_dot_expression,
      $.method_pointer_expression,
      $.direct_field_access_expression,
      $.method_reference_expression,
      $.object_creation_expression,
      $.array_creation_expression,
      $.parenthesized_type_cast,
      $.cast_expression,
      $.membership_expression,
      $.instanceof_expression,
      $.switch_expression,
    ),

    // §4 — Groovy 4 switch-expression. Single rule for both classic
    // `case … :` and arrow `case … -> …` branches; arrow form
    // closes murtaza64 #36 (second half). Used as a statement via
    // expression_statement, or as an expression directly per the
    // §8.2 #36 regression.
    switch_expression: $ => seq(
      'switch',
      '(',
      field('value', $._expression),
      ')',
      field('body', $.switch_block),
    ),

    switch_block: $ => seq(
      '{',
      repeat(choice(
        $.switch_case,
        $.switch_arrow_case,
        $.switch_default,
      )),
      '}',
    ),

    switch_case: $ => seq(
      'case',
      field('value', $._expression),
      ':',
      repeat(choice($._statement, ';', $._statement_break)),
    ),

    // The arrow body may be terminated by `;` (`case 1 -> foo();`),
    // matching the Java/Groovy `case L -> expr;` arrow form. The `;`
    // is anonymous and absorbed here rather than by the switch block,
    // which has no statement-list of its own.
    switch_arrow_case: $ => seq(
      'case',
      field('value', $._expression),
      '->',
      field('body', choice($.block, $._statement)),
      optional(';'),
    ),

    switch_default: $ => seq(
      'default',
      choice(
        seq(':', repeat(choice($._statement, ';', $._statement_break))),
        seq('->', field('body', choice($.block, $._statement)), optional(';')),
      ),
    ),

    // §3.2 — `x as Type`. RELATIONAL level (10), left-associative.
    // RHS is `_cast_type`, a `_type`-shaped sibling that swaps in
    // `_dotted_type` for `qualified_type` so the trailing
    // `.identifier` of `x as java.util.Map` extends the type
    // through every dot segment (instead of being claimed by
    // `field_access` at higher precedence than `cast_expression`).
    cast_expression: $ => prec.left(PREC.RELATIONAL, seq(
      field('value', $._expression),
      'as',
      field('type', $._cast_type),
    )),

    // §3.2.2 — RHS type of `as`, `instanceof`, and `!instanceof`.
    // Same alternatives as `_type` except `qualified_type` is
    // replaced by `_dotted_type`, whose tail shifts each dot at a
    // precedence tighter than `field_access`.
    _cast_type: $ => choice(
      alias($.identifier, $.type_identifier),
      $.array_type,
      alias($._dotted_type, $.qualified_type),
      $.generic_type,
    ),

    _dotted_type: $ => seq(
      alias($.identifier, $.type_identifier),
      $._dotted_type_tail,
    ),

    _dotted_type_tail: $ => prec.right(PREC.ACCESS + 1, seq(
      '.',
      alias($.identifier, $.type_identifier),
      optional($._dotted_type_tail),
    )),

    // §3.2 — `x in xs` and `x !in xs`. RELATIONAL level. Both
    // operators share one node kind; the `operator` field
    // distinguishes them. `!in` is wrapped in `token(prec(1, …))`
    // so it lexes as a single token despite the embedded space
    // requirement implied by §3.2.2's trailing-context rule —
    // Apache's `NOT_IN` predicate is approximated by requiring at
    // least one trailing whitespace, `[`, `(`, or `{` after `!in`.
    // §3.2 / §3.2.2 — `x in xs` and `x !in xs`. `!in` is a single
    // token whose match requires a trailing whitespace / bracket
    // character (Apache `NOT_IN` predicate) so `a !inXs` does not
    // mis-tokenise as `!in` followed by `Xs`.
    membership_expression: $ => {
      const ops = [
        'in',
        token(prec(1, seq('!in', /[ \t\r\n\[({]/))),
      ];
      return choice(...ops.map(op => prec.left(PREC.RELATIONAL, seq(
        field('element', $._expression),
        field('operator', op),
        field('collection', $._expression),
      ))));
    },

    // §3.2 / §3.2.2 — `x instanceof T` and `x !instanceof T`.
    // `!instanceof` is a single token whose match requires a
    // trailing whitespace (Apache `NOT_INSTANCEOF` predicate) so
    // `a !instanceofX` does not mis-tokenise as `!instanceof`
    // followed by identifier `X`.
    instanceof_expression: $ => {
      const ops = [
        'instanceof',
        token(prec(1, seq('!instanceof', /[ \t\r\n]/))),
      ];
      return choice(...ops.map(op => prec.left(PREC.RELATIONAL, seq(
        field('value', $._expression),
        field('operator', op),
        field('type', $._cast_type),
      ))));
    },

    // §3.2 / §5.15 — `new Foo(args)`. We only handle the plain
    // constructor-call form here; array creation `new Foo[10]`,
    // generic types, and array initialisers land later. PREC.PRIMARY
    // so it composes with the access tier from the left
    // (`new Foo().bar`).
    object_creation_expression: $ => prec.right(PREC.PRIMARY, seq(
      'new', field('type', $._type), field('arguments', $.argument_list),
      optional(field('body', $.class_body)),
    )),

    array_creation_expression: $ => prec.right(PREC.PRIMARY, seq(
      'new', field('type', choice(alias($.identifier, $.type_identifier), $.qualified_type, $.generic_type)),
      repeat1(seq('[', optional($._expression), ']')),
      optional($.array_initializer),
    )),

    array_initializer: $ => seq('{', optional(seq($._expression, repeat(seq(',', $._expression)), optional(','))), '}'),

    parenthesized_type_cast: $ => prec.right(PREC.UNARY, seq(
      '(',
      field('type', $._type),
      ')',
      field('value', $._expression),
    )),

    // §4 — `_type`. Supports unqualified type identifiers,
    // qualified types (`java.util.List`), array types
    // (`int[]`, `String[][]`), and generic types (`List<String>`,
    // `Map<String, Integer>`). `generic_type` always carries explicit
    // angle brackets, so it never competes with comparison operators
    // in expression context (which only reach `_type` through a
    // separate parse stack).
    _type: $ => choice(
      alias($.identifier, $.type_identifier),
      $.array_type,
      $.qualified_type,
      $.generic_type,
    ),

    // Near-twin of `_dotted_type` below; the two cannot be merged
    // without breaking either field-access (which needs
    // `qualified_type` to NOT outrank `field_access`) or
    // cast / instanceof RHS (which needs `_dotted_type` to outrank
    // `field_access`). Edits here should be mirrored in
    // `_dotted_type` / `_dotted_type_tail`.
    qualified_type: $ => prec.left(seq(
      alias($.identifier, $.type_identifier),
      repeat1(seq('.', alias($.identifier, $.type_identifier))),
    )),

    array_type: $ => prec(PREC.ARRAY_TYPE, seq($._type, '[', ']')),

    // §4 / §5.14 — Java-style generic type arguments. Allows nested
    // generics (`Map<String, List<Integer>>`) and bounded wildcards
    // (`? extends Foo`, `? super Bar`). The `<` is `token.immediate`
    // so it only matches when the angle bracket directly follows the
    // base name with no intervening whitespace — `a < b` still parses
    // as a binary expression because of the space before `<`.
    generic_type: $ => prec.right(PREC.ARRAY_TYPE, seq(
      field('base', choice(
        alias($.identifier, $.type_identifier),
        $.qualified_type,
      )),
      field('arguments', $.type_arguments),
    )),

    type_arguments: $ => seq(
      token.immediate('<'),
      optional(seq($._type_argument, repeat(seq(',', $._type_argument)))),
      '>',
    ),

    _type_argument: $ => choice(
      $._type,
      $.wildcard,
    ),

    wildcard: $ => seq(
      '?',
      optional(seq(
        choice('extends', 'super'),
        $._type,
      )),
    ),

    // §3.1 ACCESS tier / §3.2 — seven distinct node kinds for the
    // dot family. All at PREC.ACCESS left-associative so chains like
    // `a.b.c` parse `(a.b).c`. Right operand is the property /
    // field / method name (an identifier; method_reference also
    // accepts the `new` keyword per §5.14). Each operator emits a
    // unique node kind so downstream tooling (`get_op_type()`) can
    // discriminate without tree-shape heuristics.

    field_access: $ => prec.left(PREC.ACCESS, seq(
      field('object', $._expression),
      '.',
      field('field', $._property_name),
    )),

    safe_navigation_expression: $ => prec.left(PREC.ACCESS, seq(
      field('object', $._expression),
      '?.',
      field('property', $._property_name),
    )),

    // Groovy 4+ — see SPECIFICATION.md §3.2. Unlike `?.`, which only
    // short-circuits when the immediate receiver is null, `??.`
    // propagates null through the whole chain.
    safe_chain_dot_expression: $ => prec.left(PREC.ACCESS, seq(
      field('object', $._expression),
      '??.',
      field('property', $._property_name),
    )),

    spread_dot_expression: $ => prec.left(PREC.ACCESS, seq(
      field('object', $._expression),
      '*.',
      optional('@'),
      field('property', $._property_name),
    )),

    method_pointer_expression: $ => prec.left(PREC.ACCESS, seq(
      field('object', $._expression),
      '.&',
      field('method', $._property_name),
    )),

    direct_field_access_expression: $ => prec.left(PREC.ACCESS, seq(
      field('object', $._expression),
      '.@',
      field('field', $._property_name),
    )),

    // §5.2 — property / method names after a dot operator (or as a
    // method declaration name) can be either an identifier or a
    // *quoted identifier*: a string literal that escapes spaces,
    // keywords, or special chars. `map."key with spaces"` and
    // `def "abstract"() { … }` both rely on this.
    _property_name: $ => choice(
      $.identifier,
      alias($.string_literal, $.quoted_identifier),
    ),

    // §5.14 — method reference. Groovy 3+. The RHS is either an
    // identifier (`String::length`) or the `new` keyword
    // (`Foo::new`).
    method_reference_expression: $ => prec.left(PREC.ACCESS, seq(
      field('target', $._expression),
      '::',
      field('name', choice($.identifier, 'new')),
    )),

    // §3.2 — postfix `++` / `--`. Distinct node kind from the
    // prefix form (`unary_update_expression`) per the spec table.
    // PREC.POSTFIX, left-associative.
    update_expression: $ => prec.left(PREC.POSTFIX, seq(
      field('operand', $._expression),
      field('operator', choice('++', '--')),
    )),

    // §3.2 — prefix `++` / `--`. Apache groups these with UNARY_ADD
    // (level 3); we use PREC.UNARY to match.
    unary_update_expression: $ => prec.right(PREC.UNARY, seq(
      field('operator', choice('++', '--')),
      field('operand', $._expression),
    )),

    // §3.2 POSTFIX / §3.2.1 — `f(args)` and the subscript-yielding-
    // callable composition `arr[i](args)`. The callee slot is any
    // expression so subscript / future field_access / future
    // safe_navigation chains compose naturally. PREC.POSTFIX
    // matches subscript so chains like `obj[i](x)[j]` work.
    method_invocation: $ => prec.left(PREC.POSTFIX, seq(
      field('function', $._expression),
      choice(
        seq(field('arguments', $.argument_list), optional(field('closure', $.closure))),
        field('closure', $.closure),
      ),
    )),

    // §3.2 / §5.11 — argument list. Positional args are `_expression`;
    // named args (Groovy's `k: v` in call position) are `map_entry`
    // aliased to `named_argument` so downstream tools can tell them
    // apart from positional values. Spread (`*expr`) lands later.
    argument_list: $ => seq(
      '(',
      optional(seq(
        $._argument,
        repeat(seq(',', $._argument)),
        optional(','),
      )),
      ')',
    ),

    _argument: $ => choice($.annotation,
      $._expression,
      alias($.map_entry, $.named_argument),
      $.spread_arguments,
      $.spread_map_entry,
    ),

    // §5.11 — `*expr` only inside argument list / list literal.
    // Distinct from a binary `*` because the LHS is absent.
    spread_arguments: $ => seq(
      '*',
      field('value', $._expression),
    ),

    // §3.1 POSTFIX / §3.2 — subscript and safe-subscript. The
    // distinction `arr?[k]` (safe) vs `arr[k]` (regular) is the
    // explicit ask of dekobon #247 for the `?[]` row. Both at
    // PREC.POSTFIX, left-associative — `a[b][c]` parses as
    // `(a[b])[c]`.
    subscript_expression: $ => prec.left(PREC.POSTFIX, seq(
      field('object', $._expression),
      '[',
      field('index', $._expression),
      ']',
    )),

    // `?[` is one token. `a ? [b] : c` (with space) still parses
    // as ternary plus list because tree-sitter's lexer never
    // crosses a whitespace boundary to extend a multi-char token.
    safe_subscript_expression: $ => prec.left(PREC.POSTFIX, seq(
      field('object', $._expression),
      '?[',
      field('index', $._expression),
      ']',
    )),

    // §3.1 level 2 / §3.2 — `**`. Distinct node kind so metric tools
    // count power separately from multiplicative ops. Right-assoc
    // per Apache: `1 ** 2 ** 3` parses as `1 ** (2 ** 3)`. POWER
    // binds tighter than UNARY (+ -) but looser than UNARY_NOT
    // (! ~) — see PREC table comments.
    power_expression: $ => prec.right(PREC.POWER, seq(
      field('left', $._expression),
      '**',
      field('right', $._expression),
    )),

    // §3.1 level 13.5 / §3.2 — `==>`. Right-associative per Apache
    // (`<assoc=right>`), distinct node kind so downstream tools can
    // discriminate from `=>` (lambda arrow, not yet wired).
    logical_implication_expression: $ => prec.right(PREC.IMPLICATION, seq(
      field('left', $._expression),
      '==>',
      field('right', $._expression),
    )),

    // §3.1 level 15 / §3.2 — single `assignment_expression` node
    // covers every augmented form (`+=`, `-=`, `*=`, `/=`, `%=`,
    // `**=`, `<<=`, `>>=`, `>>>=`, `&=`, `^=`, `|=`) plus plain `=`
    // and Elvis-assign `?=`. The operator field distinguishes them.
    // Right-associative per Apache. ASSIGN is the loosest precedence
    // tier and binds looser than CONDITIONAL above it.
    assignment_expression: $ => {
      const ops = [
        '=',
        '+=', '-=', '*=', '/=', '%=', '**=',
        '<<=', '>>=', '>>>=',
        '&=', '^=', '|=',
        '?=',
      ];
      return choice(...ops.map(op => prec.right(PREC.ASSIGN, seq(
        field('left', $._expression),
        field('operator', op),
        field('right', $._expression),
      ))));
    },

    // §3.1 level 14 / §3.2 — ternary and Elvis share the CONDITIONAL
    // precedence level and are both right-associative. Apache groups
    // them in one `conditionalExprAlt` so `a ? b ?: c : d` parses as
    // `a ? (b ?: c) : d` because the middle slot recursively re-enters
    // the same precedence level. We replicate this with two distinct
    // rules at PREC.CONDITIONAL; right-associativity + recursion via
    // `$._expression` give the same result.
    //
    // elvis_expression is the headline ask of
    // dekobon/big-code-analysis #246: downstream tools need to
    // discriminate elvis from ternary by node kind alone.
    ternary_expression: $ => prec.right(PREC.CONDITIONAL, seq(
      field('condition', $._expression),
      '?',
      field('consequence', $._expression),
      ':',
      field('alternative', $._expression),
    )),

    elvis_expression: $ => prec.right(PREC.CONDITIONAL, seq(
      field('value', $._expression),
      '?:',
      field('default', $._expression),
    )),

    // §3.2 — distinct-node ops that share precedence with existing
    // binary_expression tiers but emit unique node kinds per the
    // operator-coverage matrix. The integration target (`get_op_type()`
    // in dekobon/big-code-analysis) needs to discriminate these by
    // node kind alone, not by tree-shape heuristics.

    // §3.2 range_expression — inclusive `..`, exclusive `..<` (right),
    // exclusive `<..` (left), exclusive `<..<` (both). SHIFT_OR_RANGE
    // precedence level, left-associative.
    range_expression: $ => {
      const ranges = ['..', '..<', '<..', '<..<'];
      return choice(...ranges.map(op => prec.left(PREC.SHIFT_OR_RANGE, seq(
        field('start', $._expression),
        field('operator', op),
        field('end', $._expression),
      ))));
    },

    // §3.2 identity_expression — `===` (identity) and `!==`
    // (non-identity), EQUALITY level. Per Apache, these live in
    // the equality alternative.
    identity_expression: $ => {
      const ops = ['===', '!=='];
      return choice(...ops.map(op => prec.left(PREC.EQUALITY, seq(
        field('left', $._expression),
        field('operator', op),
        field('right', $._expression),
      ))));
    },

    // §3.2 spaceship_expression — `<=>`, EQUALITY level. Three-way
    // comparison; no `operator` field since there is only one form.
    spaceship_expression: $ => prec.left(PREC.EQUALITY, seq(
      field('left', $._expression),
      '<=>',
      field('right', $._expression),
    )),

    // §3.2 regex_find_expression — `=~`, EQUALITY level. Matches a
    // pattern at any position. Fields are `subject` / `pattern` so
    // downstream tools can tell the regex-bearing operand from the
    // input expression.
    regex_find_expression: $ => prec.left(PREC.EQUALITY, seq(
      field('subject', $._expression),
      '=~',
      field('pattern', $._expression),
    )),

    // §3.2 regex_match_expression — `==~`, EQUALITY level. Anchored
    // whole-string match. Same field shape as regex_find_expression.
    regex_match_expression: $ => prec.left(PREC.EQUALITY, seq(
      field('subject', $._expression),
      '==~',
      field('pattern', $._expression),
    )),

    // §3.2 — single binary_expression node with an `operator` field
    // for every Java-shaped binary operator (per §3.2 table row 1).
    // Apache groups arithmetic operators across two precedence
    // levels (MUL: * / %, ADD: + -); we keep them on one rule with
    // distinct precedence numbers so the *node kind* stays uniform
    // while the precedence ladder matches Apache exactly.
    //
    // Currently covers ADDITIVE and MULTIPLICATIVE. Shift/range,
    // relational, equality, bitwise, and logical levels join this
    // rule in later iterations — each is a single line addition.
    binary_expression: $ => {
      /* eslint-disable no-multi-spaces --
       * Aligned columns make the precedence ladder readable.
       */
      const ops = [
        // ADDITIVE — §3.1 level 5
        ['+',   PREC.ADDITIVE],
        ['-',   PREC.ADDITIVE],
        // MULTIPLICATIVE — §3.1 level 4
        ['*',   PREC.MULTIPLICATIVE],
        ['/',   PREC.MULTIPLICATIVE],
        ['%',   PREC.MULTIPLICATIVE],
        // SHIFT — §3.1 level 6. Ranges (`..`, `..<`, etc.) share
        // this precedence level but use distinct node kinds, so
        // they live in their own rule (lands later).
        ['<<',  PREC.SHIFT_OR_RANGE],
        ['>>',  PREC.SHIFT_OR_RANGE],
        ['>>>', PREC.SHIFT_OR_RANGE],
        // RELATIONAL — §3.1 level 7. `in`, `!in`, `instanceof`,
        // `!instanceof`, and `as` share this level but emit
        // distinct node kinds (`membership_expression`,
        // `instanceof_expression`, `cast_expression`), so they
        // land in their own rules later.
        ['<',   PREC.RELATIONAL],
        ['<=',  PREC.RELATIONAL],
        ['>',   PREC.RELATIONAL],
        ['>=',  PREC.RELATIONAL],
        // EQUALITY — §3.1 level 8. `===`, `!==`, `<=>`, `=~`,
        // `==~` share this level but emit distinct node kinds.
        ['==',  PREC.EQUALITY],
        ['!=',  PREC.EQUALITY],
        // BITWISE — §3.1 levels 9, 10, 11 (each tighter than the
        // next: & < ^ < | by precedence).
        ['&',   PREC.BITWISE_AND],
        ['^',   PREC.BITWISE_XOR],
        ['|',   PREC.BITWISE_OR],
        // LOGICAL — §3.1 levels 12, 13 (&& tighter than ||).
        ['&&',  PREC.LOGICAL_AND],
        ['||',  PREC.LOGICAL_OR],
      ];
      /* eslint-enable no-multi-spaces */
      return choice(...ops.map(([op, p]) => prec.left(p, seq(
        field('left', $._expression),
        field('operator', op),
        field('right', $._expression),
      ))));
    },

    // §5.8 — closure expression. `{ -> }` is the explicit no-parameter
    // form; `{ a, b -> body }` has identifier parameters;
    // `{ String a, int b -> body }` has typed parameters; `{ body }`
    // has no `->` and an implicit `it` parameter (we do NOT emit a
    // synthetic `it` per §5.8). Default values and varargs land later.
    closure: $ => seq(
      '{',
      repeat($._statement_break),
      optional($.closure_parameters),
      repeat(choice($._statement, ';', $._statement_break)),
      '}',
    ),

    closure_parameters: $ => seq(
      optional(seq(
        $.closure_parameter,
        repeat(seq(',', $.closure_parameter)),
      )),
      '->',
    ),

    lambda_expression: $ => prec.dynamic(-1, prec.right(PREC.ASSIGN, seq(
      field('parameters', choice($.identifier, $.formal_parameters)),
      '->',
      field('body', choice($.block, $._expression)),
    ))),

    closure_parameter: $ => choice(
      prec(2, seq(optional(field('type', $._type)), field('name', $.identifier))),
      prec.dynamic(1, seq(optional(field('type', $._type)), field('name', $.identifier), '=', field('value', $._expression))),
    ),

    list_literal: $ => seq(
      '[',
      optional(seq(
        choice($._expression, $.spread_arguments),
        repeat(seq(',', choice($._expression, $.spread_arguments))),
        optional(','),
      )),
      ']',
    ),

    // §3.2 PRIMARY / §5.3 — map literal. `[:]` is the empty map;
    // anything else is one-or-more `key: value` entries. The
    // distinction between map_entry and a labelled statement is
    // contextual — labels are statement-start-only (see §5.3 and
    // the LABEL_COLON scanner branch), so inside `[ … ]` a `:` after
    // an identifier is always a map key separator.
    map_literal: $ => {
      const entry = choice($.map_entry, $.spread_map_entry);
      return seq(
        '[',
        choice(
          ':',
          seq(
            entry,
            repeat(seq(',', entry)),
            optional(','),
          ),
        ),
        ']',
      );
    },

    map_entry: $ => seq(
      field('key', $._expression),
      ':',
      field('value', $._expression),
    ),

    // §5.11 — `*: expr` only inside a map literal. `*:` is a single
    // anonymous token (longest-match keeps it intact); a bare `*`
    // followed by a `:` colon outside this context would have to be
    // syntactically impossible by the surrounding rules.
    spread_map_entry: $ => seq(
      '*:',
      field('value', $._expression),
    ),

    // §3.2 PRIMARY — `( expr )`. Distinct from `parenthesized_type_cast`
    // (§5.15), which lands once we have `_type`. The parser disambiguates
    // by what's inside the parens.
    tuple_expression: $ => seq('(', $._expression, repeat1(seq(',', $._expression)), ')'),

    parenthesized_expression: $ => seq('(', $._expression, ')'),

    // §3.2 — single `unary_expression` node kind but two precedence
    // alternatives, matching Apache §3.1:
    //   UNARY_NOT (`!` `~`)  — level 16, tighter than POWER (15),
    //                          so `~2 ** 3` parses as `(~2) ** 3`.
    //   UNARY     (`+` `-`)  — level 14, looser than POWER (15),
    //                          so `-2 ** 3` parses as `-(2 ** 3)`.
    // Both alternatives produce the same node kind, so downstream
    // tools see one consistent shape and discriminate by operator.
    unary_expression: $ => choice(
      prec.right(PREC.UNARY_NOT, seq(
        field('operator', choice('!', '~')),
        field('operand', $._expression),
      )),
      prec.right(PREC.UNARY, seq(
        field('operator', choice('+', '-')),
        field('operand', $._expression),
      )),
    ),

    // §3.2 PRIMARY — boolean and null are keywords in Groovy. The
    // `word: $.identifier` directive above lets tree-sitter resolve
    // the keyword-vs-identifier ambiguity automatically: a literal
    // 'true' / 'false' / 'null' that matches the identifier regex
    // is reclassified as the keyword token when one of these rules
    // is the only legal next reduction.
    boolean_literal: _ => choice('true', 'false'),

    null_literal: _ => 'null',

    // §5.7 — string flavours. Single and triple-single never
    // interpolate. Double-quoted and triple-double interpolate
    // `$identifier(.identifier)*` and `${expression}` as structured
    // children, so consumers can walk into the GString's expression
    // segments. Slashy bodies and dollar-slashy bodies remain flat
    // tokens (their internal `$` is rare in practice and the
    // scanner cannot interrupt a slashy mid-body without per-flavor
    // state).
    string_literal: $ => choice(
      $._single_quoted_string,
      $._triple_single_quoted_string,
      $._double_quoted_string,
      $._triple_double_quoted_string,
      $._dollar_slashy_string,
      $._slashy_string,
    ),

    // §5.7 — slashy regex `/.../`. The opening `/` is emitted by
    // the external scanner (after the context guard from §5.4 / §6.2
    // and a lookahead that confirms a closing `/` exists on the same
    // line). The body is parsed as `gstringPart` so `$identifier` and
    // `${expr}` segments expose the same structured children as
    // double-quoted GStrings. The closing `/` is `token.immediate`
    // so it cannot skip whitespace between the body and the closer.
    _slashy_string: $ => seq(
      $._slashy_string_start,
      repeat(gstringPart($, $._slashy_text)),
      token.immediate('/'),
    ),

    _slashy_text: _ => token.immediate(repeat1(choice(
      /[^\/\\$\n\r]/,
      /\\[\s\S]/,
    ))),

    _single_quoted_string: _ => token(seq(
      '\'',
      repeat(choice(
        /[^'\\\n\r]/,
        /\\[\s\S]/,
      )),
      '\'',
    )),

    _triple_single_quoted_string: _ => token(seq(
      '\'\'\'',
      repeat(choice(
        /[^'\\]/,
        /\\[\s\S]/,
        /'[^']/,
        /''[^']/,
      )),
      '\'\'\'',
    )),

    // §5.7 / §6.3 — double-quoted GString. The opening `"` is
    // tokenised separately from the body so that `$identifier` and
    // `${expr}` segments parse as proper child nodes. The text
    // fragment stops before any `$`, so `$identifier` and `${expr}`
    // get a chance to match; literal `$` (followed by `"` or a
    // digit etc.) is recovered as `$` plus the fallback shape.
    _double_quoted_string: $ => seq(
      '"',
      repeat(gstringPart($, $._gstring_text)),
      token.immediate('"'),
    ),

    _gstring_text: _ => token.immediate(repeat1(choice(
      /[^"\\$\n\r]/,
      /\\[\s\S]/,
    ))),

    // Bare `$` inside a GString body that does not start a valid
    // interpolation (`$"`, `$5`, `$$`, `$ `) is matched as a one-
    // char literal fragment. This rule must have lower precedence
    // than `_gstring_dollar_path` so `$name` still wins.
    _gstring_literal_dollar: _ => token.immediate(prec(-1, /\$/)),

    // §5.7 — triple-double GString. Like `_double_quoted_string`
    // but newlines and embedded `"` / `""` are part of the body.
    _triple_double_quoted_string: $ => seq(
      '"""',
      repeat(gstringPart($, $._triple_gstring_text)),
      token.immediate('"""'),
    ),

    _triple_gstring_text: _ => token.immediate(repeat1(choice(
      /[^"\\$]/,
      /\\[\s\S]/,
      /"[^"]/,
      /""[^"]/,
    ))),

    // §5.7 — `$identifier(.identifier)*` interpolation. The whole
    // `$identifier...` shape is matched as a single immediate token
    // so the `$` only consumes when followed by an identifier start;
    // `_gstring_literal_dollar` handles the lone-`$` fallback above.
    gstring_dollar_interpolation: $ => seq(
      field('value', alias(
        $._gstring_dollar_path,
        $.identifier,
      )),
    ),

    _gstring_dollar_path: _ => token.immediate(
      /\$[A-Za-z_][A-Za-z0-9_]*(\.[A-Za-z_][A-Za-z0-9_]*)*/,
    ),

    // §5.7 / §6.3 — `${expression}` interpolation.
    gstring_brace_interpolation: $ => seq(
      token.immediate('${'),
      field('value', choice($._expression, $.command_chain)),
      '}',
    ),

    // §5.7 — `$/...$/` dollar-slashy strings. The body is parsed
    // through `gstringPart` so `$identifier` and `${expr}` segments
    // expose structured children. `_dollar_slashy_text` consumes
    // every literal-text alternative: `/X` where X is not `$`,
    // `$$` (literal `$`), `$/` (literal `/`), and any other
    // non-`/` non-`$` char. The closer `/$` is `token.immediate`
    // so it always wins over text when `/` is followed by `$`.
    _dollar_slashy_string: $ => seq(
      '$/',
      repeat(gstringPart($, $._dollar_slashy_text)),
      token.immediate('/$'),
    ),

    _dollar_slashy_text: _ => token.immediate(repeat1(choice(
      /[^/$]/,
      /\/[^$]/,
      /\$\$/,
      /\$\//,
    ))),

    // §5.1 — hex (0x…), binary (0b…), decimal int / float / scientific,
    // each with optional type suffix from { G g L l I i D d F f }.
    // Underscores between digits are accepted permissively (the spec
    // forbids leading/trailing `_`, but that is a semantic check,
    // not a grammar one — see §1.2).
    number_literal: _ => token(choice(
      /0[xX][0-9a-fA-F_]+[GgLlIi]?/,
      /0[bB][01_]+[GgLlIi]?/,
      /[0-9][0-9_]*\.[0-9][0-9_]*([eE][+-]?[0-9][0-9_]*)?[GgFfDd]?/,
      /[0-9][0-9_]*[eE][+-]?[0-9][0-9_]*[GgFfDd]?/,
      /[0-9][0-9_]*[GgLlIiDdFf]?/,
    )),

    identifier: _ => /[A-Za-z_$][A-Za-z0-9_$]*/,
  },
});
