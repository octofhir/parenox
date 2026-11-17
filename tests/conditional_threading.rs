use parenox::{Report, Session};

fn check(body: &str) -> Report {
    let source = format!(
        r#"(ns demo (:require [clojure.spec.alpha :as s] [clojure.string :as str]))
{body}"#
    );
    Session::default().check("conditional.clj", &source)
}

#[test]
fn conditional_steps_continue_after_nil_and_false_and_respect_argument_position() {
    let report = Session::default().check(
        "conditional.clj",
        include_str!("../examples/conditional.clj"),
    );
    assert!(report.ok && report.complete, "{report:#?}");
    for body in [
        r#"(defn f {:t/sig '[:-> string?]} [] (cond-> "A" false inc true (str "B")))"#,
        r#"(defn f {:t/sig '[:-> string?]} [] (cond-> nil 0 (str "A") [] (str "B")))"#,
        r#"(defn ordered {:t/sig '[number? string? :-> string?]} [n text] text) (defn f {:t/sig '[:-> string?]} [] (cond->> "Ada" true (ordered 7)))"#,
        r#"(defn f {:t/sig '[:-> string?]} [] (cond-> "Ada"))"#,
    ] {
        let report = check(body);
        assert!(report.ok && report.complete, "{body}: {report:#?}");
    }
    let report =
        check(r#"(defn f {:t/sig '[:-> string?]} [] (cond-> "A" false str (inc "bad") str))"#);
    assert_eq!(report.diagnostics.len(), 1, "{report:#?}");
    assert_eq!(report.diagnostics[0].code, "PNX2322");
    let report = check("(defn f {:t/sig '[:-> number?]} [] (cond-> 1 false missing-function))");
    assert!(
        report.diagnostics.iter().any(|d| d.code == "PNX2304"),
        "{report:#?}"
    );
}

#[test]
fn unknown_conditions_join_paths_and_guards_narrow_seed_and_step_arguments() {
    for body in [
        r#"(defn f {:t/sig '[boolean? :-> (s/or :n number? :s string?)]} [flag] (cond-> 1 flag str))"#,
        r#"(defn f {:t/sig '[(s/nilable string?) :-> (s/nilable string?)]} [name] (cond-> name (some? name) str/upper-case))"#,
        r#"(defn f {:t/sig '[string? :-> string?]} [text] (cond-> 0 (string? text) (str text)))"#,
        r#"(defn append {:t/sig '[string? number? :-> string?]} [text n] (str text n)) (defn f {:t/sig '[(s/nilable number?) :-> string?]} [n] (cond-> "count" (some? n) (append n)))"#,
        r#"(defn- string? [x] false) (defn f {:t/sig '[clojure.core/string? :-> number?]} [text] (cond-> 0 (string? text) (str text)))"#,
    ] {
        let report = check(body);
        assert!(report.ok && report.complete, "{body}: {report:#?}");
    }
    let report = check(
        r#"(defn append {:t/sig '[string? number? :-> string?]} [text n] (str text n)) (defn f {:t/sig '[(s/nilable number?) :-> string?]} [n] (cond-> "count" (some? n) (append n) true (append n)))"#,
    );
    assert!(!report.ok && report.complete, "{report:#?}");
    assert_eq!(report.diagnostics.len(), 1);
}

#[test]
fn named_threading_rebinds_each_expression_and_keeps_outer_scope() {
    for body in [
        r#"(defn f {:t/sig '[:-> string?]} [] (as-> false value (str value) nil (str value)))"#,
        r#"(defn f {:t/sig '[:-> keyword?]} [] (as-> {} value :name))"#,
        r#"(defn f {:t/sig '[:-> number?]} [] (as-> 1 value))"#,
        r#"(defn f {:t/sig '[:-> number?]} [] (let [value 42 answer (as-> "seed" value (str/upper-case value))] (+ value (count answer))))"#,
        r#"(defn f {:t/sig '[:-> string?]} [] (-> "Ada" (as-> value (str "Hi " value))))"#,
        r#"(defn cond-> {:t/sig '[number? number? :-> number?]} [a b] (+ a b)) (defn f {:t/sig '[:-> number?]} [] (cond-> 1 2))"#,
        r#"(defn as-> {:t/sig '[number? number? :-> number?]} [a b] (+ a b)) (defn f {:t/sig '[:-> number?]} [] (as-> 1 2))"#,
    ] {
        let report = check(body);
        assert!(report.ok && report.complete, "{body}: {report:#?}");
    }
    let report =
        check(r#"(defn f {:t/sig '[:-> number?]} [] (as-> (inc "bad") value (+ value value)))"#);
    assert_eq!(report.diagnostics.len(), 1, "{report:#?}");
}

#[test]
fn errors_preserve_original_positions_and_named_binding_fixes_recheck() {
    let source = include_str!("../examples/conditional_bad.clj");
    let report = Session::default().check("bad.clj", source);
    assert!(!report.ok && report.complete, "{report:#?}");
    assert_eq!(report.diagnostics.len(), 3);
    assert_eq!(
        report.diagnostics[0].actual.as_deref(),
        Some("Number | String")
    );
    assert!(
        report.diagnostics[0]
            .notes
            .iter()
            .any(|n| n.contains("both paths"))
    );
    let second = &report.diagnostics[1];
    assert_eq!(
        &source[second.span.start.offset..second.span.end.offset],
        "value"
    );
    assert!(second.notes.iter().any(|n| n.contains("as->")));
    let suggestion = &second.suggestions[0];
    let mut fixed = source.to_owned();
    fixed.replace_range(
        suggestion.span.start.offset..suggestion.span.end.offset,
        &suggestion.replacement,
    );
    assert_eq!(
        Session::default()
            .check("bad.clj", &fixed)
            .diagnostics
            .len(),
        2
    );
    for body in [
        "(defn f {:t/sig '[:-> number?]} [] (cond-> 1 true))",
        "(defn f {:t/sig '[:-> number?]} [] (as-> 1))",
        "(defn f {:t/sig '[:-> number?]} [] (as-> 1 user/value value))",
        "(defn f {:t/sig '[:-> number?]} [] (as-> 1 & &))",
    ] {
        assert!(!check(body).ok, "{body}");
    }
    let report =
        check(r#"(defn f {:t/sig '[:-> string?]} [] (as-> {:name "Ada"} {:keys [name]} name))"#);
    assert!(!report.ok && !report.complete, "{report:#?}");
}

#[test]
fn conditional_chains_remain_iterative_and_do_not_duplicate_join_notes() {
    let report = check(&format!(
        "(defn f {{:t/sig '[boolean? :-> number?]}} [flag] (cond-> 1 {}))",
        "flag inc ".repeat(2000)
    ));
    assert!(report.ok && report.complete, "{report:#?}");
    let updates = (0..12)
        .map(|i| format!("flag (assoc :key{i} {i}) "))
        .collect::<String>();
    let report = check(&format!(
        "(defn f {{:t/sig '[boolean? :-> string?]}} [flag] (cond-> {{}} {updates}))"
    ));
    assert_eq!(report.diagnostics.len(), 1, "{report:#?}");
    let actual = report.diagnostics[0].actual.as_ref().unwrap();
    assert!(actual.contains(":key0?: Number"), "{actual}");
    assert!(
        !actual.contains(" | "),
        "Conditional map shapes multiplied: {actual}"
    );
    let report = check(
        "(defn f {:t/sig '[(s/keys) boolean? :-> string?]} [p flag] (:name (cond-> p flag (assoc :name \"Ada\"))))",
    );
    assert!(
        !report.ok && !report.complete,
        "Unknown open-map values were invented: {report:#?}"
    );
    assert!(report.diagnostics.iter().any(|d| d.code == "PNX9003"));
    let with_notes = format!(
        "(s/def ::name string?) (s/def ::patient (s/keys :opt-un [::name])) (defn f {{:t/sig '[::patient boolean? :-> string?]}} [p flag] (cond-> (:name p) {}))",
        "flag identity ".repeat(12)
    );
    let report = check(&with_notes);
    assert_eq!(report.diagnostics.len(), 1, "{report:#?}");
    assert!(
        report.diagnostics[0].notes.len() <= 3,
        "Join duplicated causal notes: {report:#?}"
    );
    let source = format!(
        "(defn f {{:t/sig '[:-> number?]}} [] {}1{})",
        "(cond-> ".repeat(40),
        ")".repeat(40)
    );
    let report = check(&source);
    assert!(!report.ok && !report.complete, "{report:#?}");
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.message.contains("nesting limit"))
    );
}

#[test]
fn private_helpers_in_named_threading_infer_and_invalidate_callers() {
    let source = r#"(ns demo (:require [clojure.spec.alpha :as s]))
(defn f {:t/sig '[(s/nilable string?) :-> string?]} [name] (as-> name value (normalize value)))
(defn- normalize [name] (or name "X"))"#;
    let mut session = Session::default();
    assert!(session.check("cache.clj", source).ok);
    assert_eq!(session.check("cache.clj", source).stats.reused_functions, 1);
    let report = session.check("cache.clj", &source.replace("\"X\"", "nil"));
    assert!(!report.ok && report.complete, "{report:#?}");
    assert_eq!(report.stats.checked_functions, 1);
    assert_eq!(
        report.diagnostics[0].actual.as_deref(),
        Some("Nil | String")
    );
}
