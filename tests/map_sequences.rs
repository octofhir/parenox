use parenox::Session;

#[test]
fn dictionary_views_supply_typed_items_and_allow_empty_inputs() {
    let source = include_str!("../examples/map_sequences.clj");
    let report = Session::default().check("map_sequences.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");

    let edited = source.replace("(s/nilable keyword?)", "keyword?");
    let report = Session::default().check("map_sequences.clj", &edited);
    let [error] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert!(!report.ok && report.complete, "{report:#?}");
    assert_eq!(error.code, "PNX2322");
    assert_eq!(error.actual.as_deref(), Some("Nil | Keyword"));
}

#[test]
fn known_map_sizes_preserve_nil_false_and_out_of_range_positions() {
    let source = r#"
        (defn no-keys {:t/sig '[:-> nil?]} [] (keys nil))
        (defn no-values {:t/sig '[:-> nil?]} [] (vals {}))
        (defn nil-value {:t/sig '[:-> #{true}]} [] (some? (vals {:entry nil})))
        (defn false-value {:t/sig '[:-> #{false}]} [] (first (vals {:entry false})))
        (defn one-key {:t/sig '[:-> #{"entry"}]} [] (first (keys {"entry" nil})))
        (defn after-singleton {:t/sig '[:-> nil?]} [] (next (vals {:entry false})))
        (defn outside {:t/sig '[:-> #{:missing}]} []
          (nth (keys {:left 0 :right 1}) 2 :missing))
        (defn absent-field {:t/sig '[(t/map {:req {:key number?} :opt {:absent #{}}}) :-> nil?]}
          [row]
          (next (keys row)))
        (defn safe-empty {:t/sig '[:-> nil?]} []
          (try (keys {}) (catch IllegalArgumentException _ :unexpected)))
    "#;
    let report = Session::default().check("map_sequences.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn each_position_can_have_any_of_the_present_value_types() {
    let source = r#"
        (ns demo (:require [clojure.spec.alpha :as s]))
        (defn value {:t/sig '[{:count number? :name string?} :-> (s/or :count number? :name string?)]}
          [row]
          (first (vals row)))
        (defn optional-value {:t/sig '[(s/nilable (t/map {:opt {:name string?}})) :-> (s/nilable string?)]}
          [row]
          (first (vals row)))
    "#;
    let report = Session::default().check("map_sequences.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");

    let edited = source.replace("(s/or :count number? :name string?)", "number?");
    let report = Session::default().check("map_sequences.clj", &edited);
    let [error] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert!(!report.ok && report.complete, "{report:#?}");
    assert_eq!(error.code, "PNX2322");
    assert_eq!(error.actual.as_deref(), Some("Number | String"));
}

#[test]
fn invalid_targets_and_analysis_gaps_are_reported_at_the_argument() {
    let source = r#"
        (defn bad-target {:t/sig '[:-> nil?]} [] (keys 42))
        (defn open-map {:t/sig '[map? :-> nil?]} [row] (vals row))
        (defn seq-of-entries {:t/sig '[:-> nil?]} [] (keys (seq {:entry 1})))
    "#;
    let report = Session::default().check("map_sequences.clj", source);
    let [wrong, open, entries] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert!(!report.ok && !report.complete, "{report:#?}");
    assert_eq!(wrong.contract.as_ref().unwrap().name, "clojure.core/keys");
    assert!(open.message.contains("open map"));
    for (diagnostic, code, target) in [
        (wrong, "PNX2322", "42"),
        (open, "PNX9001", "row"),
        (entries, "PNX9001", "(seq {:entry 1})"),
    ] {
        assert_eq!(diagnostic.code, code);
        let span = diagnostic.span;
        assert_eq!(&source[span.start.offset..span.end.offset], target);
    }
}
