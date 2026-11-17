use parenox::{Report, Session};

fn check(body: &str) -> Report {
    let source = format!(
        r#"
(ns demo (:require [clojure.spec.alpha :as s] [clojure.string :as str]))
{body}
"#
    );
    Session::default().check("threading.clj", &source)
}

#[test]
fn threading_inserts_first_or_last_arguments_and_preserves_precise_values() {
    let source = include_str!("../examples/threading.clj");
    let report = Session::default().check("threading.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");
    assert_eq!(
        parenox::reader::parse(source).syntax().text().to_string(),
        source
    );
    for body in [
        r#"(defn f {:t/sig '[:-> string?]} [] (-> "Ada" ^String str/upper-case))"#,
        r#"(defn ordered {:t/sig '[number? string? :-> string?]} [n text] text) (defn f {:t/sig '[:-> string?]} [] (->> "Ada" (ordered 7)))"#,
        r#"(s/def ::status #{"active"}) (defn f {:t/sig '[:-> ::status]} [] (-> "active" identity))"#,
        r#"(defn f {:t/sig '[:-> string?]} [] (-> "Ada"))"#,
        r#"(defn f {:t/sig '[:-> string?]} [] (-> {:name "Ada"} (assoc :name "Grace") :name))"#,
    ] {
        let report = check(body);
        assert!(report.ok && report.complete, "{body}: {report:#?}");
    }
    let report = check(
        r#"(defn ordered {:t/sig '[number? string? :-> string?]} [n text] text) (defn f {:t/sig '[:-> string?]} [] (-> "Ada" (ordered 7)))"#,
    );
    assert!(!report.ok && report.complete, "{report:#?}");
    assert_eq!(report.diagnostics.len(), 2);
}

#[test]
fn some_threading_skips_nil_but_keeps_false_and_compiles_skipped_steps() {
    for body in [
        "(defn f {:t/sig '[:-> string?]} [] (some-> false str))",
        "(defn f {:t/sig '[:-> nil?]} [] (some-> nil str))",
        "(defn f {:t/sig '[:-> nil?]} [] (some-> nil (str (inc \"bad\"))))",
        "(defn f {:t/sig '[:-> nil?]} [] (some-> {:name nil} :name str))",
        "(defn f {:t/sig '[:-> string?]} [] (some->> false (str \"flag:\")))",
    ] {
        let report = check(body);
        assert!(report.ok && report.complete, "{body}: {report:#?}");
    }
    let report = check("(defn f {:t/sig '[:-> nil?]} [] (some-> nil missing-function))");
    assert!(
        report.diagnostics.iter().any(|d| d.code == "PNX2304"),
        "{report:#?}"
    );
    let report = check("(defn f {:t/sig '[:-> nil?]} [] (some-> nil (inc 2)))");
    assert!(
        report.diagnostics.iter().any(|d| d.code == "PNX2554"),
        "{report:#?}"
    );
}

#[test]
fn diagnostics_keep_source_origins_and_only_propose_edits_for_original_forms() {
    let source = include_str!("../examples/threading_bad.clj");
    let report = Session::default().check("bad.clj", source);
    assert!(!report.ok && report.complete, "{report:#?}");
    assert_eq!(report.diagnostics.len(), 3);
    let first = &report.diagnostics[0];
    assert_eq!(
        &source[first.span.start.offset..first.span.end.offset],
        ":name"
    );
    assert_eq!(first.actual.as_deref(), Some("Nil | String"));
    assert!(first.suggestions.is_empty());
    assert!(
        first
            .notes
            .iter()
            .any(|n| n.contains("expanding clojure.core/->"))
    );
    let second = &report.diagnostics[1];
    assert_eq!(
        &source[second.span.start.offset..second.span.end.offset],
        "\"not a number\""
    );
    assert_eq!(second.actual.as_deref(), Some("String"));
    let suggestion = &report.diagnostics[2].suggestions[0];
    let mut fixed = source.to_owned();
    fixed.replace_range(
        suggestion.span.start.offset..suggestion.span.end.offset,
        &suggestion.replacement,
    );
    let report = Session::default().check("bad.clj", &fixed);
    assert_eq!(report.diagnostics.len(), 2, "{report:#?}");
    let report = check("(defn f {:t/sig '[:-> number?]} [] (-> (inc \"bad\") inc inc))");
    assert_eq!(
        report.diagnostics.len(),
        1,
        "Seed was analyzed more than once: {report:#?}"
    );
}

#[test]
fn temporary_bindings_do_not_capture_user_locals_namespaced_bindings_or_functions() {
    for body in [
        r#"(defn choose {:t/sig '[number? string? :-> string?]} [n text] text) (defn f {:t/sig '[string? :-> string?]} [parenox__thread_0] (-> 1 (choose parenox__thread_0)))"#,
        r#"(defn parenox__thread_0 {:t/sig '[number? :-> string?]} [n] (str n)) (defn f {:t/sig '[:-> string?]} [] (-> 1 parenox__thread_0))"#,
        r#"(defn f {:t/sig '[:-> string?]} [] (->> "seed" (let [{:keys [user/parenox__thread_0]} {:user/parenox__thread_0 42}])))"#,
        r#"(defn -> {:t/sig '[number? number? :-> number?]} [a b] (+ a b)) (defn f {:t/sig '[:-> number?]} [] (-> 1 2))"#,
        r#"(defn f {:t/sig '[:-> string?]} [] (-> "Ada" (some-> str/upper-case)))"#,
    ] {
        let report = check(body);
        assert!(report.ok && report.complete, "{body}: {report:#?}");
    }
}

#[test]
fn flat_pipelines_are_iterative_and_nested_expansion_is_bounded() {
    let report = check(&format!(
        "(defn f {{:t/sig '[:-> number?]}} [] (-> 1 {}))",
        "inc ".repeat(2000)
    ));
    assert!(report.ok && report.complete, "{report:#?}");
    let source = format!(
        "(defn f {{:t/sig '[:-> number?]}} [] {}1{})",
        "(-> ".repeat(40),
        ")".repeat(40)
    );
    let report = check(&source);
    assert!(!report.ok && !report.complete, "{report:#?}");
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.message.contains("expansion nesting limit"))
    );
    for body in [
        "(defn f {:t/sig '[:-> number?]} [] (->))",
        "(defn f {:t/sig '[:-> number?]} [] (-> 1 ()))",
    ] {
        assert!(!check(body).ok, "{body}");
    }
}

#[test]
fn private_helpers_in_pipelines_infer_and_invalidate_cached_callers() {
    let source = r#"
(ns demo (:require [clojure.spec.alpha :as s]))
(defn f {:t/sig '[(s/nilable string?) :-> string?]} [name] (-> name normalize))
(defn- normalize [name] (or name "X"))
"#;
    let mut session = Session::default();
    let report = session.check("cache.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");
    assert_eq!(session.check("cache.clj", source).stats.reused_functions, 1);
    let report = session.check("cache.clj", &source.replace("\"X\"", "nil"));
    assert!(!report.ok && report.complete, "{report:#?}");
    assert_eq!(report.stats.checked_functions, 1);
    assert_eq!(
        report.diagnostics[0].actual.as_deref(),
        Some("Nil | String")
    );
}
