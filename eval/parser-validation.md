# Parser validation — 2026-09-07

This report combines the five parser reviews. The final findings come first;
earlier measurements, fixes, baseline corrections and superseded limits remain
in the historical sections below.

- [Pinned corpus](parser-validation.toml): one 13-repository manifest for current checks.
- [Results](parser-validation-results.json): all five original result objects under
  `runs`, with their 9-, 11- and 13-repository scopes in `corpus_members`.
- [Final implementation and validation](#final-implementation-and-validation).
- [Review history](#review-history).

Historical JSON fields retain their original file paths and development-binary
versions as provenance, not current input paths. Raw graphs, compiler dumps and
frozen binaries remain in their existing ignored `synaptic-out/eval/` directories.
The subsequent 1.2.0 release preparation passes 2,774 workspace tests and fixes
the desktop `chart` catalog omission reported in pass 2.

## Final implementation and validation

This pass implements the four workstreams left by the
[fourth review](#pass-4-native-macros-and-aliases): Groovy syntax/compiler facts,
Fortran semantics/build inputs, native preprocessing/build targets, and
compiler-backed validation. It uses the same 13 revisions in
[the pinned manifest](parser-validation.toml).

### Implemented changes

| Review area | Implementation | Evidence |
|---|---|---|
| Groovy | Qualified command/DSL calls, statement boundaries, typed fields, closure defaults, annotation defaults, generic methods, arrays, tuples, loops, enum bodies and mixed modifiers. Optional compiler facts include AST-transform methods and resolved calls; unresolved calls retain dynamic-site metadata. | 666 real source files have zero Tree-sitter errors; 523 grammar cases pass. Groovy compiler exports preserve generated overload identity and reject stale source inputs. |
| Fortran | Type-bound bindings, inheritance and override candidates; generic selection using argument types, explicit kinds, ranks, keywords and optional parameters; submodule host/prototype association; external-interface implementation resolution; expanded intrinsic catalog; build flags and reverse module/call dependencies. | GFortran compiles all 59 FFTPACK library units and both upstream transform examples. Its oracle agrees on 65 definitions and all 66 direct repository calls, including implementation files. |
| Native | Compilation-database preprocessing, response files, source-line mapping, CMake target/dependency membership, header extraction inside a real translation unit, C/C++ language linkage and internal-linkage filtering. | Configured CMINPACK has zero parser errors, 99 preprocessed files, and no build diagnostics. Clang agrees on 163 definitions and all 283 direct repository calls. |
| Compiler ground truth | Runnable Clang, GFortran and Groovy checks, including generated methods, exact callee files, source freshness and deliberately damaged graphs. | A wrong-file C++ callee with the same name fails the C oracle. The Fortran oracle caught 18 interface-versus-implementation errors, all corrected. Missing generated Groovy calls also fail validation. |

The native and Fortran oracles compare source files as well as names. Name-only
comparison hid the FFTPACK interface-target error: its 66 call names looked
correct, but only 48 pointed to the implementation file before the final fix.
The corrected result is 66/66, with no unexpected calls. Likewise, the C
compiler comparison exposed 19 missing calls caused by unrelated C++ overloads.

### Independent Groovy declaration check

The official Groovy 5.0.1 compiler parses each original source through its
CONVERSION phase, before dependency resolution or AST transforms. Across 665
accepted files, all **5,851 compiler declarations** match distinct graph nodes
by name and overlapping source span, including columns. This check exposed 45
missing declarations before the last fix: overload ID collisions, anonymous-body
method collisions and an escaped quoted name. Overloads now have distinct IDs,
including declarations on the same line; ambiguous source calls remain unresolved.
Quoted method labels decode escapes and retain the original spelling for anchors.
A negative control removes overload nodes and must fail the declaration check.

One of 666 source files, HTTP Builder NG's `EncodersSpec.groovy`, duplicates the
`MULTIPART_MIXED` static import and is rejected by Groovy 5. It is recorded as a
compiler failure, excluded from compiler recall, and left unchanged. Tree-sitter
parses all 666 files without errors. This compiler declaration oracle does not
claim successful builds of Spock or HTTP Builder NG.

```text
groovy scripts/groovy-declaration-oracle.groovy ROOT SOURCE_LIST OUTPUT
```

### Source-only comparison

| Corpus/language | Files | Previous parser-error files | Current | Current exact anchors |
|---|---:|---:|---:|---:|
| Spock / Groovy | 542 | 53 | 0 | 5,083/5,083 |
| HTTP Builder NG / Groovy | 75 | 27 | 0 | 385/385 |
| Groovy WSLite / Groovy | 49 | 8 | 0 | 402/402 |
| LAPACK / Fortran | 3,613 | 0 | 0 | 3,993/3,993 |
| FPM / Fortran | 221 | 8 | 8 | 1,409/1,409 |
| CMINPACK / C | 109 | 51 | 51 | 192/192 |

The unconfigured and compiler-assisted measurements are separate. Build flags
are required to select macro branches and source forms correctly. Supplying the
five reviewed FPM compilation entries fixes all five corresponding failures:
fixed form in a `.f90` file, free form in a `.f` file, two preprocessing examples,
and `fpm_os.F90`. Two standalone include fragments and the grammar's unsupported
empty derived-type array constructor remain parser diagnostics in that checkout.
Configured CMINPACK drops from 51 parser-error files to zero across the project.

All 13 source-quality gates pass: 13,000 files and 82,591/82,591 exact declaration
anchors, with deterministic extraction and incremental equivalence. Groovy
parser-error bounds are now zero. Spock's empty-file bound changes only because
`spock-core/core.gradle:113` contains a `filter(...)` call, not a function
declaration; the earlier parser invented that declaration. A negative regression
check now prevents it. HTTP Builder NG also loses bogus declarations such as
`called`, `equalTo`, `responds`, `toURI`, and `getInputStream`; the independent compiler check confirms that all accepted source declarations remain.

### Compiler-assisted operation

The CLI and incremental extractor discover `compile_commands.json` at the root
or under `build/`, or use `SYNAPTIC_COMPILE_COMMANDS`. They read preprocessing
and dialect flags as data and do not execute the database's shell command.
`SYNAPTIC_NATIVE_COMPILER` and `SYNAPTIC_FORTRAN_COMPILER` select compatible
drivers; defaults are `gcc` and `gfortran`. Missing response files, malformed
commands and compiler failures produce `build_diagnostic` metadata and retain
the source graph.

For CMake target membership, create an empty
`<build>/.cmake/api/v1/query/codemodel-v2` before configuring. The extractor reads
the [CMake File API](https://cmake.org/cmake/help/latest/manual/cmake-file-api.7.html)
reply next to the compilation database. Header nodes identify their selected
real include context in `header_translation_unit`. Preprocessing preserves line
anchors; expanded columns describe compiler-expanded text.

For AST transforms, run
[`export-groovy-facts.groovy`](../scripts/export-groovy-facts.groovy) with the
project's Groovy compiler and dependency classpath:

```text
groovy scripts/export-groovy-facts.groovy ROOT OUTPUT SOURCE_LIST [CLASSPATH]
```

`SOURCE_LIST` contains one root-relative source path per line. Use
`ROOT/.synaptic/compiler-facts.json` as output, or set `SYNAPTIC_COMPILER_FACTS`.
Export runs compiler phases through class generation, including AST transforms;
it does not invoke application methods. Source contents and classpath metadata
are checked before import. Generated nodes retain compiler provenance, their
signature, and a generating-class anchor. Compiler-resolved calls use
`compiler_resolved_call`; generated overloads are never merged by label alone.

The tested Groovy transform fixture has seven methods, five generated, and two
resolved repository calls. Seven unchanged WSLite production sources compile
with Groovy 5.0.1 and export 47 methods, including 27 generated methods. Those
dynamic WSLite sources supply no compiler-resolved repository calls; no such
accuracy claim is inferred from their successful compilation.

The final Rust run passes 1,272 tests across 36 suites. Strict Clippy, formatting, the minimal-feature build, four previous source-regression validators, and the new compiler validator pass. The minimal-feature build retains 14 pre-existing dead-code warnings.

### Reproduction and validation scope

Evidence is under `synaptic-out/eval/parser-completion-2026-09-07/`. Reports and
executable hashes are summarized in
[`parser-validation-results.json`](parser-validation-results.json).

```powershell
cargo test -p synaptic-core -p synaptic-extract -p synaptic-graph -p synaptic-eval -p synaptic-incremental -p synaptic --lib --tests --locked
cargo clippy -p synaptic-core -p synaptic-extract -p synaptic-graph -p synaptic-eval -p synaptic-incremental -p synaptic --all-targets --all-features --locked -- -D warnings
cargo check -p synaptic-extract --no-default-features --locked
synaptic eval quality --manifest eval/parser-validation.toml --baselines crates/synaptic-eval/quality-baselines.toml --skip-oracle
python scripts/validate-parser-completion.py synaptic-out/eval/parser-completion-2026-09-07
python scripts/validate-parser-sources.py synaptic-out/eval/parser-completion-2026-09-07 --binary target/debug/synaptic.exe
```

The CMINPACK build uses CMake's MinGW Makefiles generator, exported compilation
commands, double precision, and `USE_BLAS=OFF`, `USE_LAPACK=OFF`. Its 44 upstream
CTest cases pass. The independent C oracle command is:

```text
python scripts/validate-compiler-graph.py --repo ROOT --database COMPILE_COMMANDS --graph GRAPH --clang CLANG --out REPORT
```

On this Windows host Clang additionally uses
`--clang-arg=--target=x86_64-w64-windows-gnu` and
`--clang-arg=--sysroot=C:/Strawberry/c`. The Fortran counterpart is
[`validate-fortran-compiler.py`](../scripts/validate-fortran-compiler.py), with
the same repo/database/graph/output arguments. FFTPACK's database lists
`src/rk.f90`, `src/fftpack.f90`, the remaining 57 library sources, and the two
upstream complex/real transform examples in dependency order. The examples
compile, link and execute successfully; the repository's full test-drive suite
was not run.

These are finite compiler checks on selected builds, not proof of every runtime
execution. Arbitrary dynamic names, runtime metaprogramming, virtual receivers
and linker interposition require runtime evidence for a unique actual target.
The graph exposes uncertainty through dynamic sites or inferred candidates.
Conflicting compilation entries require selecting one active configuration;
build-configured incremental changes currently rebuild conservatively so changed
headers, flags or compiler facts cannot leave stale graph fragments. The reverse
Fortran dependency index handles ordinary source updates without whole-language
replay. No exhaustive coverage claim is made for unbuilt dependencies or other
compiler dialects.

## Review history

These sections describe their respective completed passes, not current behavior.
Their limits and test failures may be superseded by the final findings above.
Repeated build/quality commands are consolidated into the final reproduction
section. The original corpus scopes remain in the combined results; current
quality commands use the shared 13-repository manifest.

### Pass 1: initial language review

Targeted fixes improve Fortran declaration and call coverage, Groovy recovery and
source locations, and YAML resource locations. Validation covers **nine pinned
open-source repositories, 12,455 files, and 99,497 resulting graph nodes**.
All nine passed deterministic and incremental-equivalence checks; none was skipped.

The [machine-readable evidence](parser-validation-results.json) contains
before/after counters, source revisions, per-language failures, independent oracle
comparisons, and baseline changes. The [manifest](parser-validation.toml)
pins every repository. The before executable was built from tracked revision
`9619fd783885e6be63548ceccbe5d5b9b86c4db5`; after is this modified working tree.
Both used debug builds on Windows/x86_64 with 16 logical CPUs. No speed claim is made.

#### Findings and changes

- **Fortran:** the free-form grammar was parsing fixed-form comments as code and
  missing column-six continuations. LAPACK's `DGESV` pointed to its documentation
  example at line 21 instead of its declaration at line 121. Fixed-form adaptation
  now preserves source lines, handles comments/continuations, and ignores sequence
  columns. The walker also visits internal procedures and calls in assignment
  statements, resolves local calls without case sensitivity, and gives Fortran
  symbols identities distinct from same-named C translations.
- **Groovy:** malformed grammar nodes could absorb preceding fields into a method,
  or mistake a field for the constructor name. The shared walker uses the explicit
  callable name to correct the anchor while retaining the body span. Recovery now
  handles primitive return types and quoted Spock feature names, and excludes
  comments and embedded triple-quoted source. Recovered nodes remain marked inferred.
- **YAML:** Kubernetes resources were all anchored at line 1, including later
  documents in a stream. They now point to their own `metadata.name`, falling back
  to `kind` for unnamed resources. The evaluator checks the literal resource name
  in `Kind/name` labels, including numeric and hyphenated names.

PHP and C/C++ were also remeasured because they ranked poorly in the older published
results. Their current code already handles the sampled anonymous-declaration and
macro cases. The fmt, libuv, Slim, and Guzzle controls retained identical graph
node/edge counts and 100% parsed-anchor scores after these changes.

#### Measured results

Counts below are per language, not pooled across unrelated files in each repository.

| Repository / language | Metric | Before | After |
|---|---|---:|---:|
| LAPACK / Fortran | Files with parse errors | 3,578 / 3,611 | **2 / 3,611** |
| LAPACK / Fortran | Files with no declarations | 2,367 / 3,611 | **3 / 3,611** |
| LAPACK / Fortran | Parsed checkable declarations | 1,317 | **3,990** |
| LAPACK / Fortran | Declarations found only by ctags | 2,682 / 3,973 | **3 / 3,973** |
| LAPACK / Fortran | Graph call edges from Fortran nodes | 330 | **18,823** |
| CMINPACK / Fortran | Files with parse errors | 45 / 46 | **0 / 46** |
| CMINPACK / Fortran | Files with no declarations | 9 / 46 | **2 / 46** |
| CMINPACK / Fortran | Declarations found only by ctags | 8 / 45 | **0 / 45** |
| CMINPACK / Fortran | Graph call edges from Fortran nodes | 23 | **58** |
| fortran-lang/stdlib / Fortran | Parsed checkable declarations | 1,402 | **1,427** |
| Spock / Groovy | Correct parsed anchors | 1,104 / 1,113 | **1,113 / 1,113** |
| Spock / Groovy | Recovered checkable declarations | 1,361 | **3,881** |
| SchemaStore / YAML | Correct parsed anchors | 216 / 231 | **231 / 231** |

CMINPACK was cloned as a fresh holdout after the initial fixes, at
[`32d343ac33ac297594b5ffac57741e5615b4bb07`](https://github.com/devernay/cminpack/tree/32d343ac33ac297594b5ffac57741e5615b4bb07).
It exposed the C/Fortran identity collision during validation; the final correction
was rerun on it and the original corpus. It is therefore validation-driven evidence,
not an untouched final test set. It is now included in the permanent quality corpus.

The final corpus has **70,984 / 70,984 correct parsed anchors**, plus 3,881 / 3,881
correct recovered anchors in their separate bucket. These location checks do not
establish exhaustive declaration recall or call-edge precision.

#### Benchmark corrections and independent checks

The old oracle filter excluded Fortran `subroutine`, `program`, `module`, and
`submodule` tags. CMINPACK consequently appeared to match every oracle declaration
while the comparison considered just two functions. The filter now includes those
Fortran kinds and respects case-insensitive identifiers. **Both saved before and
after graphs were rescored with the same corrected evaluator and Universal Ctags
6.1.0** for the oracle rows above. Oracle input is written concurrently with output
collection to avoid pipe-buffer deadlocks on large file lists.

The name-on-line anchor heuristic also accepted commented Fortran documentation as
a declaration. A targeted source-reviewed check now verifies the real `DGESV`
declaration and these three graph edges against the pinned source:

- LAPACK `SRC/dgesv.f::DGESV` → `SRC/dgetrs.f::DGETRS` (call at line 170).
- CMINPACK `examples/hybdrv.f::fcn` → `examples/vecfcn.f::vecfcn` (line 106).
- CMINPACK `fortran/chkder.f::chkder` → `fortran/dpmpar.f::dpmpar` (line 93).

The check also requires the C and Fortran `ssqfcn` translations to coexist as
distinct nodes. It **passes on the final graphs and fails on the baseline** at the
incorrect `DGESV` anchor. This is a small reviewed sample, not an estimate of the
precision of all 18,823 LAPACK call edges.

#### Validation and reproduction

- 846 tests passed across `synaptic-extract`, `synaptic-eval`, and
  `synaptic-incremental`; the evaluator's 102 tests were rerun after the oracle I/O fix.
- Clippy passed for the extraction/evaluation crates with all targets/features and
  warnings denied. Formatting and diff whitespace checks passed.
- Existing quality bounds were tightened without loosening any; CMINPACK received
  a new baseline. Oracle bounds were refreshed only where the oracle was measured.

See [current reproduction commands](#reproduction-and-validation-scope) for the consolidated build and quality checks.

The broad nine-repository run deliberately skips the oracle. Universal Ctags was
run separately over the saved LAPACK and CMINPACK graphs. A broader exploratory
oracle run was stopped; no whole-corpus oracle result is claimed. To compare saved
graphs with the same evaluator, put Universal Ctags on PATH and run:

```powershell
cargo run -p synaptic-eval --example rescore_graph -- synaptic-out/bench/lapack BEFORE_GRAPH.json AFTER_GRAPH.json
python scripts/validate-parser-sources.py synaptic-out/eval/language-review-2026-09-07 --check review --stage final
```

The source check expects `lapack-final.json` and `cminpack-final.json` in that artifact
directory; produce them by running `synaptic extract <checkout> --directed --no-store`
and copying each checkout's `synaptic-out/graph.json`. `--stage before` tests the
corresponding `*-before.json` files. Raw local reports, graph snapshots, logs, and
frozen binaries are under `synaptic-out/eval/language-review-2026-09-07/`.

#### Historical limits

These are the limits at the end of the first pass. The
[follow-up review](#pass-2-identity-and-call-resolution) records subsequent fixes,
fresh holdouts, and the limits that remain.

Groovy's grammar still reports errors in 501 / 542 files; recovered declarations
have inferred file-containment edges and do not gain full method-body semantics.
Escaped quoted method names and slashy-string recovery remain outside this bounded
recovery pass. The general anchor heuristic can still accept names in comments;
the new source checks cover the demonstrated Fortran failure rather than replacing
that heuristic with another parser.

LAPACK still misses the two external-ETIME wrappers and the `LA_ISNAN` interface
against ctags. Some calls remain unresolved where several candidate implementations
exist; for example, this run adds `DGESV` → `DGETRS` but does not resolve all three
calls in `DGESV`. Fortran fixed-form parsing assumes the standard 72-column source
area; compiler-specific extended line lengths are not modeled. The distinction is
documented in the [GNU Fortran dialect options](https://gcc.gnu.org/onlinedocs/gcc-6.4.0/gfortran/Fortran-Dialect-Options.html).

### Pass 2: identity and call resolution

This continues the [first review](#pass-1-initial-language-review). The baseline is
the completed first-pass implementation, not the older original HEAD. Both runs
use the same pinned source revisions. The [manifest](parser-validation.toml)
adds Groovy WSLite and FFTPACK as fresh holdouts; neither prompted extractor changes.

#### Measured changes

| Repository / metric | First-pass baseline | Follow-up |
|---|---:|---:|
| LAPACK Fortran declarations missed relative to Universal Ctags | 3 | **0** |
| LAPACK Fortran parsed declarations checked | 3,990 | **3,993** |
| LAPACK calls originating in Fortran | 18,823 | **22,544** |
| CMINPACK calls originating in Fortran | 58 | **58** |
| Spock Groovy parsed declarations checked | 1,113 | **4,417** |
| Spock Groovy declarations requiring recovery | 3,881 | **619** |
| Spock Groovy files with parser errors | 501 / 542 | **410 / 542** |
| Spock calls originating in Groovy | 454 | **1,180** |
| Groovy WSLite parsed Groovy declarations checked | 105 | **250** |
| Groovy WSLite Groovy files with parser errors | 48 / 49 | **45 / 49** |
| Groovy WSLite calls originating in Groovy | 64 | **113** |
| FFTPACK parsed Fortran declarations checked | 149 | **162** |

Parsed and recovered anchors in the target languages remain 100% correct under
the strengthened anchor diagnostic. These are diagnostic counts, not exhaustive
declaration recall or proof that every new call edge is correct. Universal Ctags
6.1.0 agrees on 3,973 LAPACK Fortran declarations, misses none found by ctags, and
leaves 20 Synaptic-only declarations. CMINPACK's Fortran oracle agreement stays
45 with zero ctags-only declarations. The oracle compares declarations, not calls.

#### What changed and what verifies it

- **File identity:** `ETIME.f` and `ETIME_.f` formerly normalized to the same ID.
  Their procedures parsed correctly but one file overwrote the other during graph
  assembly. File IDs now include a stable fingerprint of the relative path.
  Python relative imports and Bash source resolution use the shared helper.
  Incremental updates detect legacy file IDs and rebuild the AST once, preventing
  a mixture of old and new file identities. Existing symbol IDs based on language
  scopes are not a universal collision-free symbol scheme.
- **Fortran interfaces and calls:** named generic interfaces produce nodes and
  references to their module procedures. `.f`, `.for`, and modern Fortran suffixes
  share a language family. A unique sibling external procedure can resolve an
  otherwise ambiguous name; those edges remain **INFERRED**, with explicit
  `fortran_sibling_call` context. Unassociated module/internal procedures are
  excluded from this external-procedure fallback.
- **Groovy method bodies:** quoted names are adapted to identifier tokens for the
  Java-shaped grammar while preserving every byte position and reading names from
  the original source. Methods can therefore retain class ownership, spans, and
  calls. Punctuation-sensitive quoted names keep distinct IDs. Recovery recognizes
  escaped quotes and excludes declarations inside slashy/dollar-slashy strings in
  supported expression contexts. The lexical rules follow the
  [Groovy syntax documentation](https://docs.groovy-lang.org/latest/html/documentation/core-syntax.html).
- **Anchor scoring:** code evidence masks common line/block comment syntax,
  including fixed-form Fortran comments. Rationale nodes still use their actual
  comment text. The filename exception is limited to document nodes and source
  formats that declare file-named components/models, including `.cshtml` views.
- **Extended fixed form:** `SYNAPTIC_FORTRAN_FIXED_LINE_LENGTH=132` selects an
  extended source area; `0` means unlimited. Unset or invalid values use 72.
  The selected width participates in fixed-form AST cache keys. Library callers
  can use `extract_fortran_source_with_line_length` directly. Match this setting
  to the compiler's [fixed-form line-length option](https://gcc.gnu.org/onlinedocs/gfortran/Fortran-Dialect-Options.html),
  then run a full extraction.

The [source check](../scripts/validate-parser-sources.py) verifies both ETIME
wrapper variants, the `LA_ISNAN` interface and its two references, and all three
`DGESV` calls: `XERBLA` at line 159, `DGETRF` at 165, and `DGETRS` at 170. It also
checks WSLite's quoted REST test method at line 39, its class ownership, and its
call to `getMockResponse` at line 44. It passes on the final snapshots and fails
on the baseline. The previous review's three call checks also still pass.
The width/cache check runs 72 → 132 → unlimited → 72 against the same source.

The first follow-up run exposed a real regression: adding modern Fortran suffixes
made module procedures compete with external routines in CMINPACK. The final
resolver excludes those inaccessible candidates and preserves all 58 prior
Fortran call edges. Final measurements use that correction.

#### Benchmark counting corrections

File identity also restores seven previously overwritten C file nodes in CMINPACK
and one file node in SchemaStore. Their existing declarations were not recovered
by this fix. Counting these files honestly changes the zero-declaration ceilings:

| Repository | Previous bound | Corrected bound | Measured count |
|---|---:|---:|---:|
| CMINPACK | 0.0750 | 0.1138 | 19 / 167 files |
| SchemaStore | 0.5239 | 0.5240 | 1,706 / 3,256 files |

These two deliberate baseline adjustments correct hidden files, not improved
extraction. CMINPACK's seven restored C paths are `examples/{chkdrv,hybdrv,hyjdrv,
ibmdpdr,lmddrv,lmfdrv,lmsdrv}.c`; they still need C extraction work. C oracle
agreement remains 151 with 41 ctags-only declarations. Other updated bounds
tighten or remain unchanged. The two fresh holdouts now have pinned baselines.

#### Validation and reproduction

The final corpus covers 11 repositories, 12,601 files, and 101,190 graph nodes.
All 75,025 checked parsed anchors pass; all 11 repositories pass determinism,
incremental equivalence, and the explicitly revised benchmark bounds.
Raw graphs, frozen binaries, oracle comparisons, test logs, and quality reports
are under `synaptic-out/eval/language-followup-2026-09-07/`.
The portable [results](parser-validation-results.json) record the final evidence.

See [current reproduction commands](#reproduction-and-validation-scope) for the consolidated build and quality checks.

The source checks consume saved `<repo>-final.json` snapshots, produced with
`synaptic extract <checkout> --directed --no-store` and copied from the checkout's
`synaptic-out/graph.json`. Use `--stage before` to check the baseline snapshots.
To rescore both versions with the same evaluator, use the existing `rescore_graph`
example with Universal Ctags on PATH. The broad corpus run skips the oracle;
LAPACK and CMINPACK have separate measured oracle comparisons.

The affected-crate test run passed 1,242 tests; the final evaluator changes passed
a separate rerun of 104 unit and eight integration tests. Clippy passed for all
affected crates with all targets/features and warnings denied, including a final
evaluator rerun. Formatting and diff whitespace checks pass.
The broader workspace run reached 2,260 passing tests, three
ignored tests, and one unrelated failure: the UI command catalog omits the existing
`chart` CLI command. No UI catalog changes are included.

#### Historical limits

Groovy's grammar still errors on 410 Spock files and 45 WSLite files. Quoted method
adaptation improves body extraction; it does not implement the entire Groovy DSL
or optional-semicolon grammar. Recovery's slashy-string detection is conservative
around command-style arguments versus division, and quoted labels retain their
source escape spelling rather than evaluating dynamic/interpolated identifiers.

Ambiguous sibling procedures remain unresolved. Fortran cross-file `USE`
association is not a full compiler symbol resolver, and the line-length setting
does not automatically read per-file compiler flags from build systems. The
extended-width check is synthetic; the OSS runs use their default 72-column setting.
Anchor scoring remains a lexical diagnostic: strings, macros, uncommon comment
forms, and misleading same-name text can still require source review. LAPACK's
two remaining parse-error files and the CMINPACK C declaration gaps remain visible.

### Pass 3: native Groovy grammar and scope

This pass continues the [language follow-up](#pass-2-identity-and-call-resolution).
It replaces the Groovy grammar and fixes gaps found in its real-source output,
adds Fortran lexical and `USE` association, and restores C declarations lost to
file-name collisions and missing aggregate extraction.

The baseline is the completed second pass, frozen before these changes. HTTP
Builder NG and Fortran Package Manager were measured with that same executable
before their sources were used for validation. The [manifest](parser-validation.toml)
pins all 13 repositories; [portable results](parser-validation-results.json)
contain the before/after measurements, call counts, oracle results, and binary hash.

#### Groovy grammar and graph extraction

| Repository | Groovy files | Files with parser errors, before → after | Parsed declarations checked, before → after | Recovered declarations checked, before → after |
|---|---:|---:|---:|---:|
| Spock | 542 | 410 → 64 | 4,417 → 4,967 | 619 → 91 |
| Groovy WSLite | 49 | 45 → 8 | 250 → 377 | 128 → 4 |
| HTTP Builder NG, fresh holdout | 75 | 48 → 33 | 254 → 399 | 54 → 1 |

All checked Groovy declaration anchors remain exact. Spock's parser-error file
count falls 84.4%; declarations increasingly come from parsed syntax with bodies
and owners instead of declaration recovery.

The published [dekobon Groovy grammar](https://github.com/dekobon/tree-sitter-groovy/tree/654e4c2d736571dbc512043a1dc26017c7aedc85)
0.3.0 substantially improved parsing, but source review exposed regressions around
annotated parameters, nested types, and trailing closures. A small grammar patch
implements those constructs. All 520 upstream and added grammar corpus cases,
plus the highlighting assertions and ESLint, pass. The runtime is vendored with
its original licenses, grammar source, generated C, tests, and
[regeneration instructions](../vendor/tree-sitter-groovy/README.codegraph.md).
Normal Rust builds do not require Node.js or a grammar generator.

The extractor now understands traits, records, annotation declarations, native
quoted names, command calls, safe navigation, spread calls, qualified types,
and annotations in the new AST. The old quoted-name byte substitution is removed.
Source checks verify `EmbeddedSpecCompiler.compile` → `doCompile` at line 105
and `compileSpecBody` → `compileWithImports` at line 118, both restored after
fixing annotated parameters in the grammar.

Groovy-origin call edges increase from 1,180 to 1,546 in Spock, 113 to 187 in
WSLite, and 105 to 292 in HTTP Builder NG. These are graph counts, not a measured
call-recall score. Some old edges disappear because receiver and owner information
changes, or unsupported constructs remain. The portable results include retained,
removed, and added semantic pairs; an increased count alone is not treated as proof
of correctness.

#### Fortran scope, imports, and main programs

Calls now search the nearest lexical scope and then host scopes. Cross-file `USE`
resolution uses declared module names rather than filenames, respects `ONLY`,
renames, explicit/default accessibility, and re-exports, and rejects ambiguous or
unavailable bindings. Generic interfaces require scope/import evidence; they are
not treated as globally visible external procedures. Named and unnamed main
programs now contribute calls.

| Repository | Fortran-origin call edges, before → after |
|---|---:|
| LAPACK | 22,544 → 22,988 |
| CMINPACK | 58 → 87 |
| Fortran Package Manager, fresh holdout | 947 → 2,289 |

CMINPACK retains all 58 previous semantic call relationships and adds 29 from
previously skipped main programs. FPM has 1,751 calls resolved with explicit
scope/import evidence. Source checks cover `has_manifest` → `exists` and
`join_path` at `app/main.f90:107`, `build_package` → `mkdir` at
`src/fpm_backend.F90:88`, and the main program's imported
`get_command_line_settings` call at line 31.

An incremental regression test switches a re-export from module A to module B,
then makes B's procedure private. Unchanged callers retarget and then lose the
inaccessible call, matching fresh rebuilds at each step. Fortran edits replay
Fortran extraction fragments; unchanged sources use the AST cache. Obsolete
unreferenced import stubs are removed while semantic and hyperedge references
remain preserved.

#### C declarations and upgrade handling

The seven CMINPACK C drivers that previously appeared empty had symbol IDs that
collided with underscore-suffixed source variants. C/C++ symbol namespaces now use
the complete file identity. Struct, union, and enum definitions are extracted;
type uses and forward references do not masquerade as definitions, and C function
pointer fields do not become C++ methods.

Universal Ctags agreement for CMINPACK C declarations rises from 151 to 179;
ctags-only declarations fall from 41 to 13, with zero Synaptic-only declarations
in that comparison. Source checks verify all seven `main` variant pairs and
`struct refnum` at `examples/hybdrv.c:36`. The surviving oracle gaps include
typedefs/anonymous aggregates. Independent-parser agreement is not exhaustive
ground-truth precision or recall.

Grammar dependency and vendored C changes now invalidate the AST cache. File
nodes record the extractor version, so an incremental update of an older graph
rebuilds its AST even when source files are unchanged. A regression test checks
the upgrade and the subsequent unchanged cache-preserving update.

#### Validation and reproduction

The final evidence is in `synaptic-out/eval/parser-upgrade-2026-09-07/verified/`.
It covers 13,000 files and 105,719 graph nodes across 13 pinned repositories.
All repositories pass deterministic extraction, incremental equivalence, and
their pinned bounds. Of 78,546 checked anchors, 78,538 pass. The eight failures
are unchanged from the holdout baseline in vendored minified jQuery in HTTP
Builder NG; its Groovy anchors all pass. No existing bound was loosened. The two
holdouts are added to the main 65-repository corpus; this pass measured the 13
listed repositories, not all 65.

All 1,250 affected-crate tests and Clippy with warnings denied pass. The separate grammar suite passes
520 cases and all highlighting assertions. All three source-validation scripts
pass, including the earlier LAPACK/CMINPACK and quoted-Groovy checks. These runs
validate extraction against real repositories; they do not build or execute
each upstream project's entire application test suite.

See [current reproduction commands](#reproduction-and-validation-scope) for the consolidated build and quality checks.

Source checks consume saved extraction snapshots. The existing
`synaptic-eval` `rescore_graph` example rescored both stages using the same
evaluator and Universal Ctags 6.1.0. Older `candidate/`, `grammar/`, and `complete/`
directories retain intermediate evidence; `verified/` is the final gate run.

#### Historical limits

- Groovy still reports errors in 64 Spock, eight WSLite, and 33 HTTP Builder NG
  files. More DSL/command-chain forms, lambdas, and newer syntax remain; dynamic
  names and escaped labels are not evaluated by a Groovy runtime.
- Fortran resolution is not a compiler frontend. Type-bound dispatch, overload
  selection, intrinsic overrides, submodule semantics, and preprocessing/build
  configuration remain incomplete. Missing/ambiguous imports are kept unresolved.
- Per-file fixed-form compiler flags are not automatically read from build
  systems. The existing 72/132/unlimited setting remains available. LAPACK's
  `BLAS/TESTING/dblat1.f` and `sblat1.f` still report parse errors.
- CMINPACK retains 13 C and five C++ ctags-only declaration differences; its C
  parser-error files also remain visible in the results.
- Anchor scoring remains lexical and is not a substitute for source review or
  compiler-backed ground truth. The unchanged jQuery failures illustrate that limit.
- Fortran incremental updates replay that language's files from cache. A module
  dependency index could reduce work if this becomes a measured bottleneck.

### Pass 4: native macros and aliases

This pass continues the [parser upgrade](#pass-3-native-groovy-grammar-and-scope), using its
completed executable as the baseline and the same [13 pinned repositories](parser-validation.toml).
The [portable results](parser-validation-results.json) include every
repository's before/after measurements, call-edge changes, oracle comparisons,
environment, and delivered executable hash. These repositories are regression
validation for this pass; none is claimed as a new unseen holdout.

#### Measured changes

| Repository / language | Files | Files with parser errors, before → after | Declaration anchors checked, before → after |
|---|---:|---:|---:|
| LAPACK C | 2,907 | 2,871 → 13 | 247 → 3,085 |
| LAPACK Fortran | 3,613 | 2 → 0 | 3,993 → 3,993 |
| CMINPACK C | 109 | 51 → 51 | 179 → 192 |
| CMINPACK C++ | 2 | 0 → 0 | 22 → 34 |
| Spock Groovy | 542 | 64 → 53 | 4,967 → 4,967 |
| HTTP Builder NG Groovy | 75 | 33 → 27 | 399 → 401 |
| Groovy WSLite Groovy | 49 | 8 → 8 | 377 → 377 |

All checked anchors in these rows are exact. A file can contain extracted
declarations and still report a syntax error elsewhere; removing that error does
not necessarily increase its declaration count. Zero-declaration C files in
LAPACK fall from 2,833 to eight.

#### Native declarations and calls

C normalization was stripping `API_SUFFIX(symbol)` before the parser could see
the function declaration. It now preserves macro-wrapped declaration names and
extracts the full source expression, such as `API_SUFFIX(cblas_dgemm)()`.
Call extraction and resolution use that same expression. Distinct arguments,
wrapper names, and trailing underscores retain distinct symbol identities.
Function-pointer-returning functions still use their actual function name.
Weak-symbol annotations and typed uppercase function names also survive
normalization.

Source checks verify CBLAS `cblas_dgemm` at line 12 with 14 parameters and its
call to the production `cblas_xerbla` definition. Recognizing that weak definition
and preferring a unique candidate in the caller's directory prevents selection
of the testing replacement. Directory-based resolution remains explicitly
inferred, with confidence 0.8; it is not evidence of a compiler/linker binding.

C/C++ extraction now includes multiple typedef declarators, anonymous aggregate
aliases, function-pointer aliases, and C++ `using` declarations. Alias ownership
preserves scope: CMINPACK's nine separately scoped `Fn` aliases have nine distinct
IDs. Named aggregate definitions remain represented once; unions and enums are
included in C++ extraction.

Universal Ctags 6.1 comparisons use the same updated evaluator for both stages:

| Repository / language | Agreements, before → after | Ctags-only, before → after | Synaptic-only, before → after |
|---|---:|---:|---:|
| LAPACK C | 218 → 3,085 | 2,868 → 1 | 29 → 0 |
| CMINPACK C | 179 → 192 | 13 → 0 | 0 → 0 |
| CMINPACK C++ | 21 → 25 | 4 → 0 | 0 → 0 |

The remaining LAPACK Ctags-only declaration is
`LAPACKE/mangling/Cintface.c:18 C_INTFACE`. Ctags comparison collapses repeated
file/name pairs, so it does not measure scope identity; the nine `Fn` declarations
are separately verified against source. Macro names represent source expressions,
not expanded ABI symbols.

#### Groovy and Fortran

The vendored Groovy grammar now supports annotation/modifier-led methods with
implicit return types, modifier-only fields, empty diamond type arguments, and
Java-style lambda parameters and bodies. Existing Groovy closure parameter shapes
are preserved. These forms are grounded in the
[Apache Groovy grammar](https://raw.githubusercontent.com/apache/groovy/master/src/antlr/GroovyParser.g4).
Real-source checks include Spock's `LambdaSpec.groovy` feature at line 12.
The grammar source and generated artifacts are delivered together, with updated
corpus coverage and [regeneration instructions](../vendor/tree-sitter-groovy/README.codegraph.md).

Fortran intrinsic names now participate in lexical and `USE` association before
being treated as built-ins. Explicit `INTRINSIC` prevents a false external edge;
explicit `EXTERNAL` permits an unambiguous external binding. External candidates
are indexed once rather than scanning the graph for every call. Ambiguous
bindings remain unresolved, and cross-file external fallback is marked inferred.
Tests cover host/import overrides, explicit declarations, and accidental
same-file name matches.

Fixed-form normalization joins spaces in numeric exponents, such as `-1. D0`,
while preserving quoted strings, Hollerith payloads, comments, and source line
numbers. This fixes both LAPACK `BLAS/TESTING/dblat1.f` and `sblat1.f`; their
program anchors remain at line 36. This treatment follows fixed-form blank
insignificance outside character contexts, as documented in the
[Intel Fortran reference](https://www.intel.com/content/dam/develop/external/us/en/documents/oneapi_fortran_compiler.pdf).

#### Benchmark corrections and interpretation

The JavaScript anchor scorer now uses the existing JavaScript grammar's comment
nodes. Regex literals containing quote/comment-like text no longer cause the
scorer to mask later declarations. Re-scoring the old HTTP Builder NG graph
changes its JavaScript anchors from 112/120 to 120/120. Those eight were scoring
false alarms, not eight newly recovered functions.

Operator normalization now preserves `operator()` and consistently handles
operator whitespace in both Ctags and Synaptic keys. The prior CMINPACK C++
report's five misses become four when its old graph is re-scored; the parser
fixes those four. The raw repository baseline remains available alongside the
same-scorer comparisons in the portable results.

Full extraction call counts rise from 208 to 410 in CMINPACK and from 24,668 to
33,404 in LAPACK. The results also publish added, retained, and removed semantic
pairs. Renamed source-expression labels and corrected targets change those pairs;
larger graph counts alone are not a call precision/recall measurement. Anchor
exactness and independent-parser agreement likewise do not establish complete
semantic correctness.

#### Validation and reproduction

The final gate covers 13,000 files, 109,719 nodes, 202,010 edges, and
82,381/82,381 exact declaration anchors. All 13 repositories pass deterministic
extraction, incremental equivalence, and their bounds. CMINPACK and LAPACK also
pass separate final gates with Universal Ctags enabled. Baselines were tightened
for native declaration misses, parse errors, zero-declaration files, and HTTP
Builder NG anchors; no bounds were loosened. The full 65-repository corpus was
not rerun.

The affected-crate test run passes 1,258 tests in 36 suites. After the final
native annotation and sibling-resolution changes, all 477 extraction and 153
graph unit tests were rerun successfully. Clippy passes for all affected targets
and features with warnings denied. The Groovy corpus passes all 521 cases,
highlighting assertions, and ESLint. All four source-validation scripts pass on
the delivered snapshots. This validates graph extraction from upstream source;
it does not run each upstream application's own build/test suite.

See [current reproduction commands](#reproduction-and-validation-scope) for the consolidated build and quality checks.

Local evidence lives in `synaptic-out/eval/parser-limits-2026-09-07/`:
`delivered/report.json`, `ctags-lapack/report.json`, `ctags-cminpack/report.json`,
and six `*-delivered.json` full extraction snapshots. To recreate a snapshot, run
`synaptic extract synaptic-out/bench/<repo> --directed --no-store` and copy that
checkout's `synaptic-out/graph.json` to `<repo>-delivered.json` in the evidence
directory. The `rescore_graph` example in `synaptic-eval` compares saved graphs
under one evaluator. Intermediate executables/reports are not the final gate.

#### Historical limits

- Groovy still has the parser-error counts shown above. More command/DSL chains
  and other syntax remain unsupported; dynamic names and AST transforms are not
  evaluated.
- Fortran type-bound dispatch, submodule association, overload selection, the
  intrinsic catalog, and compiler preprocessing remain incomplete. Per-file
  build flags are not automatically imported. Cached Fortran fragments are
  replayed during incremental updates; no module dependency index was added.
- Native preprocessing remains partial. Custom parameter/annotation macros can
  still produce syntax errors, including the unchanged 51 CMINPACK C files.
  Source-expression identity and inferred directory matching do not replace
  macro expansion, build-target modeling, or linker resolution.
- Source checks cover concrete regressions. Exhaustive compiler-backed
  declaration/call ground truth remains outside these benchmark measurements.
