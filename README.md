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
just checked-guards  # predicates that prove what their results imply
just contextual-callbacks # check fn and #() against a callback contract
just generic-callbacks    # infer callback input and result types
just generic-zip          # infer two independent inputs and a callback result
just map-boundary         # check a storage extern through to an HTTP response
just split-externs        # load two declaration files for one namespace
just validate-externs     # check library promises from config, then show drift
just export-externs       # write the showcase's public contracts as EDN
just literal-externs      # check a client against exported constants
just diff-externs         # compare two versions of a library contract
just showcase             # check one end-to-end Clojure example
just showcase-errors      # show six deliberately broken changes
just showcase-run         # run the same example on JVM
just macro-scopes    # resolve macro bindings and show the analysis limit
just paths           # nested map lookup and update with source diagnostics
just presence        # presence guards for optional fields
just updates         # update and update-in with callback diagnostics
just vectors pretty always  # mapv and filterv with colored callback diagnostics
just entries pretty always  # map entry pipelines and diagnostics
just dictionaries pretty always  # map-of contracts and diagnostics
just map-keys pretty always  # string and dynamic map keys
just map-projection  # verify projected and redacted public responses
just map-sequences   # typed keys and values from a dictionary
just reduce-kv       # typed reductions over map entries and vector indices
just update-vals     # transform map values while retaining key types and presence
just map-builders    # build a map with zipmap and rename keys with update-keys
just group-by        # group typed items without losing known positions
just merge-with      # combine values only for overlapping map keys
just dynamic-paths pretty always  # nested maps with dynamic path segments
just ordered-paths pretty always  # saved vector paths and positional contracts
just indices pretty always  # indexed reads and out-of-bounds diagnostics
just loops            # accumulators and recur diagnostics
just cond             # ordered guards and fallthrough diagnostics
just libspecs         # bare requires and renamed imports across files
just namespace-specs # alias-only imports and project-wide named specs
just case             # grouped dispatch and missing-case diagnostics
just tagged-maps      # narrow a map union by its tag
just sequences        # seq traversal, precise positions and empty inputs
just accumulators     # growing vectors with retained element types
just exceptions       # explicit errors and structured exception data
just handlers         # try/catch/finally and recur boundaries
just literal-vars     # literal config values across namespaces
just core-imports     # core referrals and local replacements
just list-accumulators # list and cons with a loop accumulator
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

Exit codes: **0** no unsuppressed diagnostics, **1** diagnostics, **2** usage or I/O failure.
Suggested edits require review and are never applied automatically.

## Targeted suppressions

Put a comment before the form whose finding you want to suppress:

```clojure
(defn label {:t/sig '[:-> string?]} []
  ;; parenox-ignore PNX2322: the legacy caller handles a numeric label
  42)
```

The comment requires one rule code and a nonempty reason. It matches only
findings of that rule inside the next form, not its siblings. Supported codes
are `PNX2322` (type mismatch), `PNX2741` (missing checked key), `PNX2801`
(out-of-bounds index), and `PNX2802` (unhandled `case` value). Place the
comment inside the selected branch of a reader conditional. Comments in
inactive branches, discarded forms, and quoted data do not apply.

JSON reports keep each suppressed finding, its original `id`, the reason and
the comment span in `suppressed`. Text output lists suppressed findings as
notes. Suppressing a finding does not change the inferred type. A report with
only suppressed findings is `ok: true` but `complete: false`.
Invalid directives report `PNX9006`. A directive that matches no finding
reports `PNX9007` when no other reader, resolution, or analysis error prevented
checking.

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
Externs are trusted declarations for library functions. A matching project
source declaration takes precedence, and source privacy applies to every caller.
An extern for a name that Parenox already models reports `PNX2790`; it does not
replace the built-in rule. An unmodeled name in the same namespace is allowed.

Repeat `--externs` to load separate EDN files. Two files can declare different
names in the same namespace. If they declare the same qualified name, both
locations report `PNX2790` and neither extern is used. Each contract keeps its
own declaration path in diagnostics. Run `just split-externs` for an example.

### Project configuration

Put a `parenox.edn` file beside the project:

```edn
{:paths ["src"]
 :externs ["externs/api.edn"]
 :dependency-externs ["externs/dependencies.edn"]}
```

Run `parenox check` from that directory, or pass `--config path/to/parenox.edn`
from anywhere. Relative paths in the config resolve from the config directory.
Explicit source paths, `--externs` or `--dependency-externs` replace the matching
configured list; they don't append to it. If you pass source paths without
`--config`, Parenox doesn't load the default file. All three keys are optional;
unknown or repeated keys and non-string paths are errors. The config is read
as data. `serve` still uses the in-memory snapshots sent by the client.

### Validating library externs

Use `parenox validate-externs --config parenox.edn` to check a library against
the externs it ships. It checks the source bodies and verifies that the public
functions accept the calls promised in `:externs` and return compatible types.
For variadic signatures, it checks both the fixed clauses selected at the
promised argument counts and the variadic tail.
Literal externs must match the public source initializer.

The selected externs do not fill in missing source definitions. Separate
`:dependency-externs` (or `--dependency-externs`) are trusted while checking
source bodies, but are not themselves validated. Ordinary `check` trusts
both sets. A duplicate across the sets is still an error; neither declaration
is used.

