use parenox::Session;

#[test]
fn frequencies_preserves_counts_and_map_key_semantics() {
    let source = r#"
        (ns demo (:require [clojure.spec.alpha :as s]))
        (defn empty-input {:t/sig '[:-> {}]} [] (frequencies nil))
        (defn markers {:t/sig '[:-> {nil (s/and int? #{2}) false (s/and int? #{1})}]} []
          (frequencies [nil false nil]))
        (defn chars {:t/sig '[:-> (s/and int? #{2})]} []
          (get (frequencies "aba") \a))
        (defn equivalent {:t/sig '[:-> (s/and int? #{2})]} []
          (get (frequencies [1 1N]) 1))
        (defn set-items {:t/sig '[:-> {:a (s/and int? #{1}) :b (s/and int? #{1})}]} []
          (frequencies #{:a :b}))
        (defn repeated {:t/sig '[:-> {:a (s/and int? #{1000000000})}]} []
          (frequencies (repeat 1000000000 :a)))
        (defn bounded {:t/sig '[(s/coll-of keyword? :kind vector? :max-count 3)
                                :-> (s/map-of keyword? (s/and int? #{1 2 3}))]}
          [items] (frequencies items))
        (defn optional-key {:t/sig '[(s/coll-of #{:a} :kind vector? :max-count 3)
                                     :-> (t/map {:opt {:a (s/and int? #{1 2 3})}})]}
          [items] (frequencies items))
    "#;
    let report = Session::default().check("demo.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");

    let wrong = source.replace("false (s/and int? #{1})", "false (s/and int? #{2})");
    let report = Session::default().check("demo.clj", &wrong);
    assert!(!report.ok && report.complete, "{report:#?}");
    assert_eq!(report.diagnostics[0].code, "PNX2322");
}

#[test]
fn frequencies_accepts_primitive_and_reference_arrays() {
    let source = r#"
        (ns demo (:require [clojure.spec.alpha :as s]))
        (defn byte-counts {:t/sig '[:-> (s/map-of int? int?)]} []
          (frequencies (byte-array [1 1 2])))
        (defn boolean-counts {:t/sig '[:-> (s/map-of boolean? int?)]} []
          (frequencies (boolean-array [false true false])))
        (defn ref-counts {:t/sig '[:-> #{true}]} []
          (map? (frequencies (object-array [nil :a nil]))))
    "#;
    let report = Session::default().check("demo.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");

    let wrong = source.replace("(s/map-of boolean? int?)", "(s/map-of keyword? int?)");
    let report = Session::default().check("demo.clj", &wrong);
    assert!(!report.ok && report.complete, "{report:#?}");
}

#[test]
fn frequencies_reports_non_reducible_inputs_and_unmodeled_traversal() {
    for (body, code) in [
        ("(frequencies false)", "PNX2322"),
        ("(frequencies)", "PNX2554"),
    ] {
        let source = format!("(defn run {{:t/sig '[:-> any?]}} [] {body})");
        let report = Session::default().check("demo.clj", &source);
        let [error] = report.diagnostics.as_slice() else {
            panic!("{report:#?}");
        };
        assert_eq!(error.code, code, "{report:#?}");
    }

    let report = Session::default().check(
        "demo.clj",
        "(defn run {:t/sig '[any? :-> any?]} [x] (frequencies x))",
    );
    let [error] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(error.code, "PNX9001");
}
