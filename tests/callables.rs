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
    let [map, keyword, bounds] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(map.code, "PNX2554");
    assert!(map.message.contains("Map invocation"));
    assert_eq!(keyword.code, "PNX2554");
    assert!(keyword.message.contains("Keyword invocation"));
    assert_eq!(bounds.code, "PNX2801");

    let source = "(ns demo (:require [clojure.spec.alpha :as s]))
       (defn labels {:t/sig '[(s/keys) :-> string?]} [shape]
         ((identity :label) shape))";
    let report = Session::default().check("callables.clj", source);
    assert!(!report.complete, "{report:#?}");
    assert_eq!(report.diagnostics[0].code, "PNX9003");
}

#[test]
fn vector_invocation_uses_jvm_integer_keys_and_strict_bounds() {
    for (expression, contract) in [
        (r#"([1 "chosen"] 1)"#, "string?"),
        (r#"(let [index 1] ([1 "chosen"] index))"#, "string?"),
        (r#"(apply [1 "chosen"] [1])"#, "string?"),
        (r#"([1 "chosen"] 4294967297)"#, "string?"),
        (r#"([1 "chosen"] 01)"#, "string?"),
        (
            r#"(mapv (fn [entry] (entry 1)) {:a "value"})"#,
            "(clojure.spec.alpha/coll-of string? :kind vector?)",
        ),
    ] {
        let source = format!("(ns demo) (defn run {{:t/sig '[:-> {contract}]}} [] {expression})");
        let report = Session::default().check("index.clj", &source);
        assert!(report.ok && report.complete, "{expression}: {report:#?}");
    }
    for (expression, code) in [
        ("([1] 0 2)", "PNX2554"),
        ("([1] 1)", "PNX2801"),
        ("([1] -1)", "PNX2801"),
        ("([1] 0.0)", "PNX2322"),
        ("([1] :missing)", "PNX2322"),
    ] {
        let source = format!("(ns demo) (defn run {{:t/sig '[:-> number?]}} [] {expression})");
        let report = Session::default().check("index.clj", &source);
        let [diagnostic] = report.diagnostics.as_slice() else {
            panic!("{expression}: {report:#?}");
        };
        assert_eq!(diagnostic.code, code);
        if code == "PNX2801" {
            assert!(
                !diagnostic
                    .help
                    .as_deref()
                    .unwrap()
                    .contains("third argument")
            );
        }
    }
    let report = Session::default().check(
        "index.clj",
        "(ns demo) (defn run {:t/sig '[number? :-> number?]} [i] ([1] i))",
    );
    assert!(!report.complete, "{report:#?}");
    assert_eq!(report.diagnostics[0].code, "PNX9001");
}
