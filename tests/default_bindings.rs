use parenox::{Report, Session};

fn check(body: &str) -> Report {
    let source = format!(
        r#"
(ns demo (:require [clojure.spec.alpha :as s]))
(s/def ::name string?)
(s/def ::patient (s/keys :opt-un [::name]))
{body}
"#
    );
    Session::default().check("defaults.clj", &source)
}

#[test]
fn defaults_handle_absence_and_preserve_nil_false_and_precise_values() {
    let report = Session::default().check("defaults.clj", include_str!("../examples/defaults.clj"));
    assert!(report.ok && report.complete, "{report:#?}");
    for body in [
        r#"(defn f {:t/sig '[:-> string?]} [] (let [{:keys [name] :or {name "Unknown"}} nil] name))"#,
        r#"(defn f {:t/sig '[::patient :-> string?]} [{:keys [name] :or {:name "Unknown"}}] name)"#,
        r#"(defn f {:t/sig '[:-> boolean?]} [] (let [{:keys [active] :or {active true}} {:active false}] active))"#,
        r#"(s/def ::status #{"active"}) (defn f {:t/sig '[:-> ::status]} [] (let [{:keys [status] :or {status "active"}} {}] status))"#,
    ] {
        let report = check(body);
        assert!(report.ok && report.complete, "{body}: {report:#?}");
    }
    let report = check(
        r#"(defn f {:t/sig '[:-> string?]} [] (let [{:keys [name] :or {name "Unknown"}} {:name nil}] name))"#,
    );
    assert!(!report.ok && report.complete, "{report:#?}");
    assert_eq!(report.diagnostics[0].actual.as_deref(), Some("Nil"));
    let report =
        check(r#"(defn f {:t/sig '[::patient :-> number?]} [{:keys [name] :or {name 1}}] name)"#);
    assert!(!report.ok && report.complete, "{report:#?}");
    assert_eq!(
        report.diagnostics[0].actual.as_deref(),
        Some("Number | String")
    );
}

#[test]
fn string_key_defaults_distinguish_explicit_nil_from_absence() {
    let source = include_str!("../examples/json_fields.cljc");
    let mut session = Session::default();
    let checked = session.check("json_fields.cljc", source);
    assert!(checked.ok && checked.complete, "{checked:#?}");
    let edit = source.replace("(or name \"Unknown\")", "name");
    let bad = session.check("json_fields.cljc", &edit);
    let [error] = bad.diagnostics.as_slice() else {
        panic!("{bad:#?}");
    };
    assert_eq!(error.code, "PNX2322");
    assert_eq!(error.actual.as_deref(), Some("Nil | String"));

    let result = check(
        r#"(defn f {:t/sig '[:-> string?]} [] (let [{:strs [name] :or {:name 1 "name" "default"}} {}] name))"#,
    );
    assert!(result.ok && result.complete, "{result:#?}");
}

#[test]
fn defaults_are_eager_and_share_outer_scope_instead_of_new_map_locals() {
    let report = Session::default().check("bad.clj", include_str!("../examples/defaults_bad.clj"));
    assert!(!report.ok && report.complete, "{report:#?}");
    assert_eq!(
        report
            .diagnostics
            .iter()
            .map(|d| d.code.as_str())
            .collect::<Vec<_>>(),
        ["PNX2322", "PNX2322", "PNX1001"]
    );
    let report = check(
        r#"(defn f {:t/sig '[:-> string?]} [] (let [x "outer" {:keys [x y] :or {x 1 y x}} {}] y))"#,
    );
    assert!(report.ok && report.complete, "{report:#?}");
    let report =
        check("(defn f {:t/sig '[:-> number?]} [] (let [{:keys [x y] :or {x 1 y x}} {}] y))");
    assert!(
        report.diagnostics.iter().any(|d| d.code == "PNX2304"),
        "{report:#?}"
    );
    let report = check(
        r#"(defn f {:t/sig '[:-> string?]} [] (let [{:keys [name] :or {unused (inc "bad")}} {:name "Ada"}] name))"#,
    );
    assert!(
        report.diagnostics.iter().any(|d| d.code == "PNX2322"),
        "{report:#?}"
    );
}

#[test]
fn renamed_namespaced_and_nested_bindings_use_local_or_lookup_key_defaults() {
    for body in [
        r#"(defn f {:t/sig '[:-> string?]} [] (let [{n :name :or {n "Unknown"}} {}] n))"#,
        r#"(defn f {:t/sig '[:-> string?]} [] (let [{n :name :or {:name "Unknown"}} {}] n))"#,
        r#"(defn f {:t/sig '[:-> string?]} [] (let [{:person/keys [name] :or {:person/name "Unknown"}} {}] name))"#,
        r#"(defn f {:t/sig '[:-> string?]} [] (let [{:keys [person/name] :or {name "Unknown"}} {}] name))"#,
        r#"(defn f {:t/sig '[:-> string?]} [] (let [{{:keys [name] :or {name "Unknown"}} :patient :or {:patient {}}} {}] name))"#,
        r#"(defn f {:t/sig '[:-> string?]} [] (let [{:req! {{:keys [name] :or {name "Unknown"}} :patient}} {:patient nil}] name))"#,
    ] {
        let report = check(body);
        assert!(report.ok && report.complete, "{body}: {report:#?}");
    }
    let report = check(
        r#"(defn f {:t/sig '[:-> string?]} [] (let [{:keys [person/name] :or {person/name "Unknown"}} {}] name))"#,
    );
    assert!(!report.ok && report.complete, "{report:#?}");
    assert_eq!(report.diagnostics[0].actual.as_deref(), Some("Nil"));
}

#[test]
fn ambiguous_or_required_defaults_are_invalid_and_point_at_the_default_key() {
    for pattern in [
        r#"{:keys [name] :or {name "A" :name "B"}}"#,
        r#"{:keys! [name] :or {name "A"}}"#,
        r#"{:req! {n :name} :or {:name "A"}}"#,
        r#"{:keys! [id & :name] :or {:name "A"}}"#,
    ] {
        let source = format!(
            "(defn f {{:t/sig '[:-> string?]}} [] (let [{pattern} {{:id 1 :name \"Ada\"}}] \"ok\"))"
        );
        let report = Session::default().check("bad.clj", &source);
        assert!(!report.ok && report.complete, "{pattern}: {report:#?}");
        assert_eq!(report.diagnostics.len(), 1, "{report:#?}");
        let diagnostic = &report.diagnostics[0];
        assert_eq!(diagnostic.code, "PNX1001");
        assert!(matches!(
            &source[diagnostic.span.start.offset..diagnostic.span.end.offset],
            "name" | ":name"
        ));
    }
    let report =
        check("(defn f {:t/sig '[:-> number?]} [] (let [{:keys [id] :or []} {:id 1}] id))");
    assert_eq!(report.diagnostics[0].code, "PNX1001");
}

#[test]
fn parameter_defaults_see_ordinary_arguments_and_prior_destructured_parameters() {
    for body in [
        r#"(defn f {:t/sig '[::patient string? :-> string?]} [{:keys [name] :or {name fallback}} fallback] name)"#,
        r#"(defn f {:t/sig '[::patient ::patient :-> string?]} [{x :name :or {x "first"}} {:keys [name] :or {name x}}] name)"#,
        r#"(defn- label [{:keys [name] :or {name fallback}} fallback] name) (defn f {:t/sig '[:-> string?]} [] (label {} "Unknown"))"#,
    ] {
        let report = check(body);
        assert!(report.ok && report.complete, "{body}: {report:#?}");
    }
}

#[test]
fn helper_edits_in_parameter_defaults_invalidate_cached_callers() {
    let source = r#"
(ns demo (:require [clojure.spec.alpha :as s]))
(s/def ::name string?) (s/def ::patient (s/keys :opt-un [::name]))
(defn f {:t/sig '[::patient :-> string?]} [{:keys [name] :or {name (fallback)}}] name)
(defn- fallback [] "X")
"#;
    let mut session = Session::default();
    let report = session.check("cache.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");
    assert_eq!(session.check("cache.clj", source).stats.reused_functions, 1);
    let changed = source.replace("\"X\"", "nil");
    let report = session.check("cache.clj", &changed);
    assert!(!report.ok && report.complete, "{report:#?}");
    assert_eq!(report.stats.checked_functions, 1);
    assert_eq!(
        report.diagnostics[0].actual.as_deref(),
        Some("Nil | String")
    );
}
