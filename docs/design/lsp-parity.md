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

Against Clojure LSP 2026.05.05-12.58.26 the comparison went from 11 to
26 of 33 advertised entries. The seven missing entries are file rename
requests and notifications, command execution, project tree, ClojureDocs,
and server and cursor inspection. All 62 reference commands remain open.

| Workflow | Covered | Work still open |
|---|---|---|
| Navigation | Source definitions and declarations, multimethod implementations, references, signature help, source call hierarchy | Protocol implementations, external/JVM navigation, opaque macro calls |
| Editing | Full and range formatting, ordinary source var and lexical rename, local linked editing, folding and selection | Namespace and file rename, rename of keywords and aliases, structural refactorings |
| Assistance | Source completions and qualified function documentation, semantic tokens, source var reference counts | External and core documentation, dependency and Java completions, broader macro and CLJS token coverage, test-specific reference counts |
| Project views | Source Clojure test trees on open and edit for opted-in clients | Project tree, stubs, ClojureDocs and server/cursor inspection |
| Project analysis | Tracked source and overlays, static contracts, spec diagnostics | Artifact indexing, ClojureScript, kondo and project lints, custom analysis integrations |

For a cross-file var rename on the same fixture, applying either server's
edits produced byte-identical source. Parenox prepares the editable part
of a qualified name and retains the actual open document version.
Both servers emitted the same 13 semantic tokens for a fixture with
lexical shadowing and a Unicode string. Both returned the arglist and
docstring on resolving a qualified completion from a closed source file.
The pinned-source hierarchy smoke found callers of `uri->path` and
callees of `filename->uri`. The test tree matched the reference response
on a fixture with nested scopes and lexical shadowing. Go to
Implementation returned all ten source methods of the pinned pilot's
`clojure-lsp.queries/find-definition` multimethod.

`just verify` passed 669 tests, including 31 LSP tests. Both source pilot
diagnostic sets matched the preceding build. Median CLI time over 15
alternating runs was 64.40 → 64.72 ms on the ordinary pilot and
65.59 → 66.21 ms with dependency sources. The clinic scenario fixed all
three planted errors and warm p95 was 35.2 → 36.9 µs across five runs.
