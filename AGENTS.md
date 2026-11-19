# Parenox development

- Use Rust for implementation, developer tools and performance scenarios. Do not add Python.
- Use `justfile` for common commands and Clojure/EDN for language fixtures and contracts.
- Keep the workspace split by responsibility. Put the reader and CST in `parenox-frontend`, checking and project sessions in `parenox-analysis`, formatting in `parenox-formatter`, and editor protocol and language features in `parenox-lsp`. Keep CLI entry points and disk project wiring in `parenox`.
- Organize checker code by responsibility. Extend its existing model, binding, flow, call or map modules as appropriate.
- Prefer small, maintainable code and few dependencies. Measure performance changes.
- Add tests for meaningful behavior and regressions; avoid tests that mirror implementation.
- Target Clojure 1.13 semantics; pin the JVM reference in deps.edn and report unsupported features explicitly.
- Keep Clojure semantics explicit, including nil, truthiness, maps, vectors and sets.
- Use Rowan for lossless CST storage and keep one reader for semantic forms and syntax capture.
- Own the frontend API; share forms and source spans across consumers. Keep name resolution, macro expansion and execution outside the reader.
- Preserve comments, whitespace, reader discards and invalid text; keep CST capture optional for checking.
- Report unsupported analysis instead of silently claiming a complete check.
- Run `just verify` after Rust changes. Use `just agent-loop` for the clinic edit/check scenario.
- Use Conventional Commits. Public documentation should describe Parenox as an independent project.
