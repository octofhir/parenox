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
When a separate contract supplied the argument types of an inline callback,
`callback_contract` gives that origin alongside the contract for the failing
operation inside the callback.
For a nested argument mismatch, `mismatch_path` describes the failing field
or element. Literal maps and vectors can locate the exact value in source.
Computed arguments retain their range and report the failing path in a note.
A call with a definite argument mismatch has an unknown result type.
Signature help shows the expected arguments and declared result for that call.

A function body without a checked signature can still report a proved
argument mismatch at a call to a required library with an EDN extern.
The body retains its coverage warning. Unresolved macros and aliases
that do not load a library do not establish an evaluated call.
In a file with statically resolved `defmulti` and `defmethod` declarations,
the checker also reports definite misuse of loaded `clojure.string` functions
inside method bodies. Parameters start with unknown types. Hover can show
types proved inside the method, and the diagnostic report marks dispatch and
the rest of the body analysis as incomplete.

### Targeted suppressions

Write `;; parenox-ignore PNX2322: reason` before the next form. The reason
is required. The directive applies only to that rule and form, not siblings,
quoted data, discards, or inactive reader branches. Supported rules are
`PNX2322` (type mismatch), `PNX2741` (missing key), `PNX2801`
(out-of-bounds index), and `PNX2802` (unhandled dispatch value). Suppressions
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
Public functions need a contract for full checking. Parenox can infer calls to
supported single-arity private helpers from their arguments. A reachable typed
call establishes coverage. The checker still resolves names in dead branches.
It can also check a private function independently when the body builds
literal data from plain parameters. Recursive helpers and helpers passed
as values require a signature. Other private bodies need a reachable typed
call or a signature.

An initializer can capture the immediately preceding local `def` when its
root contains inert data. Parenox supports a bare read, vector elements, and
map values after a static namespace declaration. Public snapshots contain
the captured data, including values read from a private root. Intervening
expressions, dynamic roots, evaluated metadata, and reads in map keys or
set members leave an analysis gap.

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
- `:as-alias` also resolves auto-qualified keywords and supports navigation.
  Runtime calls through an alias-only namespace report `PNX9003` when loading
  is unconfirmed. Ordinary `ns` requires establish loading directly or through
  fully read project namespaces with unambiguous source resources. The implicit
  `clojure.core` namespace and references to the current namespace are already
  available. A `:refer` on an alias-only libspec requires proof of an earlier
  direct load in that `ns`.
- Repeating a field with the same spec in `s/keys` preserves its type and
  required presence. Different unqualified specs that resolve to the same
  field still report a collision.
- `(s/coll-of (s/tuple K V) :kind map?)` checks map entries using `K`
  for keys and `V` for values. Without `:kind`, `s/coll-of` supports
  `any?` and modeled map element shapes, including `::locations` from
  lsp4clj. For a map input, the elements are its entries. An empty map
  satisfies the element constraint. An individual map entry can itself
  satisfy the constraint when both its key and value have the required shape.
- `s/fspec` uses `s/cat` and `:ret` for callback contracts. Direct
  fixed-arity calls can supply types to `fn` and `#()` arguments.
  General callback and variadic inference remain limited.
- `t/all` supports a fixed generic arity with up to two input type
  parameters and one immediate callback result parameter. Inference uses
  arguments, not the expected return. Use `(a :< number?)` for a supported
  scalar bound. Empty collections alone do not establish an element type.
  A dictionary can supply independent key and value types through
  `(s/map-of k v)`, including for the parameters of an inline callback.
  See [generic callbacks](../examples/generic_callbacks.clj),
  [two-input inference](../examples/generic_zip.clj), and
  [typed map boundaries](../examples/map_boundary).

A successful `s/valid?` narrows supported scalars, open maps and
collection element shapes in the true branch.
For a named `s/coll-of` of modeled `s/keys :req-un` maps, `s/conform`
can convert fields recursively in a vector of known elements.
It preserves extra unqualified fields, returns `::s/invalid` when an
element fails, and tracks overflow in nested `int` calls. For a map
entry, it preserves `MapEntry` when both elements retain their JVM
identity and returns a vector when one changes. Open maps,
registry-sensitive qualified keys, unknown entry identity and other
unmodeled reconstruction cases report an analysis gap.
An approximate map shape does not prove the runtime predicate returns true.
Recursive specs, dynamic spec names, and unsupported projections report gaps.

A direct spec alias can use a target registered earlier in the same
source. A reference to a later registration reports `PNX9001` on the
alias. An existing `s/keys` map may resolve its field registrations later.
[JVM fixture](../tests/fixtures/spec-alias-order.clj)

A modeled `s/or` with named map branches returns a `MapEntry` with the
selected tag and conformed value. It tries branches in source order and
preserves exceptions from branches it visits. The input contract remains
the union of accepted maps. [JVM fixture](../tests/fixtures/spec-document-changes-entry.clj)

The `::document-changes` pattern in lsp4clj conforms a collection of
tagged entries and projects their values with `map second`. The checker
models the resulting sequence for known ordered or empty collections and
small set literals. For a nonempty set, the result preserves the possible
value types. Set traversal and deduplication can change the order and
length. Validation and conversion errors precede the lazy sequence.
[JVM fixture](../tests/fixtures/spec-document-changes.clj)

The `::changes` form uses `(s/coll-of (s/tuple string? ::edits) :kind map?)`.
On a known closed map, the checker preserves string keys and conforms
each edit collection. A key of another type rejects its entry before
its value is conformed. `::workspace-edit` can use this dictionary and
`:document-changes` in the same input. [JVM fixture](../tests/fixtures/spec-workspace-changes.clj)

As a spec predicate, core `identity` accepts truthy values and excludes
`:clojure.spec.alpha/invalid`. For a named scalar spec defined directly
with `identity`, `s/conform` returns the input on success and `s/valid?`
can narrow the input in the true branch.

A named spec of the form `(s/and string? #(= % "create"))` checks one
literal string when both predicates resolve to core functions. It preserves
the input on success and narrows `s/valid?` to the matching string. The
exported contract uses a singleton set and can be imported without the
provider source. [JVM fixture](../tests/fixtures/spec-file-kind.clj)

