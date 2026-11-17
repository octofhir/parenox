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
