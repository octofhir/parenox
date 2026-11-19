# Parenox reference

Command, contract, and editor reference for [Parenox](../README.md). Static
analysis checks Clojure source without running it. Run repository examples
from the project root; `just --list` lists more examples.

## CLI conventions

Build with `cargo build --locked --release`. If `parenox` is not on `PATH`,
use `./target/release/parenox` from the checkout. Use `parenox --help` or
`parenox COMMAND --help` for all flags.

`check` prints pretty diagnostics to stderr; `--format plain` and
`--format json` print to stdout. `--json` is short for `--format json`.
Use `--color always|never` to override automatic color. Suggested edits
are never applied automatically.

| Check exit code | Meaning |
|---|---|
| 0 | No unsuppressed diagnostics, or only warnings with `--allow-incomplete` |
| 1 | Diagnostics not accepted by the selected options |
| 2 | Usage, configuration, or I/O error |

`complete: true` means the implemented checks had enough information,
not that the program is correct at runtime. `--allow-incomplete` changes
the exit status, not what the checker analyzes. It accepts coverage warnings
but not findings reported as errors. JSON uses a direct report for a single
file without externs; a project report groups per-file results under `files`.

Each diagnostic has a stable rule `code` and an opaque occurrence `id`. IDs
can change when the source span or path changes. The optional `contract`
object gives the origin's qualified `name`, `kind`, and file `path` when
available. The primary span remains at the use being checked.

### Targeted suppressions

Write `;; parenox-ignore PNX2322: reason` before the next form. The reason
is required. The directive applies only to that rule and form, not siblings,
quoted data, discards, or inactive reader branches. Supported rules are
`PNX2322` (type mismatch), `PNX2741` (missing key), `PNX2801`
(out-of-bounds index), and `PNX2802` (unhandled `case` value). Suppressions
do not change inferred types; JSON retains the original finding in
`suppressed` and the report remains incomplete. Invalid directives and
unmatched ones in fully checked code report `PNX9006` and `PNX9007`.

## A project-sized example

The [clinic example](../examples/clinic) checks a service and HTTP boundary
across namespaces. Run `just errors` to see three deliberate type errors,
or `just agent-loop` to fix them in memory and measure incremental checking.

### Project configuration

From a project directory, run `parenox check`, or supply one or more files
and directories. Source scans include `.clj` and `.cljc`, skip hidden entries
and do not follow symlinks within a directory. Explicitly supplied source
roots may be symlinks. The checker uses the `:clj` branches of `.cljc`.

```edn
{:paths ["src"]
 :externs ["externs/api.edn"]
 :dependency-externs ["externs/dependencies.edn"]
 :dependency-sources ["../vendor/library/src"]}
```

All keys in this `parenox.edn` example are optional. Relative paths resolve
from its directory. Select another file with `--config FILE`. For `check`,
positional paths replace `:paths` and skip the default `parenox.edn` unless
`--config` is supplied. Repeated `--externs` or `--dependency-externs` flags
replace their matching configured list.

Without configured source roots, Parenox falls back to `deps.edn`. Use
`--deps FILE` to select a different manifest or repeat `--alias NAME` to
include alias paths. Either option selects manifest paths instead of
`:paths`. As an alternative, use `--classpath classpath.txt`, produced by
`clojure -Spath -M:test > classpath.txt`. This uses only directories, not
JARs, and cannot be combined with positional paths, `--deps`, or `--alias`.

`:dependency-sources` adds explicit files or directories for analysis by
`check`, `lsp`, and disk-backed `serve` sessions, including their named specs.
Bulk formatting excludes dependency-only paths. Parenox does not fetch
dependencies or load Maven artifacts. Supply a source checkout or use externs.

### Formatting

`fmt` supports `.clj`, `.cljc`, `.cljs`, and `.edn`. The CLI and language
server both use `:format` from `parenox.edn`. The formatter reads the default
configuration even with explicit source paths.

```edn
{:format {:indent-width 2
          :insert-final-newline? true
          :indent-rules {my.macros/with-resource :block
                         my.macros/combine :align}}}
```

The default indent is two spaces (allowed: 1–8). `:block` uses a fixed indent;
`:align` aligns with the first argument when it shares the head's line.
Rules match the head symbol as written. Formatting preserves literal text,
comments, discards, and reader branches. It does not wrap lines.