Use `int?` in source contracts and conditions to check for a JVM `Byte`,
`Short`, `Integer`, or `Long`. For example, `2147483648` satisfies
`int?` and `1N` satisfies only `integer?`. Parenox reports `PNX9004`
when an argument's class is required by a contract and cannot be proven.

Guarded finite sets such as `(s/and int? #{9223372036854775807})`
retain exact signed long values through checking and extern import.
An `integer?` guard also accepts equal BigInt values.
[JVM fixture](../tests/fixtures/signed-long-types.clj)

With proven fixed-precision integer operands, `+`, `-`, `*`, `inc`, and
`dec` return a fixed-precision integer on success. Safe results retain their
known value or range. When overflow is possible, the checker accounts
for `ArithmeticException` without assuming the compiler's unchecked
math setting. [JVM fixture](../tests/fixtures/fixed-arithmetic.clj)

For `(s/and integer? (s/conformer int))` with the core functions,
`integer?` accepts Clojure integers, including BigInt. A successful input
must fit a signed 32-bit integer and can have any integral storage class.
The normal result is a JVM `Integer` and satisfies `int?`.
In a direct `s/conform` on this named scalar spec, a non-integer returns
`:clojure.spec.alpha/invalid` and overflow throws. `s/valid?` distinguishes
true, false, and an exception. Arbitrary conformers and user predicates
remain outside the modeled cases. See the
[JVM fixture](../tests/fixtures/spec-int-conformer.clj).

`(s/conformer name)` and the core `name` call preserve String values and
extract the name of an exact keyword. A `Symbol` can return String or nil.
A general JVM `Named` implementation can return either value or throw.
For a loaded registration, `s/valid?` returns true on a normal result and
propagates exceptions, including `NullPointerException` for nil input.
The unrestricted conformer has no static input projection. Function
contracts and extern snapshots can declare an explicit supported input
domain such as `(s/or :text string? :key keyword?)`.

Named specs can use the same literal enum map as a predicate and a
conformer. In `(s/and int? event-types (s/conformer event-types))`,
`event-types` must be a uniquely defined, non-dynamic map declared earlier
in the same file. The checker supports up to 32 signed 32-bit decimal
keys and keyword, nil or false values. A mapped keyword other than
`:clojure.spec.alpha/invalid` is a successful result. On a successful
`s/valid?`, the original value narrows to one of the accepted integer
keys. Direct `s/conform` returns the mapped keyword or the invalid marker.
Exported EDN uses a guarded set such as
`(clojure.spec.alpha/and clojure.core/int? #{1 2 3})`. The guard
preserves the JVM integer-class restriction when importing the contract.
For example, the imported contract accepts `1` and rejects `1N`.

For the two-arm `:code-action/kind` spec in lsp4clj, the checker models
the local keyword-to-string table and the `string?` branch. A mapped
keyword produces its table value. String input keeps its identity.
The accepted input types also survive export and source-free import.
[JVM fixture](../tests/fixtures/spec-code-action-kind.clj)

For a local `clojure.set/map-invert` of a prior literal keyword-to-code
table, `(s/and integer? table (s/conformer table))` models the inverse
spec with a finite, class-independent input domain. The table needs
unique signed 32-bit decimal codes and an explicit load of `clojure.set`.
An exported `(s/and integer? #{1 2})` accepts `1` and `1N` as the same
code. The inverse var itself exports as an inert map, including any
entries whose value is the spec invalid marker.

### EDN externs

Externs are trusted, inert declarations for qualified names, not
implementations:

```edn
{vendor.store/find [string? :-> (clojure.spec.alpha/nilable map?)]
 vendor.uri/decode (t/nil-preserving [string? :-> string?])
 vendor.store/default-limit {:value 50}
 vendor.bindings/with-bindings {:macro :let-like}}
```

A checked source declaration takes precedence. Duplicate externs or attempts
to replace a modeled built-in report `PNX2790`. Literal values describe the
initializer, not later runtime changes. A `:let-like` description supplies
lexical scope for sequential bindings, not the macro's result, effects, or
tail positions; such calls still report `PNX9009`.
Known source macros with unknown scope report `PNX9001` at the call.
Their arguments are treated as syntax until a supported scope describes
how the macro binds them.

Use `(t/nil-preserving [Arg :-> Result])` in an extern for a unary function
whose result is Nil exactly when its input is Nil. Write the non-Nil input
and result types in the vector. A call with a proven non-Nil argument returns
the written result type. A call with Nil returns Nil. A call whose argument
can be either retains the nullable signature. Extern validation requires
evidence of the relation between the input and result.

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

### Proposing named spec contracts

`parenox propose-contracts` reads named `s/def` forms and writes EDN to
stdout. Each proposal includes the original form and its location, the input
domain, the result of `s/conform`, and a list of unknown parts. `Input (Any)`
binds the value named `Input` in the result type, including when the value is
`nil`, `false`, or `:clojure.spec.alpha/invalid`.
Use `t/any` in a type signature when the slot accepts every Clojure value.

The `:externs` map contains the generated entries whose conversions
round-trip independently of the source. Use `--externs-only` to print this
map directly. Load it with `check --externs` or `:externs` in `parenox.edn`.
The command marks resolved entries already supplied by an extern file as
`:existing` and leaves them in that file. Proposals whose conversions cannot
be exported have `:extern nil`.

Named spec externs use a qualified keyword and a map with `:spec` and `:ns`.
`:ns` names the namespace that registers the spec. The consumer needs an
explicit `:require` or `:use` of that namespace for `s/conform` or `s/valid?`.
Parenox also uses the input domain in source function signatures. A source
registration or a source file providing the declared namespace takes
precedence. References to named spec externs from
another `s/def` or from signatures inside extern files remain outside this
workflow. The current exporter supports strings, finite sets of strings, and
single-field map conformers. The conditional forms cover `when (vector? input)`
and the signature-help pattern that passes through maps after testing for
vectors. Literal maps retain their shape in the return type. A general map
type includes both results because a JVM object can implement both interfaces.

