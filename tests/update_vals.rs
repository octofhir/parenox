use parenox::{Report, Session};

fn check(expression: &str, contract: &str) -> Report {
    let source = format!("(defn run {{:t/sig '[:-> {contract}]}} [] {expression})");
    Session::default().check("update_vals.clj", &source)
}

#[test]
fn transforms_dictionaries_and_heterogeneous_fields_without_promoting_optional_keys() {
    let source = include_str!("../examples/update_vals.clj");
    let report = Session::default().check("update_vals.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");

    let edited = source.replace(
        ":-> (t/map {:req {:count number?} :opt {:label string?}})",
        ":-> {:count number? :label string?}",
    );
    let report = Session::default().check("update_vals.clj", &edited);
    let [error] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert!(!report.ok && report.complete, "{report:#?}");
    assert_eq!(error.code, "PNX2322");
    assert!(error.actual.as_deref().unwrap().contains(":label?"));
}

#[test]
fn nil_false_and_reduced_are_values_and_absent_keys_stay_absent() {
    let source = r#"
        (ns demo (:require [clojure.spec.alpha :as s]))
        (defn transformed {:t/sig '[:-> {:a #{true} :b #{false}}]} []
          (update-vals {:a nil :b false} nil?))
        (defn wrapped {:t/sig '[:-> #{true}]} []
          (reduced? (:a (update-vals {:a 1} reduced))))
        (defn removed {:t/sig '[(s/map-of keyword? string?) :-> #{false}]} [row]
          (contains? (update-vals (dissoc row :hidden) str) :hidden))
    "#;
    let report = Session::default().check("presence.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn throwing_updaters_only_allow_normal_results_when_the_failing_key_is_absent() {
    let source = r#"
        (ns demo (:require [clojure.spec.alpha :as s]))
        (defn required {:t/sig '[:-> #{:value}]} []
          (try (update-vals {:required nil} (fn [_] (throw (ex-info "fail" {:stage :value}))))
               (catch clojure.lang.ExceptionInfo e (:stage (ex-data e)))))
        (defn optional {:t/sig '[(t/map {:opt {:label string?}}) :-> {}]} [row]
          (try (update-vals row (fn [_] (throw (ex-info "fail" {}))))
               (catch clojure.lang.ExceptionInfo _ {})))
        (defn dictionary {:t/sig '[(s/map-of keyword? string?) :-> {}]} [row]
          (try (update-vals row (fn [_] (throw (ex-info "fail" {}))))
               (catch clojure.lang.ExceptionInfo _ {})))
    "#;
    let report = Session::default().check("effects.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn empty_inputs_skip_updater_invocation_but_preserve_argument_effects() {
    for body in [
        "(update-vals nil 42)",
        "(update-vals {} (fn [] (throw (ex-info \"unused\" {}))))",
    ] {
        let report = check(body, "{}");
        assert!(report.ok && report.complete, "{body}: {report:#?}");
    }
    let report = check("(update-vals {:label nil} (fn [] 7))", "map?");
    let [error] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(error.code, "PNX2554");
    for (target, expected) in [
        ("nil", "#{:updater}"),
        (
            r#"(throw (ex-info "input failed" {:stage :target}))"#,
            "#{:target}",
        ),
    ] {
        let expression = format!(
            r#"(try (update-vals {target} (fnil inc (throw (ex-info "default failed" {{:stage :updater}}))))
                    (catch clojure.lang.ExceptionInfo e (:stage (ex-data e))))"#
        );
        let report = check(&expression, expected);
        assert!(report.ok && report.complete, "{report:#?}");
    }
}

#[test]
fn diagnostics_point_to_the_original_value_and_explain_analysis_limits() {
    use std::fmt::Write;

    let source = r#"(defn run {:t/sig '[:-> map?]} [] (update-vals {:count "12"} inc))"#;
    let report = Session::default().check("literal.clj", source);
    let [error] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(error.code, "PNX2322");
    let span = error.span;
    assert_eq!(&source[span.start.offset..span.end.offset], r#""12""#);
    assert!(error.notes.iter().any(|note| note.contains(":count")));

    let report = Session::default().check(
        "open.clj",
        "(defn run {:t/sig '[map? :-> map?]} [row] (update-vals row identity))",
    );
    assert!(!report.complete, "{report:#?}");
    assert_eq!(report.diagnostics[0].code, "PNX9001");
    assert!(report.diagnostics[0].message.contains("open map"));

    let mut map = String::from("{");
    for index in 0..129 {
        write!(map, ":k{index} {index} ").unwrap();
    }
    map.push('}');
    let report = check(&format!("(update-vals {map} inc)"), "map?");
    let [error] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert!(!report.complete, "{report:#?}");
    assert_eq!(error.code, "PNX9001");
    assert!(error.message.contains("value analysis limit"));
}
