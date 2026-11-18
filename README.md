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

Run project tests with `parenox test --alias test my-app.routes-test`. It uses
the standard Clojure CLI and reports local assertion locations and a CI exit
status. [JVM test reference](docs/reference.md#jvm-tests)

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
without a JVM and reports gaps in coverage explicitly. Public type checks require
a supported contract. [Supported language forms](docs/reference.md#current-scope)

## Documentation

- [Command and language reference](docs/reference.md)
- [Clinic example](examples/clinic)
- [Tooling roadmap](docs/design/type-checker-research.md)

For development, run `just verify` for formatting, Clippy, and tests.
`just agent-loop` exercises the clinic edit-and-check workflow.

## License

[MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE).