Guarded map projections also cover the semantic-tokens pattern. It tests
keyword lookups for truthiness and then constructs a nested map from further
lookups and boolean conversions. Repeated reads from a literal map retain
the types established by the guard. For opaque inputs, each lookup can
return a different value or throw. The proposal describes the normal result
and marks the accepted input domain and exception classes as unknown.
Its EDN still supports checking calls to `s/conform` and `s/valid?`.
Using that spec as a function argument type reports an unchecked input.

A map projection can also contain a lookup and fallback that read the same
Var. The text-document-sync proposal includes the adjacent literal initializer
in `:initial-table`. The initializer and spec must follow the namespace
declaration. The EDN contract reads the qualified Var in both places
and preserves Clojure truthiness. A root change can alter the lookup or fallback
result, so the runtime value and exception classes remain unknown.

`s/keys` with direct child specs declared in the same source file and runtime
namespace can be exported as a bundle. Selecting the parent adds its referenced
specs to the generated EDN. Generation reuses matching contracts
from files passed with `--externs`. Pass the generated and reused files to the
consumer. Import resolves children across all supplied EDN files. Missing or
conflicting children prevent checking the parent. Each child must have the
same runtime `:ns` as the parent, and the consumer must load that namespace.

Bundles can contain nested `s/keys` and direct keyword aliases. A source
alias must follow its target registration in the same file. The exported
alias keeps its target keyword, as in `{:spec :wire/position :ns wire}`.
Import resolves the graph from all supplied entries before using it. Cycles,
missing targets and graphs exceeding the supported bounds report a gap.
To evaluate the forms on a JVM, register alias targets before the aliases.

Imported bundles check literal maps with known keyword fields at each level.
Additional qualified keys use their loaded registry definitions. The checker reports
an analysis gap when the registry entry is unavailable, or when the input
may be an opaque map implementation. Required keys are tested before
field conversion. Child specs with unchecked inputs retain that limitation
when the parent is used as an input type.

The lsp4clj `position` bundle reuses the checked `int` conversion for `line`
and `character`. The successful input range is signed `Int32`, and `int`
returns a JVM `Integer`. For standard JVM integer classes, exceeding the
signed `int` range throws `ArithmeticException` if the value fits a `long`.
Exceeding the signed `long` range throws `IllegalArgumentException`.
Non-integer input returns the spec invalid marker, or `false` from `s/valid?`.
Proposal result types describe return values. The exported form retains the
conversion and its exception behavior. Broad arbitrary-width input types
allow additional exceptions from custom JVM numeric methods.
For imported integer conversions, `s/valid?` alone gives no range guarantee
about the original integers. Virtual conversion methods can return a different
numeric value.

The lsp4clj `:file-event/type` contract converts the fixed integers `1`, `2`
and `3` to `:created`, `:changed` and `:deleted`. It preserves the `int?`
guard, so an equal arbitrary-precision integer such as `1N` is invalid.
The exporter supports 1 to 32 distinct decimal `Int32` keys with literal
keyword values. Auto-resolved keywords and `:clojure.spec.alpha/invalid`
are outside this form. The predicate and conformer must use identical
literal tables or a single literal table from the immediately preceding
`def`. The JVM captures the table when it registers the spec. The exported
form inlines it in both stages and can be imported alone or inside the
`did-change-watched-files-params` bundle.

The default `s/coll-of` form with a named map spec can also be exported.
The source element spec must be registered earlier in the same file. The
spec on the JVM resolves the element on first use and caches the outcome.
Parenox checks imported calls on proven literal vectors of up to 32 elements.
It evaluates vector expressions before checking elements in encounter order.
Conformance stops at an invalid element or an exception. The normal result
preserves the vector shape and each converted element. Other collection
shapes can report an analysis gap.
This also works for a literal vector in a literal map field, such as `:edits`
in the lsp4clj `text-document-edit` spec. At every map level the checker
requires proof of an ordinary map and checks required keys before values.
When traversal order is unknown, different map fields can contribute both
an invalid result and a possible exception.

The lsp4clj `changes` spec also exports the map form
`(s/coll-of (s/tuple string? ::edits) :kind map?)`. For a literal map with
known keys, Parenox checks each key before its value and preserves each
URI in the output alongside its conformed vector of edits. A rejected key
skips its own value's spec. Another entry may already have thrown an
exception. The imported contract checks literal maps and bounded literal
vectors. Other collection implementations can report an analysis gap.

Run the lsp4clj 2.0.1 examples with the pinned `coercer.clj` source.

```sh
coercer=$(mktemp -d)/coercer.clj
curl -fsSL https://raw.githubusercontent.com/clojure-lsp/lsp4clj/1ff6c687f4e2af214d30df0d50749ea4a15b42c8/src/lsp4clj/coercer.clj -o "$coercer"
just build
target/release/parenox propose-contracts "$coercer" \
  --spec :server-capabilities/code-lens-provider \
  --spec :server-capabilities/signature-help-provider \
  --spec :server-capabilities/semantic-tokens-provider \
  --spec :server-capabilities/text-document-sync \
  --spec :lsp4clj.coercer/server-capabilities \
  --spec :lsp4clj.coercer/position \
  --spec :lsp4clj.coercer/range \
  --spec :lsp4clj.coercer/additional-text-edits \
  --spec :lsp4clj.coercer/text-document-edit \
  --spec :lsp4clj.coercer/changes \
  --spec :file-event/type \
  --spec :lsp4clj.coercer/did-change-watched-files-params \
  --spec :markup/kind
just pilot-contract-smoke "$coercer"
clojure -M tests/fixtures/spec-code-lens-provider.clj
clojure -M tests/fixtures/spec-conditional-provider.clj
clojure -M tests/fixtures/spec-signature-help-provider.clj
clojure -M tests/fixtures/spec-semantic-tokens-provider.clj
clojure -M tests/fixtures/spec-text-document-sync.clj
clojure -M tests/fixtures/spec-server-capabilities.clj
clojure -M tests/fixtures/spec-position.clj
clojure -M tests/fixtures/spec-range.clj
clojure -M tests/fixtures/spec-additional-text-edits.clj
clojure -M tests/fixtures/spec-text-document-edit.clj
clojure -M tests/fixtures/spec-workspace-changes.clj
clojure -M tests/fixtures/spec-enum-conformer.clj
```

