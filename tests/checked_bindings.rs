use parenox::{Report, Session};

fn check(body: &str) -> Report {
    let source = format!(
        r#"
(ns demo (:require [clojure.spec.alpha :as s] [clojure.string :as str]))
(s/def ::name string?)
(s/def ::maybe-name (s/nilable string?))
(s/def ::patient (s/keys :opt-un [::name]))
(s/def ::nullable-patient (s/keys :opt-un [::maybe-name]))
{body}
"#
    );
    Session::default().check("checked.clj", &source)
}

#[test]
fn checked_bindings_guard_presence_while_explicit_nil_still_needs_handling() {
    let report = Session::default().check("checked.clj", include_str!("../examples/checked.clj"));
    assert!(report.ok && report.complete, "{report:#?}");
    let report = Session::default().check("bad.clj", include_str!("../examples/checked_bad.clj"));
    assert!(!report.ok && report.complete, "{report:#?}");
    assert_eq!(report.diagnostics.len(), 2);
    assert_eq!(report.diagnostics[0].code, "PNX2741");
    assert!(report.diagnostics[0].message.contains(":name"));
    assert_eq!(report.diagnostics[1].actual.as_deref(), Some("Nil"));
    let ordinary = check("(defn f {:t/sig '[::patient :-> string?]} [{:keys [name]}] name)");
    assert!(!ordinary.ok && ordinary.complete, "{ordinary:#?}");
    let nullable = check(
        "(defn f {:t/sig '[::nullable-patient :-> string?]} [{:keys! [maybe-name]}] maybe-name)",
    );
    assert!(!nullable.ok && nullable.complete, "{nullable:#?}");
    assert_eq!(
        nullable.diagnostics[0].actual.as_deref(),
        Some("Nil | String")
    );
}

#[test]
fn required_lookup_checks_definite_absence_and_joins_only_successful_union_paths() {
    for (body, missing) in [
        (
            "(defn f {:t/sig '[:-> string?]} [] (req! {:name \"Ada\"} :name))",
            false,
        ),
        (
            "(defn f {:t/sig '[:-> boolean?]} [] (req! {:active false} :active))",
            false,
        ),
        (
            "(defn f {:t/sig '[:-> keyword?]} [] (req! #{:active} :active))",
            false,
        ),
        (
            "(defn f {:t/sig '[(s/nilable ::patient) :-> string?]} [p] (req! p :name))",
            false,
        ),
        ("(defn f {:t/sig '[:-> string?]} [] (req! {} :name))", true),
        ("(defn f {:t/sig '[:-> string?]} [] (req! nil :name))", true),
        (
            "(defn f {:t/sig '[:-> keyword?]} [] (req! #{:other} :active))",
            true,
        ),
    ] {
        let report = check(body);
        assert_eq!(report.ok, !missing, "{body}: {report:#?}");
        assert!(report.complete, "{body}: {report:#?}");
        assert_eq!(
            report
                .diagnostics
                .iter()
                .filter(|d| d.code == "PNX2741")
                .count(),
            usize::from(missing)
        );
    }
    let report = check("(defn f {:t/sig '[::patient :-> string?]} [p] (req! p :unknown))");
    assert!(!report.ok && !report.complete, "{report:#?}");
    assert!(report.diagnostics.iter().any(|d| d.code == "PNX9003"));
}

