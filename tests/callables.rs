use parenox::Session;

#[test]
fn maps_and_keywords_are_functions_in_direct_calls_and_callbacks() {
    let report =
        Session::default().check("callables.clj", include_str!("../examples/callables.clj"));
    assert!(report.ok && report.complete, "{report:#?}");

    for (body, ret) in [
        ("(:label #{:other})", "nil?"),
        ("(:label :not-a-map \"safe\")", "string?"),
        ("({:label false} :label true)", "#{false}"),
        ("(apply :label [{:label \"ready\"}])", "string?"),
        ("(apply {:label \"ready\"} [:label])", "string?"),
        (
            "(let [f (if flag {:label \"ready\"} {})] (f :label \"fallback\"))",
            "string?",
        ),
    ] {
        let source = format!("(ns demo) (defn f {{:t/sig '[boolean? :-> {ret}]}} [flag] {body})");
        let report = Session::default().check("callables.clj", &source);
        assert!(report.ok && report.complete, "{body}: {report:#?}");
    }
}

#[test]
fn callable_collections_report_arity_and_keep_undeclared_field_diagnostics() {
    let report = Session::default().check(
        "callables_bad.clj",
        include_str!("../examples/callables_bad.clj"),
    );
    let [map, keyword] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(map.code, "PNX2554");
    assert!(map.message.contains("Map invocation"));
    assert_eq!(keyword.code, "PNX2554");
    assert!(keyword.message.contains("Keyword invocation"));

    let source = "(ns demo (:require [clojure.spec.alpha :as s]))
       (defn labels {:t/sig '[(s/keys) :-> string?]} [shape]
         ((identity :label) shape))";
    let report = Session::default().check("callables.clj", source);
    assert!(!report.complete, "{report:#?}");
    assert_eq!(report.diagnostics[0].code, "PNX9003");
}