The smoke test imports generated EDN in a consumer with no provider source.
It checks the exact output field type of the lens conformer, both branches
of the vector conformers, and the signature-help map and vector inputs. It
checks `s/conform` and `s/valid?` on the capabilities bundle and a type error
for `"html"` in place of `"markdown"`. In the editor, hover on a named spec
declaration or a resolved type reference shows its inferred contract and source
location.

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
| Flow | Lexical scopes, conditionals, literal `case` and `condp =`, `loop`/`recur`, `try`/`catch`, core threading, and selected predicates |
| Maps | Presence, required/optional fields, open and closed shapes, dictionaries, stable lookup narrowing, and supported reads, updates, and projections |
| Collections | Vectors, sets, map entries, sequences, positional destructuring, and selected calls including `mapv`, `filterv`, `some`, `every?`, `reduce`, and `reduce-kv` |
| JVM | Selected regex and string operations and known exception classes, not general Java interop |
| Unannotated code | Source name resolution, known arities, selected string checks, and argument mismatches against explicit externs |

Clojure truthiness distinguishes nil and false from zero and empty
collections. A missing key is not a present nil or false. `get` and map
destructuring defaults apply only to absence, and their default expressions
are evaluated eagerly. Clojure 1.13 checked lookups such as `req!` and
`:keys!` require presence but preserve the field's value type. Positional
bindings fill missing positions with nil; `[head & tail :as whole]`
keeps the original collection in `whole` and a remaining seq or nil in
`tail`. See `just checked`, `just defaults`, and `just presence`.

The core `map?` predicate recognizes modeled persistent maps and narrows
an unknown local or a known field to a map in the true branch. A general
numeric type can overlap with `IPersistentMap` through a JVM subclass.
Those numeric values keep both branches possible.

`for` and `doseq` check seqable inputs, sequential bindings, destructuring,
and `:let`, `:when`, and `:while`. `for` evaluates the first source expression
immediately but defers traversal and the body. `doseq` is eager and normally
returns nil; with no bindings or only `:let` clauses it returns the last body
expression. `some->` and `some->>` skip the remaining steps on nil and pass
false through. A stable read of the original input keeps the non-nil type
in later steps. A standard `seq` guard narrows a stable local or map lookup by
emptiness. `when-first` makes the same nonempty refinement inside its body
and binds the first element, which can itself be nil or false.
A tuple spec checks indexed positions. Sequence and reduction operations
require evidence of ordinary vector behavior before they use those positions.
For a structural tuple with unconfirmed traversal, Parenox reports a coverage
gap and withholds exact callback types. Indexed reads retain the tuple contract.
In a direct map destructuring binding, Clojure 1.13 evaluates the `:or`
defaults before the value to destructure. A default can run even when its
key is present or unused. A proven runtime failure in a default or initializer
leaves later bindings and the body subject to compiler checks.
`recur` must target a supported tail position and analysis is
bounded when argument types keep growing.

When `if` chooses between two inline functions passed to `reduce`,
`some` or `every?`, the collection operation keeps its chosen callback throughout.
The callback sees the branch's types for captured locals and receives the
inferred element type. A reducer also sees the evolving accumulator type.
`some` returns the first truthy callback result, or nil if every result is
falsey or the collection is empty. On sequences with known positions, the
checker stops visiting elements once the result is guaranteed truthy.
`every?` returns false on the first falsey result and true when it
reaches the end. Its callback sees the inferred collection element type.
An inline `fn` saved in `let` can specialize in direct calls and supported
collection operations, including a checked callback contract. It retains
the lexical bindings from its definition. A named local `fn` can call itself
through its lexical name, including across supported arities. Its parameters
and inner bindings can shadow that name. Recursive analysis is bounded.
Using that function as data leaves a coverage gap.
`partial` can supply a callback to modeled calls such as `mapv` and `reduce`.
Its saved arguments evaluate at construction and are checked against the
target when it runs. A call to `partial` with just the target keeps that value.
`comp` supplies a callback to modeled calls and can be called directly.
It evaluates its functions in written order. At invocation, the rightmost
function receives all the arguments and each remaining function receives
the previous result, including nil or false.
`juxt` also evaluates its functions in written order. It invokes each one
with the original arguments in that same order and returns their results
in a vector. With a bounded number of functions, each position keeps its
own result type.

Vector `:pre` conditions are checked after parameter binding and on each
`recur` iteration. Vector `:post` conditions run after a normal return, with
`%` bound to the body's value. Successful assertions can refine later
conditions. The body is checked with the original bindings because
`*assert*` can elide preconditions and postconditions at compile time.
A failed assertion can throw `AssertionError`. Even an empty `:post`
vector removes the function body's tail position. The first body map takes
precedence over parameter metadata when a body form follows it.

Literal `case` branches compare integer values across fixed and arbitrary
precision classes within the JVM signed `long` range. A selected branch keeps
the subject's numeric class and can refine a known field or local. `nth`
accepts exact integer indices in the signed `int` range across those classes.
`count` returns a JVM fixed-precision integer in the signed `int` range.
It keeps exact sizes for tuple shapes and map entries, and counts UTF-16
code units in literal strings. Counting an open sequence can realize
values and throw.

Standard `condp =` checks literal nil, boolean, character, string, keyword,
and quoted symbol tests with a known subject of those types. Matching clauses
narrow a local or stable map selector in the result expression. Tests run in
source order and the first match wins. Dynamic predicates, numeric or computed
tests, unknown subjects, and `:>>` clauses report incomplete analysis.

Standard `apply str` can consume one `map` expression over a proven ordinary
finite vector or nil when the mapper is `str` or `partial` applied to `str`.
Captured arguments are evaluated before the collection. The consumer can keep
the exact joined text when the mapped strings are known.