```sh
parenox fmt src/demo/core.clj
parenox fmt --check
parenox fmt --write src
```

The first command previews one file on stdout. `--check` exits 1 if any
file would change. `--write` updates files only after the whole batch parses.

## Contracts and externs

### Function and data contracts

Declare contracts with named `s/def` specs, source `s/fdef`, or quoted
`:t/sig` metadata. Matching `s/fdef` and `:t/sig` declarations must agree.
Public functions need a contract for full checking. Parenox can infer direct
calls to supported single-arity private helpers from their arguments. Recursive
helpers, unused helpers without typed callers, and helpers passed as values
require a signature.

```clojure
(ns example.labels (:require [clojure.spec.alpha :as s]))

(s/def ::name string?)
(s/def ::label (s/keys :req-un [::name]))

(defn label {:t/sig '[::label :-> string?]} [entry]
  (:name entry))

(s/fdef render :args (s/alt :short (s/cat :text string?)
                            :prefixed (s/cat :prefix string? :text string?))
               :ret string?)
(defn render
  ([text] text)
  ([prefix text] (str prefix text)))
```

- `s/fdef` targets a local `defn` or `defn-`. Use `s/cat` with fixed
  arguments and an optional final `(s/* T)`, or `s/alt` with distinct fixed
  arities and one shared `:ret`. Other argument regexes, overlapping
  arities, and `:fn` conditions remain unchecked.
- For several `:t/sig` arities, use a vector or list of signature vectors.
  `[number? & string? :-> number?]` gives each extra argument the same type;
  an exact fixed clause takes precedence at a matching argument count.
- `s/keys` is an open map shape. A literal map contract, such as
  `{:name string?}`, is closed. Use `(t/map {:req {:id string?} :opt {:label string?} :extra :open})`
  for inline required and optional fields. Omit `:extra` for a closed shape.
  An optional field can be absent; a present nil still needs a nilable type.
- `s/keys` checks key presence even when the value spec is missing. The
  unknown field type still reports incomplete coverage. A missing spec
  referenced directly is an error. Named specs resolve across source files
  and via declared aliases, including `:as-alias`.
- Repeating a field with the same spec in `s/keys` preserves its type and
  required presence. Different unqualified specs that resolve to the same
  field still report a collision.
- `(s/coll-of (s/tuple K V) :kind map?)` checks map entries using `K`
  for keys and `V` for values. Collection specs without `:kind` remain
  incomplete.
- `s/fspec` uses `s/cat` and `:ret` for callback contracts. Direct
  fixed-arity calls can supply types to `fn` and `#()` arguments.
  General callback and variadic inference remain limited.
- `t/all` supports a fixed generic arity with up to two input type
  parameters and one immediate callback result parameter. Inference uses
  arguments, not the expected return. Use `(a :< number?)` for a supported
  scalar bound. Empty collections alone do not establish an element type.
  See [generic callbacks](../examples/generic_callbacks.clj),
  [two-input inference](../examples/generic_zip.clj), and
  [typed map boundaries](../examples/map_boundary).

A successful `(s/valid? ::label entry)` narrows supported scalars and open
map shapes in the true branch. The false branch does not exclude that shape.
An approximate map shape does not prove the runtime predicate returns true.
Recursive specs, dynamic spec names, and unsupported projections report gaps.

As a spec predicate, core `identity` accepts truthy values and excludes
`:clojure.spec.alpha/invalid`. For a named scalar spec defined directly
with `identity`, `s/conform` returns the input on success and `s/valid?`
can narrow the input in the true branch.

Use `int?` in source contracts and conditions to check for a JVM `Byte`,
`Short`, `Integer`, or `Long`. For example, `2147483648` satisfies
`int?` and `1N` satisfies only `integer?`. Parenox reports `PNX9004`
when an argument's class is required by a contract and cannot be proven.

For `(s/and integer? (s/conformer int))` with the core functions,
`integer?` accepts Clojure integers, including BigInt. A successful input
must fit a signed 32-bit integer and can have any integral storage class.
The normal result is a JVM `Integer` and satisfies `int?`.
In a direct `s/conform` on this named scalar spec, a non-integer returns
`:clojure.spec.alpha/invalid` and overflow throws. `s/valid?` distinguishes
true, false, and an exception. Map conformation, arbitrary conformers, and
user predicates are not executed or generally inferred. See the
[JVM fixture](../tests/fixtures/spec-int-conformer.clj).