#[test]
fn keys_directives_resolve_namespaces_and_check_unbound_keys_after_ampersand() {
    for body in [
        r#"(defn f {:t/sig '[:-> string?]} [] (let [{:person/keys! [name & :id]} {:person/name "Ada" :person/id nil}] name))"#,
        r#"(defn f {:t/sig '[:-> string?]} [] (let [{:keys! [person/name]} {:person/name "Ada"}] name))"#,
        r#"(defn f {:t/sig '[:-> string?]} [] (let [{::keys! [name]} {::name "Ada"}] name))"#,
        r#"(defn f {:t/sig '[:-> string?]} [] (let [{:person/keys [name & :id]} {:person/name "Ada"}] name))"#,
        r#"(defn f {:t/sig '[::patient :-> string?]} [{:keys! [name & :trace/id]}] name)"#,
    ] {
        let report = check(body);
        assert!(report.ok && report.complete, "{body}: {report:#?}");
    }
    let report = check(
        r#"(defn f {:t/sig '[:-> string?]} [] (let [{:person/keys! [name & :id]} {:person/name "Ada"}] name))"#,
    );
    assert!(!report.ok && report.complete, "{report:#?}");
    assert_eq!(report.diagnostics[0].code, "PNX2741");
    assert!(report.diagnostics[0].message.contains(":person/id"));
    let report = check(
        r#"(defn f {:t/sig '[:-> string?]} [] (let [{:keys! [name & :id]} {:name "Ada" :id 1}] id))"#,
    );
    assert!(
        report.diagnostics.iter().any(|d| d.code == "PNX2304"),
        "{report:#?}"
    );
}

#[test]
fn renamed_and_nested_patterns_preserve_checked_levels_and_report_unsupported_modes() {
    for (body, success) in [
        (
            r#"(defn f {:t/sig '[:-> string?]} [] (let [{:req! {{:keys! [name]} :patient}} {:patient {:name "Ada"}}] name))"#,
            true,
        ),
        (
            r#"(defn f {:t/sig '[:-> string?]} [] (let [{:req! {{:keys! [name]} :patient}} {:patient nil}] name))"#,
            false,
        ),
        (
            r#"(defn f {:t/sig '[:-> string?]} [] (let [{n :name} {:name "Ada"}] n))"#,
            true,
        ),
    ] {
        let report = check(body);
        assert_eq!(report.ok, success, "{report:#?}");
        assert!(report.complete, "{report:#?}");
    }
    for body in [
        "(defn f {:t/sig '[:-> number?]} [] (let [{:keys! [name] :or {name 1}} {}] 1))",
        "(defn f {:t/sig '[:-> number?]} [] (let [{:req! [:name]} {}] 1))",
        "(defn f {:t/sig '[:-> number?]} [] (let [{:person/keys! [other/name]} {}] 1))",
        "(defn f {:t/sig '[:-> number?]} [] (req! {} :name 1))",
    ] {
        let report = check(body);
        assert!(!report.ok, "{body}: {report:#?}");
        assert!(
            report
                .diagnostics
                .iter()
                .any(|d| matches!(d.code.as_str(), "PNX9001" | "PNX2554" | "PNX1001")),
            "{report:#?}"
        );
    }
}

#[test]
fn checked_lookup_preserves_literal_inference_and_project_functions_shadow_core() {
    let body = r#"
(s/def ::status #{"active"})
(defn- require-status [m] (req! m :status))
(defn f {:t/sig '[:-> ::status]} [] (require-status {:status "active"}))
"#;
    let report = check(body);
    assert!(report.ok && report.complete, "{report:#?}");
    let report = check(&body.replace("{:status \"active\"}", "{:status \"deleted\"}"));
    assert!(!report.ok && report.complete, "{report:#?}");
    assert_eq!(report.diagnostics[0].actual.as_deref(), Some("\"deleted\""));
    let report = check(
        "(defn req! {:t/sig '[number? number? :-> number?]} [x y] (+ x y)) (defn f {:t/sig '[:-> number?]} [] (req! 1 2))",
    );
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn checked_string_binding_requires_presence_but_accepts_a_nil_marker() {
    let source = r#"(defn f {:t/sig '[:-> string?]} []
      (let [{:strs! [name & "schema"]} {"name" "Ada" "schema" nil}] name))"#;
    let report = check(source);
    assert!(report.ok && report.complete, "{report:#?}");
    let report = check(&source.replace(" \"schema\" nil", " :schema nil"));
    assert!(!report.ok && report.complete, "{report:#?}");
    assert!(report.diagnostics[0].message.contains("\"schema\""));

    let report = check(&source.replace(
        ":strs! [name & \"schema\"]",
        ":strs! [name & \"schema\"] :or {\"name\" \"Default\"}",
    ));
    assert!(
        report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "PNX1001"),
        "{report:#?}"
    );
}
