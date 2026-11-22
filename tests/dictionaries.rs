use parenox::{Report, Session, Source};

fn check(body: &str) -> Report {
    let source = format!("(ns demo (:require [clojure.spec.alpha :as s]))\n{body}");
    Session::default().check("maps.clj", &source)
}

#[test]
fn checks_lsp_change_maps_with_tuple_entry_specs() {
    let source = r#"
(s/def ::edits (s/coll-of string? :kind vector?))
(s/def ::changes (s/coll-of (s/tuple string? ::edits) :kind map?))
(defn accept {:t/sig '[::changes :-> number?]} [changes] (count changes))
(defn edits {:t/sig '[::changes :-> ::edits]} [changes]
  (get changes "file" []))
(defn send {:t/sig '[:-> number?]} [] (accept {}) (accept {"file" ["edit"]}))
"#;
    let report = check(source);
    assert!(report.ok && report.complete, "{report:#?}");

    for bad in [
        r#"{"file" [1]}"#,
        r#"{:file ["edit"]}"#,
        r#"[["file" ["edit"]]]"#,
        "nil",
    ] {
        let report = check(&source.replace(r#"{"file" ["edit"]}"#, bad));
        let [finding] = report.diagnostics.as_slice() else {
            panic!("{bad}: {report:#?}")
        };
        assert!(report.complete, "{bad}: {report:#?}");
        assert_eq!(finding.code, "PNX2322", "{bad}: {finding:#?}");
    }
}

#[test]
fn leaves_custom_map_kinds_and_other_entry_specs_unchecked() {
    for (kind, predicate, entry) in [
        ("map?", "", "(s/tuple string?)"),
        (
            "map?",
            "(defn map? {:t/sig '[string? :-> boolean?]} [x] true)",
            "(s/tuple string? number?)",
        ),
    ] {
        let report = check(&format!(
            "{predicate} (defn accept {{:t/sig '[(s/coll-of {entry} :kind {kind}) :-> number?]}} [x] 0)"
        ));
        let [finding] = report.diagnostics.as_slice() else {
            panic!("{report:#?}")
        };
        assert!(!report.complete, "{report:#?}");
        assert_eq!(finding.code, "PNX9001");
    }
}

#[test]
fn iteration_and_updates_respect_dictionary_contracts() {
    let mut session = Session::default();
    let example = session.check(
        "dictionaries.clj",
        include_str!("../examples/dictionaries.clj"),
    );
    assert!(example.ok && example.complete, "{example:#?}");

    let source = include_str!("../examples/dictionaries_bad.clj");
    let report = session.check("dictionaries_bad.clj", source);
    assert!(report.complete && !report.ok, "{report:#?}");
    let [callback, returned] = &report.diagnostics[..] else {
        panic!("{report:#?}");
    };
    assert_eq!(
        &source[callback.span.start.offset..callback.span.end.offset],
        "sku"
    );
    assert_eq!(callback.expected.as_deref(), Some("Number"));
    assert_eq!(callback.actual.as_deref(), Some("String"));
    assert_eq!(returned.expected.as_deref(), Some("Map<Keyword, Number>"));
    assert!(
        returned
            .actual
            .as_deref()
            .unwrap()
            .contains(":open: String")
    );
}

#[test]
fn dictionary_lookups_allow_custom_comparators_and_preserve_explicit_nil() {
    for (spec, ret, expression, expected) in [
        (
            "(s/map-of string? number?)",
            "boolean?",
            "(get m :open false)",
            Some("PNX2322"),
        ),
        (
            "(s/map-of string? number?)",
            "#{false}",
            "(contains? m :open)",
            Some("PNX2322"),
        ),
        (
            "(s/map-of string? number?)",
            "number?",
            "(req! m :open)",
            None,
        ),
        (
            "(s/map-of keyword? (s/nilable number?))",
            "number?",
            "(get m :open 0)",
            Some("PNX2322"),
        ),
        (
            "(s/map-of keyword? (s/nilable number?))",
            "number?",
            "(if (contains? m :open) (inc (:open m)) 0)",
            Some("PNX2322"),
        ),
        (
            "(s/map-of keyword? number?)",
            "number?",
            "(if (contains? m :open) (inc (:open m)) (get m :open 0))",
            None,
        ),
        (
            "(s/map-of #{:closed} number?)",
            "number?",
            "(req! m :open)",
            None,
        ),
        (
            "(s/map-of #{:closed} number?)",
            "number?",
            "(req! {:closed 1} :open)",
            Some("PNX2741"),
        ),
        (
            "(s/map-of #{:open} number?)",
            "(s/coll-of string? :kind vector?)",
            r#"(mapv val (assoc m :open "ready"))"#,
            None,
        ),
        (
            "(s/map-of keyword? (s/map-of keyword? number?))",
            "number?",
            "(get-in (update-in m [:today :open] (fnil inc 0)) [:today :open])",
            None,
        ),
    ] {
        let report = check(&format!(
            "(defn f {{:t/sig '[{spec} :-> {ret}]}} [m] {expression})"
        ));
        assert!(report.complete, "{expression}: {report:#?}");
        let codes: Vec<_> = report.diagnostics.iter().map(|d| d.code.as_str()).collect();
        assert_eq!(codes, expected.as_slice(), "{expression}: {report:#?}");
    }
}

#[test]
fn compares_keyword_shapes_and_dictionary_key_value_types() {
    let report = check(
        r#"(defn tally {:t/sig '[(s/map-of keyword? number?) :-> number?]} [m] (count m))
           (defn ok {:t/sig '[:-> number?]} [] (+ (tally {}) (tally {:open 1})))
           (defn wrong {:t/sig '[:-> number?]} [] (tally {:open "12"}))
           (defn wrong-key {:t/sig '[:-> number?]} [] (tally {"open" 1}))
           (defn widened {:t/sig '[map? :-> number?]} [m] (tally m))
           (defn other {:t/sig '[(s/map-of string? number?) :-> number?]} [m] (tally m))"#,
    );
    assert!(report.complete && !report.ok, "{report:#?}");
    let [value, key, widened, other] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert!(report.diagnostics.iter().all(|d| d.code == "PNX2322"));
    assert_eq!(value.expected.as_deref(), Some("Number"));
    assert_eq!(value.actual.as_deref(), Some("\"12\""));
    assert!(value.message.contains("field :open"));
    assert_eq!(key.expected.as_deref(), Some("Keyword"));
    assert_eq!(key.actual.as_deref(), Some("\"open\""));
    assert!(key.message.contains("map key"));
    assert_eq!(widened.expected.as_deref(), Some("Map<Keyword, Number>"));
    assert_eq!(other.expected.as_deref(), Some("Map<Keyword, Number>"));

    let report = check(
        "(defn f {:t/sig '[(s/map-of keyword? number? :min-count 1) :-> number?]}
           [m] (count m))",
    );
    assert!(!report.complete && !report.ok, "{report:#?}");
    assert_eq!(report.diagnostics[0].code, "PNX9001");
}

#[test]
fn rechecks_cached_callers_when_dictionary_entry_types_change() {
    let mut session = Session::default();
    let check_project = |session: &mut Session, definition: &str| {
        session.check_project(
            &[
                Source {
                    path: "store.clj",
                    source: definition,
                },
                Source {
                    path: "report.clj",
                    source: "(ns report (:require [store :as store] [clojure.spec.alpha :as s]))
                      (defn increments {:t/sig '[:-> (s/coll-of number? :kind vector?)]}
                        [] (mapv (fn [[_ n]] (inc n)) (store/counts)))",
                },
            ],
            None,
        )
    };
    let before = "(ns store (:require [clojure.spec.alpha :as s]))
                  (defn counts {:t/sig '[:-> (s/map-of keyword? number?)]} [] {:open 3})";
    let after = before.replace("number?", "string?").replace("3}", "\"3\"}");
    assert!(check_project(&mut session, before).ok);
    assert_eq!(
        check_project(&mut session, before).stats.checked_functions,
        0
    );
    let report = check_project(&mut session, &after);
    assert_eq!(report.stats.checked_functions, 2, "{report:#?}");
    assert_eq!(
        report.files[1].diagnostics[0].actual.as_deref(),
        Some("String")
    );
}
