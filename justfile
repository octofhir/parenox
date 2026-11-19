# Show available commands.
default:
    @just --list

# Build the optimized CLI.
build:
    cargo build --locked --release

pilot_revision := "1e93ac58bede574786c5cc75b0fd3a51bc224f7a"
pilot_deps := "playground/clojure-lsp/deps.edn"

# Download the pinned Clojure LSP checkout without touching local edits.
pilot-fetch:
    #!/bin/sh
    set -eu
    dest=playground/clojure-lsp
    revision={{quote(pilot_revision)}}
    if [ -e "$dest" ] || [ -L "$dest" ]; then
        if [ -d "$dest/.git" ] && [ "$(git -C "$dest" rev-parse --verify HEAD 2>/dev/null)" = "$revision" ]; then
            printf '%s\n' "$dest" >&2
            exit 0
        fi
        printf '%s\n' "$dest already exists but is not at $revision. Move it aside and retry." >&2
        exit 1
    fi
    mkdir -p playground
    tmp=$(mktemp -d playground/.clojure-lsp.XXXXXX)
    trap 'rm -rf "$tmp"' EXIT
    trap 'exit 1' HUP INT TERM
    git init --quiet "$tmp"
    git -C "$tmp" remote add origin https://github.com/clojure-lsp/clojure-lsp.git
    git -C "$tmp" fetch --quiet --depth=1 origin "$revision"
    git -C "$tmp" checkout --quiet --detach FETCH_HEAD
    mv "$tmp" "$dest"
    printf '%s\n' "$dest" >&2

# Prepare the local pilot for RustRover.
pilot-setup: pilot-fetch build

# Check all configured pilot sources, including files not open in the editor.
pilot-check format="pretty": build
    @test -f {{quote(pilot_deps)}} || { echo "Run just pilot-setup first." >&2; exit 2; }
    @./target/release/parenox check --deps {{quote(pilot_deps)}} --alias test --allow-incomplete --format {{quote(format)}}

# Verify formatting is lossless and idempotent on the local pilot without writing to it.
pilot-format-smoke project="playground/clojure-lsp":
    PARENOX_FORMAT_PILOT={{quote(project)}} cargo test --locked --release --test formatter pilot_formatting_is_lossless_and_idempotent -- --ignored --nocapture

# Verify project search and references on the local pilot, including unopened source and test files.
pilot-navigation-smoke project="playground/clojure-lsp":
    PARENOX_LSP_PILOT={{quote(project)}} cargo test --locked --release --test lsp clojure_lsp_pilot_navigation -- --ignored --nocapture

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
agent-loop iterations="100" binary="target/release/parenox" extra_externs="0" shifted_functions="0" transport="snapshot" extra_consumers="0" extra_extern_files="0": build
    cargo run --locked --release --example agent_loop -- --project examples/clinic --iterations {{quote(iterations)}} --binary {{quote(binary)}} --extra-externs {{quote(extra_externs)}} --shifted-functions {{quote(shifted_functions)}} --project-transport {{quote(transport)}} --extra-consumers {{quote(extra_consumers)}} --extra-extern-files {{quote(extra_extern_files)}}

# Measure synthetic single-file sessions and fresh CLI processes.
bench iterations="100" functions="1000": build
    cargo run --locked --release --example agent_loop -- --iterations {{quote(iterations)}} --functions {{quote(functions)}}

# Measure checked source and verified EDN for a chain or a shared spec graph.
bench-export shape="shared" levels="15" iterations="10":
    cargo run --locked --release --example extern_expansion -- {{quote(shape)}} {{quote(levels)}} {{quote(iterations)}}

# Run the Rust tests.
test:
    cargo test --locked --workspace

# Check lints without warnings.
lint:
    cargo clippy --locked --workspace --all-targets -- -D warnings

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

# Check editor behavior and CLI agreement on a local pinned lambdaisland/uri checkout.
uri-editor-smoke project:
    PARENOX_URI_PILOT={{quote(project)}} cargo test --locked --release --test lsp uri_pilot_editor_workflow -- --ignored

# Measure fresh initialization and 200 checked editor edits on the pinned URI checkout.
uri-editor-bench project binary="target/release/parenox": build
    PARENOX_URI_PILOT={{quote(project)}} PARENOX_LSP_BINARY={{quote(binary)}} cargo test --locked --release --test lsp uri_pilot_editor_benchmark -- --ignored --nocapture

