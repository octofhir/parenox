<p align="center">
  <img src="assets/parenox.png" alt="Parenox — a copper fox framed by matching parentheses" width="100%">
</p>

<h1 align="center">Parenox</h1>

<p align="center"><b>Fast Clojure feedback. Native Rust tooling.</b></p>

<p align="center">
  <a href="https://github.com/octofhir/parenox/actions/workflows/ci.yml"><img src="https://github.com/octofhir/parenox/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <img src="https://img.shields.io/badge/status-experimental-orange" alt="Experimental">
  <img src="https://img.shields.io/badge/built_with-Rust-dea584" alt="Built with Rust">
  <a href="#license"><img src="https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg" alt="License: MIT OR Apache-2.0"></a>
</p>

<p align="center">
  <a href="#quick-start">Quick start</a> ·
  <a href="#a-project-sized-example">Project example</a> ·
  <a href="#agent-sessions">Agent sessions</a> ·
  <a href="#current-scope">Current scope</a>
</p>

Parenox reads Clojure source, checks supported type contracts, and explains errors
at the expression that caused them. It is built for short edit–check–fix loops,
with readable terminal diagnostics and a persistent JSON Lines interface for agents.

> **Early prototype.** The supported language subset is intentionally small. Parenox
> reports unsupported analysis explicitly. It does not execute checked code, load
> project namespaces into a JVM, or prove arbitrary runtime specs.

## Quick start

Requires Rust; the current development toolchain is Rust 1.100.0. No JVM is needed
for static checking. One crate, one CLI binary. The language compatibility target
is Clojure 1.13; the optional JVM reference is pinned to 1.13.0-alpha11 in
[deps.edn](deps.edn). This target does not imply complete language support.

```sh
git clone https://github.com/octofhir/parenox.git
cd parenox
cargo build --release

./target/release/parenox check examples/good.clj
./target/release/parenox check examples/bad.clj
```