Named specs can use the same literal enum map as a predicate and a
conformer. In `(s/and int? event-types (s/conformer event-types))`,
`event-types` must be a uniquely defined, non-dynamic map declared earlier
in the same file. The checker supports up to 32 signed 32-bit decimal
keys and keyword, nil or false values. A mapped keyword other than
`:clojure.spec.alpha/invalid` is a successful result. On a successful
`s/valid?`, the original value narrows to one of the accepted integer
keys. Direct `s/conform` returns the mapped keyword or the invalid marker.
Exporting these finite integer contracts as standalone EDN is currently
unsupported.

### EDN externs

Externs are trusted, inert declarations for qualified names, not
implementations:

```edn
{vendor.store/find [string? :-> (clojure.spec.alpha/nilable map?)]
 vendor.store/default-limit {:value 50}
 vendor.bindings/with-bindings {:macro :let-like}}
```

A checked source declaration takes precedence. Duplicate externs or attempts
to replace a modeled built-in report `PNX2790`. Literal values describe the
initializer, not later runtime changes. A `:let-like` description supplies
lexical scope for sequential bindings, not the macro's result, effects, or
tail positions; such calls still report `PNX9009`.

### Validating library externs

`parenox validate-externs --config parenox.edn` checks source bodies and
compares the library's `:externs` promises against its public definitions.
`:dependency-externs` are trusted for calls but not validated. Validation
requires a complete check. A known mismatch reports `PNX2322`; missing
definitions and unproven compatibility report `PNX9011`.

### Exporting public contracts

`parenox export-externs > current-api.edn` writes a checked snapshot of public
signatures and literal vars, with named specs inlined. It requires a complete
source check and verifies the generated EDN before printing it. It does not
export private vars, imported externs, macro scopes, or guard refinements.
Snapshots above the export size or nesting limits report `PNX9012`.

### Comparing releases

```sh
parenox diff-externs --before previous-api.edn --after current-api.edn --json
```

Use `--before-config` and `--after-config` when either side relies on source
specs. Each side resolves its own specs; dependency externs are not part of
the public comparison. Exit 0 means that the new snapshot preserves the old
promises. Incompatible and unproven changes exit 1; usage and I/O errors
exit 2. Changed literal values can break compatibility even at the same
type. JSON includes a structured path to the failure, such as an argument's
nested field.

## Current scope

| Area | What Parenox checks |
|---|---|
| Flow | Lexical scopes, conditionals, literal `case`, `loop`/`recur`, `try`/`catch`, core threading, and selected predicates |
| Maps | Presence, required/optional fields, open and closed shapes, dictionaries, stable lookup narrowing, and supported reads, updates, and projections |
| Collections | Vectors, sets, map entries, sequences, positional destructuring, and selected calls including `mapv`, `filterv`, `reduce`, and `reduce-kv` |
| JVM | Selected regex and string operations and known exception classes, not general Java interop |
| Unannotated code | Source name resolution, known arities, and selected string mismatches; no claim of full type coverage |

Clojure truthiness distinguishes nil and false from zero and empty
collections. A missing key is not a present nil or false. `get` and map
destructuring defaults apply only to absence, and their default expressions
are evaluated eagerly. Clojure 1.13 checked lookups such as `req!` and
`:keys!` require presence but preserve the field's value type. Positional
bindings fill missing positions with nil; `[head & tail :as whole]`
keeps the original collection in `whole` and a remaining seq or nil in
`tail`. See `just checked`, `just defaults`, and `just presence`.

`for` and `doseq` check seqable inputs, sequential bindings, destructuring,
and `:let`, `:when`, and `:while`. `for` evaluates the first source expression
immediately but defers traversal and the body. `doseq` is eager and normally
returns nil; with no bindings or only `:let` clauses it returns the last body
expression. `some->` stops on nil, not false. `recur` must target a supported
tail position and analysis is bounded when argument types keep growing.

Verified `:t/guard '[:then value string?]` narrows the argument after a true
result. Use `:both` only if the checker can also verify the false branch.
An unproved guard reports `PNX9008` and doesn't narrow callers. See
[checked predicates](../examples/checked_guards.clj).

