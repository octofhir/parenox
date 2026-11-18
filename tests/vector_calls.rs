use parenox::{Report, Session};

fn check(body: &str) -> Report {
    let source = format!("(ns demo (:require [clojure.spec.alpha :as s]))\n{body}");
    Session::default().check("vectors.clj", &source)
}

#[test]
fn checks_vector_pipelines_and_narrows_elements_with_core_predicates() {
    let report = Session::default().check("vectors.clj", include_str!("../examples/vectors.clj"));
    assert!(report.ok && report.complete, "{report:#?}");

    for (expr, element) in [
        (r#"(filterv string? [nil "Ada" 1])"#, r#"#{ "Ada" }"#),
        (r#"(filterv nil? [nil "Ada"])"#, "nil?"),
        (r#"(filterv identity [false nil ""])"#, "string?"),
        ("(filterv some? [false nil])", "boolean?"),
        ("(mapv identity #{:active})", "keyword?"),
        (r#"(mapv identity "Ada")"#, "char?"),
        ("(mapv inc [] [1])", "string?"),
        (r#"(mapv inc (next (seq ["skipped" 1 2])))"#, "number?"),
        (
            r#"(filterv string? (seq [nil "Ada" false]))"#,
            r#"#{ "Ada" }"#,
        ),
        ("(mapv inc (next (seq [1])))", "string?"),
        ("(filterv (fn [x] false) [1])", "string?"),
    ] {
        let report = check(&format!(
            "(defn f {{:t/sig '[:-> (s/coll-of {element} :kind vector?)]}} [] {expr})"
        ));
        assert!(report.ok && report.complete, "{expr}: {report:#?}");
    }
}

#[test]
fn points_at_elements_and_callback_bodies_without_suggesting_collection_replacements() {
    let source = include_str!("../examples/vectors_bad.clj");
    let report = Session::default().check("vectors_bad.clj", source);
    assert!(!report.ok && report.complete, "{report:#?}");
    let [direct, inline] = &report.diagnostics[..] else {
        panic!("{report:#?}");
    };
    let fragment = |d: &parenox::Diagnostic| &source[d.span.start.offset..d.span.end.offset];
    assert_eq!(fragment(direct), r#"["2"]"#);
    assert_eq!(direct.actual.as_deref(), Some("String"));
    assert!(direct.suggestions.is_empty());
    assert!(
        direct
            .help
            .as_deref()
            .is_some_and(|help| help.contains("elements"))
    );
    assert_eq!(fragment(inline), "name");
    assert_eq!(inline.expected.as_deref(), Some("String"));
    assert_eq!(inline.suggestions[0].replacement, r#"(or name "")"#);
}

#[test]
fn empty_inputs_skip_invocation_but_still_check_callback_expressions() {
    for (expr, code) in [
        (r#"(mapv (fnil inc (inc "bad")) [])"#, "PNX2322"),
        ("(mapv (fn [x y] missing) [])", "PNX2304"),
        ("(mapv inc [1] [2])", "PNX2554"),
        ("(mapv inc 42)", "PNX2322"),
        ("(mapv inc {:x 1})", "PNX2322"),
        (r#"(mapv inc (seq [1 "wrong"]))"#, "PNX2322"),
        ("(filterv number? [1] [2])", "PNX2554"),
    ] {
        let report = check(&format!(
            "(defn f {{:t/sig '[:-> (s/coll-of number? :kind vector?)]}} [] {expr})"
        ));
        assert_eq!(report.diagnostics.len(), 1, "{expr}: {report:#?}");
        assert_eq!(report.diagnostics[0].code, code, "{expr}: {report:#?}");
    }
}

#[test]
fn some_predicate_preserves_false_and_respects_shadowing() {
    for body in [
        r#"(defn f {:t/sig '[:-> (s/coll-of string? :kind vector?)]}
             [] (filterv some? [nil false "Ada"]))"#,
        r#"(defn- some? [value] true)
           (defn f {:t/sig '[:-> (s/coll-of string? :kind vector?)]}
             [] (filterv some? [nil "Ada"]))"#,
    ] {
        let report = check(body);
        assert!(report.complete && !report.ok, "{report:#?}");
        assert_eq!(report.diagnostics.len(), 1, "{report:#?}");
        assert_eq!(report.diagnostics[0].code, "PNX2322", "{report:#?}");
    }
}

#[test]
fn closed_map_entries_preserve_key_value_and_absence_semantics() {
    let report = Session::default().check("entries.clj", include_str!("../examples/entries.clj"));
    assert!(report.ok && report.complete, "{report:#?}");

    for (args, params, expr, element) in [
        (
            "boolean?",
            "[include?]",
            r#"(mapv val (if include? {:x "Ada"} {}))"#,
            "string?",
        ),
        ("", "[]", "(mapv val {:x nil})", "nil?"),
        ("", "[]", "(mapv inc {})", "string?"),
        (
            "",
            "[]",
            "(mapv (fn [[_ _ missing]] missing) {:x 12})",
            "nil?",
        ),
        (
            "",
            "[]",
            "(mapv (fn [[_ _ :as entry]] (count entry)) {:x 12})",
            "number?",
        ),
        (
            "",
            "[]",
            "(mapv (fn [[_ {:keys [n]}]] (inc n)) {:x {:n 12}})",
            "number?",
        ),
        (
            "",
            "[]",
            "(mapv identity {:x 12})",
            "(s/coll-of (s/or :k keyword? :v number?) :kind vector?)",
        ),
    ] {
        let report = check(&format!(
            "(defn f {{:t/sig '[{args} :-> (s/coll-of {element} :kind vector?)]}} {params} {expr})"
        ));
        assert!(report.ok && report.complete, "{expr}: {report:#?}");
    }
}

#[test]
fn map_entry_diagnostics_distinguish_vectors_and_open_maps() {
    let source = include_str!("../examples/entries_bad.clj");
    let report = Session::default().check("entries_bad.clj", source);
    assert!(report.complete && !report.ok, "{report:#?}");
    let [element, vector] = &report.diagnostics[..] else {
        panic!("{report:#?}");
    };
    assert_eq!(element.code, "PNX2322");
    assert_eq!(
        &source[element.span.start.offset..element.span.end.offset],
        "total"
    );
    assert_eq!(element.expected.as_deref(), Some("Number"));
    assert_eq!(element.actual.as_deref(), Some("String"));
    assert!(element.suggestions.is_empty(), "{element:#?}");
    assert_eq!(vector.code, "PNX2322");
    assert_eq!(vector.expected.as_deref(), Some("MapEntry"));
    assert_eq!(vector.actual.as_deref(), Some("Vector<Number | :admitted>"));

    let report = check(
        "(s/def :sample/n number?)
         (defn f {:t/sig '[(s/keys :req-un [:sample/n]) :-> (s/coll-of number? :kind vector?)]}
           [counts] (mapv val counts))",
    );
    assert!(!report.complete && !report.ok, "{report:#?}");
    assert_eq!(report.diagnostics.len(), 1, "{report:#?}");
    assert_eq!(report.diagnostics[0].code, "PNX9001");
    assert!(
        report.diagnostics[0].message.contains("open map"),
        "{report:#?}"
    );
}

#[test]
fn vector_calls_evaluate_the_callback_expression_then_their_collection_arguments() {
    for (label, expression) in [
        (
            "callback",
            r#"(mapv (fnil identity (throw (ex-info "callback" {:stage :callback})))
                     (throw (ex-info "collection" {:stage :collection})))"#,
        ),
        (
            "first",
            r#"(mapv + (throw (ex-info "first" {:stage :first}))
                       (throw (ex-info "second" {:stage :second})))"#,
        ),
        (
            "collection",
            r#"(filterv #(some? %) (throw (ex-info "collection" {:stage :collection})))"#,
        ),
    ] {
        let report = check(&format!(
            "(defn f {{:t/sig '[:-> #{{:{label}}}]}} []
               (try {expression}
                 (catch clojure.lang.ExceptionInfo e (:stage (ex-data e)))))"
        ));
        assert!(report.ok && report.complete, "{expression}: {report:#?}");
    }

    let report = check(
        "(defn f {:t/sig '[:-> nil?]} []
           (mapv (fn [x] (dummy x)) (throw (ex-info \"stop\" {}))))",
    );
    assert_eq!(report.diagnostics.len(), 1, "{report:#?}");
    assert_eq!(report.diagnostics[0].code, "PNX2304");
}

#[test]
fn guaranteed_callbacks_can_terminate_vector_pipelines() {
    for operation in ["mapv", "filterv"] {
        let body = format!(
            r#"(ns demo)
              (defn f {{:t/sig '[:-> #{{:row}}]}} []
                (try
                  ({operation} (fn [_] (throw (ex-info "failed" {{:stage :row}}))) [:only])
                  (catch ExceptionInfo e (:stage (ex-data e)))))"#
        );
        let report = Session::default().check("vectors.clj", &body);
        assert!(report.ok && report.complete, "{operation}: {report:#?}");
    }
}

#[test]
fn empty_collections_still_return_and_open_collections_can_be_empty() {
    let source = r#"(ns demo (:require [clojure.spec.alpha :as s]))
      (defn available {:t/sig '[(s/coll-of number? :kind vector?) :-> (s/coll-of number? :kind vector?)]}
        [xs] (filterv (fn [_] (throw (ex-info "rejected" {}))) xs))
      (defn no-exception {:t/sig '[:-> (s/coll-of number? :kind vector?)]} []
        (try (mapv identity [1 2])
             (catch ExceptionInfo _ :unexpected)))
      (defn shortest-empty {:t/sig '[:-> (s/coll-of number? :kind vector?)]} []
        (try (mapv (fn [_ _] (throw (ex-info "unused" {}))) [1 2] [])
             (catch ExceptionInfo _ :unexpected)))"#;
    let report = Session::default().check("vectors.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");
}
