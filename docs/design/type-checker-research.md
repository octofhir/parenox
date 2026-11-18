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
| December 20 | CI and JVM test workflow | Check the same project in CI and compare its public EDN contracts with a prior release. Map a failure from one explicitly invoked project test to its source. Audit the reader, macro and standard-library gaps found on the pilot projects. |
| December 31 | Tooling preview | Verify the editor and CI workflow on two pinned public projects. Run one portable fixture on JVM Clojure and clojurust and record the supported operations and any divergence. Publish the resource and correctness results with the preview. |

The first implementation slice is `deps.edn` source-root and alias support wired through the existing session, with one cross-namespace definition query that works on ordinary unannotated code. Then expose that query and the existing diagnostics through LSP. Regex literals, string declarations and JVM member checks enter the release backlog where they unblock a pilot. Formatting, completion and rename are follow-up consumers of the same source and resolution data.

### First pilot project

Use [lambdaisland/uri at `e439c2ca`](https://github.com/lambdaisland/uri/tree/e439c2ca068849df620e1c9275466cb1e8171aaa). It has a small source tree, a `deps.edn` test alias, `.cljc` reader conditionals, a URI record and a JVM-specific namespace for encoding operations. Keep the pinned checkout in a temporary test workspace. Parenox's Cargo dependency graph stays independent of the pilot. [Project configuration](https://github.com/lambdaisland/uri/blob/e439c2ca068849df620e1c9275466cb1e8171aaa/deps.edn), [Upstream tests](https://github.com/lambdaisland/uri/blob/e439c2ca068849df620e1c9275466cb1e8171aaa/test/lambdaisland/uri_test.cljc)

1. Open the project with the `:test` alias and resolve `normalize/percent-decode` from `lambdaisland.uri` to its definition in `lambdaisland.uri.normalize`.
2. Misspell that var in an unsaved editor buffer, verify the diagnostic and its disappearance after a correction, and compare the result with a clean CLI check of the same text.
3. Run the upstream JVM test suite. Change one expectation in a temporary copy and verify that the failure points to the correct test and source line.

The first editor trial used Neovim 0.12.5 with its built-in LSP client. The call in `src/lambdaisland/uri.cljc` at line 134 resolves to `src/lambdaisland/uri/normalize.cljc` at line 82. Hover identifies the qualified var, and an unfinished form appended in the editor produces a reader diagnostic while the earlier reference still resolves. The project has no checked contract for this var, so the hover reports unavailable type information.

### Performance and correctness

- Run `just agent-loop` for regressions in the existing incremental path. Verify that warm diagnostics and definition targets match a clean session after source, alias and dependency changes. Check that CLI and editor diagnostics agree on the same revision and that stale editor results are discarded.
- Measure end-to-end cold startup, peak and idle RSS, idle CPU, release binary size and p95 editor latency on fixed inputs. Include an unsaved edit with a diagnostic, hover, navigation and their protocol overhead. Measure explicit runtime invocations with their child processes included. Compare the equivalent editor and CI workflows on the same hardware and report diagnostic accuracy and unsupported features alongside performance.
- Use one analysis process per workspace and block on incoming requests while idle. Keep warm edits within 50 ms p95 on the pinned pilot projects as an initial target. Set a peak RSS budget once the two project baselines are recorded and preserve the clinic baseline with each added feature.