On known String receivers, checked JVM calls include `length`, `charAt`,
`startsWith(String)`, `endsWith(String)`, and `indexOf(String)`.
Offsets count UTF-16 code units. A String hint identifies the class but
does not rule out nil. General interop and other overloads remain outside
the supported subset. Known explicit throws can narrow the set of reachable
handlers; ordinary calls may throw.

Unknown macros, runtime-created declarations, and unsupported reader or
numeric semantics remain explicit coverage gaps. In checked code, an
untyped source call reports `PNX9002` and a comparison without enough
type information can report `PNX9004`. An unknown field does not hide a
definite mismatch elsewhere in the value.

## Lossless syntax

`parenox syntax FILE` prints the Rowan concrete syntax tree and reports
reader errors. Comments, whitespace, original literal spellings, discards,
inactive reader branches, and malformed input remain in the tree. The
shared reader exposes semantic forms with original spans; checking does
not build the syntax tree. `reader::parse`, `parse_with`, and `parse_edn`
are available to Rust callers. None of these run tagged readers or macros.

## Editor integration

Start `parenox lsp` over stdin and stdout. It shares project configuration
with `check`; `lsp --alias test` includes test roots. Unsaved buffers
participate in analysis and navigation. A nested project can use its nearest
`parenox.edn` or `deps.edn`. Outside configured roots and without a nested
project, a buffer has no project context.

- `.clj` and `.cljc`: diagnostics, completion, definition, hover, find usages,
  highlights, document outline, and workspace symbols. Hover shows checked
  signatures and origins when available. Closed source files can contribute
  declarations and usages. Opaque macro expansions are not indexed.
- **Signature Help** shows parameters, docstrings, overloads and the active
  argument for calls to source `defn` and `defn-` functions. It works in an
  unsaved call with missing closing delimiters at the end of the file.
- **Go to Declaration** opens the local alias or refer clause for a source
  usage. **Go to Definition** opens the provider. **Go to Implementation**
  opens source `defmethod` registrations for a resolved `defmulti`.
- **Rename** returns versioned workspace edits when the client supports
  them. It covers ordinary source vars and lexical bindings. Linked editing
  covers proven lexical bindings within one file.
- **Call Hierarchy** groups source calls by resolved caller or callee.
  **Code Lens** resolves reference counts on source var declarations.
- Semantic tokens work for the whole file or a specified range. Folding
  follows multi-line declarations, and selection expansion follows the
  syntax tree. Clients that opt into `experimental.testTree` get source
  `clojure.test` trees for open buffers.
- **Go to Definition** on a named spec in `:t/sig`, `s/fdef`, `s/def`, or a
  supported direct `s/valid?` call opens its source `s/def`. It follows
  qualified keys and aliases. Ordinary keyword values and ambiguous declarations
  have no spec target. Completed forms before reader errors remain usable.
- **Generate s/fdef** works on supported top-level, single-arity `.clj`
  functions. Unknown types produce a commented draft for review; existing
  contracts are not replaced.
- **Reformat Code** uses project settings for the whole unsaved document
  or the physical lines touched by a selection. Range formatting uses the
  surrounding indentation and preserves unselected lines. Invalid syntax
  produces no edits. Open `.edn` buffers also report reader diagnostics.
  Incremental LSP positions are UTF-16.

Watch events from the editor can refresh closed files and configuration.
A failed project reload preserves the last valid project snapshot.

### RustRover and JetBrains IDEs