# Measure checked Malli edits with full or range changes on assertions or core.
malli-editor-bench project binary="target/release/parenox" transport="full" document="assertions": build
    PARENOX_MALLI_PILOT={{quote(project)}} PARENOX_LSP_BINARY={{quote(binary)}} PARENOX_LSP_TRANSPORT={{quote(transport)}} PARENOX_MALLI_DOCUMENT={{quote(document)}} cargo test --locked --release --test lsp malli_pilot_editor_benchmark -- --ignored --nocapture

# Exercise passing, failing and unloadable tests on Clojure/JVM (optional).
jvm-test-smoke:
    cargo test --locked --test runtime_cli -- --ignored

# Run the same portable test namespace on JVM Clojure and a supplied cljrs build.
portable-runtime-smoke native: build
    #!/bin/sh
    set -eu
    fixture=$(mktemp -d)
    trap 'rm -rf "$fixture"' EXIT
    cp -R tests/fixtures/jvm-project/. "$fixture"
    target/release/parenox test --project "$fixture" --alias test demo.portable-test
    {{quote(native)}} test --no-config --src-path "$fixture/test" demo.portable-test

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

# Check one-sided and two-sided predicates with verified bodies.
checked-guards format="pretty": build
    @./target/release/parenox check examples/checked_guards.clj --format {{quote(format)}}

# Check inline callbacks against an expected s/fspec contract.
contextual-callbacks format="pretty": build
    @./target/release/parenox check examples/contextual_callbacks.clj --format {{quote(format)}}

# Infer callback input and result types from a generic contract.
generic-callbacks format="pretty": build
    @./target/release/parenox check examples/generic_callbacks.clj --format {{quote(format)}}

# Infer independent inputs for a callback that combines two vectors.
generic-zip format="pretty": build
    @./target/release/parenox check examples/generic_zip.clj --format {{quote(format)}}

# Check a storage extern, generic map contracts and an HTTP response.
map-boundary format="pretty": build
    @./target/release/parenox check examples/map_boundary/src --externs examples/map_boundary/externs.edn --format {{quote(format)}}

# Check a namespace that loads declarations from two extern files.
split-externs format="pretty": build
    @./target/release/parenox check examples/split_externs/app.clj --externs examples/split_externs/queries.edn --externs examples/split_externs/formatting.edn --format {{quote(format)}}

# Validate library promises against source, then show a mismatched input type.
validate-externs format="pretty": build
    @./target/release/parenox validate-externs --config examples/split_externs/parenox.edn --format {{quote(format)}}
    @./target/release/parenox validate-externs --config examples/split_externs/parenox.edn --externs examples/split_externs/drift.edn --format {{quote(format)}} || test "$?" -eq 1

# Export the checked public function contracts and literal values as standalone EDN.
export-externs path="examples/showcase.clj": build
    @./target/release/parenox export-externs {{quote(path)}}

# Check a client with library constants from an exported EDN snapshot only.
literal-externs format="pretty": build
    @snapshot=$(mktemp); trap 'rm -f "$snapshot"' EXIT; ./target/release/parenox export-externs examples/literal_vars/config.clj > "$snapshot" && ./target/release/parenox check examples/literal_vars/app.clj --externs "$snapshot" --format {{quote(format)}}

# Compare two library versions with independent named specs and show a breaking change.
diff-externs format="plain": build
    @./target/release/parenox diff-externs --before-config examples/extern_diff/before/parenox.edn --after-config examples/extern_diff/after/parenox.edn --format {{quote(format)}} || test "$?" -eq 1

# Check the end-to-end batch processing showcase.
showcase format="pretty": build
    @./target/release/parenox check examples/showcase.clj --format {{quote(format)}}

# Show deliberately broken changes to the showcase, with diagnostic locations.
showcase-errors format="pretty" color="auto": build
    @status=0; ./target/release/parenox check examples/showcase.clj examples/showcase_bad.clj --format {{quote(format)}} --color {{quote(color)}} || status=$?; test "$status" -eq 1

# Run the same single-file example on the pinned Clojure JVM.
showcase-run:
    clojure -M -i examples/showcase.clj -e '(prn (demo.showcase/summarize-batch (demo.showcase/sample-batch)))'

# Compare an ordinary let with a macro whose scope is known but effects are not.
macro-scopes format="pretty": build
    @./target/release/parenox check examples/macro_scopes/use.clj --externs examples/macro_scopes/externs.edn --format {{quote(format)}} || test "$?" -eq 1

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