A known mismatch reports `PNX2322` at the EDN declaration and names the source
file. For a signature mismatch, a note points to the rejected argument,
result, or nested field. If there is no matching checked public definition,
or compatibility cannot be proven, Parenox reports `PNX9011`. Macro scope verification and general
generic subtyping are not supported. Validation succeeds only if the whole
report is both `ok` and `complete`, so a suppressed source error does not
certify a library. Run `just validate-externs` for passing and failing examples.

### Exporting public contracts

Save a checked library's public function contracts and literal `def` values.
Clients can then check against the snapshot without the library's source or specs:

```sh
just export-externs > /tmp/showcase.edn
target/release/parenox check examples/showcase_client.clj --externs /tmp/showcase.edn
```

`export-externs` accepts source paths or `--config` like `check`. It trusts any
supplied externs as dependencies but doesn't copy them into the snapshot. The
output sorts declarations by qualified name and inlines named specs. It preserves
arities, callbacks, type parameters and map shapes. It doesn't export private
vars, macro scopes or the narrowing behavior of `:t/guard`.

EDN goes to stdout only after a complete source check and a second check of the
generated snapshot in isolation. Parenox compares the reloaded signatures and
literal values with the originals. Export stops at the 1 MiB snapshot limit or
if an expanded signature exceeds the reader's nesting limit. Both limits
report `PNX9012` at the affected declaration. A failed export exits with status
`1` and writes diagnostics to stderr in the requested `--format`.
The resulting file works with `check`, `validate-externs` and `diff-externs`.

Run `just bench-export chain 45` and `just bench-export shared 15` to measure
the nesting and snapshot size limits on synthetic named specs.

### Literal externs

Use `{:value ...}` for a public literal `def`:

```edn
{demo.config/default-plan {:value {:limit 4 :mode :demo.config/online}}}
```

The checker reads the value as inert EDN. A list is data, not a call.
Maps, vectors, sets, lists and scalars retain the same type precision as in
source; map keys must be keywords or strings. Export resolves auto-keywords
and preserves numeric spelling, but not metadata. It describes the
initializer, not later changes to a Var's root. Run `just literal-externs`
to check a client using exported constants instead of library source.

### Comparing releases

`diff-externs` compares the new promises to the old ones without checking
implementation bodies. Repeat `--before` and `--after` for standalone EDN files:

```sh
parenox diff-externs --before previous/api.edn --after current/api.edn --json
```

If an extern names a project spec, use `--before-config` and `--after-config`
to resolve each release's `:paths` and `:externs` separately. Dependency externs
are not part of the comparison. Explicit EDN flags replace that side's
configured extern list. An empty EDN map represents an empty surface.

Besides function and macro changes, the report lists changed literals as
`value_changed`. A changed value is incompatible even if its type is the
same. Map and set entry order doesn't count; different numeric spellings
are compared conservatively as a change.

For signature failures, the report shows both declaration locations, a call
argument count, and one failed requirement. For example:

```text
argument 1 → field :payload → field :title: expected Keyword, got String
```

JSON puts this explanation in `detail` with a `reason` and a structured `path`.
Argument numbers start at one; collection indices start at zero. The expected
type is the one the value needs to fit: the new input for a call, the old
output for a return. Callback inputs reverse direction again. Unproven
generic compatibility is reported as such, not as a specific type mismatch.

Invalid input reports its diagnostics
instead of guessing which promises were removed. Exit status is `0` only if
every old promise is still covered, `1` for breaking or incomplete comparisons,
and `2` for usage or I/O errors. Run `just diff-externs` for an example where
the extern text is unchanged but a referenced spec breaks compatibility.

### External macro scopes

An extern can describe where a macro introduces local names:

```edn
{vendor.bindings/with-bindings {:macro :let-like}}
```

`(vendor/with-bindings [label input decorated (str label "!")] decorated)`
has sequential bindings. Each initializer sees the preceding bindings but
not its own new binding. The body sees all of them. They do not escape
the macro call. Use simple unqualified symbols in the binding vector.

This is a scope description, not an expansion or a function signature.
Parenox checks name resolution in the forms it understands without loading
or running the macro. `PNX9009` marks the call incomplete: its result, effects
and tail positions are not inferred. Lexical bindings and project functions
still take precedence over the extern. Run `just macro-scopes` for the
comparison with an ordinary `let`.

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