1. Install [LSP4IJ](https://plugins.jetbrains.com/plugin/23257-lsp4ij), then
   run `cargo install --locked --path .` from this checkout.
2. In **Settings → Languages & Frameworks → Language Servers (LSP4IJ)**,
   choose **Import from custom template…** and select the
   [Parenox template directory](../integrations/lsp4ij/parenox).
3. Set the command to `parenox lsp` or the full path to the built binary
   followed by `lsp`. Disable overlapping file mappings from other
   Clojure language servers to avoid duplicate diagnostics.

### Local Clojure LSP pilot

Run `just pilot-setup` for the pinned checkout at
`playground/clojure-lsp` and a local release build. It does not overwrite
local changes to an existing checkout. Run `just pilot-check json`
for the read-only project report; Maven dependencies are not resolved.

In the Parenox RustRover workspace, use this command to analyze the pilot:

```text
"$PROJECT_DIR$/target/release/parenox" lsp --deps "$PROJECT_DIR$/playground/clojure-lsp/deps.edn" --alias test
```

Keep `:format` in the workspace's `parenox.edn`. Use
`just pilot-format-smoke` and `just pilot-navigation-smoke` for read-only
checks of the editor workflows. Reports on the unmodified pilot can include
coverage gaps, not upstream bugs.

## JVM tests

```sh
parenox test --alias test my-app.routes-test
parenox test --alias test --var my-app.routes-test/decodes-unicode
parenox test --alias test --discover
```

`test` launches the project's standard Clojure CLI and runs `clojure.test`
against saved files. Repeat `--var` to select several qualified tests;
discovery finds `*_test.clj` and `*_test.cljc` in the selected source roots.
Both namespace and var runs use test fixtures; only whole-namespace runs
use namespace hooks.
Use `--project DIR` or `--clojure EXECUTABLE` to override the defaults.
Reports include local failure locations when the runtime supplies them.
With `--format json`, stdout carries one report and runtime logs go to
stderr. Exit 0 means success, 1 means test failures or errors, and 2 means
the requested run did not complete. Run `just jvm-test-smoke` for examples.

## Local nREPL

```sh
parenox nrepl --port 7888
parenox nrepl --port 7888 --eval '(def total 40)' --eval '(+ total 2)'
```

The client only connects explicitly to an existing server on `127.0.0.1`.
Without `--eval`, it lists supported operations. Otherwise it clones a
session, sends forms in order, stops after an eval error, and closes the
session. JSON separates values, `out`, `err`, exceptions, and completion.
Exit 1 means an eval error; exit 2 means a connection or protocol error.
`--timeout-ms` changes the per-operation timeout (default 5000 ms).
A timeout does not cancel evaluation or guarantee session closure.
Opening or checking sources never starts or connects to nREPL.

## Agent sessions

`parenox serve` reads JSON Lines. Each request gets one flushed response
with `id` and `result`, or an error. Use full file contents, not patches.

```json
{"id":1,"path":"demo.clj","source":"(defn f {:t/sig '[:-> number?]} [] 1)"}
{"id":2,"op":"open","files":[{"path":"demo.clj","source":"(ns demo)"}]}
{"id":3,"op":"update","changes":[{"path":"demo.clj","source":"(ns demo) (def limit 5) (defn f [] limit)"}]}
{"id":4,"op":"definition","path":"demo.clj","offset":35}
```

- A request with `files` and no `op` checks a complete snapshot without
  changing the tracked project. `open` replaces the tracked snapshot.
- `update` accepts optional `changes` and `remove`. An omitted `externs`
  retains the extern snapshots; an array of `{path, source}` objects
  replaces them. A single object is also accepted. Use `null` or `[]`
  to clear externs. A valid edit is kept even if it introduces diagnostics.
- `{"op":"open_project","root":"/path/to/project","aliases":["test"]}`
  loads the project from disk. Specify `classpath` instead of `aliases`
  for a resolved classpath. `refresh_project` reloads disk while
  retaining unsaved overlays and extern overrides.
- `definition` and `hover` use a zero-based UTF-8 byte offset.
  Definition returns a source `path` and `span` or `null`. Hover returns
  a var's qualified name, display signatures, and any contract origin.
  Use `parenox definition --file FILE --offset N` for a disk-backed query.

Results include `ok`, `complete`, diagnostics, and statistics for parsed
and reused files and checked and reused functions. The in-memory session
reuses unaffected results on edits; there is no on-disk cache.

## Performance and development

- `parenox-frontend` owns the reader, forms and CST. `parenox-analysis`
  owns contracts, type checking and project sessions. `parenox-formatter`
  uses the frontend's CST. `parenox-lsp` owns protocol, text coordinates
  and editor features. The root package wires the CLI and disk projects.
- `just verify` runs formatting checks, Clippy, and tests.
- `just agent-loop` measures the clinic edit scenario; `just bench` runs a
  synthetic workload. `parenox bench FILE` measures in-process checks.
- `just runtime-smoke` uses Clojure/JVM. Run
  `clojure -M tests/fixtures/spec-int-conformer.clj` and
  `clojure -M tests/fixtures/spec-enum-conformer.clj` to verify the
  conformers against the pinned JVM reference.

See [research and pilot results](design/type-checker-research.md),
[benchmarks](../benchmarks), and the [next development prompt](next-prompt.md).
