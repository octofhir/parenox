# Storage to HTTP response

Run `just map-boundary` from the project root. Parenox reads only `src/` and
`externs.edn`. The extern is a trusted declaration of `fetch`, not a check of
its implementation or of a database schema.

The response builder gets the row type from the required `:row` field of the
store result. It gets the body type from the renderer's required `:body` field.
The handler distinguishes three cases:

- No row: `{:status 404 :body nil}`.
- Row without `:display-name`: `{:status 200 :body "Unknown"}`.
- Row with `:display-name nil`: `{:status 200 :body nil}`.

`just runtime-smoke` runs those cases and a named patient on the pinned Clojure
JVM. It uses the stand-in under `runtime/`; no HTTP server or database is needed.
