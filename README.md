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
for static checking. One crate, one CLI binary.

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
just good            # passing example
just syntax          # inspect source-preserving syntax
just agent-loop      # fix/recheck and cache measurements
just verify          # format, Clippy and tests
```

The `errors` recipe accepts the expected diagnostic exit code 1.

The second single-file example deliberately fails with three type errors. Default output
shows source, highlighted expressions, expected/found types, and proposed diffs:

```text
error[PNX2322]: Argument 1: expected Number, got String
  --> examples/bad.clj:27:14
   |
27 |   (increment "not a number"))
   |              ^^^^^^^^^^^^^^ this expression has type `String`
```

```sh
parenox check examples/bad.clj --format plain
parenox check examples/bad.clj --format json
parenox check examples/bad.clj --color always
```

Use the built binary's full path if it is not on `PATH`. Pretty output goes to
stderr; plain and JSON go to stdout. Colors are automatic in terminals and
respect `NO_COLOR` and `TERM=dumb`. `--color always|never` overrides auto.
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

## Lossless syntax

```sh
just syntax                          # comments, commas, quotes, sets and #_ forms
just syntax examples/syntax_bad.clj   # deliberate mismatch; exit 1, later code retained
```

`parenox syntax FILE` prints the CST to stdout and source-highlighted reader
errors to stderr. It uses [Rowan](https://github.com/rust-analyzer/rowan) 0.17,
with the same reader as the checker. The library exposes `reader::parse(source)`:
its `syntax()` view preserves every input byte, including whitespace, comments,
commas, discarded forms and malformed input. Valid top-level forms and reader
errors are returned separately. Check the errors before using recovered forms.

`reader::read(source)` is the strict semantic path used by checking: it stops at
the first reader error and does not construct a CST. CST capture is optional;
ordinary checks keep using semantic forms and their existing caches.

The same reader handles the currently supported common Clojure/EDN syntax.
This does not implement a complete EDN reader, macro expansion, incremental
reparsing or a formatter. A future formatter can consume the CST and verify
that its output preserves significant tokens and comments.

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
- Annotated body edits recheck that function while preserving callers' cached results.
- Edits to unannotated private helpers invalidate their transitive callers in the same file.
- Public contract changes invalidate dependent namespaces.
- Dependency invalidation is conservative at namespace granularity.
- Removed files and externs disappear from the current environment immediately.
- Source ranges are tied to exact source snapshots; moved functions are rechecked.
- There is no disk cache, filesystem watcher, socket daemon or cache eviction yet.
  Retained file snapshots live until the process exits.

`source_hash` is a session fingerprint, not a cryptographic digest. Cache validity
uses exact source, positions and interface comparisons.

## Current scope

Types are read as data from `s/def` and quoted `:t/sig` metadata. Public functions
need signatures. Direct calls to private helpers (`defn-` or `{:private true}`)
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

Recursive helpers, unused helpers with no typed caller, and helpers used as
function values currently require `:t/sig`. Private inference is bounded to 32
nested calls and 256 specializations per checked entry function; hitting a limit
reports an explicit analysis gap. Function statistics count annotated definitions;
inferred helper work is included in the check of their callers.

| Area | Supported |
|---|---|
| Types | Strings, numbers, booleans, keywords, nil, literal constraints, maps, vectors, sets, unions |
| Spec forms | `s/keys`, `s/nilable`, `s/or`, vector/set `s/coll-of` |
| Control flow | `let`, `do`, `if`, `when`, `or`, `and`; local predicate narrowing |
| Data operations | Keyword lookup, literal-key `get` and `assoc`, simple map destructuring |
| Functions | Single arity, forward calls, namespace aliases, `:as-alias`, simple `:refer`, private-var boundaries, direct private-helper inference |
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

Limitations include user macros, Java interop, generics, higher-order builtins,
recursive specs, multi-arity/variadic definitions, string-key maps, full numeric
tower semantics and reader conditionals. All numbers currently share `Number`.
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

Optional independent runtime smoke checks require Clojure:

```sh
clojure -M scripts/runtime_smoke.clj
```

The implementation stays in one crate: source reader, type model, declaration and
expression analysis, project sessions, terminal rendering and CLI. Next steps are
broader real-code coverage, better library contracts and measured improvements
to the analysis model. A future runtime is a direction, not a current feature.

## License

Licensed under either [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option.
