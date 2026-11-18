# Type checker research for Parenox

Parenox checks Clojure source without loading namespaces or running project code. Keep the Rust reader, checker and incremental session; borrow rules that improve the guarantees we can give on existing Clojure.

## Current checker

| Area | What this means for Parenox |
|---|---|
| Contracts and externs | Parenox reads `s/def`, `:t/sig`, single-arity `s/fdef` and one EDN file with trusted library signatures. Externs cannot replace a modeled built-in. Versioned sidecars are not yet supported. |
| Nil, maps and guards | `get` distinguishes explicit nil from absence. Truthiness and `some?` disagree on false. Right-biased `merge` preserves required keys when an optional field can overwrite their value. |
| Compatibility | Structural checks distinguish a proven match, a mismatch and an unproven comparison. An unknown field cannot hide a known mismatch elsewhere. Recursive specs still report a gap. Function compatibility checks variance for matching arities. |
| Diagnostics | Parenox exposes spans, stable rule codes, occurrence IDs, the contract source and a `complete` flag. Occurrence IDs are stable for repeated checks of the same snapshot, not across source moves. Targeted suppression, trust baselines and coverage are not yet supported. |

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

Do not compare Crabjure's cold lint benchmarks against Parenox's warm type-check timings. Use the same corpus, separate cold and warm runs, and report coverage alongside speed. Keep Parenox's one crate and source-only default; JNI reflection, clj-kondo config compatibility and a new reader are not prerequisites.

## What to implement next

1. **Rule-specific suppression.** Apply it to one source form and rule code, without hiding unrelated diagnostics. Require a reason and distinguish a suppressed diagnostic from proven coverage.
2. **Stable paths and checked guards.** Add a tagged-map fixture that narrows the owner when a field is tested. A one-sided guard must not narrow the false branch; rebinding must invalidate facts attached to the old value.
3. **Contextual callbacks.** Check an inline callback against an expected contract and cover a wrong return type. Measure the edit/check cost before expanding this to generic constraints and user-defined higher-order functions.
4. **Macro scope descriptions.** Compare a known binding macro with its hand-written equivalent, including shadowing. Without a reviewed body rule, resolve its locals but report that its result and effects were not checked.

Keep JVM contracts, recursive types, general rows, and latent effect summaries as separate slices. Do not add runtime instrumentation or project macro execution to make a static check pass.
