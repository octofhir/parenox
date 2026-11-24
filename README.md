<p align="center">
  <img src="assets/parenox.png" alt="A copper fox framed by matching parentheses" width="520">
</p>

# Parenox

**Native Clojure tooling for editors, agents, and CI.**

Parenox checks Clojure source, explains mismatches at the expression that caused
them, and lets you navigate across namespaces. The same Rust analysis engine
powers the CLI, a stdio language server, and an incremental JSON Lines session.

[![CI](https://github.com/octofhir/parenox/actions/workflows/ci.yml/badge.svg)](https://github.com/octofhir/parenox/actions/workflows/ci.yml)
[![License: MIT or Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#license)

## Quick start

Main preview builds are available for macOS ARM64 and Linux x86-64 (glibc).
Each archive contains `parenox` and the license files. Verify downloads against
`SHA256SUMS`, then put the extracted binary on your `PATH`.
[Download main preview](https://github.com/octofhir/parenox/releases/tag/main-preview)

To build from source:

```sh
git clone https://github.com/octofhir/parenox.git
cd parenox
cargo build --locked --release
./target/release/parenox check examples/good.clj
```

Point the binary at your source, or run `parenox check` from a project with
`deps.edn` or `parenox.edn`. Add `--alias test` to include the `:test` source
paths from `deps.edn`. CI can use `--format json` for machine-readable diagnostics.
[CLI and project configuration](docs/reference.md#a-project-sized-example)

Format Clojure and EDN with `parenox fmt --write src` or **Reformat Code**
in your editor. Both use the same [`parenox.edn` settings](docs/reference.md#formatting).

`parenox check --allow-incomplete` accepts coverage warnings when adding
contracts gradually. The report still shows which checks were incomplete.

Evaluate forms with `parenox eval --expr '(+ 1 2)'` and launch an application
with `parenox run --main my-app.main -- arg`. [Runtime reference](docs/reference.md#jvm-evaluation)

Try the built-in Rust evaluator with `parenox eval --backend parenox --expr '(+ 40 2)'`.
[Supported forms](docs/reference.md#parenox-evaluation)

Run project tests with `parenox test --alias test --discover` or name a test
namespace. Parenox uses the standard Clojure CLI and reports local assertion
locations and a CI exit status. [JVM test reference](docs/reference.md#jvm-tests)

Try the included clinic project to see a diagnostic with its contract source:

```sh
./target/release/parenox check examples/clinic --externs examples/clinic/externs.edn
```

```text
error[PNX2322]: Argument 1: mismatched types
  --> examples/clinic/http.clj:12:37
   |
12 |         patient (store/find-patient 42)]
   |                                     ^^ expected `String`, found `Number`
   |
   = note: Contract for clinic.store/find-patient: :t/sig in examples/clinic/store.clj
```

## Use it in your editor

Launch `parenox lsp` as a stdio language server. It publishes diagnostics for
`.clj` and `.cljc`, resolves qualified vars across namespaces, and shows checked
signatures and their origins on hover. Open `.edn` buffers get reader diagnostics.
Unsaved edits participate in analysis. [Editor setup](docs/reference.md#editor-integration)

For RustRover and other JetBrains IDEs, import the supplied LSP4IJ template.
[RustRover setup](docs/reference.md#rustrover-and-jetbrains-ides)

## What Parenox checks

- **Function contracts.** Read supported `clojure.spec` declarations, `:t/sig`
  metadata, and EDN externs. Check calls and returns against those contracts.
- **Clojure data.** Track nil, truthiness, key presence and map shapes through
  supported branches and collection operations.
- **Library compatibility.** Export verified public signatures and values as EDN.
  Compare snapshots with `diff-externs` to catch breaking contract changes.
- **Incremental workflows.** Use `parenox serve` to send source snapshots and
  unsaved edits over JSON Lines, then query the same project state.

Parenox is an independent preview targeting Clojure 1.13. Static analysis runs
without a JVM and reports gaps in coverage explicitly. Unannotated code can
receive project var, arity, and selected string input diagnostics. Supported
contracts extend checking to call and return types.
[Supported language forms](docs/reference.md#current-scope)

## Documentation

- [Command and language reference](docs/reference.md)
- [Clinic example](examples/clinic)
- [Tooling roadmap](docs/design/type-checker-research.md)
- [First product iteration and pilot measurements](docs/design/first-product-iteration.md)

For development, run `just verify` for formatting, Clippy, and tests.
`just agent-loop` exercises the clinic edit-and-check workflow.
`just pilot-setup` prepares a larger [local pilot for RustRover](docs/reference.md#local-clojure-lsp-pilot).

## License

[MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE).
