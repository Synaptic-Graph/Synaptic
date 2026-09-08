# CodeGraph Groovy grammar

Vendored from [dekobon/tree-sitter-groovy](https://github.com/dekobon/tree-sitter-groovy),
version 0.3.0, revision `654e4c2d736571dbc512043a1dc26017c7aedc85`.
Upstream's MIT and Apache-2.0 licenses are preserved.

Local changes implement annotated/final formal parameters, nested class/trait/
interface/enum/annotation declarations, and trailing-closure method invocations.
The fork also supports annotation/modifier-led implicit return types, modifier-only fields, diamond type arguments, and Java-style lambdas while retaining Groovy closure parameters.
Qualified command calls, named arguments, typed fields, statement boundaries,
closure defaults, array creation, typed tuples/loops, annotation defaults, and
enum bodies are covered by the completion corpus. The scanner now emits an
explicit statement-break token where newline whitespace would join statements.
They restore real Spock and HTTP Builder NG constructs that the published grammar misses.
The changes are in `grammar.js`; generated files must never be hand-edited.
Regression cases live in `test/corpus/regressions.txt`.

Regenerate with the locked CLI (0.26.13):

```powershell
npm ci --ignore-scripts
node node_modules/tree-sitter-cli/install.js
npx --no-install tree-sitter generate
npx --no-install tree-sitter test
npm run lint
```

The Rust binding builds checked-in C sources, so normal CodeGraph builds need
neither Node.js nor a grammar generator. Keep this fork until an upstream release
includes these fixes and passes CodeGraph's pinned OSS comparison.

The corpus is grouped into six topic files; all upstream and local cases are
preserved. Nine independent stress snippets are embedded in
`bindings/rust/tests/parse_stress.rs` and checked for ERROR and MISSING nodes.

## Stress-corpus sources

Each inline case is a synthetic Groovy snippet authored
for this project (dual-licensed under Apache-2.0 OR MIT, same as the
parent grammar) or adapted from public-domain examples. The stress test
(`bindings/rust/tests/parse_stress.rs`) parses these cases and
asserts the parser produces zero `ERROR` and zero `MISSING` nodes
for every case.

| Inline case | Coverage focus |
|------|---------------|
| `arithmetic_and_ranges.groovy` | numeric literals, binary expressions, range_expression, parenthesisation |
| `class_with_methods.groovy` | class declaration, multiple methods, formal parameters with types and defaults |
| `closures_and_lists.groovy` | closures, lists, maps, method invocations with closure arguments |
| `control_flow.groovy` | if/else, while, for-in, switch (classic + arrow), try-catch-finally with multi-catch |
| `imports_and_package.groovy` | package, plain / static / wildcard / aliased imports |
| `operators_grab_bag.groovy` | dot-family access, regex match / find, spaceship, identity, Elvis, ternary, range |
| `generics.groovy` | `generic_type`, `type_arguments`, class / interface / trait `type_parameters`, method `method_type_parameters`, wildcards (`?`, `? extends`, `? super`), nested and qualified generic bases |
| `jenkins_pipeline.groovy` | realistic Jenkinsfile shape — nested `pipeline { stages { stage(...) { steps { ... } } } }` closures, GString interpolation, named-argument command chains |
| `gradle_buildscript.groovy` | Gradle DSL — `plugins { id '...' }`, `dependencies { implementation '...' }`, `tasks.named('test') { ... }`, configuration closures |

Adding new inline stress cases is encouraged — keep them syntactically
valid Groovy that exercises constructs from `SPECIFICATION.md` §3
and §4. Each new case must keep the integration test green
(zero ERROR / zero MISSING).
