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

- Keep the `parenox` package as the public library and binary entry point. Use modules for boundaries that currently share the same consumers. A new workspace crate should enforce a dependency direction that matters to a real consumer or show a measurable build benefit.
- `parenox-frontend` owns source locations, forms, the reader and optional Rowan capture. Standalone builds use Rowan, with Serde available as an opt-in feature. Project sessions and checking live in `parenox`. The LSP adapter uses the existing session and JSON stack. Explicit runtime invocations can start as process adapters using the standard library.
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
branches. Whole-file indentation and spacing are available now; range
formatting, line wrapping, and richer macro indentation are not.

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
