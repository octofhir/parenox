# Clinic example

Run from the repository root:

```sh
just errors          # highlighted source and proposed edits
just errors plain    # compact text
just errors json     # one project report
just agent-loop      # fix in memory, recheck, measure cache reuse
```

The fixture intentionally contains three errors:

| Location | Problem | Fix |
|---|---|---|
| `http.clj`, `store/find-patient` | `42` is a number; patient ids are strings | Pass the local `id` |
| `http.clj`, `str/starts-with?` | The first argument must be a string | Pass the local `id` |
| `service.clj`, `str/upper-case` | An optional name can be nil | Use `(or (:name patient) "Unknown")` |

`externs.edn` provides the trusted external library signature. The fixture uses
four namespaces and shared spec shapes. No Clojure/JVM process is needed for
static checking. `just agent-loop` applies the fixes only to in-memory snapshots;
these intentionally broken source files remain available for manual inspection.

The checker itself returns exit code 1 for diagnostics. `just errors` accepts
that expected exit status, while preserving usage and I/O failures.
