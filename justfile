# Show available commands.
default:
    @just --list

# Build the optimized CLI.
build:
    cargo build --locked --release

# Check a file or directory: just check path [pretty|plain|json] [auto|always|never].
check path="examples/good.clj" format="pretty" color="auto": build
    @./target/release/parenox check {{quote(path)}} --format {{quote(format)}} --color {{quote(color)}}

# Show the three deliberate clinic errors; diagnostic exit 1 is expected.
errors format="pretty" color="auto": build
    @./target/release/parenox check examples/clinic --externs examples/clinic/externs.edn --format {{quote(format)}} --color {{quote(color)}} || test "$?" -eq 1

# Check the passing single-file example.
good: build
    @./target/release/parenox check examples/good.clj

# Start the persistent JSON Lines checker.
serve: build
    @./target/release/parenox serve

# Fix clinic errors in memory, then measure warm checks and edits.
agent-loop iterations="100" binary="target/release/parenox" extra_externs="0" shifted_functions="0" transport="snapshot": build
    cargo run --locked --release --example agent_loop -- --project examples/clinic --iterations {{quote(iterations)}} --binary {{quote(binary)}} --extra-externs {{quote(extra_externs)}} --shifted-functions {{quote(shifted_functions)}} --project-transport {{quote(transport)}}

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

# Check map presence guards and show nilable-value and absent-key errors.
presence format="pretty": build
    @./target/release/parenox check examples/presence.clj --format {{quote(format)}}
    @./target/release/parenox check examples/presence_bad.clj --format {{quote(format)}} || test "$?" -eq 1

# Check update and update-in, then show callback errors at the updated keys.
updates format="pretty" color="auto": build
    @./target/release/parenox check examples/updates.clj --format {{quote(format)}} --color {{quote(color)}}
    @./target/release/parenox check examples/updates_bad.clj --format {{quote(format)}} --color {{quote(color)}} || test "$?" -eq 1

# Check vector pipelines and show direct and inline callback diagnostics.
vectors format="pretty" color="auto": build
    @./target/release/parenox check examples/vectors.clj --format {{quote(format)}} --color {{quote(color)}}
    @./target/release/parenox check examples/vectors_bad.clj --format {{quote(format)}} --color {{quote(color)}} || test "$?" -eq 1

# Check map entry pipelines and show a bad entry value and a bad key argument.
entries format="pretty" color="auto": build
    @./target/release/parenox check examples/entries.clj --format {{quote(format)}} --color {{quote(color)}}
    @./target/release/parenox check examples/entries_bad.clj --format {{quote(format)}} --color {{quote(color)}} || test "$?" -eq 1

# Check map-of contracts and show callback and map return-type diagnostics.
dictionaries format="pretty" color="auto": build
    @./target/release/parenox check examples/dictionaries.clj --format {{quote(format)}} --color {{quote(color)}}
    @./target/release/parenox check examples/dictionaries_bad.clj --format {{quote(format)}} --color {{quote(color)}} || test "$?" -eq 1

# Check string keys and show explicit Nil and dictionary return-type errors.
map-keys format="pretty" color="auto": build
    @./target/release/parenox check examples/map_keys.clj --format {{quote(format)}} --color {{quote(color)}}
    @./target/release/parenox check examples/map_keys_bad.clj --format {{quote(format)}} --color {{quote(color)}} || test "$?" -eq 1

# Check nested operations using computed keys and show two deliberate errors.
dynamic-paths format="pretty" color="auto": build
    @./target/release/parenox check examples/dynamic_paths.clj --format {{quote(format)}} --color {{quote(color)}}
    @./target/release/parenox check examples/dynamic_paths_bad.clj --format {{quote(format)}} --color {{quote(color)}} || test "$?" -eq 1

# Check fixed vector positions and show reversed and missing items.
ordered-paths format="pretty" color="auto": build
    @./target/release/parenox check examples/ordered_paths.clj --format {{quote(format)}} --color {{quote(color)}}
    @./target/release/parenox check examples/ordered_paths_bad.clj --format {{quote(format)}} --color {{quote(color)}} || test "$?" -eq 1

# Check indexed reads and show a bounds error and a present nil value.
indices format="pretty" color="auto": build
    @./target/release/parenox check examples/indices.clj --format {{quote(format)}} --color {{quote(color)}}
    @./target/release/parenox check examples/indices_bad.clj --format {{quote(format)}} --color {{quote(color)}} || test "$?" -eq 1

# Check loop accumulators and show later-iteration, arity and tail errors.
loops format="pretty" color="auto": build
    @./target/release/parenox check examples/loops.clj --format {{quote(format)}} --color {{quote(color)}}
    @./target/release/parenox check examples/loops_bad.clj --format {{quote(format)}} --color {{quote(color)}} || test "$?" -eq 1

# Check a dispatch ladder and show a missing fallback and a bad branch.
cond format="pretty" color="auto": build
    @./target/release/parenox check examples/cond.clj --format {{quote(format)}} --color {{quote(color)}}
    @./target/release/parenox check examples/cond_bad.clj --format {{quote(format)}} --color {{quote(color)}} || test "$?" -eq 1

