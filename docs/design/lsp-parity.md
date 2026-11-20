# Clojure LSP parity

The target is the full editor feature set, with the same user workflows
on normal files, unsaved edits and real projects. The reference checkout
is pinned by `just pilot-fetch`. The local executable serves as the
protocol oracle. [Clojure LSP capabilities](https://clojure-lsp.io/capabilities/)

`just lsp-parity` runs `initialize` against both executables and reports
missing providers, feature options and commands. Use
`just lsp-parity /path/to/clojure-lsp strict` to make gaps fail the check.
An advertised capability needs behavioral tests before its scope can be
called complete.

Against Clojure LSP 2026.05.05-12.58.26, all 33 advertised entries
match. Seventeen of 62 reference commands are available, `server-info`,
`cursor-info`, `cycle-privacy`, `cycle-keyword-auto-resolve`, `thread-first`, `thread-first-all`,
`thread-last`, `thread-last-all`, `unwind-thread`, `unwind-all`,
`cycle-coll`, `change-coll`, `if->cond-refactor`, `cond->if-refactor`,
`get-in-all`, `get-in-less` and `get-in-none`.

| Workflow | Covered | Work still open |
|---|---|---|
| Navigation | Source definitions and declarations, public vars and namespaces in selected JARs, public JDK class imports, qualifiers, evaluated names, constructor operands and symbolic type hints, multimethod and explicit protocol method implementations, references, signature help, source call hierarchy | Protocol-name usage search, generated implementations, Java member locations, string and array tags, opaque macro calls |
| Editing | Full and range formatting, ordinary source var and lexical rename, namespace edits on safe source file moves, local linked editing, folding and selection, privacy toggling for basic definitions, selected keyword auto-resolution, nested lookup consolidation, first and last argument chains, single-step threading and full unwinding | Directory moves, general namespace, keyword and alias rename, collection and other structural refactorings |
| Assistance | Namespace completion in `:require`, public vars in `:refer` and after aliases, source and JAR hover docstrings and arglists, cached ClojureDocs raw lookup and hover, Java import classes and static and qualified instance methods, semantic tokens, source var reference counts | Computed docstrings, nested Java class and unqualified instance-member completion, macro-generated completions, broader macro and CLJS token coverage, test-specific reference counts |
| Inspection | Server settings and counts, cursor syntax and source analysis, two inspection commands | Dependency definitions, clj-kondo analysis metadata, matching log format |
| Project views | Test trees for source, project tree for workspace and selected local JAR source, dependency contents for selected Clojure and JDK resources | Java class branches and stubs |
| Project analysis | Tracked source and overlays, static contracts, spec diagnostics | Artifact analysis for checking and navigation, ClojureScript, kondo and project lints, custom analysis integrations |

For a cross-file var rename on the same fixture, applying either server's
edits produced byte-identical source. Parenox prepares the editable part
of a qualified name and retains the actual open document version.
The `cycle-privacy` protocol probe found identical text edits for
`defn` and `defn-`, adding and removing `^:private` on `def`, and
toggling an unsaved `defn-`. Parenox used the actual open version
throughout. Stdio tests also cover rejecting an edit, acknowledging
two queued commands out of order, and applying the inline code action.
The keyword auto-resolution command matches the selected token edit in
both directions, including an exclusive UTF-16 end position and buffer
version 7. Parenox edits the selected occurrence directly. The reference
asks about other usages when it finds several. For a namespace with
metadata, Parenox uses the name symbol to construct the keyword. It skips
edits after source namespace changes, including renamed core operations.
The `get-in-all` action and command match the reference on default-free
keyword chains and compatible core calls, including UTF-16 and version 7.
Parenox retains comments and reader discards and keeps a vector-valued
key intact. Pinned Clojure confirmed the lookup order of the two-argument
vector expansion. The three-argument form can change that order when
flattened. Unknown macro argument contexts and ambiguous Clojure or
ClojureScript core bindings suppress the edit.
Quoted keyword and symbol calls use ordinary lookup and flatten with their
original key spelling. In a mixed chain an unproved bare keyword call
stays as the receiver. A fully bare keyword chain uses core `get` when
flattened, which can change dispatch for a custom JVM `IKeywordLookup`.
`get-in-less` removes one leading key from a literal vector path or
turns a direct core `get` into a call to its literal keyword or quoted
symbol. It preserves comments and reader discards. On a built-in literal
receiver the keyword form matches the reference. For an unknown JVM
receiver, a two-argument lookup emits `((quote :key) receiver)`, which
takes the same lookup path as `get` even for custom `IKeywordLookup`
objects. Direct `get` accepts a default argument. A remaining dynamic
path key is accepted only in `.clj`, where the pinned compiler's
two-argument vector expansion preserves lookup order. The stdio probe
also exercises UTF-16 ranges, version 7 and client rejection.
`get-in-none` unwinds all keys in a literal vector path of keywords or
quoted symbols. It emits quoted keyword calls for intermediate receivers,
retains reader trivia between keys and follows the reader's nesting limit.
The result can be consolidated again with `get-in-all` in an unsaved
buffer. The same versioned stdio requests match the reference when key
dispatch agrees.
The `thread-first` protocol probe matched seven text edits and twelve
refusals. The probes normalized the opening document version. Both
servers sent the edit for the unsaved buffer with version 7. The cases
include cursor placement at the opening parenthesis, unary and nested
calls, comments, and a UTF-16 range after an emoji. Parenox skips quoted
forms, reader discards, and calls inside existing threading forms. The
pinned JVM confirmed equal results for a nested call and a call in a
scope where a local binding shadows `->`.
`cargo test --locked --test lsp thread_first_all -- --include-ignored`
runs the same stdio requests against both servers. The command and
`Thread first all` action agree on the replacement and UTF-16 range for
a nested call with comments and an emoji before the opener. The test
normalizes the initial open version from 0 on Clojure LSP to 1 on
Parenox. Both send version 7 after `didChange` and return the same edit
on client acceptance. For a rejection, Parenox reports the client reason
with `-32803`, and Clojure LSP returns the proposed edit. A broader probe
matched eight edits, including cursor positions in the head and on the
closing parenthesis and reader discards in retained arguments. Parenox
leaves the seed of an existing `->` alone, where Clojure LSP splices it
into the current thread.
`cargo test --locked --test lsp thread_refactoring -- --include-ignored`
also compares `Thread last all`, `Unwind thread once` and
`Unwind whole thread`. Saved and unsaved requests matched after the same
initial version normalization. The cases include first and last threading,
a cursor inside a step, UTF-16 after an emoji, and multi-line comments.
Parenox preserves outer comments when fully unwinding a thread and skips
`some->` and `some->>`, whose `nil` propagation needs separate handling.
The JVM fixture confirmed that unwinding ordinary chains preserved their
values and that the reference's `some->` rewrite changed the `nil` case.
[Clojure LSP refactoring commands](https://clojure-lsp.io/capabilities/#more-details)

The `Change nested if to cond` action sends `if->cond-refactor`.
The stdio comparison matched the action, replacement and UTF-16 range
through an unsaved version-7 edit. Parenox reports a rejected application
with the client reason. It retains comments and reader discards in the
gaps around arguments, qualifies `clojure.core/cond` under visible
shadowing, and skips quoted and discarded forms. The pinned JVM
confirmed equal values after applying nine actual edits.

The reverse `Change cond to nested if` action matches replacement text
and UTF-16 ranges for ordinary clauses, including a final `:else`, a
missing default, a single keyword test and an empty `cond`. Parenox also
accepts explicitly qualified `clojure.core/cond` when this qualifier
resolves to core, keeps auto-resolved keyword tests, and leaves shadowed
calls and odd clause counts alone. Both servers handled two concurrent
`workspace/applyEdit` responses in reverse order at version 7. Eleven
Parenox edits preserved their values on the pinned JVM.

Both servers emitted the same 13 semantic tokens for a fixture with
lexical shadowing and a Unicode string. Both returned the arglist and
docstring on resolving a qualified completion from a closed source file.
Both servers suggested `str/join` with the same replacement range
and function kind on the pinned Clojure 1.13 JAR. Parenox found it via
cached direct Maven coordinates and via an explicit Clojure CLI classpath.
Standalone literal `require` aliases also complete in unfinished forms
after the call.
Definition requests follow names, aliases and `:refer` in completed
top-level literal calls, including public vars from local JARs. Later
references use the aliases established above the cursor. Lexical
bindings take precedence for unqualified references. In the pinned
comparison Parenox skipped incomplete and quoted forms, nested
`require` and a rebound `clojure.core/require` qualifier.

Go to Implementation on a protocol method resolves separate overloads in
`defrecord` and `deftype`, grouped methods in `extend-type` and
`extend-protocol`, and `reify` inside recognized expression bodies. The
range selects only the method name. Both servers returned eleven sites
for the same method in the parity probe. Parenox leaves ambiguous
duplicate declarations and bindings inside opaque macros unresolved.

Both servers suggest indexed project namespaces in a closed `:require`
libspec and replace the entire token. Parenox also suggests them while
the libspec is unclosed. Definition requests on bare or vector libspec
names and `:as` and `:as-alias` bindings returned the same UTF-16 locations.
Both tracked a change to the provider in an unsaved buffer at version 7.
Parenox leaves the result empty when two workspace files declare the
same namespace. Clojure LSP selected one of the files in the probe.
For prefix groups, the same requests on a bare, vector or list child
resolve the expanded namespace. The prefix token itself has no target.
Parenox's file move refactoring stops on references through a prefix
group, which requires rewriting the group when the namespace changes.
Hover displays literal docstrings and arglists
from closed project files and local JAR sources, with Parenox contracts
alongside source documentation. On the arity example, the median first
hover was 1.1 ms for a project function and 23.8 ms for a core function
from the JAR. Median warm p95 was under 0.15 ms for both across five runs.

The ClojureDocs protocol probe compared the parsed JSON results of five
requests against Clojure LSP 2026.05.05-12.58.26. Both servers returned
identical entries for `clojure.core/map` and `clojure.core//` and identical
`null` results for a missing entry and positional parameters. Both the
camel-case and kebab-case object keys worked. Parenox read a local compact
EDN export with 1,528 entries and SHA-256
`0d209864c326971a62ab83e865308b4c03e937172056acfb4ddd63950477fcb6`.
Stdio tests cover markdown and plaintext hover, lexical shadowing,
unsaved edits, and a missing or invalid snapshot.
The release binary downloaded the same 2,029,184-byte snapshot over
HTTPS with `reqwest` and an empty `PATH`, then served raw entries and
hover examples in the running session. A restart with
`--clojuredocs-offline` returned the same five raw results as
`--clojuredocs FILE`. Cache tests cover delayed publication and preserving
the previous snapshot after a failed request or an empty export. On
macOS arm64 the release binary grew from 7.89 MB to 13.04 MB in the
slice that replaced the curl subprocess and added `thread-first`.
Across ten alternating launches in offline mode, median `initialize`
took 5.4 ms with an empty cache and 28.8 ms with the snapshot cached.
[Clojure LSP request](https://clojure-lsp.io/capabilities/#clojuredocs-raw),
[ClojureDocs export](https://github.com/clojure-emacs/clojuredocs-export-edn#exported-data)

For Java static completion, both returned the same replacement ranges
and item kinds for `UUID/randomUUID`, `Math/PI` and the four `Math/max`
overloads. The reference indexed local JDK source. Parenox read the JDK
class files and included parameter types in completion details. Java
completion reads base JAR entries and walks superclasses. Instance methods
also include inherited interface methods. Qualified and grouped imports
suggest public top-level classes within the package, including in an
unfinished clause. Completion replaces the full token in UTF-16
coordinates, including when the cursor sits in the middle of the token.
Across five runs, the first `UUID/` query took a median of 34 ms, and
the median warm p95 was 0.08 ms for 50 requests per run.
In a saved buffer and an unsaved edit, Parenox returned edits for
`java.util.UUI`, `[java.util UUI]`, `UUID/.toStr` and
`java.util.UUID/.compareT`. Clojure LSP 2026.05.05-12.58.26 returned no
matching items in those requests. A JDK probe also found
`List/.isEmpty` and `ArrayList/.getClass`. `UUID/.compareTo` produced
one item after suppressing the inherited signature implemented by a
compiler bridge.

In five alternating release launches on macOS arm64, the median first
`UUID/.toStr` query fell from 86 ms to 17 ms when the superclass and
interface lookup began reusing archive directories. The median first
`ArrayList/.getClass` query fell from 200 ms to 18 ms. The first
package-qualified import request took 84 ms and its repeat was below
1 ms at the probe's resolution.

Java definition requests on class imports and qualifiers returned `0:0–0:0`
in both servers with Amazon Corretto 22.0.1. Parenox returned an archive
URI and the reference returned a file URI for its extracted copy. The
same five UTF-16 queries on unsaved version 7 matched after mapping both
URIs to the source entries. `dependencyContents` returned the exact
20,275-byte `UUID.java` and 65,630-byte `ArrayList.java` entries. A URI
stayed readable after an unrelated watched-file notification. Synthetic
JDK tests covered corrupt source, absent source, and classpath precedence.

For Java type hints, Parenox follows the effective symbolic `:tag` in
direct binding positions, return hints and evaluated expressions,
including direct dot receivers. The pinned Clojure LSP agreed in 39 of
50 requests after URI normalization. JVM probes confirmed that class
hints ignore same-name locals and that the outer tag takes precedence.
Parenox skips overridden and arbitrary metadata, rejected duplicate
metadata keys and opaque macro arguments. After a UTF-16 buffer change
at version 7, the updated import led to `ArrayList.java`, qualified
`java.util.UUID` still led to `UUID.java`, and the old short tag had no
destination. Both servers returned all three tested source entries
byte-for-byte from Corretto 22.0.1, including `String.java`.

The file move fixture covers versioned edits, both JVM and CLJS
`.cljc` require clauses, rejection of occupied destinations, and editor
overlays that follow a client-owned move.
The pinned-source hierarchy smoke found callers of `uri->path` and
callees of `filename->uri`. The test tree matched the reference response
on a fixture with nested scopes and lexical shadowing. Go to
Implementation returned all ten source methods of the pinned pilot's
`clojure-lsp.queries/find-definition` multimethod.
Both servers returned the same project tree root, source branch and five
namespace leaves on the same fixture, including a symbol range after a
non-BMP character. Expanding a selected `spec.alpha` JAR also returned
the same JAR node, `clojure.spec.alpha` namespace URI and `conform` symbol
range. Archive expansion uses matching JVM namespace declarations and
bounded reads of `.clj` and `.cljc` resources. Both servers returned the
same archive source through dependency contents. Parenox rejected the same
resource URI in an unselected copy of the JAR.

`just verify` passed 723 tests, including 45 stdio LSP tests.
`just pilot-navigation-smoke` passed. The ordinary pilot diagnostics
matched the preceding build. With `lsp4clj` sources, the pilot reported
zero errors and 2,312 warnings, down from 2,321.
`just agent-loop` fixed all three planted clinic errors and measured a
warm p95 round trip of 54.8 µs in the latest run.