Ordinary call arguments, vector literal elements and bound arguments to
standard `partial` are evaluated in source order. After a proven runtime failure,
later expressions still get compiler checks such as name resolution. Their
runtime types and diagnostics are withheld, including when a vector literal
supplies `apply` arguments.
Standard `subs` and `deref` report wrong arity once their operands can return.
A reached arity failure participates in `catch clojure.lang.ArityException`.
With a fixed or generic callback contract, a function literal evaluates its
metadata at the argument position. Inline callees, prepared callbacks and
`let` initializers follow the same evaluation order. Contextual inference
runs after the eager operands can return. Compiler checks still cover the
closure when the call is unreachable.
Sequential bodies apply the same rule to later forms and to function
postconditions. A proven failure in `finally` also prevents a normal result
from the enclosing `try`.
Conditional expressions combine the types and collection facts of branches
that can return a value. A proven runtime failure in the condition or dispatch
expression leaves the results unavailable and still allows compiler checks.
Vector metadata is checked after the elements, including on `apply` spreads.
The checker supports up to eight literal keyword keys in both vector and
callback metadata. Wrappers merge before their values are evaluated, with the
outer value taking precedence for a shared key. Hover shows flow-narrowed
local types at evaluated metadata values.

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

`clojure.string/upper-case`, `lower-case`, and `capitalize` use the
value's JVM `toString` method. The checker accepts proven non-nil
arguments and reports nil as an invalid input. `false` is a non-nil
value. `clojure.string/blank?` accepts String, nil, and false in the
modeled subset. A false result implies the argument is truthy. `trim`
requires a String in the modeled subset.

A literal Boolean assignment to the core `*warn-on-reflection*` var in
the opening block after a statically resolved `ns` is supported. The
checker leaves `*assert*`, `*unchecked-math*`, and computed or unresolved
assignments as coverage gaps.

Source checking analyzes standard `clojure.test/deftest` bodies containing
`is` assertions and nested `testing` groups with literal string descriptions.
Modeled assertions include scalar literals and selected core predicates.
Operands use the same contracts and source diagnostics as checked function
bodies. The runtime test command reports assertion outcomes. Computed
messages, custom assertion dispatch, `are`, and exception assertions remain
coverage gaps.

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
  highlights, document outline, and workspace symbols. Closed files contribute
  declarations and usages. Opaque macro expansions are not indexed.
- **Diagnostic locations** link to the `:t/sig` annotation, `s/fdef`
  registration or EDN key that supplied a contract or callback type when the
  client supports related information. Source links follow unsaved edits.
  Extern links use the analysed EDN file on disk and are hidden while an open
  editor for that extern contains unsaved changes.
- **Hover** shows the var's docstring and argument lists from project source
  or a local dependency JAR. For `defmulti`, a literal `fn` dispatcher
  supplies argument lists unless the author specifies `:arglists`. Checked
  signatures and their origin appear when available. Function parameters and
  local bindings show the type inferred
  when they receive a value. Reads show the type proved at that occurrence,
  including narrowing in a branch. Markdown is used when the client prefers it.
  A keyword used for a lookup shows the result type at that occurrence,
  including keyword calls, `get`, `req!`, and each segment of an inlined
  two-argument `get-in` with a literal path. With a default argument, the
  final path keyword shows the result of the whole lookup.
  A keyword passed as a callback shows the combined type of its reached
  lookup results. An empty collection or an earlier stop can leave it
  uninvoked.
  Contextual types inside local functions require evidence about how they
  are called. A use outside checked calls can withhold these types in
  that function, its captured local functions, and its peers in a
  `letfn` group.
  A reader error later in the buffer can leave local types available in
  preceding complete functions. These prefix types are scoped to queries
  in that buffer. A named spec defined in the same file needs a complete
  source snapshot for registration.
- **Inlay hints** show compact inferred types beside local bindings in checked
  function clauses. They follow the editor's requested range and the current
  buffer. A type hint requires a reachable binding and enough type evidence.
  Functions without signatures can show proven local types with unknown input
  types. Their extern references require a loaded provider, and any available
  source arities must support the extern contract. The query leaves out
  unannotated clauses that exceed its analysis budget.
  A reached simple binding with a direct literal initializer can retain its
  hint through gaps in other parts of an unannotated clause. Other hints use
  clauses whose body check found no error or coverage gap.
  Proven local types can help explain a mismatch with the declared return type.
  Complete functions before a reader error can still show hints in their
  checked prefix. The server requests a refresh from clients that support it
  after a saved extern changes. Editor changes and disk reloads also trigger
  a refresh when they affect a public function signature, a verified guard,
  or the availability of an alias for an open buffer in the project.