# Check a bare require and a renamed project function across files.
libspecs format="pretty" color="auto": build
    @./target/release/parenox check examples/libspecs/provider.clj examples/libspecs/app.clj --format {{quote(format)}} --color {{quote(color)}}
    @./target/release/parenox check examples/libspecs/provider.clj examples/libspecs/bad.clj --format {{quote(format)}} --color {{quote(color)}} || test "$?" -eq 1

# Check alias-only imports and a named spec declared in another file.
namespace-specs format="pretty" color="auto": build
    @./target/release/parenox check examples/namespace_specs/config.clj examples/namespace_specs/app.clj examples/namespace_specs/handler.clj --format {{quote(format)}} --color {{quote(color)}}
    @./target/release/parenox check examples/namespace_specs/config.clj examples/namespace_specs/bad.clj examples/namespace_specs/handler.clj --format {{quote(format)}} --color {{quote(color)}} || test "$?" -eq 1

# Check literal dispatch, then show a bad branch and a definite missing match.
case format="pretty" color="auto": build
    @./target/release/parenox check examples/case.clj --format {{quote(format)}} --color {{quote(color)}}
    @./target/release/parenox check examples/case_bad.clj --format {{quote(format)}} --color {{quote(color)}} || test "$?" -eq 1

# Check seq traversal in a loop and show an unchecked empty input.
sequences format="pretty" color="auto": build
    @./target/release/parenox check examples/sequences.clj --format {{quote(format)}} --color {{quote(color)}}
    @./target/release/parenox check examples/sequences_bad.clj --format {{quote(format)}} --color {{quote(color)}} || test "$?" -eq 1

# Check growing vector accumulators and show a nil element diagnostic.
accumulators format="pretty" color="auto": build
    @./target/release/parenox check examples/accumulators.clj --format {{quote(format)}} --color {{quote(color)}}
    @./target/release/parenox check examples/accumulators_bad.clj --format {{quote(format)}} --color {{quote(color)}} || test "$?" -eq 1

# Check explicit throws and show invalid exception and cause arguments.
exceptions format="pretty" color="auto": build
    @./target/release/parenox check examples/exceptions.clj --format {{quote(format)}} --color {{quote(color)}}
    @./target/release/parenox check examples/exceptions_bad.clj --format {{quote(format)}} --color {{quote(color)}} || test "$?" -eq 1

# Check exception recovery and show invalid recovery and recur diagnostics.
handlers format="pretty" color="auto": build
    @./target/release/parenox check examples/handlers.clj --format {{quote(format)}} --color {{quote(color)}}
    @./target/release/parenox check examples/handlers_bad.clj --format {{quote(format)}} --color {{quote(color)}} || test "$?" -eq 1

# Check imported configuration values and show private var access.
literal-vars format="pretty" color="auto": build
    @./target/release/parenox check examples/literal_vars/config.clj examples/literal_vars/app.clj --format {{quote(format)}} --color {{quote(color)}}
    @./target/release/parenox check examples/literal_vars/config.clj examples/literal_vars/bad.clj --format {{quote(format)}} --color {{quote(color)}} || test "$?" -eq 1

# Check explicit core imports and show an excluded core name.
core-imports format="pretty" color="auto": build
    @./target/release/parenox check examples/core_imports.clj --format {{quote(format)}} --color {{quote(color)}}
    @./target/release/parenox check examples/core_imports_bad.clj --format {{quote(format)}} --color {{quote(color)}} || test "$?" -eq 1

# Check fixed arities across a namespace boundary and show bad calls.
arities format="pretty": build
    @./target/release/parenox check examples/arities/provider.clj examples/arities/app.clj --format {{quote(format)}}
    @./target/release/parenox check examples/arities/provider.clj examples/arities/bad.clj --format {{quote(format)}} || test "$?" -eq 1

# Check variadic declarations and show wrong rest arguments.
variadic format="pretty": build
    @./target/release/parenox check examples/variadic.clj --format {{quote(format)}}
    @./target/release/parenox check examples/variadic.clj examples/variadic_bad.clj --format {{quote(format)}} || test "$?" -eq 1

# Check positional argument spreading with apply.
spread format="pretty": build
    @./target/release/parenox check examples/arities/provider.clj examples/variadic.clj examples/apply.clj --format {{quote(format)}}
    @./target/release/parenox check examples/arities/provider.clj examples/variadic.clj examples/apply_bad.clj --format {{quote(format)}} || test "$?" -eq 1

# Check shorthand callbacks and show positional/type errors.
anonymous format="pretty": build
    @./target/release/parenox check examples/anonymous.clj --format {{quote(format)}}
    @./target/release/parenox check examples/anonymous_bad.clj --format {{quote(format)}} || test "$?" -eq 1

# Check source-backed :refer :all and show excluded/private names.
all-refers format="pretty": build
    @./target/release/parenox check examples/all_refers/library.clj examples/all_refers/app.clj --externs examples/all_refers/externs.edn --format {{quote(format)}}
    @./target/release/parenox check examples/all_refers/library.clj examples/all_refers/bad.clj --externs examples/all_refers/externs.edn --format {{quote(format)}} || test "$?" -eq 1
