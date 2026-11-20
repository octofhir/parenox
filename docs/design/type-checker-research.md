# Type checker research for Parenox

Parenox checks Clojure source without loading namespaces or running project code. Keep the Rust reader, checker and incremental session; borrow rules that improve the guarantees we can give on existing Clojure.

## Current checker

| Area | What this means for Parenox |
|---|---|
| Contracts and externs | Parenox reads `s/def`, `:t/sig`, single-arity `s/fdef` and explicitly selected EDN files with trusted library declarations. Cross-file extern conflicts are reported at both declarations without picking a winner. Externs cannot replace a modeled built-in. |
| Project configuration | `parenox.edn` selects source paths, library externs and separately trusted dependency externs. Relative paths resolve from the config file; JSON Lines clients still supply in-memory snapshots. |
| Library contract validation | Compare selected promises to public source contracts and check source bodies without trusting those promises. Check fixed and variadic dispatch, and report incompatible or unproved promises at their EDN location. |
| Contract change reports | Compare old and new extern snapshots with independent spec environments. Check whether every old call is still covered without analyzing implementation bodies. Report invalid inputs instead of inferring removals from them. |
| Compatibility explanations | Report a rejected arity, argument, result or nested field. Follow the failed comparison only after it has been classified, so ordinary checking does not build diagnostic paths. |
| Exported contracts | Export checked public function signatures and literal vars as standalone EDN. Bound inline expansion by snapshot bytes and reader nesting. Reload the generated snapshot without source specs and refuse incomplete, unreadable or lossy exports. |
| Literal externs | Keep exact EDN data separate from its inferred type. Compare initializer values in release reports without executing code. |
| Nil, maps and guards | `get` distinguishes explicit nil from absence. Truthiness and `some?` disagree on false. Right-biased `merge` preserves required keys when an optional field can overwrite their value. |
| Map projection and removal | `select-keys` projects finite keyword and string choices into a closed shape. `dissoc` tracks the absence of a removed literal key even in an open shape. |
| Map traversal | `keys` and `vals` preserve the selected entry component, including optional emptiness. Finite map sequences retain known length and treat each position as any possible element. |
| Entry lookup | `find` returns a typed entry for a present key and nil for absence. With a finite set of possible keys, each result entry keeps its key paired with its associated value type. |
| Key-value reduction | `reduce-kv` checks a three-argument reducer with paired map entry types or vector ordinals. It shares the accumulator and early-exit analysis used by `reduce`. |
| Map value transforms | `update-vals` checks the updater against declared field and dictionary value types. Successful returns preserve key types and track fields that may be absent. |
| Map construction | `zipmap` checks matching positions and rightmost duplicate keys. `update-keys` preserves values and rejects overlapping destination keys when uniqueness cannot be proven. |
| Grouping | `group-by` checks the key callback and preserves known order within each group. Dynamic groups use a dictionary of vectors and share the map callback budget. |
| Merge callbacks | `merge-with` invokes the callback only for possible overlaps, passes values left to right and keeps one-sided outcomes for optional conflicts. |
| Tagged unions | Literal tests on a map selector narrow its owner. Local aliases keep that relation until rebinding, and finite map tags support exhaustiveness checks in `case`. |
| Checked predicates | `:t/guard` verifies true and false implications independently for a restricted Boolean expression. A body edit that changes the proven contract invalidates that predicate's callers. |
| Contextual callbacks | Fixed-arity `s/fspec` supplies parameter and result types to inline callbacks passed in direct calls. Checking the body does not execute it or add its throws to argument evaluation. |
| Generic callbacks | In `t/all`, a direct argument or a vector's element type determines the input parameter. A callback's body or named contract can supply the result parameter, including for a vector of results. The generic body is checked with both parameters abstract. |
| Generic contract equivalence | Renaming binders leaves a contract equivalent. Incremental checks reuse passing callers, but rerun checks that emitted diagnostics so their messages use the current names. |
| Primitive bounds | A bound such as `(a :< number?)` is checked against call evidence. The generic body may use `a` as a number, but cannot return an arbitrary number as `a`. Changing the bound invalidates affected callers. |
| Named generic callbacks | Specialize from the expected fixed-arity inputs, including a known callback argument's result if needed. Check the final signature for compatibility without using the expected return as evidence. |
| Typed map boundaries | Closed literal map contracts distinguish required fields from optional or missing keys. `t/all` can infer from a required field or dictionary values, including in a callback result. The storage example uses a trusted extern, not inferred SQL or a live database. |
| Independent input parameters | A fixed-arity `t/all` can infer up to two independent inputs and one optional callback result. Roles come from the signature positions, and an unknown input does not suppress definite errors on another. |
| Nullable generic results | `(s/nilable a)` preserves a known parameter type alongside `nil`, for example in a dictionary lookup. A nullable callback result alone cannot witness a parameter. |
| Macro scopes | An EDN `:let-like` description supplies sequential lexical bindings for an external macro. Result, effects and tail positions remain explicitly unchecked. |
| Compatibility | Structural checks distinguish a proven match, a mismatch and an unproven comparison. An unknown field cannot hide a known mismatch elsewhere. Recursive specs still report a gap. Function compatibility checks variance for matching arities. |
| Diagnostics | Parenox exposes spans, stable rule codes, occurrence IDs, the contract source and a `complete` flag. Occurrence IDs are stable for repeated checks of the same snapshot, not across source moves. Source comments can suppress one rule for one form without changing the inferred type. Trust baselines and coverage are not yet supported. |

## What to borrow from other languages

