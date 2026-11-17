# Parenox development

- Use Rust for implementation, developer tools and performance scenarios. Do not add Python.
- Use `justfile` for common commands and Clojure/EDN for language fixtures and contracts.
- Keep one crate until a concrete need justifies splitting it.
- Prefer small, maintainable code and few dependencies. Measure performance changes.
- Add tests for meaningful behavior and regressions; avoid tests that mirror implementation.
- Keep Clojure semantics explicit, including nil, truthiness, maps, vectors and sets.
- Use Rowan for lossless CST storage and keep one reader for semantic forms and syntax capture.
- Preserve comments, whitespace, reader discards and invalid text; keep CST capture optional for checking.
- Report unsupported analysis instead of silently claiming a complete check.
- Run `just verify` after Rust changes. Use `just agent-loop` for the clinic edit/check scenario.
- Use Conventional Commits. Public documentation should describe Parenox as an independent project.
