use parenox::{DocumentHover, Session};

#[test]
fn mutual_calls_keep_their_definition_scope_and_contextual_types() {
    let source = r#"
(ns local-functions)
(defn walk {:t/sig '[:-> #{"captured"}]} []
  (let [label "captured"]
    (letfn [(forward [state] (step state))
            (step [state]
              (if state
                (forward nil)
                (let [text label] text)))]
      (let [label :shadowed]
        (forward true)))))
"#;
    let mut session = Session::default();
    let report = session.check("local_functions.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");
    let offset = source.find("text label").unwrap();
    let Some(DocumentHover::Local(value)) =
        session.document_hover_at("local_functions.clj", offset)
    else {
        panic!("expected contextual hover inside the peer function");
    };
    assert_eq!(value.inferred_type, r#""captured""#);
}

#[test]
fn group_scope_keeps_arity_and_shadowing_with_list_or_vector_definitions() {
    let source = r#"
(ns local-functions (:require [clojure.spec.alpha :as s]))
(defn choose {:t/sig '[:-> (s/tuple #{:single} #{:peer} #{:parameter})]} []
  (letfn ([choose ([one] one) ([one two & more] (first more))]
          (call-peer [choose] (choose {:key :parameter})))
    [(choose :single)
     (choose :ignored :ignored :peer)
     (call-peer :key)]))
(defn empty-body {:t/sig '[:-> nil?]} [] (letfn [(unused [] :unused)]))
(defn empty-group {:t/sig '[:-> #{:empty}]} [] (letfn nil :empty))
"#;
    let report = Session::default().check("local_functions.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn letfn_compiles_overwritten_and_uncalled_definitions() {
    let source = r#"
(ns local-functions)
(defn checked {:t/sig '[:-> #{:last}]} []
  (letfn [(choose [] missing-in-overwritten-body)
          (choose [] :last)
          (unused [] missing-in-unused-body)]
    (choose)))
"#;
    let report = Session::default().check("local_functions.clj", source);
    assert!(report.complete && !report.ok, "{report:#?}");
    let findings: Vec<_> = report
        .diagnostics
        .iter()
        .map(|diagnostic| {
            (
                diagnostic.code.as_str(),
                &source[diagnostic.span.start.offset..diagnostic.span.end.offset],
            )
        })
        .collect();
    assert_eq!(
        findings,
        [
            ("PNX2304", "missing-in-overwritten-body"),
            ("PNX2304", "missing-in-unused-body")
        ]
    );
}