| Source | Use in Parenox | Do not carry over |
|---|---|---|
| [TypeScript narrowing](https://www.typescriptlang.org/docs/handbook/2/narrowing) and [`satisfies`](https://www.typescriptlang.org/docs/handbook/release-notes/typescript-4-9.html#the-satisfies-operator) | Preserve literal tags through map checks. Narrow the whole map union on a discriminant and check exhaustiveness only when its domain is known. Reuse the existing inferred/precise value split. | JS `undefined`, unchecked indexed reads, or a general mapped/conditional/template type language before concrete use cases. |
| [Flow](https://flow.org/en/docs/types/type-guards/) and [Typed Racket](https://docs.racket-lang.org/ts-guide/occurrence-typing.html) | Keep independent facts for truthy and falsey branches. Track facts by lexical binding and stable path so a guard on `(:status x)` can narrow `x`. Prove each direction of a user guard separately. | Treating an arbitrary predicate as a proof, or keeping a fact across rebinding or a new mutable read. |
| [Elixir](https://elixir.hexdocs.pm/gradual-set-theoretic-types.html) | Keep key presence independent of value type. Required, optional and absent keys fit `MapSlot`; preserve them through writes, merging and destructuring before adding row variables. | Using gradual compatibility for unknown values as proof or changing Clojure truthiness to match another language. |
| [Luau](https://luau.org/types/) and [its subtyping analysis](https://luau.org/news/2022-10-31-luau-semantic-subtyping/) | Pass expected argument types into callbacks. Start with one constrained callback contract and an explicit analysis budget rather than replacing the existing callback paths. | Mutable-table inference rules for persistent maps, executing user type functions, or adopting full semantic subtyping. |
| [Python typing and typeshed](https://typing.python.org/en/latest/spec/distributing.html) and [Sorbet RBI](https://sorbet.org/docs/rbi) | Keep externs inert, record each contract's origin, and report conflicts instead of silently replacing core. A declaration for project code must supply inputs for checking its body, not hide the body. | Loading a library to discover types during `check`, or treating a library signature as proof of its implementation. |
| [Sorbet check levels](https://sorbet.org/docs/static) and [TypeScript project references](https://www.typescriptlang.org/docs/handbook/project-references.html) | Change diagnostic policy independently from type semantics. Keep exported types separate from diagnostic completeness and trust; compare warm results with a clean check after every kind of interface edit. | A new build graph or daemon before measuring the session we already have; counting annotations as coverage. |
| [Koka](https://arxiv.org/abs/1406.2061) | Extend the existing exception analysis with the distinction between effects now and effects when a returned callback or lazy value runs. A handler only removes the exceptions it can catch. | A general effect language, runtime handlers or a single `pure` flag. |

## Crabjure: a Rust reference for Clojure analysis

[Crabjure](https://github.com/ribelo/crabjure/blob/a1a1f3224d77f9e5d89de4ababf1aa2445e5ade1/README.md) is closer to Parenox than the language checkers above: it reads Clojure source, preserves trivia and resolves scopes without executing project macros. Its macro and dependency code is useful as a reference, not as a type system to port.

- **Macro scopes.** Its [archetype catalog](https://github.com/ribelo/crabjure/blob/a1a1f3224d77f9e5d89de4ababf1aa2445e5ade1/crates/crabjure-config/src/lib.rs) describes which forms introduce locals (`let-like`, `defn-params`, `for-like`, and others). Start with a declarative description of one binding macro. Keep it separate from the rule that models evaluation, result type and effects: a macro with `let`-like bindings can still catch exceptions or skip its body.
- **Binding identity.** Its [lexical HIR](https://github.com/ribelo/crabjure/blob/a1a1f3224d77f9e5d89de4ababf1aa2445e5ade1/crates/crabjure-hir/README.md) links source nodes to explicit scopes. For stable-path refinements in Parenox, add binding identity to the current environment first. That is enough to distinguish two shadowed names without replacing Rowan or adding a new full AST.
- **Resolution and dependency caches.** Its [resolver](https://github.com/ribelo/crabjure/blob/a1a1f3224d77f9e5d89de4ababf1aa2445e5ade1/crates/crabjure-resolver/src/lib.rs) records where names resolved and loads dependency information lazily. Reuse Parenox's namespace interfaces and record that origin when we add versioned externs. Consider lazy library indexes only if measurements show a need.

Compare equivalent workloads on the same corpus and report coverage alongside speed. Measure cold lint runs and warm type-check runs against their corresponding baselines.

## Tooling roadmap through December 2026

Parenox should offer one native installation for everyday Clojure work. A developer opens an existing `deps.edn` project, gets diagnostics and navigation in the editor, runs tests on the project's runtime, and checks the same contracts in CI. The type checker adds depth to this workflow with map and nil analysis, explainable diagnostics, and public API compatibility checks.

### Shared project analysis

- Extend the existing `Session` to serve the CLI, agent protocol and LSP from the same project inputs. Resolve source roots, aliases, namespace interfaces and contract origins once per relevant revision. Use the same resolution facts for diagnostics and navigation, including when a local shadows a var.
- Keep one reader for forms and source spans. Capture the lossless CST for editor and formatting operations that need it. Keep name resolution, expansion and execution outside the reader. Retain the source and analysis needed by active clients and measure the cost before adding a persistent index.
- Start with local `deps.edn` source roots and explicit aliases. Accept a classpath resolved by the Clojure CLI for dependency interfaces. Index the relevant dependencies on demand and invalidate their consumers when an interface changes. Report missing dependency information as incomplete analysis. [Clojure CLI classpath reference](https://clojure.org/reference/clojure_cli#_classpath)
- Keep static analysis source-only. Execution commands take an explicit runtime and project configuration. Use the project's Clojure/JVM for the first test or REPL integration. Evaluate native execution by invoking a pinned `cljrs` binary for a portable `.cljc` fixture. Check reader conditionals, dependency loading and test output before claiming native support for a project. [nREPL operations](https://nrepl.org/nrepl/ops.html), [clojurust at `793ad97e`](https://github.com/csm/clojurust/blob/793ad97e57cc797878a09c66771d651cfab1ee9b/README.md)

Crabjure informs lexical binding identity, declarative macro scopes and lazy dependency indexing. Clojure LSP sets the editor workflow to match. Clojure/JVM, pinned in `deps.edn`, is the semantic reference. clojurust is an actively developed native execution option with its own `:cljrs` reader branch, dependencies, REPL and test runner. [Crabjure](https://github.com/ribelo/crabjure/blob/a1a1f3224d77f9e5d89de4ababf1aa2445e5ade1/README.md), [Clojure LSP features](https://clojure-lsp.io/features/), [clojurust commit, November 17](https://github.com/csm/clojurust/commit/793ad97e57cc797878a09c66771d651cfab1ee9b)

### Crate boundaries and dependency budget

- `parenox` provides the public library and binary entry points and wires CLI, disk projects and runtime adapters. It re-exports the library API.
- `parenox-lsp` owns protocol framing, UTF-16 positions, completion, refactoring edits and editor features.
- `parenox-frontend` owns source locations, forms, the reader and optional Rowan capture. `parenox-analysis` uses it with Serde to expose checking, project sessions and externs. `parenox-formatter` uses it with Rowan for standalone formatting. Formatting and analysis can build independently.
- For each new external dependency, record the needed capability, reuse options and the minimal Cargo features. Compare `cargo tree --locked -e normal,build` before and after. Measure clean and representative incremental builds on the same toolchain and machine, and record default release binary size. Measure the default CLI build as well as any opt-in integration. Include transitive features in the review because workspace feature unification can pull optional code back into a build.

Local baseline for `34ed757` on macOS arm64 with Rust 1.100.0. A locked, offline release build took 30.17 s with an empty target directory and an existing Cargo package cache. The immediate rebuild without source edits took 0.05 s. The binary was 4,309,056 bytes. Cargo reported 27 distinct dependency packages in the normal and build graph, excluding Parenox itself.

### Release slices

| Target | Deliverable | Acceptance scenario |
|---|---|---|
| November 25 | Project context and benchmark baseline | Open a Clojure CLI project with a source alias and a local dependency. CLI and session resolve the same namespaces. Record cold process time, peak RSS and the clinic edit round trip. |
| December 8 | First editor workflow | Open that project through a stdio LSP adapter. Get diagnostics, hover with the type and contract origin, and go to a definition across namespaces. An unsaved edit refreshes the affected diagnostics. An incomplete form keeps navigation available for unaffected code. |
| December 20 | Local project workflow | Check and format the pinned Clojure LSP checkout from RustRover. Audit the reader, macro and standard-library gaps on that checkout before adding CI requirements. |
| December 31 | Tooling preview | Verify the editor workflow on two pinned public projects. Run one portable fixture on JVM Clojure and clojurust and record the supported operations and any divergence. Publish the resource and correctness results with the preview. |

The first implementation slice is `deps.edn` source-root and alias support wired through the existing session, with one cross-namespace definition query that works on ordinary unannotated code. Then expose that query and the existing diagnostics through LSP. Regex literals, string declarations and JVM member checks enter the release backlog where they unblock a pilot. Formatting, completion and rename are follow-up consumers of the same source and resolution data.

### Editor feature parity

Parenox should cover the [Clojure LSP editor workflows](https://clojure-lsp.io/features/)
before claiming it can replace Clojure LSP. The tooling preview is not that
milestone. Match the behavior users rely on, not necessarily the implementation
or command names. Use the pinned [local Clojure LSP pilot](../reference.md#local-clojure-lsp-pilot)
to compare both servers on the same files and unsaved edits in RustRover.
Track current gaps and the comparison command in the [parity matrix](lsp-parity.md).

| Slice | Workflows to compare |
|---|---|
| Project analysis | Resolve local and dependency vars, ClojureScript branches, macro scopes and Java interop. Compare lint findings, missing information and project-specific rules separately. |
| Navigation | Find declarations, implementations and references; list file and workspace symbols; highlight usages and show incoming and outgoing calls. |
| Editing assistance | Complete locals, vars, aliases, Java names, dependencies and snippets. Show documentation and signature help, and classify names for syntax highlighting. |
| Safe edits | Support rename, file and range formatting, namespace cleanup, imports, extraction, inlining, and structural edits. Compare edits and preserve comments and lexical bindings. |
| Project tools | Show references on declarations, navigate the test and project trees, and expose dependency stubs and server inspection. |

Start with two parallel tracks: fix analysis gaps on the pilot and add standard
`textDocument/codeAction` support. The first contract action should offer a
reviewable `s/fdef` for a single-arity `defn`. Reuse the namespace's existing
`clojure.spec.alpha` alias, leave an existing contract alone, and derive only
types the checker can explain. Observed calls alone do not prove which inputs
a function accepts. Leave unknown types explicit in a draft, not in a checked
contract. Compare the proposed edit and its check result on the same buffer.

Use [clj-kondo's lint configuration](https://github.com/clj-kondo/clj-kondo/blob/master/doc/config.md)
and [cljfmt's indentation rules](https://github.com/weavejester/cljfmt/blob/master/docs/INDENTS.md)
as separate benchmarks. Keep project settings in one `parenox.edn` shared by
the CLI and editor. Compare lint findings on the same sources with real bugs
reported separately from analysis gaps. For formatting, compare the actual
edits as well as whether they preserve syntax, comments, and all reader
branches. Whole-file and selected-line indentation and spacing are
available. Line wrapping and richer macro indentation remain open.

### First pilot project

The [first product iteration](first-product-iteration.md) records the final
RustRover workflow, JVM test results, and measurements on the pinned pilots.

Use [lambdaisland/uri at `e439c2ca`](https://github.com/lambdaisland/uri/tree/e439c2ca068849df620e1c9275466cb1e8171aaa). It has a small source tree, a `deps.edn` test alias, `.cljc` reader conditionals, a URI record and a JVM-specific namespace for encoding operations. Keep the pinned checkout in a temporary test workspace. Parenox's Cargo dependency graph stays independent of the pilot. [Project configuration](https://github.com/lambdaisland/uri/blob/e439c2ca068849df620e1c9275466cb1e8171aaa/deps.edn), [Upstream tests](https://github.com/lambdaisland/uri/blob/e439c2ca068849df620e1c9275466cb1e8171aaa/test/lambdaisland/uri_test.cljc)

1. Open the project with the `:test` alias and resolve `normalize/percent-decode` from `lambdaisland.uri` to its definition in `lambdaisland.uri.normalize`.
2. Misspell that var in an unsaved editor buffer, verify the diagnostic and its disappearance after a correction, and compare the result with a clean CLI check of the same text.
3. Run the upstream JVM test suite. Change one expectation in a temporary copy and verify that the failure points to the correct test and source line.

The first editor trial used Neovim 0.12.5 with its built-in LSP client. The call in `src/lambdaisland/uri.cljc` at line 134 resolves to `src/lambdaisland/uri/normalize.cljc` at line 82. Hover identifies the qualified var, and an unfinished form appended in the editor produces a reader diagnostic while the earlier reference still resolves. The project has no checked contract for this var, so the hover reports unavailable type information.

### Portable runtime trial

The [portable test namespace](../../tests/fixtures/jvm-project/test/demo/portable_test.cljc)
ran on Clojure/JVM `1.13.0-alpha11` and clojurust `793ad97e`. Each runtime passed
three tests and seven assertions, including its reader branch, present nil
values versus absent keys, Clojure truthiness, and dereferencing an updated
map. Run the comparison with `just portable-runtime-smoke /path/to/cljrs`.

On macOS 26.5 arm64 and Java 21.0.2, ten fresh processes per runtime with
warmed dependency caches had median times of 631 ms for `parenox test` on the
JVM and 15.2 ms for `cljrs test`. The clojurust release build took 3m 08s and
produced a 41.2 MB executable. These timings cover this fixture's test
execution and runtime startup. [Pinned clojurust source](https://github.com/csm/clojurust/tree/793ad97e57cc797878a09c66771d651cfab1ee9b)

The expanded corpus adds lexical bindings and closures, `loop/recur`,
sequences, immutable collections, namespace loading, macros, dynamic
vars and exception data. `just native-run-smoke /path/to/cljrs` compares
the printed value and output of a portable script, checks command-line
arguments and reader branches, and verifies an exception's exit status.
`just portable-runtime-smoke /path/to/cljrs` now passes five tests and
ten assertions on each runtime.

`just native-run-smoke /path/to/cljrs` exercises the same files on the
pinned JVM and Clojurust. Repeated `require` loads the namespace once,
and `:as` and `:refer` resolve its public var. A cross-namespace macro
resolves its helper at its definition and evaluates the argument once.
Nested dynamic bindings are restored across `catch` and `finally`.
Both runtimes preserve exception data and write uncaught exception
diagnostics to stderr with exit status 1. Binding `*out*` to `*err*`
redirects `println` only on the JVM.

The mixed `clojure.test` fixture produced four tests, four assertions,
one pass, one failure, two errors, and exit status 1 on both runtimes.
Parenox's JVM runner sends the JSON report with source locations to
stdout and the test log to stderr. The pinned native runner sends its
human-readable counts, test output and exception data to stdout and
leaves stderr empty.

The [core.async fixture](../../tests/fixtures/portable-runtime/core-async/channel.cljc)
uses an unbuffered channel. Both runtimes let `go` transfer the value
and close the channel, and the consumer prints `[{:answer 42} nil]`.
Stdout matches byte for byte, stderr is empty, and both exit 0. The
[timeout fixture](../../tests/fixtures/portable-runtime/core-async/timeout.cljc)
adds `alts!!` with a `timeout` port. On JVM core.async 1.8.741 it has the
same outcome. Native exits 1 with empty stdout because `timeout`
returns a future and `alts!!` expects channel ports. Keep the backend's
existing scheduler for the covered exchange. The
[timeout port fixture](../../tests/fixtures/portable-runtime/core-async/timeout-port.cljc)
uses the JVM port directly. Its clojurust branch waits for the built-in
future in `go` and closes an ordinary channel. Both runtimes pass the
same data-win, timer-win, repeated-read and losing-take assertions.
Code that needs a portable timeout port can make this conversion at the
call site and keep clojurust's future API available elsewhere.
[Pinned native async built-ins](https://github.com/csm/clojurust/blob/793ad97e57cc797878a09c66771d651cfab1ee9b/crates/cljrs-async/src/builtins.rs)

Parenox ran three Medley 1.9.0 calls on both backends from a temporary
project with untouched upstream `src` and the pinned JVM dependency.
`find-first`, `dissoc-in` and `update-existing` printed
`[4 {:version 1} {:count 5}]` after the same log line and both exited 0.
The native run used an empty `PATH` and no Java environment variables.
Medley's full upstream suite passed 53 tests and 280 assertions on the
JVM. The native suite stopped on `unbound symbol: deftest` during loading
because the test namespace imports `deftest` in `:clj` and `:cljs`
branches. [Pinned Medley source and tests](https://github.com/weavejester/medley/tree/54f45abc16ecd30121d05ef52014af8c65d4635f)

| Capability | JVM | Clojurust |
| --- | --- | --- |
| Script and namespace loading | Clojure CLI classpath and aliases | `.cljc` or `.cljrs` with explicit source roots |
| Reader conditionals | `:clj` | `:cljrs` and legacy `:rust` |
| Script arguments | `*command-line-args*` | Arguments to an optional `-main` |
| Dynamic vars and output | Binding `*out*` to `*err*` redirects `println` | Custom dynamic vars passed the fixture. The `*out*` probe still printed to stdout. |
| Java interop and JVM dependencies | Project JVM and dependencies | Use the JVM backend for these projects |

The engine comparison ran the same Medley script in ten fresh processes
per runtime after one warm-up on macOS arm64, with dependencies cached.
The standalone embedding probe used `Runtime::builder` in tree-walk
mode, installed `cljrs-stdlib`, and evaluated forms with the clojurust
reader. It matched the portable script and Medley output and passed the
five portable tests. [Clojurust runtime builder](https://github.com/csm/clojurust/blob/793ad97e57cc797878a09c66771d651cfab1ee9b/crates/cljrs-runtime/src/runtime.rs)

| Option | Median fresh launch | Release executable | Coverage and upkeep |
| --- | ---: | ---: | --- |
| Project JVM via Parenox | 861 ms | 7.6 MB Parenox plus project JVM | Clojure 1.13 reference and JVM libraries |
| External clojurust via Parenox | 22.9 ms | 7.6 MB Parenox plus 41.2 MB cljrs | Verified portable corpus with a separately installed EPL-1.0 executable |
| Embedded clojurust probe | 11.0 ms | 7.25 MB standalone probe | Same measured portable cases and EPL-1.0 engine crates. The host owns runtime initialization and stream and process behavior. |
| Parenox Rust engine | Unmeasured | Unmeasured | Would require new executable values, environments, and a differential semantics suite |

Keep the external backend in `parenox-runtime` for the current release.
It covers the tested portable cases and keeps Clojure CLI compatibility
and native engine upgrades independent. The embedding measurement shows
the size and startup potential of a narrower host. Future adoption needs
a host API that can preserve streams, failure reporting and cancellation.

### Pinned URI editor measurements

On macOS 26.5 arm64 with Rust 1.100.0, the same URI checkout used five Clojure
files under `src` and `test`. Both builds located `percent-decode` and detected
and cleared the exact same unsaved typo in each pair of edits.

| Workload | Baseline `6125176` | Iteration `3b4f056` |
|---|---:|---:|
| Fresh CLI check median, 50 processes | 3.20 ms | 3.36 ms |
| Fresh LSP initialize median, 10 subsequent processes | 2.12 ms | 2.20 ms |
| Warm edit and diagnostic p95, 200 edits | 0.415 ms | 0.420 ms |
| Warm hover p95 | 32 µs | 29 µs |
| Warm definition p95 | 37 µs | 35 µs |
| LSP resident memory after edits | 5.80 MiB | 5.81 MiB |
| Release binary | 4,661,024 bytes | 4,762,144 bytes |

The first interactive startup also showed a 311 ms sample on the iteration
build. The subsequent launches used warmed filesystem caches. The warm
timings cover the stdio round trip. Run `just uri-editor-bench /path/to/uri`
and pass a second argument to compare a separately built binary.

### Performance and correctness

- Run `just agent-loop` for regressions in the existing incremental path. Verify that warm diagnostics and definition targets match a clean session after source, alias and dependency changes. Check that CLI and editor diagnostics agree on the same revision and that stale editor results are discarded.
- Measure end-to-end cold startup, peak and idle RSS, idle CPU, release binary size and p95 editor latency on fixed inputs. Include an unsaved edit with a diagnostic, hover, navigation and their protocol overhead. Measure explicit runtime invocations with their child processes included. Compare the equivalent editor and CI workflows on the same hardware and report diagnostic accuracy and unsupported features alongside performance.
- Use one analysis process per workspace and block on incoming requests while idle. Keep warm edits within 50 ms p95 on the pinned pilot projects as an initial target. Set a peak RSS budget once the two project baselines are recorded and preserve the clinic baseline with each added feature.

## Two checker iterations on project source

### Contracts, comprehensions and String members

- `s/fdef` checks each fixed `s/alt` branch against the corresponding
  `defn` body and callers in other namespaces. The JVM spec instrumentation
  accepts the zero-, one- and two-argument branches of the same contract and
  rejects incorrect argument types. Same-arity alternatives and nested regex
  arities are not modeled. Callback `s/fspec` still uses `s/cat`.
- [Crabjure at `a1a1f322`](https://github.com/ribelo/crabjure/blob/a1a1f3224d77f9e5d89de4ababf1aa2445e5ade1/crates/crabjure-resolver/src/visitor.rs)
  gathers all `for-like` binding names to avoid unresolved-name findings.
  Parenox binds sequentially instead: in `[x ["ab"] x x]`, the second source
  sees the previous `x`, and the body sees a character. Tests adapt the map-entry
  iteration in the pilot's `feature/command.clj` and URI traversal in
  `feature/file_management.clj`. JVM checks distinguish the first source
  expression (evaluated immediately) from the lazy `for` body. Deferred
  exception classes are not tracked individually.
- On the JVM, `(.indexOf "😀." ".")` returns 2, and a String-hinted nil receiver
  still throws. The checker covers `startsWith(String)`, `endsWith(String)`
  and `indexOf(String)`. Type hints select the class without proving non-nil.
  Code-point and offset overloads report incomplete coverage.

### Dependency source and PostgreSQL corpus

The pinned Clojure LSP pilot (`1e93ac58bede574786c5cc75b0fd3a51bc224f7a`)
uses Maven artifact `com.github.clojure-lsp/lsp4clj 2.0.1`. The root
`--deps` run follows local source roots, not Maven artifacts, and cannot take
extra positional roots. The actual
[`:lsp4clj.coercer/range` declaration](https://github.com/clojure-lsp/lsp4clj/blob/1ff6c687f4e2af214d30df0d50749ea4a15b42c8/src/lsp4clj/coercer.clj#L31-L37)
is in `coercer.clj`. In a separate `parenox.edn`, set
`:dependency-sources` to that file from a v2.0.1 checkout and run
`parenox check --config /path/to/parenox.edn --deps playground/clojure-lsp/deps.edn --alias test`.
Both missing-`range` findings disappear. A checked call with no `:start`
now gets a missing-key error. With both keys present, the scalar
`(s/and integer? (s/conformer int))` checks remain incomplete. The JVM
accepts the valid sample but throws for `2147483648` as a line number.

Crabjure's resolver keeps source provenance and fetches matched namespaces
on demand. Parenox keeps explicit inputs and uses the existing Session:
editing or removing a dependency's spec rechecks its consumers, and
a dependent call's `s/fdef` error names the source contract file. There is
no artifact download, JAR index, or spec execution.

[Typespec HEAD `dfc58974f6ae1f6e4e90c2ad9686fd2ddce91f21`](https://github.com/HealthSamurai/typespec/commit/dfc58974f6ae1f6e4e90c2ad9686fd2ddce91f21)
was unchanged from the starting revision. Its pg tests exercise loop
fixed points, source contracts and use-site Java hints. Both checkers
accepted a numeric index loop adapted from `pg/fetch` and detected a
string back edge. The new [context-web-pg corpus](../../examples/context-web-pg)
compares the same source and the same EDN boundary without loading HTTP or
PostgreSQL. On the `reduce-separated-old` pattern from
[`context-clj/pg` at `aa8ea0eb`](https://github.com/context-clj/pg/blob/aa8ea0eb565e7f70f80f3248e6a83198628e1047/src/dsql/core.clj),
Parenox previously rejected `[field & more]` and did not detect
`(inc field)` as a type error. Both now accept the positive loop and
locate the same String/Number error in the negative loop. At that
checkpoint, Typespec narrowed an open map on `(s/valid? ::row row)`;
Parenox still reported the call as unmodeled and flagged the resulting
return type.

### Runtime and editor comparison

[cljrs HEAD `793ad97e57cc797878a09c66771d651cfab1ee9b`](https://github.com/csm/clojurust/tree/793ad97e57cc797878a09c66771d651cfab1ee9b)
was unchanged. `just portable-runtime-smoke /path/to/cljrs` passed three
tests and seven assertions per runtime: the same `.cljc` selects `:clj` on
Clojure/JVM `1.13.0-alpha11` and `:cljrs` on cljrs. Parenox checks the JVM
branch, not Java interop semantics on cljrs.

With JVM [nREPL 1.5.2](https://github.com/nrepl/nrepl/tree/7d67ef71e6957ecaa941b006f966053909de85c9)
and cljrs nREPL, both the reference client and Parenox exercised
`describe`, `clone`, `eval` and `close` on explicit loopback connections.
State survived two evals (value 42); a thrown exception produced
`eval-error` and then `done`. Both confirmed closure. In the same
`(binding [*out* *err*] (println "err"))` scenario, JVM nREPL sent `err`,
whereas cljrs sent `out`; cljrs omitted `root-ex`. Parenox preserves
the server's fields. A local test server covers timeout, mid-request
disconnect and an absent advertised operation. Timeouts do not cancel
evaluation on the server.

Both language servers handled the same unsaved extra `)` after
`(def first-value "😀")`: UTF-16 range `1:23–1:24` (zero-based). Both
cleared the error and showed the renamed declaration after the next
unsaved edit. cljrs keeps outline entries after a stray delimiter and
lists both reader branches; Parenox keeps completed declarations before
the error and shows the selected JVM branch. cljrs requests full-content
sync; Parenox accepts full-content and range changes. CLI/LSP regression
tests check matching argument findings from wave one and from an unsaved
dependency spec edit in wave two. The next editor task is jumping from a
spec reference to its source `s/def`.

### Pilot and clinic measurements

On macOS 26.7.1 arm64 with Rust 1.99.1, the unmodified pilot produced
the same diagnostics before and after both waves: 129 source files,
one error-severity missing-spec finding and 2,223 warnings. None was a
verified program error. The default pilot has no checked function bodies;
it measures source analysis, not body-check coverage. With the explicit
`lsp4clj` source, the two missing-range findings are gone. The 6 errors
and 2,337 warnings in that run include the additional source file;
all 6 errors are in that file, not the original checkout.

| Workload | Baseline | Wave 1 | Wave 2 |
|---|---:|---:|---:|
| Fresh pilot CLI median, 15 processes | 57.70 ms | 58.41 ms | 58.13 ms |
| Pilot analyzer median in those processes | 44.47 ms | 44.89 ms | 44.75 ms |
| Clinic warm round trip p95, 100 iterations | 67.7 µs | 74.4 µs | 67.7 µs |
| Clinic body edit p95, 100 iterations | 112.2 µs | 132.8 µs | 118.3 µs |

The CLI measurements alternate release binaries on the same pilot, with
two warm-up runs per binary excluded. `just agent-loop` found and fixed the
same three seeded clinic errors at each checkpoint. `just verify` passed
with both waves integrated, including the loopback nREPL tests.

## Follow-up: use named specs in code and navigate to their source

The [context-web-pg corpus](../../examples/context-web-pg) now passes
completely. Its negative files still report two type errors. In
`checked-row`, only the successful branch of `s/valid?` gets the row's
required keys and field types. There is no runtime call during analysis
and no false-branch complement. The pinned JVM can reject a map with a
correct `:title` if an extra qualified key fails its registered spec.
It also accepts a three-argument `s/valid?`; that argument is evaluated
but doesn't change lookup of a literal spec keyword.

The JVM uses the last repeated `s/keys` option and resolves colliding
unqualified fields by its own precedence. Parenox reports these cases
instead of using an ambiguous shape to narrow a value. Unqualifying
`:wire/nested/title` preserves `:nested/title`. An edit or removal of a
spec used only in a function body, including through a private helper,
invalidates the cached caller without rechecking unrelated functions.

Go to Definition resolves spec references in supported type forms and
direct `s/valid?` calls to the source key in `s/def`. Tests open a
reference in a portable file, jump to a closed file, shift the destination
in an unsaved buffer, then remove the declaration. Duplicate declarations
have no target.

`just verify` and `just agent-loop` passed. The raw Clojure LSP pilot
kept identical diagnostics (one missing-spec error and 2,223 warnings).
Alternating 15 fresh release processes per binary, median pilot CLI time
was 60.40 ms before and 61.06 ms after this follow-up. In nine alternating
clinic runs per binary, median warm p95 was 47.0 → 45.7 µs and body-edit
p95 was 71.1 → 77.3 µs. The clinic timings varied between runs.

The next precision case at that checkpoint was upstream `lsp4clj.coercer` 2.0.1:
`(s/and integer? (s/conformer int))` for the fields in `::range`.
Compare a normal integer, a string and an integer outside the Java int
range without conflating input validity with the conformed result.

## Checked JVM integer conformer

`lsp4clj` v2.0.1 uses the exact built-in pattern for `::line`,
`::character` and `::version`. A `Number` is not proof of `integer?`,
and an integer is not necessarily in the Java `int` range. The checker keeps
the predicate domain, the input on successful validation and the conformed
result separate. The successful input and result share the `Int32` value
range, but the checker does not equate their JVM classes.

Reproduce the scalar and nested-range cases on the Clojure/JVM version
pinned in `deps.edn`:

```sh
clojure -M tests/fixtures/spec-int-conformer.clj
```

| Input | `s/valid?` | `s/conform` |
|---|---|---|
| `42`, `42N`, or `84/2` | true | `java.lang.Integer` with value 42 |
| `"42"`, `42.0`, or `85/2` | false | `:clojure.spec.alpha/invalid` |
| `2147483648` or `-2147483649` | `ArithmeticException` | `ArithmeticException` |
| `9223372036854775808N` | `IllegalArgumentException` | `IllegalArgumentException` |

The fixture also checks both inclusive `int` bounds. Parenox models
these outcomes for direct scalar spec calls and rejects strings and known
overflow in typed `::range` inputs. It does not model the transformed
result of `s/conform` on the whole map, custom conformers, or dynamic spec
names. Very large ratio literals whose divisibility is unknown remain
unproven. [Spec API](https://clojure.github.io/spec.alpha/clojure.spec.alpha-api.html#var-conform)

The unmodified Clojure LSP pilot kept identical diagnostics: one
missing-spec error and 2,223 warnings. The run with the real v2.0.1
`coercer.clj` loaded through `:dependency-sources` went from 6 errors
and 2,337 warnings to 5 errors and 2,329 warnings. All nine removals
are in the dependency: `::line`, `::character`, `::version`, two other
uses of `integer?`, and the `::severity` spec. There were no new
diagnostics. A separate caller using that same source now passes an
ordinary `::range` and gets two type mismatches for a string and for
overflow at `:start :line`, instead of four unproven checks.

In 15 alternating fresh processes per release binary, median pilot CLI
time was 52.82 ms before and 52.35 ms after. The clinic edit scenario
still found and fixed all three planted errors. The checkout was not
modified.

## Fixed-precision integer predicate

The fixture in `tests/fixtures/spec-int-conformer.clj` checks the
[`:error/code` declaration in lsp4clj 2.0.1](https://github.com/clojure-lsp/lsp4clj/blob/1ff6c687f4e2af214d30df0d50749ea4a15b42c8/src/lsp4clj/coercer.clj#L17-L18)
on the Clojure/JVM version pinned in `deps.edn` (`1.13.0-alpha11`).

| Value | JVM class | `integer?` | `int?` | `s/conform :error/code` |
|---|---|---|---|---|
| `2147483648` | `Long` | true | true | `2147483648` |
| `1N` | `clojure.lang.BigInt` | true | false | invalid |
| `(biginteger 1)` | `java.math.BigInteger` | true | false | invalid |
| `:failed` | `clojure.lang.Keyword` | false | false | `:failed` |
| `"failed"` | `String` | false | false | invalid |
| `1/2` | `clojure.lang.Ratio` | false | false | invalid |

The checker tracks the fixed-precision class and numeric range separately.
For example, the reader reduces `2/2` to a `Long`, while
`9223372036854775808/9223372036854775808` produces a `BigInt` with the
same numeric value. A successful `(s/conform ::line 1N)` produces an
`Integer` that satisfies `int?`. A successful `s/valid?` check of
`::line` on its own proves the input range and preserves any existing
class evidence. A shadowed `int?` is checked as an ordinary call and
provides no built-in narrowing.

### Pilot results

Baseline `fd2410f` and the updated build use the same pinned Clojure LSP
checkout. The second run adds the upstream `coercer.clj` through
`:dependency-sources` in a temporary `parenox.edn`.

| Run | `PNX9001` errors | `PNX9001` warnings | `PNX9002` warnings | Checker time (ms) | Median CLI time (ms) |
|---|---:|---:|---:|---:|---:|
| `just pilot-check json` | 1 → 1 | 864 → 864 | 1,359 → 1,359 | 53.52 → 53.26 | 65.71 → 65.86 |
| Pilot with `coercer.clj` | 5 → 4 | 970 → 969 | 1,359 → 1,359 | 56.46 → 54.27 | 66.65 → 66.95 |

Checker time comes from the JSON reports. The median CLI time includes
process startup and JSON output over 15 alternating fresh processes per
release binary on an Apple M1. Both pilots still exit 1 for remaining errors.
The dependency run removes the `Unsupported type int?` error in
`:error/code` and the custom-predicate warning on the `int?` stage of
`:file-event/type`.

- **Confirmed type errors.** A separate caller using the same dependency
  source accepted `:failed`, `2147483648` and `(s/conform ::wire/line 1N)`.
  Passing `1N`, `"failed"` or `1/2` to the typed function produced a
  `PNX2322` error at each argument.
- **Incomplete checks.** Passing a general `integer?` argument produced
  `PNX9004` (error severity) because its class could not be verified.
  The enum map predicate and conformer stages of `:file-event/type` still
  report `PNX9001`. The input shape of `:error/code` is supported.
  Direct `s/conform` on that spec and arbitrary conformers remain unchecked.

`just verify` and the pinned JVM fixture passed. `just agent-loop` found
and fixed all three deliberate clinic errors. In the 100 warm requests,
median round-trip time was 33.3 → 25.9 µs and p95 was 119.6 → 41.0 µs
in the sampled runs.

### Workspace build measurements

With Rust 1.100.0 on the same machine, a clean locked release build took
31.28 → 38.46 s across the combined feature and crate split. The baseline
also took 36.29 s in a second source directory. After touching only
`src/output.rs`, a warm release rebuild took 21.25 → 5.36 s. Cargo
rebuilt the root package alone after the split. The release binary grew
from 6,140,176 to 6,175,456 bytes. `cargo tree --locked -e normal,build`
showed the same third-party packages.

## Literal enum map conformer

[`:file-event/type` in lsp4clj 2.0.1](https://github.com/clojure-lsp/lsp4clj/blob/1ff6c687f4e2af214d30df0d50749ea4a15b42c8/src/lsp4clj/coercer.clj#L9-L13)
uses `int?` followed by two lookups in the same map. The predicate lookup
checks truthiness. The conformer returns the corresponding keyword.
On the pinned JVM, the map alone finds `:created` for both `1` and `1N`,
while the spec rejects `1N` at `int?`. Missing keys, false, nil and
`:clojure.spec.alpha/invalid` map results all fail conformance.

Parenox proves the input set `1 | 2 | 3`, narrows the true branch of
`s/valid?` to possible input keys, and infers the corresponding result of
a direct `s/conform` on the named spec or its alias. The project cache
rechecks consumers if an enum key or result changes, including a result
edit that leaves the accepted keys and source spans unchanged. Decimal
integer keys also work in ordinary literal maps and direct lookups.

The pattern requires a unique, earlier literal map in the same file,
with at most 32 decimal keys in the signed 32-bit range. The reader may
prove an integer's class and range without proving its exact value for a
radix or ratio spelling. Such a value remains unproven at a typed enum
call. Computed maps, different predicate and conformer maps, direct
conformation of a whole event map, and export of finite integer domains
remain outside this slice.

| Pilot | Errors | `PNX9001` warnings | `PNX9002` warnings | Median CLI time (ms) |
|---|---:|---:|---:|---:|
| `just pilot-check json` | 1 → 1 | 864 → 864 | 1,359 → 1,359 | 66.10 → 61.97 |
| With upstream `coercer.clj` | 4 → 4 | 969 → 964 | 1,359 → 1,359 | 68.43 → 64.90 |

The median covers 15 alternating fresh processes per release binary on
an Apple M1. The ordinary pilot kept the same codes, severities and source
spans. The dependency run removed three warnings on the literal map's
integer keys and two on the enum stages of `:file-event/type`.

A typed caller using the upstream `::file-event` accepts codes 1, 2 and 3.
It reports `PNX2322` at inputs with code 4, `1N` or `"1"`, and
`PNX9004` when only `int?` is known for the code. A direct scalar
`(s/conform :file-event/type 1)` satisfies a `:created` result contract.

`just verify` passed 643 tests, and both conformer fixtures passed on
the pinned JVM. `just agent-loop` fixed all three deliberate clinic
errors. Across five alternating clinic runs of 100 warm requests each,
the median of the run p50 values was 26.3 → 27.7 µs, and the median of the
run p95 values was 35.0 → 39.0 µs.

### Keyword enum tables

The same literal-table pattern accepts `keyword?` keys and produces fixed
integers. On Clojure 1.13.0-alpha11, the 25 completion item kinds and two
insert-text formats in lsp4clj 2.0.1 return the `Long` values in their
tables. Accepted keys include exact qualified entries and entries mapped
to zero. False, nil, and the invalid marker remove entries from the domain.
The invalid marker is also excluded as an input key. The checker exports
the finite keyword domain for consumers using externs without source.

The configured pilot went from 2,309 to 2,293 warnings with zero errors.
Twelve enum-stage warnings disappeared from completion, symbol, code
action, inlay hint, and test tree specs. Four dependent collection-shape
warnings disappeared. Computed inverse tables and wider numeric outputs
remain analysis gaps. [JVM fixture](../../tests/fixtures/spec-keyword-enum.clj)

## Signature help and LSP data contracts

Parenox and Clojure LSP 2026.05.05-12.58.26 returned identical signature
help for the call to `get-active-parameter-index` in a temporary copy of
the pinned project's `signature_help.clj`. The integration test covers
overloads, variadic arguments, aliases, lexical shadowing and an unsaved
call missing its closing delimiters. Core built-ins and externs without
source declarations remain outside this slice. [Clojure LSP signature help](https://clojure-lsp.io/features/#signature-help)

The dependency pilot removed the `:kind map?` and repeated `:message`
errors, leaving two errors and 2,322 warnings. A typed caller using the
real specs rejects a keyword key in `::changes` and a missing `:message`
in `::diagnostic`. The upstream `::edits` element collection still lacks
`:kind`, and the upstream `::message` value spec is absent.

Across 15 alternating release runs, median CLI time was 61.76 → 65.33 ms
for the ordinary pilot and 63.37 → 66.98 ms with dependency source.
The clinic scenario fixed all three planted errors. The median warm p95
across five clinic runs was 42.3 → 40.8 µs. `just verify` passed 647
tests and `just pilot-navigation-smoke` passed.

## Range formatting and the identity predicate

On a misindented line in a temporary copy of the pinned
`signature_help.clj`, applying either server's range edits produced the
same file. Clojure LSP returned an edit for the enclosing form. Parenox
returned one insertion on the selected line. A sweep of 744 selections
across 248 pilot files preserved unselected bytes and was idempotent.

The pinned JVM fixture confirms that `identity` accepts `0`, empty
strings and empty collections in spec, while nil, false and the spec
invalid marker fail. The dependency pilot removed `Unsupported type
identity` and retains the `any?` error. The ordinary pilot kept identical
diagnostics. The source `:error` arm of `::any-or-error` still has an
unchecked conformer, so a typed caller passing false reports `PNX9004`.

Median CLI time over 15 alternating runs was 64.92 → 65.03 ms on the
ordinary pilot and 66.94 → 66.62 ms with dependency source. Clinic warm
p95 was 37.8 → 39.2 µs across five runs. `just verify` passed 653 tests,
and the clinic scenario fixed all three deliberate errors.

## The universal spec predicate

The Clojure 1.13 fixture accepts nil, false and ordinary values for
`any?`. Spec conformance rejects the reserved `::s/invalid` marker.
Parenox tracks this distinction in named specs, checked arguments and
collections with an explicit `:kind`. The pinned Clojure LSP pilot
with `lsp4clj` went from one error to zero.

Parenox also checks `(s/coll-of any?)` without `:kind`. The accepted
domain includes queues, lists, vectors, sets and maps. Map entries count
as elements, and the reserved invalid marker is excluded only at the
element boundary. Conformance can rebuild sequence-like inputs as lists.
The dependency pilot lost one warning for its command arguments spec.
The ordinary pilot's 2,224 findings matched the preceding build.

For lsp4clj's `::locations`, the checker now preserves the nested location
map as the collection's element type, including through externs. The
input check distinguishes empty maps, nonempty maps that yield entries,
and entry values whose key and value each satisfy the location shape.
A successful `s/valid?` exposes that shape to sequence operations such
as `first`. The [pinned JVM fixture](../../tests/fixtures/spec-location-collection.clj)
also covers invalid values, range overflow and conformance that rebuilds
a map entry as a plain vector.
With the lsp4clj source loaded, the pinned pilot kept zero errors and
reported 2,312 warnings, down from 2,321. Warnings for unkinded collection
specs fell from 28 to 19.

Nested `s/conform` now handles known vectors of closed maps with
unqualified keys. It resolves each required child spec, distinguishes
`::s/invalid` from a thrown overflow and stops after a failing vector
element. Editing a nested conformer invalidates callers even if their
input type is unchanged. Open or registry-sensitive maps and nonempty
collections outside the modeled result shapes still report an analysis gap.
The pinned pilot kept its 2,312 warnings and zero errors.
In ten alternating release CLI runs of that pilot, median time moved
from 113.6 ms to 118.7 ms. Timing ranges overlapped from 96.4 ms to
143.2 ms, so this run does not establish the size of a change.

For a location map entry, equal values can still yield different JVM
objects. The checker preserves a `MapEntry` when child conformers
retain both objects and yields a vector when a child changes, as in
`BigInt` to JVM `Integer`. The JVM fixture verifies identical reuse
with cached boxed integers at -128 and 127, and tests 128 against the
local VM's cache behavior. When identity cannot be proven, Parenox
reports `PNX9001`. The quoted-map CLI example now checks completely.
The pinned pilot still had 2,312 warnings and zero errors. In ten
alternating release runs across the combined snapshots, the median
was 70.4 ms before the change and 66.1 ms after it.

### Text edit collections

Lsp4clj 2.0.1 defines `::text-edit` with required `:new-text` and
`:range` fields. Its three collection specs use `s/coll-of` without
`:kind`. The [JVM fixture](../../tests/fixtures/spec-text-edit.clj)
confirms that conforming lists and sequences produces a persistent list
in encounter order. Nested positions become JVM `Integer` values, and
an invalid element stops conformance before any later overflow.

Parenox now checks this reconstruction for lists and sequences of known
length and preserves provably empty sets and vectors. Nonempty sets,
unbounded collection shapes and open or registry-sensitive map shapes
still produce a conformance gap. The CLI regression exports and validates
the required fields and the return contract for a conformed list. It
then deletes the provider source, checks the imported return as a list
and rejects six invalid inputs with diagnostics pointing to the externs.
For a small direct hash set literal, the checker tracks each input
member before joining their types. Conformance yields a set of converted
edits. Traversal order is unspecified by the static model. When one
member rejects and another overflows, either result is possible. The JVM
fixture reproduces both outcomes by varying a map field that affects its
hash. The same fixture shows how reinsertion into a sorted set can invoke
its comparator with converted values. General `set?` inputs keep a gap.

The pinned pilot's diagnostics matched before and after the change. With
the lsp4clj source configured, 130 files produced 2,312 warnings and zero
errors.

### Folding range collections

The `::folding-ranges` input is a collection of maps with required
`:start-line` and `:end-line` and a modeled optional `:kind` predicate.
Lsp4clj 2.0.1 leaves five child value specs unregistered. The checker
retains presence constraints and modeled fields inside the collection,
including on the successful side of `s/valid?`. It reports the missing
value specs individually. Adding the registration in another source file
updates the checked callers. The [JVM fixture](../../tests/fixtures/spec-folding-ranges.clj)
also covers late conversion of a coordinate and the reserved invalid
marker.

With the pinned lsp4clj source configured, the pilot reported 2,309
warnings and zero errors. Three unkinded collection warnings were
removed at lines 174, 356 and 418 of `coercer.clj`.

A named spec backed by core `keyword?` rejects
`:clojure.spec.alpha/invalid`. Parenox models that domain through
`s/valid?`, `s/conform`, aliases, and both orders of `s/and` with `any?`.
In a folding range input the known optional `:kind` field rejects the
sentinel when present. Ordinary keyword predicates and map annotations
continue to accept it. The narrower type round-trips through externs
as an `s/and` of `keyword?` and `any?`. The pinned pilot's diagnostics
matched before and after this refinement.

`s/conform` models closed keyword maps with known presence, including
qualified fields declared by `:req` and `:opt` and unqualified fields
declared by `:req-un` and `:opt-un`. It checks required keys first,
leaves absent optional keys out of the result and uses the registered
child conversion for present keys.
Fields with missing registrations retain their values and the checker
reports the missing registration. A new registration rechecks consumers
that depend on the enclosing spec. Known ordered collections, small hash
set literals and two-element map entries use the same child conversions.
An extra qualified key uses a self-contained registered conversion from
the same file or a namespace loaded through an explicit `ns` `:require`
or `:use`. This can rebuild a map entry when conversion
changes the identity of one of its values. The cache tracks registration
origin and conversion, so a consumer is checked again after the registration
moves or its output changes. Unavailable and unsupported registrations,
unproven loads, and open input shapes report analysis gaps. Aliases and
composite extra specs need load evidence for their child registrations.
[JVM optional-key fixture](../../tests/fixtures/spec-optional-keys.clj),
[JVM qualified-key fixture](../../tests/fixtures/spec-qualified-keys.clj),
[JVM extra-key fixture](../../tests/fixtures/spec-extra-registry.clj)

For lsp4clj's `:error/code`, `(s/conformer second)` unwraps the tag
from a choice of core `keyword?` and `int?` predicates. A successful
conversion preserves the original object. The checker models the fixed
integer or non-sentinel keyword input, the conformed value and the
sentinel on failure. It retains identity when that conversion runs on a
map field and supplies the input domain in source-free externs. Runtime
checks require evidence that the spec and the registrations reached
through its aliases or fields are loaded. A direct `:require` or `:use`
provides that evidence. The same source file also provides it.
[JVM error-code fixture](../../tests/fixtures/spec-error-code.clj)

The two inverse completion-item enums in lsp4clj use `integer?` and
`clojure.set/map-invert`. Fixed and arbitrary-precision classes with the
same integer value match the same code. Exact values survive a checked
`int` conversion, including when it is nested inside another conformer.
Parenox proves a bounded inversion from earlier non-dynamic literal vars
with unique codes. It checks registry load evidence, models the inverse
map as literal data and exports a standalone `(s/and integer? #{...})`
input contract. The fixture checks representation equality, a nested
map, `MapEntry` reconstruction and the marker as ordinary map data.
[JVM inverse enum fixture](../../tests/fixtures/spec-inverse-enum.clj)

The lsp4clj `:markup/kind` string set accepts `"plaintext"` and
`"markdown"`. `s/conform` keeps the original string object. A general
`String` can conform to either literal or produce the invalid marker.
When `s/valid?` succeeds, the original value narrows to the accepted
strings. `::markup-content` uses that conversion for its `:kind` field
and preserves a `MapEntry` when its fields preserve identity. The finite
input domain round-trips through externs without the provider source.
[JVM markup fixture](../../tests/fixtures/spec-markup-content.clj)

For lsp4clj's `::documentation`, `s/or` chooses a string or a named
map spec and `(s/conformer second)` projects the conformed value.
Parenox models the two branches in order, including the invalid marker
and a field conformer's exception. It checks that the named
registrations are loaded and that source declarations cannot force a
forward branch before registration. A failed resolution on the JVM can
persist after a later registration. Early registry aliases and
initializers whose effects are unknown leave a coverage diagnostic.
Externs preserve the accepted string and map input shapes.
[JVM documentation fixture](../../tests/fixtures/spec-documentation.clj)

The inline map branch of lsp4clj's `::marked-string` requires
`:language` and `:value`. Without registered field specs, the keys
only need to be present. The checker retains the field coverage
findings and models conformance for a known input value. Adding a
registration can change an unchanged caller's behavior and triggers
rechecking. Registered inline fields export their constraints as part
of the string-or-map input contract.
[JVM marked-string fixture](../../tests/fixtures/spec-marked-string.clj)

The collection branch of `::contents` accepts strings and marked maps.
It accepts an empty map and rejects nonempty maps because their members
are map entries. A nonempty markup map can then conform through the
markup branch. Ordered conformance stops at the first invalid element
or exception. A literal hash set can be checked without assuming its
iteration order. The checker reconstructs a vector when conformance
changes either member of a `MapEntry`. A successful validation of a
broad map allows the empty case as well as the markup shape.
[JVM contents fixture](../../tests/fixtures/spec-contents.clj)
