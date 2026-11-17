use parenox::{Report, Session};

fn check(body: &str) -> Report {
    let source = format!(
        "(ns demo (:require [clojure.spec.alpha :as s] [clojure.string :as str]))\n
        (s/def ::name string?)\n(s/def ::patient (s/keys :opt-un [::name]))\n{body}"
    );
    Session::default().check("presence.clj", &source)
}

#[test]
fn protects_optional_access_and_tracks_absence_in_open_maps() {
    let report = Session::default().check("presence.clj", include_str!("../examples/presence.clj"));
    assert!(report.ok && report.complete, "{report:#?}");

    for body in [
        r#"(defn f {:t/sig '[::patient :-> number?]} [p] (if (contains? p :name) (count (:name p)) (get p :name 42)))"#,
        r#"(defn f {:t/sig '[(s/keys) :-> number?]} [p] (if (not (contains? p :unknown)) (get-in p [:unknown :nested] 1) 2))"#,
        r#"(defn f {:t/sig '[::patient :-> (s/nilable string?)]} [p] (when-let [_ (contains? p :name)] (str/upper-case (:name p))))"#,
    ] {
        let report = check(body);
        assert!(report.ok && report.complete, "{body}: {report:#?}");
    }

    let report = check(
        r#"(defn f {:t/sig '[(s/keys) :-> string?]} [p] (if (contains? p :unknown) (:unknown p) ""))"#,
    );
    assert!(!report.complete);
    assert_eq!(report.diagnostics[0].code, "PNX9003");
}

#[test]
fn preserves_explicit_nil_and_false_and_reports_a_missing_required_key() {
    for (map, expected) in [("{:name nil}", "nil?"), ("{:name false}", "boolean?")] {
        let report = check(&format!(
            "(defn f {{:t/sig '[:-> {expected}]}} [] (let [p {map}] (if (contains? p :name) (:name p) 42)))"
        ));
        assert!(report.ok && report.complete, "{report:#?}");
    }

    let source = include_str!("../examples/presence_bad.clj");
    let report = Session::default().check("presence_bad.clj", source);
    assert!(!report.ok && report.complete, "{report:#?}");
    assert_eq!(report.diagnostics.len(), 2);
    assert_eq!(
        report.diagnostics[0].actual.as_deref(),
        Some("Nil | String")
    );
    let missing = &report.diagnostics[1];
    assert_eq!(missing.code, "PNX2741");
    assert_eq!(
        &source[missing.span.start.offset..missing.span.end.offset],
        ":name"
    );
}

#[test]
fn membership_resolves_core_calls_and_keeps_unsupported_targets_explicit() {
    for target in ["nil", "{}", "[:name]", "#{:another}"] {
        let report = check(&format!(
            "(defn f {{:t/sig '[:-> number?]}} [] (if (contains? {target} :name) \"unexpected\" 42))"
        ));
        assert!(report.ok && report.complete, "{target}: {report:#?}");
    }

    let report = check(
        r#"(defn contains? {:t/sig '[::patient keyword? :-> boolean?]} [p k] true)
           (defn f {:t/sig '[::patient :-> string?]} [p]
             (if (contains? p :name) (str/upper-case (:name p)) ""))"#,
    );
    assert_eq!(
        report.diagnostics[0].actual.as_deref(),
        Some("Nil | String")
    );

    for (call, code) in [
        ("(contains? 42 :name)", "PNX2322"),
        ("(contains? \"text\" :name)", "PNX2322"),
        ("(contains? [:name] 0.5)", "PNX9001"),
        ("(contains? {})", "PNX2554"),
    ] {
        let report = check(&format!("(defn f {{:t/sig '[:-> boolean?]}} [] {call})"));
        assert_eq!(report.diagnostics[0].code, code, "{report:#?}");
    }
}
