use parenox::{Report, Session};

const PRELUDE: &str = "(ns demo (:require [clojure.spec.alpha :as s]))\n\
    (s/def ::status #{\"active\" \"inactive\"})\n\
    (s/def :item/status ::status)\n\
    (s/def ::item (s/keys :req-un [:item/status]))\n";

fn check(body: &str) -> Report {
    Session::default().check("inference.clj", &format!("{PRELUDE}{body}"))
}

fn assert_ok(body: &str) {
    let report = check(body);
    assert!(report.ok && report.complete, "{:#?}", report.diagnostics);
}

#[test]
fn string_argument_infers_string_instead_of_a_singleton_type() {
    let report = check(
        "(defn increment {:t/sig '[number? :-> number?]} [x] (+ x 1))\n\
        (defn call {:t/sig '[:-> number?]} [] (increment \"not a number\"))",
    );
    assert_eq!(report.diagnostics.len(), 1);
    assert_eq!(report.diagnostics[0].actual.as_deref(), Some("String"));
    assert_eq!(report.diagnostics[0].expected.as_deref(), Some("Number"));
    assert_eq!(
        report.diagnostics[0].message,
        "Argument 1: expected Number, got String"
    );
}

#[test]
fn local_and_branch_results_infer_string_without_leaking_literal_unions() {
    for expression in [
        "(let [x \"hello\"] x)",
        "(if flag \"hello\" \"world\")",
        "(identity \"hello\")",
        "(or (when flag \"hello\") \"fallback\")",
    ] {
        let report = check(&format!(
            "(defn f {{:t/sig '[boolean? :-> number?]}} [flag] {expression})"
        ));
        assert_eq!(
            report.diagnostics.len(),
            1,
            "{expression}: {:?}",
            report.diagnostics
        );
        assert_eq!(
            report.diagnostics[0].actual.as_deref(),
            Some("String"),
            "{expression}"
        );
    }
}

#[test]
fn collection_inference_keeps_map_vector_and_set_distinct() {
    for (expression, expected) in [
        ("[\"active\" \"inactive\"]", "Vector<String>"),
        ("#{\"active\" \"inactive\"}", "Set<String>"),
        ("{:status \"active\"}", "{:status: String}"),
    ] {
        let report = check(&format!(
            "(defn f {{:t/sig '[:-> number?]}} [] {expression})"
        ));
        assert!(report.complete);
        assert_eq!(report.diagnostics.len(), 1);
        assert_eq!(report.diagnostics[0].actual.as_deref(), Some(expected));
    }
}

#[test]
fn set_in_value_position_does_not_become_a_literal_union() {
    let report = check("(defn f {:t/sig '[:-> ::status]} [] #{\"active\" \"inactive\"})");
    assert!(!report.ok && report.complete);
    assert!(
        report.diagnostics[0]
            .actual
            .as_deref()
            .unwrap()
            .starts_with("Set<")
    );
}

#[test]
fn set_spec_in_declaration_position_still_checks_allowed_values() {
    assert_ok("(defn f {:t/sig '[:-> ::status]} [] \"active\")");
    let report = check("(defn f {:t/sig '[:-> ::status]} [] \"deleted\")");
    assert!(!report.ok && report.complete);
    assert_eq!(report.diagnostics[0].actual.as_deref(), Some("\"deleted\""));
    assert!(
        report.diagnostics[0]
            .expected
            .as_deref()
            .unwrap()
            .contains("\"active\"")
    );
}

#[test]
fn precise_values_survive_locals_branches_and_short_circuiting() {
    for expression in [
        "(let [status \"active\"] status)",
        "(if flag \"active\" \"inactive\")",
        "(identity \"active\")",
        "(or (when flag \"active\") \"inactive\")",
    ] {
        assert_ok(&format!(
            "(defn f {{:t/sig '[boolean? :-> ::status]}} [flag] {expression})"
        ));
    }
    let report =
        check("(defn f {:t/sig '[boolean? :-> ::status]} [flag] (if flag \"active\" \"deleted\"))");
    assert!(!report.ok);
    assert!(
        report.diagnostics[0]
            .actual
            .as_deref()
            .unwrap()
            .contains("\"deleted\"")
    );
}

#[test]
fn precise_fields_survive_map_lookup_and_assoc() {
    for expression in [
        "{:status \"active\"}",
        "(let [x {:status \"active\"}] x)",
        "(assoc {} :status \"active\")",
    ] {
        assert_ok(&format!(
            "(defn f {{:t/sig '[:-> ::item]}} [] {expression})"
        ));
    }
    assert_ok("(defn f {:t/sig '[:-> ::status]} [] (:status {:status \"active\"}))");
    assert_ok("(defn f {:t/sig '[:-> ::status]} [] (get {} :status \"inactive\"))");
    let report = check("(defn f {:t/sig '[:-> ::item]} [] (assoc {} :status \"deleted\"))");
    assert!(!report.ok);
    assert!(
        report.diagnostics[0]
            .actual
            .as_deref()
            .unwrap()
            .contains("\"deleted\"")
    );
}

#[test]
fn precise_collection_elements_satisfy_finite_value_contracts() {
    assert_ok(
        "(defn f {:t/sig '[:-> (s/coll-of ::status :kind vector?)]} [] [\"active\" \"inactive\"])",
    );
    assert_ok(
        "(defn f {:t/sig '[:-> (s/coll-of ::status :kind set?)]} [] #{\"active\" \"inactive\"})",
    );
    let report = check(
        "(defn f {:t/sig '[:-> (s/coll-of ::status :kind set?)]} [] #{\"active\" \"deleted\"})",
    );
    assert!(!report.ok && report.complete);
}

#[test]
fn vector_and_set_are_not_assignable_to_each_other() {
    for (kind, expression) in [("set?", "[\"active\"]"), ("vector?", "#{\"active\"}")] {
        let report = check(&format!(
            "(defn f {{:t/sig '[:-> (s/coll-of string? :kind {kind})]}} [] {expression})"
        ));
        assert!(!report.ok && report.complete);
        assert_eq!(report.diagnostics[0].code, "PNX2322");
    }
}

#[test]
fn set_values_can_be_counted_and_empty_sets_are_truthy() {
    assert_ok("(defn f {:t/sig '[:-> number?]} [] (count #{\"active\" \"inactive\"}))");
    assert_ok("(defn f {:t/sig '[:-> number?]} [] (if #{} 1 \"unreachable\"))");
}

#[test]
fn set_invocation_returns_a_matching_value_or_nil_instead_of_boolean() {
    assert_ok(
        "(defn f {:t/sig '[string? :-> (s/nilable string?)]} [x] (#{\"active\" \"inactive\"} x))",
    );
    let report =
        check("(defn f {:t/sig '[string? :-> boolean?]} [x] (#{\"active\" \"inactive\"} x))");
    assert!(!report.ok && report.complete);
    assert_eq!(
        report.diagnostics[0].actual.as_deref(),
        Some("Nil | String")
    );
    assert!(
        report.diagnostics[0]
            .notes
            .iter()
            .any(|n| n.contains("not a Boolean"))
    );
}

#[test]
fn string_type_without_known_values_does_not_satisfy_a_finite_contract() {
    let report = check("(defn f {:t/sig '[string? :-> ::status]} [x] x)");
    assert!(!report.ok && report.complete);
    assert_eq!(report.diagnostics[0].actual.as_deref(), Some("String"));
}