- **ClojureDocs** downloads the public compact EDN export over HTTPS in
  the background when the cache is missing, invalid, or at least 24 hours old. The
  server uses the last valid snapshot while a refresh runs or fails. The
  cache is `parenox/clojuredocs.edn` under `$XDG_CACHE_HOME` when set to an
  absolute path, `~/Library/Caches` on macOS, `~/.cache` on other Unix systems,
  or `%LOCALAPPDATA%` on Windows. Use `parenox lsp --clojuredocs-offline`
  to use the cache without downloading, or `parenox lsp --clojuredocs FILE`
  to pin a local compact EDN export. Hover adds examples, related symbols
  and notes for a resolved var. The request
  `clojure/clojuredocs/raw` takes an object such as
  `{"symName":"map","symNs":"clojure.core"}` and returns the entry or
  `null` when no entry is available. [Export downloads](https://github.com/clojure-emacs/clojuredocs-export-edn#exported-data)
- **Project completion** suggests namespace names in `:require` and public
  vars in `:refer` or after an alias such as `provider/`. The index includes
  closed files and unsaved edits. Try hovering `provider/count-offset` and
  completing the `demo.arities.provider` libspec in
  [the arity example](../examples/arities/app.clj).
- **Local completion** suggests visible parameters and bindings in supported
  expression positions, including function operators. It follows the order of
  `let` bindings and the scope of conditional bindings. A local that shadows a
  core var appears once as a plain name. Missing closing delimiters at EOF can
  be supplied for the query. Completion replaces the whole written symbol.
  In complete forms, a candidate can show its inferred type at the cursor,
  including narrowing inside a branch. Details combine types when the checker
  visits the same occurrence more than once. Names remain available when a
  type cannot be determined.
- **Keyword value completion** suggests literal values from the declared
  argument type of a checked call. Type `:` or a prefix such as
  `:fr` to complete the keyword in place. Threaded calls use the parameter
  supplied by the written argument. Inside plain map arguments, completion
  shows known required and optional keys and finite keyword values for the
  field being edited. Keys already present elsewhere in the map are omitted.
  The query can close open delimiters at EOF or supply a temporary value for
  the last key in a plain map. Reader diagnostics describe the current buffer.
- **Dependency completion** suggests public Clojure vars when typing an
  alias such as `str/` from `[clojure.string :as str]`. It reads source in
  cached JARs for direct `deps.edn` dependencies and selected aliases.
  Aliases from a literal top-level `(require '[clojure.string :as str])`
  are available after the call, including inside an unfinished form.
  For a resolved dependency graph, run `clojure -Spath > classpath.txt`
  in the project and start `parenox lsp --classpath classpath.txt`.
  [Clojure CLI reference](https://clojure.org/reference/clojure_cli)
- **Java completion** suggests public top-level classes in an `:import`
  clause. Use a package-qualified prefix such as `java.util.UU` or a short
  prefix in a group such as `[java.util UU]`. Incomplete import groups
  work in unsaved buffers. `UUID/` suggests public static members and
  `UUID/.` suggests public instance methods, including inherited methods.
  Methods show overload signatures. Parenox reads class files in dependency
  JARs, classpath directories and the local JDK's JMOD archives or `rt.jar`.
  Set `JAVA_HOME` when the JDK is outside the editor's PATH.
  See [the Java completion example](../examples/java_import_completion.clj)
  and [Clojure's Java interop reference](https://clojure.org/reference/java_interop#_member_access).
- **Signature Help** shows parameters, docstrings, overloads and the active
  argument for calls to source functions. For a resolved `defmulti` with a
  literal `fn` dispatcher, it uses the dispatcher's parameter lists.
  Checked calls can specialize generic parameters from the actual arguments,
  including in threading
  forms. The tooltip includes the result type and the contract's origin.
  Complete functions before a reader error can still show specialized calls.
  Calls with conflicting specializations show the declaration. Source
  arglists and qualified checked declarations are also available when
  missing closing delimiters can be recovered at the end of the file.
- **Go to Declaration** opens the local alias or refer clause for a source
  usage. **Go to Definition** opens the provider. Invoke it on a namespace
  name or an `:as` or `:as-alias` binding in `:require` to jump to the
  namespace declaration. Prefix groups such as `(my.app [routes :as r])`
  resolve the child `routes` to `my.app.routes`. If two sources declare
  that namespace, the location is ambiguous. **Go to Implementation**
  opens source `defmethod` registrations for a resolved `defmulti`,
  and method definitions for a resolved protocol method in `defrecord`,
  `deftype`, `extend-type`, `extend-protocol` and `reify` in recognized
  expression bodies.
- **Go to Definition** also follows a quoted library name, alias or
  `:refer` name in a completed top-level `(require ...)` form. Later
  uses follow bindings introduced by calls above them. A lexical binding
  takes precedence for an unqualified name.
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
- **Go to Definition** also opens public vars and namespace declarations
  in local JAR sources from the selected classpath. It follows `:as`,
  `:refer` and qualified names. Source definitions and lexical bindings
  take precedence. Archive locations use `jar:` URIs and can be opened
  through `clojure/dependencyContents`.
- **Go to Definition** on an imported class, an evaluated class name,
  the class qualifier in `UUID/randomUUID` or `UUID/.toString`, or a
  constructor class in `(UUID. ...)` or `(new UUID ...)` opens the
  Java file at its start. Local bindings take precedence for evaluated
  names such as the first argument to `instance?`.
  The winning class must be public, top-level and backed by a local JDK
  module with source in `lib/src.zip`. The editor opens its `jar:` URI through
  `clojure/dependencyContents`. The location identifies the class source.
- **Generate s/fdef** works on supported top-level, single-arity `.clj`
  functions. Unknown types produce a commented draft for review; existing
  contracts are not replaced.
- **Change nested if to cond** follows the `else` chain and leaves a
  nested `if` in a result branch intact. Use the code action or invoke
  `if->cond-refactor` with arguments `[uri, line, character]` near the
  opening `if`. The edit retains comments and reader discards and
  qualifies `clojure.core/cond` when the local spelling is shadowed.
- **Change cond to nested if** rewrites core `cond` and
  `clojure.core/cond` with complete test and result pairs. An empty
  `cond` becomes `nil`. Use the code action or `cond->if-refactor`
  with arguments `[uri, line, character]` near the opening `cond`.
- **Remove one element from get/get-in** exposes the first keyword or
  quoted symbol in a two-argument `get-in` with a literal vector path,
  or turns a direct core `get` of the key into a call to that key.
  Invoke `get-in-less` with `[uri, line, character]` at the opening
  parenthesis. The edit preserves comments, reader discards and any
  default on direct `get` calls.
- **Unwind whole get/get-in** exposes all callable literal keys in a
  two-argument `get-in` with a vector path. Invoke `get-in-none` with
  `[uri, line, character]` at the opening parenthesis. `get-in-all`
  can fold the resulting quoted key calls back into the path.
- **Cycle privacy** toggles top-level `defn`, `defn-`, `def` and `defonce`.
  It preserves unrelated metadata and uses the current editor snapshot.
  Invoke the code action or send `workspace/executeCommand` with
  `"command":"cycle-privacy"` and arguments `[uri, line, character]`.
  Clients advertising `workspace.applyEdit` receive an edit request.
  Edits carry the open document version for clients with `documentChanges`.
- **Change auto-resolved keyword to namespaced** switches the selected
  `::key` to `:current.namespace/key`. The reverse action changes the
  spelling back. Invoke either action or send `cycle-keyword-auto-resolve`
  with `[uri, line, character]`. Parenox uses the explicit source namespace
  and skips tokens in reader discards, ambiguous namespace contexts and
  dependency source.
- **Thread first** rewrites a call to `->` by moving its first argument
  into the initial position of the pipeline. Send `workspace/executeCommand`
  with `"command":"thread-first"` and arguments `[uri, line, character]`
  pointing to the opening parenthesis. The command uses the current buffer
  and preserves comments in supported calls. Quoted forms, reader discards,
  and calls inside existing threading forms are skipped.
- **Thread first all** follows the first argument through nested calls and
  emits the innermost step first. The code action runs `thread-first-all`
  with `[uri, line, character]` against the current buffer. Comments and
  reader discards in retained arguments stay in the replacement.
- **Thread last all** follows the last argument and builds a `->>` chain.
  The `thread-last` command moves one argument into a `->>` form.
  **Unwind thread once** and **Unwind whole thread** expand ordinary
  `->` and `->>` chains into calls, preserving comments and reader
  discards. Their editor commands are `thread-last-all`, `unwind-thread`
  and `unwind-all`. A client that rejects `workspace/applyEdit` receives
  an error with its failure reason.
- **Change coll to list, vector, map or set** keeps the text inside the
  selected collection and changes its delimiters. At an opening delimiter,
  `cycle-coll` tries the next type in the order list, map, vector, set.
  The `change-coll` command takes `[uri, line, character, type]`.
  Parenox skips replacements with reader errors on supported branches.
- **Move all expressions to get/get-in** combines nested keyword lookups,
  resolved core `get` calls and `get-in` calls with vector paths. Invoke
  the action or `get-in-all` with `[uri, line, character]` at the opening
  parenthesis. The edit preserves comments, reader discards and vector
  keys. A lookup with a default can remain the receiver of the combined
  path. In portable source the moved keys must be literals.
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

## JVM evaluation

```sh
parenox eval --project tests/fixtures/jvm-project --format json \
  --expr "(require 'demo.routing)" --expr '(demo.routing/status {:route :home})'
```

`eval` evaluates forms in order in one JVM starting in the `user` namespace.
You can supply several forms in a single `--expr` or repeat the option.
Definitions and namespace changes are visible to subsequent forms. The
project directory defaults to the current directory. Use `--alias NAME`
to select classpath and JVM options and `--clojure EXECUTABLE` to select
the Clojure CLI. The evaluator supplies its own entry point.

Stdout carries the last value as printed by Clojure. With `--format json`,
stdout carries one report. For example, the command above returns
`{"version":1,"ok":true,"value":":found"}`. The Clojure value `nil` appears
as the string `"nil"`. Program output and diagnostics go to stderr.
Exit 0 means the value was reported. Exit 2 means evaluation or launch
failed and a JSON report includes `run_error` in place of `value`.

Use `--timeout-ms 5000` to supervise an external runtime for up to five seconds.
If the deadline expires, eval returns exit 124 and JSON reports
`run_error.phase` as `execution`. Program output already written remains
on stderr. On Unix, interrupting a supervised eval with SIGINT or SIGTERM
returns 130 or 143. With redirected stdin, Parenox stops the child's
process group. With terminal stdin or on Windows, supervision stops the
direct child.

### Parenox evaluation

```sh
parenox eval --backend parenox --format json \
  --expr '(ns demo.calc) (def total 40)' \
  --expr '(let [step 2] (println "calculating") (+ total step))'
```

The built-in Rust evaluator uses the same result, log and exit-code
conventions and starts in `user`. It supports `nil`, booleans, signed
64-bit integers, strings, keywords, vector and small map literals, quoted
data, `let`, `if`, `do`, plain `ns` and top-level initialized `def`.
The initial core functions are `+`, `inc`, `str`, `keyword`,
`println` and `prn`. Repeated source arguments share the session.
Maps accept up to eight scalar keys. Earlier complete forms retain
their effects if a later form cannot be read.

The engine runs in the Parenox process and checks `--timeout-ms` between
forms. Unsupported evaluated forms report the source argument and
location. Functions, library loading, Java interop, and the `run` and
`test` commands are outside this first backend slice.
`just own-eval-smoke` compares it with the project's pinned JVM.

### Evaluation with clojurust

```sh
parenox eval --backend clojurust --cljrs /path/to/cljrs --src-path src \
  --project my-app --expr "(require '[my-app.routes :as routes])" \
  --expr '(routes/status {:route :home})' --format json
```

The native backend uses the same value, log and exit-code conventions.
Repeated `--expr` arguments share one process and namespace state.
Use `--src-path` to load local libraries. Empty, comment-only and
discard-only arguments leave the preceding value in place. Evaluation
can define `-main` without invoking it.

Clojurust parses each argument before evaluating its forms. If an argument
contains a reader error, earlier forms in that argument have no effect.
A standalone reader conditional with no matching branch evaluates to
`nil` on the tested revision. Native errors report the underlying message
when available.

## Run an application

```sh
parenox run --project my-app --main my-app.main -- argument
parenox run --project my-app --alias dev -- argument
parenox run --project my-app script.clj -- argument
```

The JVM backend is the default. It runs the Clojure CLI in the project
directory, inherits stdin, stdout and stderr, and forwards the program's
status. Relative script paths are resolved from that directory. Arguments
after `--` go to the program. Repeat `--alias` to pass aliases in order.
Clojure CLI applies the aliases' `:main-opts` as usual, so an alias can
supply the entry point. [Clojure CLI reference](https://clojure.org/reference/clojure_cli)

`--timeout-ms` supervises the child and returns 124 if the time expires.
On Unix, SIGINT and SIGTERM during a supervised run terminate and reap the
child and return 130 and 143 respectively. With redirected input, Parenox
signals the child's process group. Terminal runs keep the child in the
foreground process group for interactive input.

### Portable scripts with clojurust

```sh
parenox run --backend clojurust --cljrs /path/to/cljrs \
  --project tests/fixtures/portable-runtime --src-path src summary.cljc

parenox run --backend clojurust --cljrs /path/to/cljrs \
  --project my-app --src-path src --main my-app.main -- argument

parenox run --backend clojurust --cljrs /path/to/cljrs \
  --project my-app --deps-paths --alias local --main my-app.main -- argument
```

Clojurust is an external executable. Parenox uses `cljrs` from PATH by
default and has tested revision `793ad97e`. The backend accepts `.cljc`
and `.cljrs` scripts and loads namespaces from explicit `--src-path`
directories. It runs without Java or a project manifest. Use `:cljrs`
for a reader conditional branch specific to this backend.
`--deps-paths` selects existing local source directories from the
project's `deps.edn` for native `run`, `eval` and `test`. Repeat `--alias`
to select path settings in order. This includes `:extra-paths`,
`:replace-paths` and alias path references. Choose either `--deps-paths`
or explicit `--src-path` roots for an invocation.

Scripts run their top-level forms. If they define `-main` in the final
namespace, clojurust calls it with the arguments after `--` and awaits an
asynchronous entry point. With `--main`, Parenox searches the selected
source roots in order, looking for `.cljrs` before `.cljc` in each root.
The selected file must declare the requested namespace and an active,
direct `-main`. Use a script path for entry points generated by a macro.
The JVM backend applies Clojure CLI dependency and launch settings and
supports Java interop.
See the tested behavior and differences in the
[portable runtime trial](design/type-checker-research.md#portable-runtime-trial).

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

Pass `--timeout-ms 5000` to stop a test run after five seconds on either
backend. A timeout exits 124 and preserves logs already written. JSON
reports an incomplete run with phase `execution` and zero summary counts.
With redirected stdin on Unix, supervision stops the runtime's process
group. Text and JSON keep their usual stream conventions.

### Native tests with clojurust

```sh
parenox test --project my-app --backend clojurust --cljrs /path/to/cljrs \
  --src-path src --src-path test my-app.routes-test
```

Pass one or more test namespaces and repeat `--src-path` for their local
source roots. `--discover` finds `*_test.cljc` and `*_test.cljrs` in those
explicit roots using the native reader branches. In text mode Parenox
forwards stdout, stderr and the clojurust exit status. On the pinned
backend, a missing test namespace exits 1 in this mode.

Use `--format json` for versioned counts and findings. Test logs go to
stderr. Tests that fail or raise errors exit 1. A failed namespace load
or a selection with zero tests exits 2 and includes `run_error`. Native
findings can include the test name and assertion or error text. Source
positions and exception classes are absent from native findings.
Repeat `--var NAMESPACE/NAME` to select specific tests in either format.
The runner loads all selected namespaces before checking those vars. It
runs private test vars owned by their namespace, and reports an invalid
selection for ordinary, missing or referred vars. `use-fixtures` is
available on the JVM backend.
`just native-run-smoke /path/to/cljrs` exercises both report formats.
`just native-medley-smoke /path/to/medley /path/to/cljrs` compares a
portable application and its JSON test report byte for byte on both
runtimes. It reads Medley revision `54f45abc16ecd30121d05ef52014af8c65d4635f`
from the local Git checkout and starts clojurust with a clean environment
and an empty executable search path. The full Medley test namespace runs
on the JVM. Its test macro import has reader branches for `:clj` and
`:cljs`, which leaves `deftest` unbound on the pinned clojurust build.

## Local nREPL

```sh
parenox nrepl --port 7888
parenox nrepl --port 7888 --eval '(def total 40)' --eval '(+ total 2)'
parenox nrepl --port 7888 --load-file src/my_app/routes.clj --eval '(my-app.routes/status {})'
printf '%s\n' '(+ total 2)' | parenox nrepl --port 7888 --eval '(def total 40)' --eval-stdin
```

The client only connects explicitly to an existing server on `127.0.0.1`.
With no load or eval request it lists supported operations. Otherwise it
clones a session, loads saved files in order, runs the `--eval` expressions,
and evaluates the UTF-8 source from `--eval-stdin` last. An editor or agent
can pipe a selection to `--eval-stdin` and close the input. The client stops
after a load or eval error and closes the session. JSON separates values,
`out`, `err`, exceptions, and completion in `loads` and `evals`.
Each result's optional `namespace` contains the last nonempty `ns` field
the server reported during that operation.
Exit 1 means a load or eval error. Exit 2 means a client or protocol error.
`--timeout-ms` changes the per-operation timeout (default 5000 ms).
After an `eval` or `load-file` timeout, Parenox keeps the values and output
already read. It opens a separate loopback connection to interrupt the
request when the server advertises `interrupt`, then asks the server to
close the session. Each cleanup operation has a one-second limit.
`session_closed` records whether the server confirmed closure. The server
determines whether it can stop the evaluation thread.
Opening or checking sources never starts or connects to nREPL.
`just nrepl-runtime-smoke /path/to/nrepl-1.5.2.jar /path/to/java /path/to/cljrs`
starts both pinned servers on loopback for a namespace and session smoke test.

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
  a var's qualified `name`, display `signatures`, and available `contract`,
  `documentation` and `arglists`. An unresolved reference returns `null`.
  Use `parenox definition --file FILE --offset N` for a disk-backed query.

Results include `ok`, `complete`, diagnostics, and statistics for parsed
and reused files and checked and reused functions. The in-memory session
reuses unaffected results on edits; there is no on-disk cache.

## Performance and development

- `parenox-frontend` owns the reader, forms and CST. `parenox-analysis`
  owns source contracts, type checking and project sessions. `parenox-types`
  owns type representations, compatibility and specialization. `parenox-formatter`
  uses the frontend's CST. `parenox-classpath` reads local dependency
  archives. `parenox-lsp` owns protocol, text coordinates and editor features.
  `parenox-runtime` owns the Parenox, JVM and clojurust backends.
  The root package wires the CLI, disk projects and process supervision.
- `just verify` runs formatting checks, Clippy, and tests.
- `just agent-loop` measures the clinic edit scenario; `just bench` runs a
  synthetic workload. `parenox bench FILE` measures in-process checks.
- `just runtime-smoke` uses Clojure/JVM. Run
  `clojure -M tests/fixtures/spec-int-conformer.clj` and
  `clojure -M tests/fixtures/spec-enum-conformer.clj` to verify the
  conformers against the pinned JVM reference.

See [research and pilot results](design/type-checker-research.md),
[benchmarks](../benchmarks), and the [next development prompt](next-prompt.md).
