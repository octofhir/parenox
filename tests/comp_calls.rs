use parenox::{DocumentHover, Session};

#[test]
fn comp_infers_the_value_passed_between_filter_callbacks() {
    let source = r#"(ns demo (:require [clojure.spec.alpha :as s]))
        (defn local-usages
          {:t/sig '[symbol? (s/coll-of {:to symbol?} :kind vector?)
                    :-> (s/coll-of {:to symbol?} :kind vector?)]}
          [source-ns inner-usages]
          (->> inner-usages (filterv (comp #(= source-ns %) :to))))"#;
    let mut session = Session::default();
    let report = session.check("compose.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");
    let offset = source.find("%)").unwrap();
    let Some(DocumentHover::Local(hover)) = session.document_hover_at("compose.clj", offset) else {
        panic!("missing hover for the intermediate value");
    };
    assert_eq!(hover.inferred_type, "Symbol");

    let wrong = source.replace("source-ns %", "source-ns (inc %)");
    let report = session.check("compose.clj", &wrong);
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(finding.code, "PNX2322");
    assert_eq!(finding.actual.as_deref(), Some("Symbol"));
    let range = finding.span.start.offset..finding.span.end.offset;
    assert_eq!(&wrong[range], "%");
    assert_eq!(finding.contract.as_ref().unwrap().name, "clojure.core/inc");
}

#[test]
fn comp_applies_all_arguments_to_the_rightmost_stage_and_forwards_one_value() {
    let source = r#"(ns demo)
        (defn empty-identity {:t/sig '[:-> #{"value"}]} [] ((comp) "value"))
        (defn one-keyword {:t/sig '[:-> #{:exact}]} []
          (case ((comp :a) {:a 5}) 5 :exact :lost))
        (defn multi-stage {:t/sig '[:-> string?]} [] ((comp str inc count) "abc"))
        (defn map-stage {:t/sig '[:-> #{:exact}]} []
          (case ((comp {:kept 17} :k) {:k :kept}) 17 :exact :lost))
        (defn variadic {:t/sig '[:-> number?]} []
          ((comp inc (fn [first second & rest] (+ first second (count rest)))) 1 2 3 4))
        (defn zero-arguments {:t/sig '[:-> #{:exact}]} []
          (case ((comp inc (fn [] 5))) 6 :exact :lost))
        (defn applies {:t/sig '[:-> #{:exact}]} []
          (case (apply (comp inc +) [1 2]) 4 :exact :lost))
        (defn forwards-false {:t/sig '[:-> #{:false}]} []
          ((comp (fn [x] (if x :lost (if (nil? x) :lost :false))) (fn [] false))))
        (defn forwards-nil {:t/sig '[:-> #{:nil}]} []
          ((comp (fn [x] (if (nil? x) :nil :lost)) (fn [] nil))))"#;
    let report = Session::default().check("compose.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn comp_preserves_constructor_and_invocation_order() {
    let source = r#"(ns demo)
        (defn constructor {:t/sig '[:-> #{:constructor}]} []
          (try
            ((comp identity
                   (throw (ex-info "constructor" {:stage :constructor}))
                   (inc "later construction"))
             (inc "later argument"))
            (catch clojure.lang.ExceptionInfo error (:stage (ex-data error)))))
        (defn rightmost {:t/sig '[:-> #{:right}]} []
          (try
            ((comp (fn [_] (throw (ex-info "left" {:stage :left})))
                   (fn [_] (throw (ex-info "right" {:stage :right}))))
             nil)
            (catch clojure.lang.ExceptionInfo error (:stage (ex-data error)))))"#;
    let report = Session::default().check("compose.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn comp_checks_uninvoked_callback_syntax_without_evaluating_its_body() {
    let source = r#"(ns demo (:require [clojure.spec.alpha :as s]))
        (defn f {:t/sig '[:-> (s/coll-of number? :kind vector?)]} []
          (mapv (comp (fn [x] unbound)
                      (fn [x] (inc "not invoked")))
                []))"#;
    let report = Session::default().check("compose.clj", source);
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(finding.code, "PNX2304");
    let range = finding.span.start.offset..finding.span.end.offset;
    assert_eq!(&source[range], "unbound");
}

#[test]
fn a_rejected_rightmost_arity_stops_composition() {
    for rightmost in ["inc", "(fn [item] item)"] {
        let source = format!(
            "(ns demo)
             (defn f {{:t/sig '[:-> #{{:arity}}]}} []
               (try ((comp (fn [_] (inc \"unreachable\")) {rightmost}))
                    (catch clojure.lang.ArityException _ :arity)))",
        );
        let report = Session::default().check("compose.clj", &source);
        let [finding] = report.diagnostics.as_slice() else {
            panic!("{report:#?}");
        };
        assert_eq!(finding.code, "PNX2554");
    }
}
