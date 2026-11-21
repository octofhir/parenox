use parenox::{DocumentHover, Session};

#[test]
fn juxt_preserves_the_types_and_order_of_projected_fields() {
    let source = r#"(ns demo (:require [clojure.spec.alpha :as s]))
        (defn keep-pairs
          {:t/sig '[(s/coll-of (s/tuple string? number?) :kind vector?)
                    :-> (s/coll-of (s/tuple string? number?) :kind vector?)]}
          [pairs]
          pairs)
        (defn projected
          {:t/sig '[(s/coll-of {:keep-key string? :keep-val number?} :kind vector?)
                    :-> (s/coll-of (s/tuple string? number?) :kind vector?)]}
          [rows]
          (let [pairs (mapv (juxt :keep-key :keep-val) rows)]
            (keep-pairs pairs)))"#;
    let mut session = Session::default();
    let report = session.check("juxt.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");
    let offset = source.rfind("pairs").unwrap();
    let Some(DocumentHover::Local(hover)) = session.document_hover_at("juxt.clj", offset) else {
        panic!("missing result hover");
    };
    assert_eq!(hover.inferred_type, "Vector<Tuple<String, Number>>");

    let wrong = source.replace("juxt :keep-key :keep-val", "juxt :keep-val :keep-key");
    let report = session.check("juxt.clj", &wrong);
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(finding.code, "PNX2322");
    assert_eq!(
        finding.actual.as_deref(),
        Some("Vector<Tuple<Number, String>>")
    );
    let range = finding.span.start.offset..finding.span.end.offset;
    assert_eq!(&wrong[range], "pairs");
}

#[test]
fn juxt_passes_all_arguments_and_keeps_each_falsey_result() {
    let source = r#"(ns demo (:require [clojure.spec.alpha :as s]))
        (defn pair {:t/sig '[:-> (s/tuple nil? #{false})]} []
          ((juxt (fn [left right] (if left right nil))
                 (fn [left right] (if right left false)))
           false nil))
        (defn zero-invocation {:t/sig '[:-> (s/tuple #{:first})]} []
          ((juxt (fn [] :first))))"#;
    let report = Session::default().check("juxt.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn juxt_stops_calling_functions_after_the_first_exception() {
    let source = r#"(ns demo)
        (defn f {:t/sig '[:-> #{:first}]} []
          (try
            ((juxt (fn [_] (throw (ex-info "first" {:where :first})))
                   (fn [_] (inc "uninvoked")))
             0)
            (catch clojure.lang.ExceptionInfo error (:where (ex-data error)))))"#;
    let report = Session::default().check("juxt.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn juxt_requires_a_component_before_evaluating_outer_arguments() {
    let source = r#"(ns demo)
        (defn f {:t/sig '[:-> #{:arity}]} []
          (try
            ((juxt) (inc "unreachable"))
            (catch clojure.lang.ArityException error :arity)))"#;
    let report = Session::default().check("juxt.clj", source);
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(finding.code, "PNX2554");
}

#[test]
fn juxt_stops_after_a_keyword_arity_error() {
    let source = r#"(ns demo)
        (defn f {:t/sig '[:-> #{:arity}]} []
          (try
            ((juxt :path (fn [] (inc "unreachable"))))
            (catch clojure.lang.ArityException error :arity)))"#;
    let report = Session::default().check("juxt.clj", source);
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(finding.code, "PNX2554");
    assert!(finding.message.starts_with("Keyword invocation"));
}