From a Clojure CLI project, `parenox check` can read source roots from the local
`deps.edn`. Use `parenox check --alias test` to include the paths of the `:test`
alias. Repeat `--alias` in composition order and use `--deps FILE` for a different
project manifest. Explicit `parenox.edn` source paths are used by default when
available. A selected manifest or alias supplies the source paths instead.
Parenox reads project paths, including path aliases, `:extra-paths`, and path
replacements, as EDN. Supply dependency sources as paths or configure their
contracts as externs. [Clojure CLI project configuration](https://clojure.org/reference/deps_edn)

Named specs resolve against the supplied project declarations, including keys
whose namespace differs from the file that declares them. Input file order does
not affect resolution. An alias created with `:as-alias` can qualify keywords
without creating a namespace load dependency. Competing declarations of the
same spec key in separate files report an analysis gap.

Auto-resolved keywords such as `::schema/name` require a declared alias.
The checker validates these tokens in discarded forms and inactive reader
branches. Externs use ordinary EDN keywords, such as `:schema/name`.
Run `just keyword-aliases` for examples.

`:refer-clojure` supports `:exclude`, `:only` and nonoverlapping `:rename`
mappings. Qualified calls to `clojure.core` continue to resolve when an
unqualified name is excluded.

`:require` supports `:refer :all` for a namespace supplied as project source,
including `:exclude` and `:rename`. It resolves public functions with contracts
and public literal vars. Changes to the set of exports refresh the referral
mapping. Externs describe selected contracts, so use an explicit referral list
for extern-only namespaces. Run `just all-refers` for examples.

`:import` resolves short class names in `catch` for modeled JVM exception
classes, including the `IOException` hierarchy. It accepts qualified names
and package groups with lists or vectors. Custom exception classes report
an analysis gap when used in `catch`. Run `just imports` for examples.

`def` supports literal maps, vectors, sets and scalar values. Their inferred
types are available to importing namespaces, subject to `^:private`. Changes
to exported values invalidate their consumers. Dynamic vars, unbound vars and
computed initializers report an analysis gap.

Quoted lists, vectors, sets and maps with literal keyword or string keys can
also be used as data and exported as literal vars. Quoted symbols have type
`Symbol`. Run `just quoted` for examples.

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
Clojure numeric tokens include octal, hexadecimal, radix integers, ratios,
and the `N` and `M` suffixes. The checker tracks these as `Number` and reports
invalid tokens at their source spans. Run `just numbers` for a checked example.

The reader preserves the original text of regex literals such as `#"\d+"`.
The checker reports incomplete type analysis for regex values.

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

See [type checker research](docs/design/type-checker-research.md) for proposed
next steps, including Crabjure's macro scopes and dependency resolution.

## Editor integration

Configure your editor to start `parenox lsp` over stdin and stdout. The server
loads one workspace using the same `parenox.edn` and `deps.edn` settings as
`check`. Use `parenox lsp --alias test` to include test source roots.

The server publishes Parenox diagnostics for open `.clj` and `.cljc` files and
supports go-to-definition for qualified source vars, including namespace aliases.
Unsaved buffers participate in checks and navigation. Positions use UTF-16 and
the server requests full document synchronization from the editor.

## Agent sessions

```sh
./target/release/parenox serve
```

One request and one flushed response per line. A single-file request:

```json
{"id":1,"path":"demo.clj","source":"(defn f {:t/sig '[:-> number?]} [] 1)"}
```

A project request contains a **complete snapshot** of its files, plus optional
extern files. The `source` values are full file contents, not patches:

```json
{"id":2,"files":[{"path":"a.clj","source":"(ns a)"}],"externs":[{"path":"store.edn","source":"{}"},{"path":"http.edn","source":"{}"}]}
```

The previous single extern object still works. Rust callers can use
`Session::check_project_with_externs` with a slice of extern snapshots.

Responses have `id` and `result`. Results include `ok`, `complete`, diagnostics,
and `stats`: elapsed microseconds, parsed/reused files and checked/reused functions.
Project results contain a `files` array. Invalid JSON returns an error and the
process continues.

Each diagnostic in a check report has an `id` for that occurrence. Use it to
match diagnostics between repeated checks of the same snapshot, including
diagnostics with the same `code` and `span`. Treat the ID as opaque: it can
change when a file is renamed or a diagnostic moves. Use `code` to identify
the rule. The top-level response `id` still identifies the request.

When a diagnostic checks a known function contract, text output names its
source and JSON includes a `contract` object. Scope diagnostics and literal
extern validation can also carry their declaration. The object contains the
qualified `name`, the `kind` (`t_sig`, `s_fdef`, `extern`, `extern_literal`,
`builtin` or `macro_scope`) and, for
file-backed contracts, the `path`. The primary diagnostic span still points
to the call or return being checked.

For editor sessions, open a tracked project once and send the changed files as
full source strings in each update. This example checks one file, edits it, and
then removes it:

```json
{"id":3,"op":"open","files":[{"path":"a.clj","source":"(ns a)"}]}
{"id":4,"op":"update","changes":[{"path":"a.clj","source":"(ns a) (def limit 5)"}]}
{"id":5,"op":"update","remove":["a.clj"]}
```

`open` replaces the tracked project. `update` accepts optional `changes` to add
or replace files and `remove` with paths to remove. An empty update checks the
current project. Omit `externs` in an update to keep the current set. An array
replaces the full set; a single object replaces it with one file. Use `null`
or `[]` to clear it. Each operation returns a full project report ordered by
source path, followed by the extern reports in the supplied order. Invalid
operations leave the tracked project intact. A valid edit is
saved even when it produces diagnostics. Single-file and complete-snapshot
requests work independently of the tracked project.

Query a qualified source var after `open` or `update` with a zero-based UTF-8
byte offset inside the reference:

```json
{"id":6,"op":"definition","path":"src/app.clj","offset":105}
```

The response contains the target `path` and `span`, or a `null` result when no
unique source definition can be resolved. The query supports qualified `def`
and `defn` references, including namespace aliases and functions without type
annotations. For a file-backed query, run
`parenox definition --file src/app.clj --offset 105` with the same project path
options as `check`.

- Unchanged sources reuse their parsed forms and declaration models.
- Unchanged externs reuse their declarations until a referenced spec changes.
- An extern edit rechecks callers of the changed declarations, not all callers
  in the same namespace.
- Annotated body edits recheck that function while preserving callers' cached results.
- Edits to unannotated private helpers invalidate their transitive callers in the same file.
- Public contract changes recheck functions that reference the changed var,
  including references reached through inferred private helpers.
- A guard body edit also rechecks its callers if the verified refinement
  changes, even when its signature stays the same.
- Local var changes recheck their consumers in the same file. Changes to
  namespace resolution or local spec declarations can still recheck that file.
- Removed files and externs disappear from the current environment immediately.
- Unchanged functions with successful checks can be reused after their source ranges move.
  Functions with diagnostics are rechecked if their own ranges or a helper's ranges move.
- Tracked projects discard cached files when they are removed or the project is
  reopened. One-off snapshot requests have their own cache.
- The one-off cache lasts for the process lifetime. The protocol uses in-memory
  snapshots; it has no filesystem watcher or disk cache.

`source_hash` is a session fingerprint, not a cryptographic digest. Cache validity
uses exact source, positions and interface comparisons.

To measure the same clinic edit sequence with a larger extern file, run
`just agent-loop 1000 target/release/parenox 512`. This adds synthetic library
contracts that refer to the clinic's patient spec. Run
`just agent-loop 1000 target/release/parenox 0 128` to append 128 typed functions
whose ranges move as the agent edits the first function in their file.

Run `just agent-loop 1000 target/release/parenox 512 0 delta` to measure the
same edit sequence through tracked updates.

Run `just agent-loop 1000 target/release/parenox 512 0 delta 0 8` to split the
512 synthetic contracts across eight extern files and keep the clinic contract
in its own file. The final edit changes one unused extern and verifies that
only that file is parsed, with no new function checks.

Run `just agent-loop 1000 target/release/parenox 0 0 snapshot 128` to add 128
consumer functions and alternate edits to an unused export in their provider.
The final signature edit changes the export those consumers actually call.
Set the fourth argument to `128` as well to add 128 functions to the edited file
and measure reuse within that file.

## Showcase: batch processing

[examples/showcase.clj](examples/showcase.clj) is a single Clojure file that
turns a batch of meter readings and notes into a typed response. Run
`just showcase` to check it and `just showcase-run` to print its response on JVM.

- `event-row` uses an exhaustive `case` to select the right type of `:value`.
- `zip-results` infers two input types and the result of its named callback.
- `find-label` returns a nullable value. Parenox checks `use-title?` before
  trusting it to narrow the title to `String`.
- Typed exception data supplies the error response; a generic envelope keeps
  the normal response's body type. Private helpers get types from their callers.

Run `just showcase-errors` to check
[examples/showcase_bad.clj](examples/showcase_bad.clj) against the original
contracts. It shows six diagnostics without editing either file:

| Regression | What Parenox shows |
|---|---|
| A branch for `:note` was dropped | The missing tag in `case` |
| An enabled flag is mistaken for proof that a title exists | The unproved guard, then the nullable call with a reviewable fallback suggestion |
| The two input vectors are swapped | The inferred callback signature and the contract it violates in the other file |
| The response score is formatted as text | `String` in the `:score` field where the API promises `Number` |
| The error handler puts an internal code in the title | `Number` in the `:title` field, traced through `ex-info` and `catch` |

Use `just showcase-errors plain` for compact output or `just showcase-errors json`
for spans, contract origins, and suggestions as data. `just showcase` checks
the unchanged implementation completely without suppressions.

## Current scope

Types are read from `s/def`, `s/fdef` and quoted `:t/sig` metadata. Public functions
need signatures. Direct calls to single-arity private helpers (`defn-`, `^:private` or `{:private true}`)
can omit them: the checker uses the actual argument types to analyze the body and
infer its result. This is inference for each call, not a generalized function signature.

Use `s/fdef` to check an existing single-arity function without adding metadata:

```clojure
(ns example.labels (:require [clojure.spec.alpha :as s]))

(s/fdef label :args (s/cat :name string?) :ret string?)
(defn label [name] (clojure.string/upper-case name))
```

Parenox checks the function body and its callers. The target must be a `defn`
or `defn-` in the same namespace. The supported `:args` form is `s/cat` with
fixed arguments and an optional final `(s/* T)` for a rest parameter. Both
`:args` and `:ret` are required. Other options such as `:fn` and more general
argument regexes report an analysis gap. If both signature sources are present,
they must agree. Checking does not evaluate the spec or instrument the function.

At calls and returns, an unknown type does not hide a mismatch in a different
part of the value. If missing type information prevents checking the contract
and there is no local diagnostic yet, Parenox reports `PNX9004` and marks the
check incomplete.

### Checked predicates

```clojure
(defn enabled-text?
  {:t/sig '[boolean? (s/nilable string?) :-> boolean?]
   :t/guard '[:then value string?]}
  [enabled value]
  (and enabled (string? value)))
```

`:then` narrows the named argument only when the call returns true. False could
mean `enabled` was false even when `value` was a string. Use `:both` only when
the body also proves that a false result excludes the declared type.

The checker verifies both claims without running the function. The subset
requires one fixed arity with plain symbol parameters, a Boolean result, and
one expression built from Boolean parameters or literals, `and`, `or`, `not`,
`if`, and the supported core predicates: `string?`, `char?`, `number?`,
`boolean?`, `keyword?`, `symbol?` and `nil?`. The body may also use `some?`.
A shadowed primitive or other unsupported form cannot prove a claim. An
unproved claim reports `PNX9008` and does not narrow callers. Run
`just checked-guards` for both directions.

### Contextual callbacks

```clojure
(s/def ::text-transform
  (s/fspec :args (s/cat :text string?) :ret string?))

(defn transform {:t/sig '[::text-transform string? :-> string?]} [f text]
  (f text))

(defn clean-label {:t/sig '[string? :-> string?]} [label]
  (transform #(clojure.string/trim %) label))
```

`s/fspec` describes a callable value. Parenox checks calls to the callback in
the receiving function. At a direct call with a fixed-arity callback contract,
it uses that contract to check the parameters and result of a passed `fn` or
`#()`. Errors point inside the callback and its body is not run during
argument evaluation. Named functions are checked against the contract too:
they must accept every declared input and return a compatible result.

The spec uses the same `:args (s/cat ...)` and `:ret` subset as `s/fdef`.
Named specs and externs can supply it. No generative tests are run.
Typing a standalone `fn` without context, inferring a callback's type through
`apply`, and contextual checking against a variadic callback contract are
not supported yet. Run `just contextual-callbacks`.

### Generic callbacks

```clojure
(defn transform
  {:t/sig '(t/all [a] [(s/fspec :args (s/cat :value a) :ret a) a :-> a])}
  [f value]
  (f value))

(defn next-id {:t/sig '[number? :-> number?]} [id]
  (transform #(inc %) id))
```

Parenox infers `a` from the first argument declared directly as `a` (`value`
here). It uses that type for the callback and the result, then checks any other
occurrences against it. The body of `transform` is checked without assuming
that `a` is a number, a string, or any other concrete type.

Use a second parameter when the callback changes the type:

```clojure
(defn convert
  {:t/sig '(t/all [a b] [(s/fspec :args (s/cat :value a) :ret b) a :-> b])}
  [f value]
  (f value))

(defn label-length {:t/sig '[string? :-> number?]} [label]
  (convert #(count %) label))
```

Here `label` gives the callback its input type (`String`), and the callback
gives the call its result type (`Number`). A named callback supplies that type
from its matching fixed-arity signature. A callback can produce a concrete
map or collection; the type it produces is inferred without running it.

A contract can also infer `a` from vector elements and return a vector of
callback results:

```clojure
(defn map-values
  {:t/sig '(t/all [a b] [(s/fspec :args (s/cat :element a) :ret b)
                        (s/coll-of a :kind vector?) :-> (s/coll-of b :kind vector?)])}
  [f values]
  (mapv f values))
```

If no argument is declared directly as `a`, the first declared argument that
uses it as a vector element, dictionary value, or required map field supplies
it. For vectors and dictionaries, Parenox considers all possible element or
value types. An empty literal `[]` or `{}` alone cannot supply that type.
Parenox reports `PNX9010` instead of guessing.

Source contracts and EDN externs support one fixed generic arity, with one
or two parameters inferred from ordinary arguments. A parameter without such
an argument can be inferred from one immediate fixed-arity callback's result,
if it also appears in the call's result. Parameters in map keys, sets, general
unions, nested collections, or deeper callbacks are not supported. Generic calls
through `apply` require a spread with known positions and do not infer inline
callback bodies. Run `just generic-callbacks`.

### Two independent inputs

```clojure
(defn zip-with
  {:t/sig '(t/all [a b c]
             [(s/fspec :args (s/cat :left a :right b) :ret c)
              (s/coll-of a :kind vector?) (s/coll-of b :kind vector?)
              :-> (s/coll-of c :kind vector?)])}
  [combine left right]
  (mapv combine left right))
```

Each vector supplies its own element type. The callback supplies `c`. The
order of the names in `t/all` doesn't decide their roles: a parameter that
appears in an ordinary argument gets its type from that argument, even if it
also appears in the callback result. If one input lacks evidence, Parenox
reports an analysis gap without hiding a definite mismatch on the other
input. Run `just generic-zip`.

A result can be `(s/nilable a)` when `a` has a separate source of evidence.
For example, `[(s/map-of keyword? a) keyword? :-> (s/nilable a)]` describes
a dictionary lookup. Its result must be checked for `nil` before it is used
as an `a`. A nullable callback result alone does not supply evidence for a
parameter: `nil` might mean there was no result.

Renaming the parameters of a `t/all` declaration without changing how they're
used leaves the contract equivalent. An incremental check can reuse passing
callers after that edit. Callers with diagnostics are checked again so their
messages use the current names. Changing a bound changes the contract.

### Type parameter bounds

Write `(a :< number?)` to require that the type inferred for `a` is numeric.
Any input parameter or the optional callback result parameter can have a
bound. Supported bounds are `number?`, `string?`, `boolean?`, `char?`,
`keyword?`, and `symbol?`.

```clojure
(defn number-label
  {:t/sig '(t/all [(a :< number?)] [a :-> string?])}
  [value]
  (str (inc value)))

(defn labels {:t/sig '[:-> (s/coll-of string? :kind vector?)]} []
  (map-values number-label [1 2]))
```

Inside `number-label`, `value` can be used as a number. A declaration returning
`a` would still need to return a value of that same abstract type, not just any
number. At a call, a bound is checked against the inferred type; it is not a
default when evidence is missing.

A named generic function can be specialized to a fixed-arity callback. Its
input parameters come from the expected callback inputs; a result parameter
can come from a known non-generic callback argument of that signature. The
expected return is checked after specialization, not used to infer either
parameter. Parenox reports an analysis gap when the context cannot supply
that evidence. It does not recursively specialize generic callback arguments.

### Generic map contracts

A literal map type in `:t/sig` or an EDN extern is closed: every declared key
is required and additional keys are rejected. Use `s/keys` for an open shape
with named specs and optional fields, or inline the fields in a contract:
`(t/map {:req {:id string?} :opt {:label string?} :extra :open})`. The fields in
`:opt` may be absent, but a present `nil` still needs a nilable type. Omit
`:extra` or use `:extra :closed` to reject unlisted keys.

A parameter can describe a direct value
in a literal map type or the value type in `(s/map-of key-spec a)`. When a
literal map type has several fields with the same parameter, Parenox infers
from the first such field in key order and checks the other fields against it.

```clojure
(defn ok
  {:t/sig '(t/all [a b] [(s/fspec :args (s/cat :row a) :ret {:body b})
                        {:row a} :-> {:status number? :body b}])}
  [render result]
  (assoc (render (:row result)) :status 200))
```

Here the required `:row` field supplies `a`, and the renderer's required
`:body` field supplies `b`. A present field with value `nil` is evidence of
type `Nil`; a missing or optional field does not satisfy a required field.
Run `just map-boundary` for a storage declaration and a response handler that
keep these cases distinct. This checks the declared boundary, not a database
schema.

### Arity and argument spreading

For a multi-arity definition, write one signature per clause in a
quoted vector or list, for example `:t/sig '[[:-> string?] [string? :-> number?]]`.
Each body is checked against its signature. Calls select the signature for their
argument count, including when the function is passed as a callback. Externs
accept the same vector of signatures as EDN data. Run `just arities` for a
multi-namespace example with arity and argument diagnostics.

A variadic signature uses `&` followed by the type of each extra argument, for
example `:t/sig '[number? & string? :-> number?]`. Clojure supplies extra arguments
as a seq, or nil when there are none. An exact fixed clause takes precedence
over a variadic clause at the same argument count. Inside the variadic body
`recur` takes one argument for the rest binding. Run `just variadic` for examples.

`apply` checks positional arguments from nil and collections with known
positions, including vector literals. It selects a signature for each exact
shape and supports inline `fnil` wrappers. For an open vector or seq, it
checks possible counts and element types against declared signatures and
supported built-ins. Applying `merge` with an open spread reports an analysis
gap because the result depends on how many maps are passed and which keys they
overwrite. Run `just spread` for examples.

Use `#(...)` in immediate calls and supported callbacks. `%` and `%1` name the
first argument, `%2` and later positions set the required count, and `%&` holds
the remaining arguments. The reader preserves the original shorthand tokens
and spans. Run `just anonymous` for examples.
Map destructuring of rest arguments reports an analysis gap.

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
An inferred helper also carries known exception data to the caller's handler.
Run `just helper-effects` to check this through a private request helper.

Core threading macros `->`, `->>`, `some->` and `some->>` are supported. The checker
resolves names before selecting a rule, so local and project functions can shadow
these names. `some->` variants stop on nil; false continues through the chain.
Ordinary threading expands each step in place, preserving evaluation order and
tail position when the input becomes part of a special form. Long chains of bare
built-ins or keyword lookups are processed iteratively. Other nested expansions
are bounded to 32 active forms and report an analysis gap at the limit.
Compiler-level resolution and arity checks still apply to skipped steps; runtime
type and missing-key errors are excluded from proven skipped paths.
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

`if-not` and `when-not` select their body when the condition is falsey.
Their guards narrow values in the selected branch and preserve tail position.

`if-let` and `when-let` bind truthy values. `if-some` and `when-some` also bind
`false`. Each initializer is checked once. The successful branch can use the
narrowed value through a local symbol or a supported map destructuring pattern.
The fallback runs in the outer scope and defaults to nil.
`when-first` enters its body for a nonempty input and binds the first element,
including nil or false. Its final expression can recur to the enclosing loop.
Known scalar literals let `=` resolve branches, including status comparisons
on each iteration of that loop. Run `just guards`
to see the result type when no fallback is supplied and a type error when
`if-some` passes `false` into a numeric call.

`merge` accepts maps and nil. Rightmost present values win, including explicit
nil. An optional right-hand field can change the value type without removing a
required left-hand field. An undeclared field in an open map can overwrite a
known value; if its type remains unknown after the merge, Parenox reports a gap.

`merge-with` checks direct calls on maps and nil. On a conflict the callback
receives the accumulated value and the next map's value in that order. An
optional conflict keeps the possible one-sided results, and a present nil or
false is a value. An overlap with an undeclared value in an open map reports an
analysis gap. The callback shares the map callback budget with `group-by`,
`update-keys` and `update-vals`. Run `just merge-with` for examples.

`select-keys` derives a closed result shape from a map or nil and a
collection of statically known keyword or string choices. It preserves field
types, including present nil and false. A field is required when the input
requires it and every possible selection includes it. Unbounded key types and
selected fields with undeclared types report analysis gaps. Run
`just map-projection` to check a public response derived from an open record.

`dissoc` tracks a removed keyword or string even when the input map is open.
A dynamic key makes overlapping required fields optional and preserves the
others. The one-argument call returns its input and removing keys from nil
returns nil. An `:opt` entry with type `#{}` can declare an absent field in
an open result contract. An `apply` spread with unknown length reports an
analysis gap.

`update-vals` checks direct calls on closed maps, typed dictionaries, and nil.
It checks the updater against each declared field's value type and preserves
key types and presence on normal returns. Present nil and false are passed to
the updater. Nil and empty maps produce an empty map with zero invocations.
A field whose updater always throws can only be absent in a successful result.
An open map with undeclared values or excessive callback analysis reports a gap.
Run `just update-vals` for examples.

`zipmap` pairs sequence items up to the shorter input and keeps the later
value for duplicate keys. Fixed positions produce precise fields. A later pair
that may be absent leaves room for an earlier value at the same key. Typed
sequences can produce a dictionary. The supported keys are keywords and strings.

`update-keys` checks direct calls on maps with finite keyword or string keys,
and on nil. The updater must produce known key choices. The returned shape
preserves the source values and tracks which destination keys are guaranteed.
Definite collisions report `PNX2790`. Possible collisions and excessive key
expansion report `PNX9001`. Run `just map-builders` for examples.

`group-by` checks the grouping callback and returns map entries whose values
are vectors of the original items. Fixed inputs with known group keys retain
the item order and length in each group. With dynamic groups, the checker
tracks a typed dictionary and the collection may be empty. Group keys must
be keywords or strings. The operation shares the map callback analysis limit
with `update-vals` and `update-keys`. Run `just group-by` for examples.

`get-in`, `assoc-in`, and `update-in` accept vectors and sequences with
known keyword or string positions, including computed paths such as
`(list sku tier)`. Paths can pass through locals or typed arguments.
Each complete conditional path is checked separately. An empty or nil read
path returns the input. A missing key uses the `get-in` fallback and a
present terminal nil or false is preserved. Writes create missing or nil intermediate maps, preserve
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

Lookup and update arguments run from left to right. The default expression in
`get` and `get-in` runs even when the key exists. `assoc` and `assoc-in`
evaluate every argument before validating the write. Run `just lookup-order` to
see which error reaches the handler and how the checker reports a mismatched
recovery type.

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
private helpers. Inline `fnil` evaluates the target and defaults when it creates
the wrapper. At invocation, it replaces nil arguments and preserves false.
For example, `(update patient :visits (fnil inc 0))` works when visits is absent.
Parenox supports one to three defaults when the wrapper is called immediately
or passed directly as a supported callback. Storing the wrapper for later use
reports an analysis gap.

Inline `(fn [params] ...)` works in immediate calls and as a supported callback.
Its parameters take their types from the call, supported map destructuring
works in the parameter vector, and the body can access outer locals.
Diagnostics inside the body point at its original expressions and include
the call location. Anonymous functions can define several fixed arities and
one variadic clause. An inline call checks the selected body with its argument
types and checks compiler constraints across the other bodies.
Local storage of the function itself, named functions, and preconditions or
postconditions report analysis gaps.
Run `just updates` to see both an implicit old-value error and a body error.

`mapv` takes one callback argument from each input collection and returns a
vector of callback results. It stops at the shortest input. `filterv` retains
elements for which the callback returns a truthy value. Core scalar predicates
narrow the retained type. `some?` removes nil and keeps false.

Both support vectors, sets, strings (as characters), closed maps,
`s/map-of` dictionaries, and nil (as empty). User-defined predicates retain the
input element type unless the callback is known to always return a falsey
value. On a guaranteed nonempty input, a callback that throws prevents the
pipeline from returning. Empty inputs skip callback invocation. Run
`just vectors pretty always` for a patient-label pipeline, recovery from a
rejected row, and callback diagnostics.

Maps and keywords work in function position and as inline callbacks, for
example `(mapv :active patients)`. Lookup defaults apply to missing entries
and preserve explicit nil and false. Vectors and map entries accept one integer
index and report known out-of-bounds calls. Run `just callables` for examples.

`list` preserves argument order and `cons` prepends to any modeled sequence.
Use `(s/coll-of element-spec :kind seq?)` for an ordered sequence that may be
empty. An empty list is truthy, while `(seq (list))` yields nil. Run
`just list-accumulators` for reversal with `loop` and a non-seqable tail diagnostic.

`reduce` checks known positions in order and supports vectors, sequences, maps,
sets, and strings. Maps supply entries, sets supply their element type, and
strings supply UTF-16 characters. Map and set inference allows any iteration
order. Without
an initial value it invokes the reducer with zero arguments on empty input and
uses the first item as the accumulator on nonempty input. With an initial value,
an empty input returns that value. Reducer diagnostics identify the failing
step and point at literal elements when possible. For typed vectors and seqs
with unknown length, Parenox follows possible accumulator types across steps
and widens collections whose length grows. Excessive shape growth reports an
analysis gap. A step returning `(reduced value)` stops that path and returns
the wrapped value. Unexecuted later steps leave its result intact. Run
`just reductions` for examples.

`reduce-kv` checks direct calls over maps with declared entry types, vectors
and nil. The reducer receives the accumulator, key and value. Map keys stay
paired with their value types and each step allows any possible entry.
Fixed vectors supply ordered integer indices starting at zero. Empty input
returns the initial value and `reduced` stops the fold. Open map entries,
protocol fallbacks and reductions beyond the analysis limits report gaps.
Run `just reduce-kv` for examples.

A closed map supplies a `MapEntry<K, V>` for each present field. Use `key`,
`val`, or fixed positional bindings such as `(fn [[k v :as entry]] ...)`.
Optional fields contribute entries only when present, and a present nil remains
nil. An entry satisfies vector contracts for its two items. Iterating an open
map, including an `s/keys` contract, reports an analysis gap because undeclared
fields can have arbitrary types. Run `just entries pretty always` for examples.

`find` returns the entry for a present key and nil for an absent one. Use
`if-let` with `find` when a fallback should apply only to missing fields.
An entry holding nil or false still takes the present branch. The checker
supports maps, vectors and map entries as targets with keyword or string
keys or known decimal integer indices in the i32 range.

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
| Control flow | `let`, `do`, `if`/`if-not`, `when`/`when-not`, `cond`, `case`, `or`, `and`, `if-let`/`when-let`, `if-some`/`when-some`, `loop`/`recur`, local predicate narrowing |
| Core macros | `->`, `->>`, `some->`, `some->>`, `cond->`, `cond->>`, simple-binding `as->` with source diagnostics |
| Data operations | Keyword calls, string and keyword lookups and updates, ordered vector paths, indexed reads, positional and nested map destructuring and checked keys |
| Functions | Fixed and variadic arities, forward calls, bare requires, namespace aliases, `:as-alias`, explicit `:refer` with `:exclude` and `:rename`, private-var boundaries, direct private-helper inference |
| Libraries | Built-in rules and EDN extern files; checked public signatures and literal values can be exported as EDN |
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

`case` narrows local values in each branch, including grouped constants. An
omitted default throws when no clause matches, and Parenox reports a definite
miss at the input expression. Supported constants are nil, booleans, characters,
strings, keywords and known decimal integers in the i32 range.

For a union of map shapes, testing a literal tag can narrow the whole map.
For example, in `(if (= :note (:kind event)) ...)`, each branch sees the
remaining variants of `event` and their own `:value` types. `cond` narrows
later clauses using earlier tests. `case` on a map tag also reports unhandled
values from a known finite set when there is no default.

Supported selectors are keyword calls, `get` with a literal key, and
`get-in` with a literal vector of keys. A `let` alias retains the relation to
the original binding until either name is rebound. Missing keys still read as
nil; `some?` also accepts false. A lookup with a default, a computed key, or a
user-defined predicate is not evidence for a tag. Run `just tagged-maps`.

`seq`, `first`, `next` and `rest` model traversal of vectors, sequences,
sets, strings and maps with declared entry types. Strings use JVM UTF-16
positions. Map traversal preserves entry types and known lengths without
assuming their order. An exhausted `rest` returns a truthy empty list. Use `seq` to
detect exhaustion and refine the remaining elements. The `empty?` and
`not-empty` guards narrow known empty and nonempty alternatives. Empty
collections, empty strings and nil satisfy `empty?`; a collection holding nil or
false still has an element. Guarding
the result of `seq` or `next` establishes that a first item exists. That item
can still be nil. A seq made from a fixed vector retains its positions for
`first`, `next`, `nth` and positional destructuring.

`keys` and `vals` preserve the key or value types of a typed map. Nil and an
empty map produce nil. A nonempty map produces a sequence even if its values
are nil or false. The checker retains known lengths and uses all possible
element types at each position. Open maps with undeclared entry types and
sequences passed directly to `keys` or `vals` report analysis gaps. Run
`just map-sequences` for a reduction over typed dictionary values.

`conj` appends to vectors, prepends to seqs, and adds items to sets. Short fixed
vectors retain their positions. Repeated growth across loop iterations widens
the collection length while keeping the inferred element types. Map `conj`
reports an analysis gap.

`throw` ends the current execution path. Ordinary function calls and vector
literals preserve that exit, while still checking whether later expressions
resolve and compile. `ex-info` retains the data map for `ex-data` and follows the
JVM's treatment of a nil data argument as an empty map.

`try` joins normal results with values from possible `catch` clauses. Explicit
throws preserve `ExceptionInfo` data for a matching handler. Catch analysis
recognizes common `java.lang` throwable classes and conservatively allows
ordinary calls to throw. The return value of `finally` is discarded. A `recur`
can target a loop or function defined inside a `try`, and Parenox diagnoses
transfers to targets outside the `try`.

Limitations include user macros, Java interop, generics, lazy sequence functions,
recursive specs, full numeric
tower semantics and some reader forms. Numeric expressions use `Number`,
with decimal integer literals retaining their value for indexed reads.
Externs describe qualified functions, literal vars and supported macro scopes; they do not load JARs or
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