With [just](https://github.com/casey/just) installed, the common commands are:

```sh
just                 # list commands
just errors          # three clinic errors with highlighted source
just errors plain
just errors json
just errors pretty always  # force color in a captured terminal
just good            # passing example
just syntax          # inspect source-preserving syntax
just reader          # metadata, characters and tagged EDN
just portable        # JVM branches in a .cljc source file
just checked         # Clojure 1.13 checked bindings and two deliberate errors
just defaults        # destructuring defaults and three deliberate errors
just threading       # ->, ->>, some->, some->> and three deliberate errors
just conditional     # cond->, cond->>, as-> and three deliberate errors
just guards          # if-let, when-let, if-some and when-some
just paths           # nested map lookup and update with source diagnostics
just presence        # presence guards for optional fields
just updates         # update and update-in with callback diagnostics
just vectors pretty always  # mapv and filterv with colored callback diagnostics
just entries pretty always  # map entry pipelines and diagnostics
just dictionaries pretty always  # map-of contracts and diagnostics
just map-keys pretty always  # string and dynamic map keys
just dynamic-paths pretty always  # nested maps with dynamic path segments
just ordered-paths pretty always  # saved vector paths and positional contracts
just indices pretty always  # indexed reads and out-of-bounds diagnostics
just loops            # accumulators and recur diagnostics
just cond             # ordered guards and fallthrough diagnostics
just libspecs         # bare requires and renamed imports across files
just namespace-specs # alias-only imports and project-wide named specs
just agent-loop      # fix/recheck and cache measurements
just verify          # format, Clippy and tests
```

The `errors` recipe accepts the expected diagnostic exit code 1.

The second single-file example deliberately fails with three type errors. Default output
shows source, highlighted expressions, expected/found types, and proposed diffs:

```text
error[PNX2322]: Argument 1: mismatched types
  --> examples/bad.clj:27:14
   |
27 |   (increment "not a number"))
   |              ^^^^^^^^^^^^^^ expected `Number`, found `String`
```

```sh
parenox check examples/bad.clj --format plain
parenox check examples/bad.clj --format json
parenox check examples/bad.clj --color always
```

Use the built binary's full path if it is not on `PATH`. Pretty output goes to
stderr. Plain and JSON go to stdout. Pretty output uses Cargo-style checking
and finish lines, red errors, blue source gutters, cyan patch help and
red/green edits. A project check prints a single final summary.
Color is automatic in a terminal. In a pipe, `CLICOLOR_FORCE=1` enables ANSI
unless `NO_COLOR` is set. `TERM=dumb` disables terminal auto-detection.
`--color always|never` takes precedence, including in `just errors pretty always`.
`--json` remains an alias for `--format json`.

Exit codes: **0** successful check, **1** diagnostics, **2** usage or I/O failure.
Suggested edits require review and are never applied automatically.

## A project-sized example

The [clinic example](examples/clinic) has four namespaces:

| Namespace | Responsibility |
|---|---|
| `clinic.model` | Patient, request and response shapes |
| `clinic.store` | A deterministic repository stand-in returning a patient or nil |
| `clinic.service` | Display-name logic using the shared patient contract |
| `clinic.http` | An HTTP-style handler with 200, 400 and 404 responses |

It also declares a contract for the real library function
`clojure.string/starts-with?` in [externs.edn](examples/clinic/externs.edn).
Externs are trusted declarations, not independently verified library implementations.

```sh
./target/release/parenox check examples/clinic \
  --externs examples/clinic/externs.edn

cargo run --locked --release --example agent_loop -- --project examples/clinic --iterations 100
```

The CLI shows three planted mistakes: a number passed as a patient id, a number
passed to a string-library function, and an optional patient name used without
handling nil. The Rust example fixes all three in memory, checks the result, measures
warm round trips and body edits, then changes a public signature and verifies
that its caller is checked again. It leaves example files unchanged.

Directories are scanned recursively for `.clj` and `.cljc` files. Symbolic links
and hidden directory entries are skipped. Supplied files form one project;
namespace cycles and duplicate namespaces are reported. JSON output for a project
is one object with per-file reports and aggregate statistics. Single-file JSON
keeps the simpler file-report shape.

Named specs resolve against the supplied project declarations, including keys
whose namespace differs from the file that declares them. Input file order does
not affect resolution. An alias created with `:as-alias` can qualify keywords
without creating a namespace load dependency. Competing declarations of the
same spec key in separate files report an analysis gap.

## Lossless syntax

```sh
just syntax                          # comments, commas, quotes, sets and #_ forms
just syntax examples/syntax_bad.clj   # deliberate mismatch; exit 1, later code retained
```

`parenox syntax FILE` prints the CST to stdout and source-highlighted reader
errors to stderr. It uses [Rowan](https://github.com/rust-analyzer/rowan)
0.18.0-alpha.2, pinned exactly while its trivia APIs evolve. The same reader
serves the checker and the CST. The library exposes `reader::parse(source)`:
its `syntax()` view preserves every input byte, including whitespace, comments,
commas, discarded forms, all reader-conditional branches and malformed input.
Valid top-level forms and reader errors are returned separately. Check the errors before using recovered forms.

`reader::read(source)` is the strict semantic path used by checking: it stops at
the first reader error and does not construct a CST. CST capture is optional;
ordinary checks keep using semantic forms and their existing caches.

The checker selects `:clj` branches in `.cljc` files. First matching feature wins;
`:default` matches in order, an unmatched `#?` contributes no form, and `#?@`
inserts a selected list or vector into its surrounding collection. Selected forms
retain their original spans for diagnostics. Inactive definitions and requires do
not enter the checked interface or dependency graph. Ordinary `.clj` input rejects
reader conditionals, while `syntax` preserves them for tooling.

The library also exposes `reader::read_with(source, Conditionals::Select(":clj"))`
and `Conditionals::Preserve`. `reader::parse_with(source, policy)` produces both
selected semantic forms and the complete CST in one pass. Selection is a reader
policy, separate from type analysis; selecting `:cljs` in the reader does not make the checker a ClojureScript
analyzer. Preserved forms still need selection and validation before checking.
Unsupported reader syntax remains a reported gap even in inactive branches.
Splicing directly through a quote, metadata prefix, tag or conditional feature/value
pair is currently unsupported; ordinary collection splicing is supported.

The same reader handles the currently supported common Clojure/EDN syntax.
This does not implement a complete EDN reader, macro expansion, incremental
reparsing or a formatter. A future formatter can consume the CST and verify
that its output preserves significant tokens and comments.

## Shared frontend and extension boundaries

Parenox owns its reader and semantic forms. A single reading pass produces
`Form` / `Kind` with source spans; optional capture also produces the lossless
Rowan CST. Consumers reuse these representations rather than implementing a
second parser. Forms and spans belong to an exact source snapshot. The source
and CST retain lexical details that the current type-oriented forms normalize,
including numeric spelling; the forms alone are not yet a runtime value model.

| Layer | Responsibility | Extension point |
|---|---|---|
| Reader and forms | Recognize syntax, preserve spans and metadata, retain tag/payload pairs | More reader forms and literal decoding |
| Optional CST | Preserve every byte, trivia and malformed input | Formatter and editor operations |
| Analysis and sessions | Resolve names, infer types, track dependencies and cache checks | Library analysis and richer contracts |
| Core transformations | Transform threading steps using existing forms and source spans | More reviewed core macro rules |
| Future runtime and lowering | Consume forms with source origins | Interpreter or compiled runtime |

Core form transformations and execution are separate semantic stages. Types, namespace
resolution and mutable runtime state do not belong in the CST. Macro-generated
code will need explicit expansion origins and its own cache dependencies; source
spans alone cannot represent an expansion history. These are extension boundaries,
not implemented runtime features. We keep one crate and add abstractions when
there is a concrete consumer.

`just reader` demonstrates name metadata (`^:private`, `^{:t/sig ...}`), parameter
hints and character contracts. Metadata is a wrapper around a form, not an extra
allocation on every ordinary node. Its descriptor is normalized to a map; the CST
retains the original spelling. Outer metadata overrides inner metadata, and a
`defn` attribute map overrides name metadata. JVM hints such as `^String` do not
coerce a value or replace a type contract.

Characters are distinct from strings. The reader accepts single JVM characters,
named escapes, four-digit Unicode escapes and octal escapes. Tagged literals retain
the tag and payload as data. The syntax command does not run data readers or Java
constructors. Checking a tagged expression currently reports unsupported analysis
instead of guessing a type from its payload. This is shared Clojure/EDN syntax
support, not a strict or complete EDN validator.

Design references include [clojure-reader](https://github.com/Grinkers/clojure-reader)
for data reading, [Crabjure](https://github.com/ribelo/crabjure) for static analysis,
and [clojurust](https://github.com/csm/clojurust) for future reader/expansion/runtime
boundaries. Parenox implements its own APIs; these projects are references rather
than runtime or frontend dependencies.

## Agent sessions

```sh
./target/release/parenox serve
```

One request and one flushed response per line. A single-file request:

```json
{"id":1,"path":"demo.clj","source":"(defn f {:t/sig '[:-> number?]} [] 1)"}
```

A project request contains a **complete snapshot** of its files, plus an optional
extern file. The `source` values are full file contents, not patches:

```json
{"id":2,"files":[{"path":"a.clj","source":"(ns a)"},{"path":"b.clj","source":"(ns b (:require [a :as a]))"}],"externs":{"path":"externs.edn","source":"{}"}}
```

Responses have `id` and `result`. Results include `ok`, `complete`, diagnostics,
and `stats`: elapsed microseconds, parsed/reused files and checked/reused functions.
Project results contain a `files` array. Invalid JSON returns an error and the
process continues.

- Unchanged sources reuse their parsed forms and declaration models.
- Unchanged externs reuse their declarations until a referenced spec changes.
- Annotated body edits recheck that function while preserving callers' cached results.
- Edits to unannotated private helpers invalidate their transitive callers in the same file.
- Public contract changes invalidate dependent namespaces.
- Dependency invalidation is conservative at namespace granularity.
- Removed files and externs disappear from the current environment immediately.
- Unchanged functions with successful checks can be reused after their source ranges move.
  Functions with diagnostics are rechecked if their own ranges or a helper's ranges move.
- There is no disk cache, filesystem watcher, socket daemon or cache eviction yet.
  Retained file snapshots live until the process exits.

`source_hash` is a session fingerprint, not a cryptographic digest. Cache validity
uses exact source, positions and interface comparisons.

To measure the same clinic edit sequence with a larger extern file, run
`just agent-loop 1000 target/release/parenox 512`. This adds synthetic library
contracts that refer to the clinic's patient spec. Run
`just agent-loop 1000 target/release/parenox 0 128` to append 128 typed functions
whose ranges move as the agent edits the first function in their file.

## Current scope

Types are read as data from `s/def` and quoted `:t/sig` metadata. Public functions
need signatures. Direct calls to private helpers (`defn-`, `^:private` or `{:private true}`)
can omit them: the checker uses the actual argument types to analyze the body and
infer its result. This is inference for each call, not a generalized function signature.

```clojure
(defn- normalized [name]
  (clojure.string/upper-case (or name "Unknown")))

(defn display-name
  {:t/sig '[(s/nilable string?) :-> string?]}
  [name]
  (normalized name))
```

Run `just inference` for passing and failing patient-label examples. Errors point
to the helper body and include notes showing its call chain. Repeated calls with
the same inputs reuse inferred results within the checked entry function.

Core threading macros `->`, `->>`, `some->` and `some->>` are supported. The checker
resolves names before selecting a rule, so local and project functions can shadow
these names. `some->` variants stop on nil; false continues through the chain.
The seed is analyzed once, and flat pipelines are processed iteratively. Nested
threading expansion is bounded to 32 active expressions and reports an analysis
gap at the limit. Compiler-level resolution and arity checks still apply to skipped
steps; runtime type and missing-key errors are excluded from proven skipped paths.
Run `just threading` for examples and three deliberate errors.

`cond->` and `cond->>` process every condition/step pair, preserving the current
value when a condition is false and continuing after nil. Unknown conditions join
the executed and skipped result types. Branch-specific guards narrow outer locals
for that step; a guard on the unchanged seed local also narrows its threaded input.
After a potentially executed transformation, repeated tests are analyzed
conservatively without correlating earlier branches. Scalar type predicates retain
known true/false results, and joined causal notes are deduplicated. Inferred map
alternatives are compacted into a shape with optional fields, keeping chains of
conditional `assoc` operations bounded by the number of fields. Correlations
between keys are not retained; declared contract unions keep their alternatives.

`as->` supports a simple unqualified symbol binding. It rebinds that local before
each original expression, allows the value in any argument position, and continues
through nil and false. Its lexical binding does not escape to the outer scope.
Clojure also permits destructuring bindings here; those remain an explicit analysis
gap in Parenox. Run `just conditional` for examples and deliberate errors.

[expand.rs](src/expand.rs) transforms individual threading steps using the shared
`Form` representation, independently of the type checker. The consumer
owns evaluation order, nil guards and temporary lifetime. Source steps and their
metadata are preserved; diagnostics include macro origin notes. Mechanical edits
inside transformed calls are withheld when source replacement could change the
threading semantics. User macro execution, a general macroexpander and full
expansion provenance remain future work.

`if-let` and `when-let` bind truthy values. `if-some` and `when-some` also bind
`false`. Each initializer is checked once. The successful branch can use the
narrowed value through a local symbol or a supported map destructuring pattern.
The fallback runs in the outer scope and defaults to nil. Run `just guards`
to see the result type when no fallback is supplied and a type error when
`if-some` passes `false` into a numeric call.

`get-in`, `assoc-in`, and `update-in` accept vector paths with keyword or
string keys, including computed segments such as `[sku tier]`. A path can
be passed through a local or a function contract when its positions are
known. A conditional path is checked for each complete alternative.
A missing path uses the `get-in` fallback and a present terminal nil or
false is preserved. Writes create missing or nil intermediate maps, preserve
sibling fields, and report scalar parents. Write paths support 1 to 32 keys.
Read paths also accept known decimal integer indices in the i32 range.
Unknown-length paths and numeric write keys report an analysis gap.
Run `just dynamic-paths pretty always` for inline examples and
`just ordered-paths pretty always` for saved paths and diagnostics.

`nth`, `get`, `req!` and `contains?` analyze known decimal integer indices
in the i32 range. Vectors and map entries retain known positions. String
reads use UTF-16 positions, matching Clojure/JVM. For an out-of-range index,
`get` yields nil or its fallback. `nth` without a fallback reports a
provably out-of-range index; on nil it returns nil. A general vector
contract gives `nth` the element type on successful reads. A supplied
fallback handles absence and preserves present nil. Index expressions
whose exact decimal integer value is unavailable report an analysis gap.
Run `just indices pretty always` for examples and diagnostics.

`contains?` checks keyword and string keys. A guard such as `(contains? patient :name)`
refines a local map in each branch. The successful branch can read a declared
optional field without adding nil for absence. The failure branch lets `get`
or `get-in` use the fallback. A field declared nilable still accepts explicit
nil, and the type of an undeclared field in an open map remains unknown.
With `(contains? prices sku)`, the guard also refines a direct lookup or the
first segment of a nested operation using the same two bindings. Rebinding
either local removes the fact.
Run `just presence` for examples and source diagnostics.

`update` accepts a keyword or string expression, and `update-in` uses a key
path. The updater receives the old value first, followed by any extra
arguments. An absent entry passes nil. The updater's result becomes the new
field type. A dynamic key may overlap a field, so Parenox combines the
old and new value types for that field. Supported updaters
include built-in functions, functions with signatures, and directly named
private helpers. Inline `fnil` replaces nil arguments and preserves false.
For example, `(update patient :visits (fnil inc 0))` works when visits is absent.
Parenox supports one to three defaults when the wrapper is called immediately
or passed directly as a supported callback. Storing the wrapper for later use
reports an analysis gap.

Inline `(fn [params] ...)` works in immediate calls and as a supported callback.
Its parameters take their types from the call, supported map destructuring
works in the parameter vector, and the body can access outer locals.
Diagnostics inside the body point at its original expressions and include
the call location. This covers anonymous functions with one fixed arity.
Local storage of the function itself, named functions, multiple arities,
rest parameters and preconditions or postconditions report analysis gaps.
Run `just updates` to see both an implicit old-value error and a body error.

`mapv` takes one callback argument from each input collection and returns a
vector of callback results. It stops at the shortest input. `filterv` retains
elements for which the callback returns a truthy value. Core scalar predicates
narrow the retained type. `some?` removes nil and keeps false.

Both support vectors, sets, strings (as characters), closed maps,
`s/map-of` dictionaries, and nil (as empty). User-defined predicates retain the
input element type unless the callback is known to always return a falsey
value. Run `just vectors pretty always` for a patient-label pipeline and
two callback diagnostics.

A closed map supplies a `MapEntry<K, V>` for each present field. Use `key`,
`val`, or fixed positional bindings such as `(fn [[k v :as entry]] ...)`.
Optional fields contribute entries only when present, and a present nil remains
nil. An entry satisfies vector contracts for its two items. Iterating an open
map, including an `s/keys` contract, reports an analysis gap because undeclared
fields can have arbitrary types. Run `just entries pretty always` for examples.

A contract of the form `(s/map-of key-spec value-spec)` types the entries of
a dictionary. It allows empty maps. The key and value specs flow into
`MapEntry<K, V>` when a dictionary is iterated. Keyword and string lookups
use the value type if the key can occur and preserve missing-key defaults.
`contains?` narrows presence, and `assoc` and `update` track the updated
entry while retaining the types of the remaining entries.
Run `just dictionaries pretty always` for contracts and
`just map-keys pretty always` for dynamic key examples. Options on `s/map-of` and lookup
keys outside keywords, strings and known decimal integer indices report gaps.

Clojure 1.13 checked lookups support keyword and string keys in `req!` and
explicit map bindings, plus `:keys!` and its namespaced directives.
Keys after `&` in a keys vector are not bound; checked directives require their
presence. Ordinary keyword renaming and nested map patterns use the same binding
analysis. Run `just checked` for passing examples and two deliberate errors.

A presence check keeps the declared value type, including explicit nil. For an
optional `String` field, a successful checked lookup yields `String`; an optional
nilable field still yields `Nil | String`. Checks describe successful execution;
runtime presence guards remain in the program. A key proven absent produces
`PNX2741` at the binding or lookup key. An undeclared value type in an open map
remains an explicit analysis gap; presence-only checks after `&` need no value type.
Refinement applies to bound locals and checked lookup results. Source-map aliases
keep their declared types.

Map `:or` defaults can use local symbols or literal keyword lookup keys, including
renamed, namespaced and nested bindings. An absent key uses its default; a present
nil or false is preserved. Every default expression is analyzed eagerly in the
outer lexical scope, including unused defaults and defaults for present keys.
Defaults in one map do not introduce locals for each other. Ordinary function
arguments are available before parameter destructuring, including later arguments.
Private helpers used in parameter defaults participate in inference and cache
invalidation. Run `just defaults` for examples and deliberate failures.

Providing both a local-symbol default and a keyword default for the same binding,
or providing a default for a checked required key, produces `PNX1001` at the
default. A default applies to the local value; `:as` retains the input collection.

Map destructuring with `:missing`, `:select`, `:all` or `:excess` and
symbol lookup keys remain unsupported. Positional destructuring covers
vectors, map entries, strings and nil. Vector literals of at most 32 elements retain
their positions, and `(s/tuple ...)` describes a vector with 1 to 32 position
types. Longer literals retain an element type. Missing positions bind nil.
A vector with unknown length contributes nil as a possible value at each
position. Rest bindings report an analysis gap.

Recursive helpers, unused helpers with no typed caller, and helpers used as
function values currently require `:t/sig`. Private inference is bounded to 32
nested calls and 256 specializations per checked entry function; hitting a limit
reports an explicit analysis gap. Function statistics count annotated definitions;
inferred helper work is included in the check of their callers.

| Area | Supported |
|---|---|
| Types | Strings, characters, numbers, booleans, keywords, nil, literal constraints, maps, vectors, sets, unions |
| Spec forms | `s/keys`, `s/map-of`, `s/tuple`, `s/nilable`, `s/or`, vector/set `s/coll-of` |
| Control flow | `let`, `do`, `if`, `when`, `cond`, `or`, `and`, `if-let`/`when-let`, `if-some`/`when-some`, `loop`/`recur`, local predicate narrowing |
| Core macros | `->`, `->>`, `some->`, `some->>`, `cond->`, `cond->>`, simple-binding `as->` with source diagnostics |
| Data operations | Keyword calls, string and keyword lookups and updates, ordered vector paths, indexed reads, positional and nested map destructuring and checked keys |
| Functions | Single arity, forward calls, bare requires, namespace aliases, `:as-alias`, explicit `:refer` with `:exclude` and `:rename`, private-var boundaries, direct private-helper inference |
| Libraries | A small built-in rule table and one optional EDN map of external signatures |
| Output | Cargo-style pretty, plain, JSON Lines; reviewable suggested edits |

Clojure distinctions are preserved: a set in expression position is a `Set<T>`;
a set in spec position describes allowed values. Nil, false and absent keys are
separate. `get` defaults preserve explicit nil; zero and empty collections are
truthy. Callable sets return a matching value or nil, not a boolean; membership
results are currently conservative.

Inference separates a general expression type from precise known values. Ordinary
strings are shown as `String`, including inside maps and collections, while exact
values remain available to check finite-value contracts.

`recur` transfers values to the nearest loop or supported function body. Each
iteration is checked with the types of its incoming arguments. Structural
errors such as wrong arity and non-tail use are reported even in branches that
will not run. Inference reports a gap if a loop keeps generating distinct or
growing types beyond its analysis budget.

Limitations include user macros, Java interop, generics, lazy sequence functions,
recursive specs, multi-arity/variadic definitions, full numeric
tower semantics and some reader forms. Numeric expressions use `Number`,
with decimal integer literals retaining their value for indexed reads.
Externs currently declare qualified function signatures; they do not load JARs or
resolve Maven/Git dependencies. There is no runtime, interpreter or VM yet.

Any error gives `ok=false`. `PNX9xxx` also gives `complete=false`: some analysis
could not be performed. `complete=true` describes coverage of the implemented
subset, not proof of all runtime behavior or application intent.

## Performance and development

```sh
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
cargo fmt --all -- --check

./target/release/parenox bench examples/good.clj --iterations 1000
cargo run --locked --release --example agent_loop -- --iterations 100 --functions 1000
cargo run --locked --release --example agent_loop -- --project examples/clinic --iterations 100
```

`bench` measures in-process checking. The Rust example measures full JSON round
trips including source transfer, encoding, pipes and decoding. Measurements are
local examples, not claims about large production projects. Recorded results are
in [benchmarks](benchmarks).

Optional independent runtime smoke checks use the pinned Clojure 1.13 reference
and require the Clojure CLI with Java 17 or newer:

```sh
clojure -M scripts/runtime_smoke.clj
```

The implementation stays in one crate: source reader, type model, declaration and
expression analysis, project sessions, terminal rendering and CLI. Next steps are
broader real-code coverage, better library contracts and measured improvements
to the analysis model. A future runtime is a direction, not a current feature.

## License

Licensed under either [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option.
