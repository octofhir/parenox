# First product iteration

The iteration ran from `2026-11-18 21:38:46 UTC` to `2026-11-19 03:38:46 UTC`
(`00:38:46` to `06:38:46` Moscow time on November 19). The starting revision
was `6125176`. The code and verification changes are in `6125176..4efbddc`.

## A deps.edn workflow

Build with `cargo build --locked --release` in the Parenox checkout. From the
Clojure project directory, use the resulting binary as `parenox`.

```sh
parenox check --alias test --allow-incomplete --format json > check.json
parenox test --alias test --discover --format json > tests.json
```

`check` can find project var and arity errors in unannotated code. Selected
operations also check facts such as a nil map value flowing into `str/replace`,
an invalid UTF-16 index, or a non-seqable `str/join` input. Supported contracts
extend checking to call and return types. An accepted report with coverage
warnings includes `ok: true` and `complete: false`.

`test` loads saved project source through the standard Clojure CLI and runs
`clojure.test` with the selected aliases and fixtures. `--var namespace/name`
selects a single test. Exit status `0` means the tests passed, `1` means an
assertion failed or a test reported an error, and `2` means the run remained
incomplete. JSON stdout contains test counts, source locations where available,
and assertion or exception details. Runtime logs go to stderr.
[JVM tests and CI](../reference.md#jvm-tests)

For a library release, `export-externs` produces the current public contracts
from a complete check of the release sources. Run `diff-externs` against the
previous snapshot. The JVM smoke fixture changes `:route` from optional to
required and confirms that the diff returns `1` with reason `optional_field`
at the first argument's `:route` field. [Contract comparison](../reference.md#comparing-releases)

Launch `parenox lsp --alias test` from the editor. The LSP4IJ template maps
`*.clj` and `*.cljc` to Clojure and `*.edn` to EDN. In RustRover, a checkout
inside the existing window can be selected with
`lsp --deps /path/to/deps.edn --alias test`. Clojure buffers outside those
source roots get standalone checks.
Unsaved range edits, hover, definitions, EDN syntax, and watched source updates
work in the configured project. [Editor setup](../reference.md#editor-integration)

The CLI, editor and JSON Lines agent share disk project selection. For source
directories of resolved dependencies, generate a classpath file with
`clojure -Spath -M:test` and pass it to `--classpath`. A local dependency in a
directory named `catalog source` provided its checked signature and definition,
and `demo.app/item` returned `"ADA"` on the pinned JVM. A JSON Lines session on
URI opened five files, found an in-memory misspelling, retained it on
`refresh_project`, cleared it on an edit, and resolved the definition at
`normalize.cljc:82:7`. [Agent protocol](../reference.md#agent-sessions)

## Results on pinned projects

| Project | Clojure files | Baseline errors (reader) | Current errors | Current coverage warnings | JVM tests / assertions |
|---|---:|---:|---:|---:|---:|
| [lambdaisland/uri `e439c2c`](https://github.com/lambdaisland/uri/tree/e439c2ca068849df620e1c9275466cb1e8171aaa) | 5 | 62 (0) | 0 | 62 | 16 / 132 |
| [Medley `54f45ab`](https://github.com/weavejester/medley/tree/54f45abc16ecd30121d05ef52014af8c65d4635f) | 2 | 2 (2) | 0 | 110 | 53 / 280 |
| [data.json `1fce907`](https://github.com/clojure/data.json/tree/1fce9070ee851f49a6c4ce6e1c0bb0db4d7594a0) | 5 | 5 (5) | 0 | 350 | 244 / 324 |
| [Malli `91d3c1b`](https://github.com/metosin/malli/tree/91d3c1bb81288d5b2e3d260fd9847da3b74c631c) | 70 | 530 (37) | 0 | 1563 | 251 / 18800 |

The starting build reported incomplete analysis at error severity in many
cases. The current counts show the remaining coverage gaps. All four current
reports have `complete: false` and exit `0` with `--allow-incomplete`. The
static baseline and current runs used the same source roots for each project.
The final CLI and JSON Lines reports matched per-file paths, source hashes,
coverage flags, diagnostics and suppressions for all four snapshots.

Temporary manifest copies added Medley's test paths and data.json's test paths
and test.check dependency. Test suites passed with the project's Clojure
version for URI (`1.11.1`), Medley (`1.9.0`) and Malli (`1.13.0-alpha8`).
All four suites also passed on the reference `1.13.0-alpha11` in manifest
copies. The pinned source checkouts were clean after the runs.
The final reference suites took 2.05 s for URI, 1.16 s for Medley, 14.15 s for
data.json, and 24.02 s for Malli in sequential runs.

- In the existing RustRover window, URI navigated from `uri.cljc:134` to
  `normalize.cljc:82:7`. An unsaved misspelling and invalid EDN both gained a
  diagnostic and cleared after correction. The final run found the repeated
  `01` in an EDN set at line `1:13`, while the file on disk kept its original
  content. A two-character LSP4IJ range edit
  placed a test typo at `normalize_test.cljc:48:21`. The diagnostic and
  definition continued to work with another checkout open. The original
  LSP4IJ command was restored after testing.
- On URI copies, removing a nil guard triggered `PNX2322` at `uri.cljc:135:54`
  and an exception on the same JVM source line. Passing an extra argument to
  `str-len` triggered `PNX2554` at `normalize.cljc:54:22`. An invalid `charAt`
  index pointed to `platform.clj:18` in both the check and JVM test report.
  The pinned editor smoke compares the unsaved nil and test-call diagnostics
  with a fresh CLI check of the same source.
- URI's changed `query-map-test` returned `1` with the failed expectation at
  `uri_test.cljc:152`. A changed fixture returned `2`, phase `execution`, with
  an `IllegalStateException` at `uri_test.cljc:237`. A malformed manifest
  returned `2`, phase `launch`. An early JVM exit returned `2`, phase `execution`.
- Static checking located a misspelling inside Medley's `thrown?` and `letfn`
  at `core_test.cljc:244:45` before the JVM hit the load error on line 244.
  In data.json, Parenox located the second spelling of a duplicate BigInt key
  on line 58, and the reference JVM rejected the same map. A runtime exception
  in `StringPBR.readChar` points to `json.clj:53`.
- Malli's `m/-properties` resolves from `util.cljc:91` to the protocol method
  on `core.cljc:39`. The call to `m/eval` resolves to the definition inside
  a top-level `let` at `core.cljc:2909`. Known source macros yield coverage
  warnings, and the misspelled `mx/defnn` still produces a hard diagnostic.

## Measurements

Measurements used macOS 26.5 arm64, Rust 1.100.0 and Java 21.0.2. The clean
release builds used independent target directories and an existing Cargo
package cache. Both workspaces have the same 27 third-party packages in the
normal and build dependency graph.

| Workload | `6125176` | `4efbddc` |
|---|---:|---:|
| Clean locked release build | 23.68 s | 22.85 s |
| Release binary | 4,661,024 bytes | 5,156,848 bytes |
| URI CLI check median, 50 fresh processes | 3.17 ms | 3.67 ms |
| URI first initialization of the newly built binary | 366 ms | 383 ms |
| URI initialization median, 10 subsequent fresh processes | 2.08 ms | 2.20 ms |
| URI first open of `uri.cljc` | 1.90 ms | 1.91 ms |
| URI edit and diagnostics p95, 200 full edits | 0.406 ms | 0.507 ms |
| URI hover and definition p95 | 28 / 36 µs | 33 / 37 µs |
| URI LSP RSS after initialization | 2,608 KiB | 2,784 KiB |
| URI LSP RSS after edits | 6,000 KiB | 6,640 KiB |
| Malli CLI check median, 30 fresh processes | 14.22 ms | 33.88 ms |

The CLI trials used the same aliases, JSON format and warmed filesystem caches.
The URI editor benchmark checked detection and correction of the same typo
and the same definition on each iteration. The first initialization row records
one sample per newly built executable. The Malli CLI times accompany the
expanded reader coverage and warning counts in the results table.
During a five-second idle sample after initialization, `ps` reported 0.0% CPU
and the same cumulative CPU time at both ends on each build.

With the same Malli workspace open, 100 full edits to `destructure_test.cljc`
took 3.45 ms p95 before open-document checks and 1.32 ms p95 on `4efbddc`.
Each edit retained six diagnostics with matching codes, severities and
messages. RSS after edits was 32,800 KiB and 31,472 KiB respectively.

On `malli/core.cljc`, the first edit's JSON body was 152,044 bytes with a full
replacement and 304 bytes with a range. The same build's p95 round trip moved
from 5.39 to 5.12 ms across 100 edits per transport. Both transports retained
212 diagnostics per edit with matching codes, severities and messages.

The clinic agent loop used 100 requests per transport. Warm check p95 was
38.6 µs for snapshots and 32.1 µs for deltas. Warm body edit p95 was 66.5 µs
and 62.7 µs respectively. Each body edit reparsed one file and reused four
others.

Eight JVM runs of the same URI namespace and its 109 assertions had median
times of 1.60 s directly through Clojure CLI and 1.63 s through `parenox test`.
Both measurements include JVM startup with warmed dependency caches. The
portable `.cljc` fixture passed 3 tests and 7 assertions on both the pinned JVM
and clojurust. Ten fresh process runs of that fixture gave medians of 779 ms
on the JVM and 15.6 ms on the native runtime. [Portable runtime trial](type-checker-research.md#portable-runtime-trial)

## Reproduce the checks

The final `just verify` passed 554 Rust tests. The six opt-in tests also ran in
the JVM smoke, pinned URI smoke, and benchmark commands.

```sh
just verify
just runtime-smoke
just jvm-test-smoke
just agent-loop
just agent-loop 100 target/release/parenox 0 0 delta
just uri-editor-smoke /path/to/uri
just uri-editor-bench /path/to/uri
just malli-editor-bench /path/to/malli target/release/parenox full core
just malli-editor-bench /path/to/malli target/release/parenox range core
just portable-runtime-smoke /path/to/cljrs
```

## Remaining coverage

General macro expansion, generated contracts, JVM dispatch and JAR signatures
remain areas of incomplete coverage. A next slice can recover declarative var
names and arities from a known source macro such as `malli.experimental/defn`.
Its acceptance test should check that adding or removing a generated var updates
CLI and editor resolution with the original definition span. Schema typing can
then build on those declarations.

The full commit series is available with
`git log --reverse --oneline 6125176..4efbddc`.