# Check public responses projected or redacted from an open record.
map-projection format="pretty": build
    @./target/release/parenox check examples/map_projection.clj --format {{quote(format)}}

# Check dictionary keys, values and possible empty results.
map-sequences format="pretty": build
    @./target/release/parenox check examples/map_sequences.clj --format {{quote(format)}}

# Check eager map value transforms and their result shapes.
update-vals format="pretty": build
    @./target/release/parenox check examples/update_vals.clj --format {{quote(format)}}

# Build and rename maps while checking key and value types.
map-builders format="pretty": build
    @./target/release/parenox check examples/map_builders.clj --format {{quote(format)}}

# Check typed groups and their ordered elements.
group-by format="pretty": build
    @./target/release/parenox check examples/group_by.clj --format {{quote(format)}}

# Check typed map conflicts and optional fields during merges.
merge-with format="pretty": build
    @./target/release/parenox check examples/merge_with.clj --format {{quote(format)}}

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
    @./target/release/parenox check --config examples/namespace_specs/parenox.edn --format {{quote(format)}} --color {{quote(color)}} || test "$?" -eq 1

# Check literal dispatch, then show a bad branch and a definite missing match.
case format="pretty" color="auto": build
    @./target/release/parenox check examples/case.clj --format {{quote(format)}} --color {{quote(color)}}
    @./target/release/parenox check examples/case_bad.clj --format {{quote(format)}} --color {{quote(color)}} || test "$?" -eq 1

# Check that tag tests narrow each map variant and its payload.
tagged-maps format="pretty" color="auto": build
    @./target/release/parenox check examples/tagged_maps.clj --format {{quote(format)}} --color {{quote(color)}}

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

# Check collections and keywords in function position.
callables format="pretty": build
    @./target/release/parenox check examples/callables.clj --format {{quote(format)}}
    @./target/release/parenox check examples/callables_bad.clj --format {{quote(format)}} || test "$?" -eq 1

# Check reduction arities and accumulator transitions.
reductions format="pretty": build
    @./target/release/parenox check examples/reductions.clj --format {{quote(format)}}
    @./target/release/parenox check examples/reductions_bad.clj --format {{quote(format)}} || test "$?" -eq 1

# Check key-value reductions over maps and vectors.
reduce-kv format="pretty": build
    @./target/release/parenox check examples/reduce_kv.clj --format {{quote(format)}}

# Check explicit namespace aliases in auto-resolved keywords.
keyword-aliases format="pretty": build
    @./target/release/parenox check examples/keyword_aliases.clj --format {{quote(format)}}
    @./target/release/parenox check examples/keyword_aliases_bad.clj --format {{quote(format)}} || test "$?" -eq 1

# Check literal configuration data and values from quoted lists.
quoted format="pretty": build
    @./target/release/parenox check examples/quoted.clj --format {{quote(format)}}
    @./target/release/parenox check examples/quoted_bad.clj --format {{quote(format)}} || test "$?" -eq 1

# Check imported exception classes and a class name collision.
imports format="pretty": build
    @./target/release/parenox check examples/imports.clj --format {{quote(format)}}
    @./target/release/parenox check examples/imports_bad.clj --format {{quote(format)}} || test "$?" -eq 1

# Check lookup evaluation order and show a mismatched recovery result.
lookup-order format="pretty": build
    @./target/release/parenox check examples/lookup_order.clj --format {{quote(format)}}
    @./target/release/parenox check examples/lookup_order_bad.clj --format {{quote(format)}} || test "$?" -eq 1

# Check exception data and eager argument failures through private helpers.
helper-effects format="pretty": build
    @./target/release/parenox check examples/helper_effects.clj --format {{quote(format)}}
    @./target/release/parenox check examples/helper_effects_bad.clj --format {{quote(format)}} || test "$?" -eq 1

# Check loop accumulation in a list and a non-seqable tail diagnostic.
list-accumulators format="pretty": build
    @./target/release/parenox check examples/list_accumulators.clj --format {{quote(format)}}
    @./target/release/parenox check examples/list_accumulators_bad.clj --format {{quote(format)}} || test "$?" -eq 1

# Check Clojure numeric syntax and show an invalid octal literal.
numbers format="pretty": build
    @./target/release/parenox check examples/numbers.clj --format {{quote(format)}}
    @./target/release/parenox check examples/numbers_bad.clj --format {{quote(format)}} || test "$?" -eq 1
