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

#[test]
fn local_signature_help_uses_the_checked_call_and_threaded_argument_positions() {
    let source = r#"
(ns local-functions)
(defn checked {:t/sig '[:-> any?]} []
  (letfn [(choose [value ignored] value)]
    (-> "🦊" (choose :explicit))))
(defn shorthand {:t/sig '[:-> any?]} []
  (let [pick #(identity %)] (pick :shorthand)))
(defn render-number {:t/sig '[number? :-> string?]} [_] "known")
(defn nested {:t/sig '[:-> any?]} []
  (letfn [(choose [value] value)] (choose (render-number 7))))
"#;
    let mut session = Session::default();
    let report = session.check("local_functions.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");
    let help = session
        .call_signatures("local_functions.clj", source.find(":explicit").unwrap())
        .unwrap();
    assert_eq!(help.name, "choose");
    assert_eq!(help.active_parameter, Some(1));
    assert_eq!(
        help.signatures[0].parameters,
        [r#"value: "🦊""#, "ignored: :explicit"]
    );
    assert_eq!(help.signatures[0].result.as_deref(), Some(r#""🦊""#));

    let offset = source.find(":shorthand").unwrap();
    let help = session
        .call_signatures("local_functions.clj", offset)
        .unwrap();
    assert_eq!(help.signatures[0].parameters, ["%1: :shorthand"]);
    assert_eq!(help.signatures[0].result.as_deref(), Some(":shorthand"));

    let offset = source.rfind("(render-number 7)").unwrap();
    let help = session
        .call_signatures("local_functions.clj", offset)
        .unwrap();
    assert_eq!(help.signatures[0].parameters, ["value: String"]);
    let broken = source.replace("(render-number 7)", "(render-number :bad)");
    let report = session.check("local_functions.clj", &broken);
    assert_eq!(report.diagnostics.len(), 1, "{report:#?}");
    assert!(
        session
            .call_signatures("local_functions.clj", offset)
            .is_none()
    );
}

#[test]
fn an_unchecked_later_invocation_withholds_the_earlier_local_signature() {
    let source = r#"
(ns local-functions)
(defn untyped [unknown]
  (letfn [(choose [value] value)
          (cross [x] (choose x))]
    (cross :first)
    (cross unknown)))
"#;
    let mut session = Session::default();
    session.check("local_functions.clj", source);
    let offset = source.find("choose x").unwrap() + "choose ".len();
    assert!(
        session
            .call_signatures("local_functions.clj", offset)
            .is_none()
    );

    let changed = source.replace("(cross unknown)", "(cross :first)");
    session.check("local_functions.clj", &changed);
    let help = session
        .call_signatures("local_functions.clj", offset)
        .unwrap();
    assert_eq!(help.signatures[0].parameters, ["value: :first"]);
    assert_eq!(help.signatures[0].result.as_deref(), Some(":first"));
}
