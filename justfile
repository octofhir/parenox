# Show available commands.
default:
    @just --list

# Build the optimized CLI.
build:
    cargo build --locked --release

# Check a file or directory: just check path [pretty|plain|json].
check path="examples/good.clj" format="pretty": build
    @./target/release/parenox check {{quote(path)}} --format {{quote(format)}}

# Show the three deliberate clinic errors; diagnostic exit 1 is expected.
errors format="pretty": build
    @./target/release/parenox check examples/clinic --externs examples/clinic/externs.edn --format {{quote(format)}} || test "$?" -eq 1

# Check the passing single-file example.
good: build
    @./target/release/parenox check examples/good.clj

# Start the persistent JSON Lines checker.
serve: build
    @./target/release/parenox serve

# Fix clinic errors in memory, then measure warm checks and edits.
agent-loop iterations="100": build
    cargo run --locked --release --example agent_loop -- --project examples/clinic --iterations {{quote(iterations)}}

# Measure synthetic single-file sessions and fresh CLI processes.
bench iterations="100" functions="1000": build
    cargo run --locked --release --example agent_loop -- --iterations {{quote(iterations)}} --functions {{quote(functions)}}

# Run the Rust tests.
test:
    cargo test --locked

# Check lints without warnings.
lint:
    cargo clippy --locked --all-targets -- -D warnings

# Format Rust sources.
fmt:
    cargo fmt --all

# Check Rust formatting.
fmt-check:
    cargo fmt --all -- --check

# Run the same checks as CI.
verify: fmt-check lint test

# Independently check example behavior on Clojure/JVM (optional).
runtime-smoke:
    clojure -M scripts/runtime_smoke.clj

# Check private inference, then show the deliberate nil error and call chain.
inference: build
    @./target/release/parenox check examples/inferred.clj
    @./target/release/parenox check examples/inferred_bad.clj || test "$?" -eq 1

# Inspect the lossless CST, including comments, punctuation and reader discards.
syntax path="examples/syntax.clj": build
    @./target/release/parenox syntax {{quote(path)}}

# Check metadata and character contracts, then inspect tagged EDN without executing readers.
reader: build
    @./target/release/parenox check examples/reader.clj
    @./target/release/parenox syntax examples/reader.edn

# Check the JVM branches of a portable source file.
portable: build
    @./target/release/parenox check examples/portable.cljc

# Check Clojure 1.13 bindings, then show missing-key and explicit-Nil errors.
checked format="pretty": build
    @./target/release/parenox check examples/checked.clj --format {{quote(format)}}
    @./target/release/parenox check examples/checked_bad.clj --format {{quote(format)}} || test "$?" -eq 1

# Check :or defaults, then show Nil, eager-expression and required-key conflicts.
defaults format="pretty": build
    @./target/release/parenox check examples/defaults.clj --format {{quote(format)}}
    @./target/release/parenox check examples/defaults_bad.clj --format {{quote(format)}} || test "$?" -eq 1

# Check threading macros, then show source-mapped type and Nil errors.
threading format="pretty": build
    @./target/release/parenox check examples/threading.clj --format {{quote(format)}}
    @./target/release/parenox check examples/threading_bad.clj --format {{quote(format)}} || test "$?" -eq 1

# Check conditional/named threading, then show union, Nil and argument errors.
conditional format="pretty": build
    @./target/release/parenox check examples/conditional.clj --format {{quote(format)}}
    @./target/release/parenox check examples/conditional_bad.clj --format {{quote(format)}} || test "$?" -eq 1

# Check binding guards and show a missing fallback and a False argument error.
guards format="pretty": build
    @./target/release/parenox check examples/guards.clj --format {{quote(format)}}
    @./target/release/parenox check examples/guards_bad.clj --format {{quote(format)}} || test "$?" -eq 1

# Check nested map paths and show the errors for an explicit Nil and a scalar parent.
paths format="pretty": build
    @./target/release/parenox check examples/paths.clj --format {{quote(format)}}
    @./target/release/parenox check examples/paths_bad.clj --format {{quote(format)}} || test "$?" -eq 1
