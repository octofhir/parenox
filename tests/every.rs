use parenox::{DocumentHover, Session};

#[test]
fn every_checks_homogeneous_elements_at_the_callback_use() {
    let source = r#"(ns demo (:require [clojure.spec.alpha :as s]))
        (defn node {:t/sig '[(s/coll-of string? :kind vector?) :-> string?]} [loc]
          (nth loc 0 ""))
        (defn all-ignorable?
          {:t/sig '[(s/coll-of (s/coll-of string? :kind vector?) :kind vector?) :-> boolean?]}
          [selected]
          (every? #(= (node %) "") selected))"#;
    let mut session = Session::default();
    let report = session.check("every.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");
    let offset = source.find("%)").unwrap();
    let Some(DocumentHover::Local(hover)) = session.document_hover_at("every.clj", offset) else {
        panic!("missing hover for the selected element");
    };
    assert_eq!(hover.inferred_type, "Vector<String>");

    let changed = source.replace("(node %)", "(do (inc %) (node %))");
    let report = session.check("every.clj", &changed);
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    let range = finding.span.start.offset..finding.span.end.offset;
    assert_eq!(finding.code, "PNX2322");
    assert_eq!(finding.actual.as_deref(), Some("Vector<String>"));
    assert_eq!(&changed[range], "%");
}

#[test]
fn every_returns_boolean_at_the_first_rejection_or_end() {
    let source = r#"(ns demo)
        (defn empty {:t/sig '[:-> #{true}]} [] (every? nil []))
        (defn nil-collection {:t/sig '[:-> #{true}]} [] (every? (fn [x] (inc "dead")) nil))
        (defn accepts-truthy {:t/sig '[:-> #{true}]} [] (every? identity ["" 0 :key]))
        (defn rejects-nil {:t/sig '[:-> #{false}]} [] (every? identity [nil "later"]))
        (defn rejects-false {:t/sig '[:-> #{false}]} []
          (every? (fn [x] (if x (inc "unreached") false)) [false 1]))
        (defn stops-on-error {:t/sig '[:-> #{:hit}]} []
          (try
            (every? (fn [x] (if x (throw (ex-info "hit" {})) false)) [true false])
            (catch clojure.lang.ExceptionInfo _ :hit)))"#;
    let report = Session::default().check("every.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn every_leaves_an_explicit_gap_for_an_opaque_collection() {
    let source = r#"(ns demo)
        (defn f {:t/sig '[t/any :-> boolean?]} [opaque]
          (every? (fn [x] x) opaque))"#;
    let report = Session::default().check("every.clj", source);
    assert!(!report.complete, "{report:#?}");
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(finding.code, "PNX9001");
    let range = finding.span.start.offset..finding.span.end.offset;
    assert_eq!(&source[range], "opaque");
}
