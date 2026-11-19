# HTTP response and PostgreSQL query corpus

This is a source-only checker example, inspired by the Typespec
`context-web-pg` example. It assembles a Ring-style response, describes the
SQL boundary with a shared EDN contract, and validates the shape of a row
without assuming that SQL proves the schema. No command below starts HTTP,
connects to PostgreSQL or loads the `pg` namespace.

`pg.edn` describes only the string-parameter vector accepted by this corpus.
The normal result of `pg/execute!` is a vector of maps, not a vector of
statically proven Todo rows. Both checkers read the same Clojure files and
that same EDN contract. The rest-binding loop is adapted from
[`dsql.core/reduce-separated-old`](https://github.com/context-clj/pg/blob/aa8ea0eb565e7f70f80f3248e6a83198628e1047/src/dsql/core.clj).

Build Parenox with `just build`, then from this directory:

```sh
../../target/release/parenox check --format plain
../../target/release/parenox check --config parenox.edn src negative --format plain
```

To compare, from a Typespec checkout at
`dfc58974f6ae1f6e4e90c2ad9686fd2ddce91f21`:

```sh
clojure -M -m typespec.main check --root /absolute/path/to/parenox/examples/context-web-pg
clojure -M -m typespec.main check --root /absolute/path/to/parenox/examples/context-web-pg src negative
```

Both checkers accept the normal corpus and catch a numeric request title
and `inc` on a String from the sequence in the negative files. In
`checked-row`, a successful `(s/valid? ::row row)` proves the required
`:title` field is a String without trusting the raw SQL result. The negative
files are for static analysis only.
