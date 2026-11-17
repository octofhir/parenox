use parenox::{Report, Session, Source};

fn check(body: &str) -> Report {
    let source = format!("(ns demo (:require [clojure.spec.alpha :as s]))\n{body}");
    Session::default().check("maps.clj", &source)
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
fn knows_which_keys_can_be_present_and_preserves_explicit_nil() {
    for (spec, ret, expression, expected) in [
        (
            "(s/map-of string? number?)",
            "boolean?",
            "(get m :open false)",
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
           (defn widened {:t/sig '[map? :-> number?]} [m] (tally m))
           (defn other {:t/sig '[(s/map-of string? number?) :-> number?]} [m] (tally m))"#,
    );
    assert!(report.complete && !report.ok, "{report:#?}");
    assert_eq!(report.diagnostics.len(), 3, "{report:#?}");
    assert!(
        report.diagnostics.iter().all(|d| {
            d.code == "PNX2322" && d.expected.as_deref() == Some("Map<Keyword, Number>")
        })
    );

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
