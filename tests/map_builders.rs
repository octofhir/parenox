use parenox::{Report, Session};

fn check(expression: &str, contract: &str) -> Report {
    let source = format!("(defn run {{:t/sig '[:-> {contract}]}} [] {expression})");
    Session::default().check("map_builders.clj", &source)
}

#[test]
fn builds_and_renames_a_typed_map_and_rejects_definite_key_collisions() {
    let source = include_str!("../examples/map_builders.clj");
    let report = Session::default().check("map_builders.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");

    let edited = source.replace(r#":unit "unit""#, r#":unit "amount""#);
    let report = Session::default().check("map_builders.clj", &edited);
    let [error] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert!(!report.ok && report.complete, "{report:#?}");
    assert_eq!(error.code, "PNX2790");
    assert!(error.message.contains(":amount") && error.message.contains(":unit"));
    assert!(error.message.contains(r#""amount""#));
}

#[test]
fn update_keys_preserves_optional_presence_and_checks_possible_collisions() {
    let source = include_str!("../examples/map_builders.clj");
    let edited = source.replace(
        r#":opt {"pending" boolean? "done" boolean?}"#,
        r#":req {"pending" boolean? "done" boolean?}"#,
    );
    let report = Session::default().check("map_builders.clj", &edited);
    let [error] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert!(!report.ok && report.complete, "{report:#?}");
    assert_eq!(error.code, "PNX2322");

    let edited = source.replace(r#":done "done""#, r#":done "pending""#);
    let report = Session::default().check("map_builders.clj", &edited);
    let [error] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert!(!report.complete, "{report:#?}");
    assert_eq!(error.code, "PNX9001");
    assert!(error.message.contains("may map both"));
}

#[test]
fn update_keys_keeps_values_and_treats_multiple_possible_destinations_as_optional() {
    let source = r#"
        (defn rename {:t/sig '[boolean? :-> (t/map {:opt {:a nil? :b nil?}})]}
          [switch?]
          (update-keys {:old nil} (fn [_] (if switch? :a :b))))
        (defn absent {:t/sig '[:-> {}]} []
          (update-keys nil (fn [] 42)))
        (defn removed {:t/sig '[:-> {:new #{false}}]} []
          (update-keys (dissoc {:old false :hidden 7} :hidden) (fn [_] :new)))
    "#;
    let report = Session::default().check("map_builders.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");

    let edited = source.replace(":opt {:a nil? :b nil?}", ":req {:a nil? :b nil?}");
    let report = Session::default().check("map_builders.clj", &edited);
    assert_eq!(report.diagnostics[0].code, "PNX2322", "{report:#?}");
}

#[test]
fn zipmap_stops_at_the_shorter_input_and_keeps_the_last_value_for_duplicates() {
    for (body, expected) in [
        (
            "(zipmap [:a :b :a :unused] [1 nil false])",
            "{:a #{false} :b nil?}",
        ),
        (r#"(zipmap '("plan") [7 :unused])"#, r#"{"plan" number?}"#),
        (
            r#"(apply zipmap [[:key] ["value"]])"#,
            r#"{:key #{"value"}}"#,
        ),
        ("(zipmap [:extra] [])", "{}"),
        ("(zipmap nil nil)", "{}"),
    ] {
        let report = check(body, expected);
        assert!(report.ok && report.complete, "{body}: {report:#?}");
    }
}

#[test]
fn zipmap_keeps_prior_values_when_a_later_duplicate_may_be_absent() {
    let source = r#"
        (ns demo (:require [clojure.spec.alpha :as s]))
        (defn value {:t/sig '[(s/coll-of #{:slot} :kind vector?) :-> {:slot #{"early" false}}]}
          [keys]
          (zipmap (cons :slot keys) ["early" false]))
        (defn fixed-keys {:t/sig '[(s/coll-of number? :kind vector?)
                                   :-> (t/map {:opt {:a number? :b number?}})]}
          [values]
          (zipmap [:a :b] values))
    "#;
    let report = Session::default().check("mixed.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");

    let edited = source.replace(r#"#{"early" false}"#, "#{false}");
    let report = Session::default().check("mixed.clj", &edited);
    let [error] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(error.code, "PNX2322");
    assert!(error.actual.as_deref().unwrap().contains("String"));
}

#[test]
fn diagnostics_distinguish_unsupported_keys_from_invalid_sequence_arguments() {
    let source = r#"
        (ns demo (:require [clojure.spec.alpha :as s]))
        (defn unknown-keys {:t/sig '[(s/map-of keyword? string?) :-> map?]} [row]
          (update-keys row identity))
        (defn numeric-key {:t/sig '[:-> map?]} [] (zipmap [1] [:value]))
        (defn invalid-seq {:t/sig '[:-> map?]} [] (zipmap nil 42))
    "#;
    let report = Session::default().check("map_builders.clj", source);
    let [unknown, numeric, invalid] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    for (error, code, token) in [
        (unknown, "PNX9001", "row"),
        (numeric, "PNX9001", "[1]"),
        (invalid, "PNX2322", "42"),
    ] {
        assert_eq!(error.code, code);
        let span = error.span;
        assert_eq!(&source[span.start.offset..span.end.offset], token);
    }
}

#[test]
fn update_keys_bounds_expansion_of_finite_key_domains() {
    let keys = (0..129)
        .map(|id| format!(":key{id}"))
        .collect::<Vec<_>>()
        .join(" ");
    let source = format!(
        r#"
        (ns demo (:require [clojure.spec.alpha :as s]))
        (defn rename {{:t/sig '[keyword? :-> #{{{keys}}}]}} [_] :key0)
        (defn too-many-inputs {{:t/sig '[(s/map-of #{{{keys}}} number?) :-> map?]}}
          [row]
          (update-keys row identity))
        (defn too-many-outputs {{:t/sig '[:-> map?]}} []
          (update-keys {{:name 1}} rename))
    "#
    );
    let report = Session::default().check("map_builders.clj", &source);
    let [input, result] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert!(!report.complete, "{report:#?}");
    assert_eq!(input.code, "PNX9001");
    assert_eq!(result.code, "PNX9001");
    assert!(input.message.contains("too many possible input keys"));
    assert!(result.message.contains("too many possible result keys"));
}
