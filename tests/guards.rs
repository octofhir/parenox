use parenox::{Report, Session};

fn check(body: &str) -> Report {
    let source =
        format!("(ns demo (:require [clojure.spec.alpha :as s] [clojure.string :as str]))\n{body}");
    Session::default().check("guards.clj", &source)
}

#[test]
fn binding_guards_follow_clojure_truthiness_and_preserve_nil_results() {
    let report = Session::default().check("guards.clj", include_str!("../examples/guards.clj"));
    assert!(report.ok && report.complete, "{report:#?}");

    for body in [
        "(defn f {:t/sig '[:-> number?]} [] (if-let [x false] (inc x) 42))",
        "(defn f {:t/sig '[:-> boolean?]} [] (if-some [x false] x 42))",
        "(defn f {:t/sig '[:-> number?]} [] (when-let [x 0] (inc x) x))",
        "(defn f {:t/sig '[:-> nil?]} [] (when-some [x false]))",
        "(defn f {:t/sig '[:-> nil?]} [] (if-some [x nil] (inc \"bad\")))",
    ] {
        let report = check(body);
        assert!(report.ok && report.complete, "{body}: {report:#?}");
    }

    let source = include_str!("../examples/guards_bad.clj");
    let report = Session::default().check("guards_bad.clj", source);
    assert!(!report.ok && report.complete, "{report:#?}");
    assert_eq!(report.diagnostics.len(), 2);
    let fallback = &report.diagnostics[0];
    assert_eq!(fallback.actual.as_deref(), Some("Nil | String"));
    let edit = &fallback.suggestions[0];
    let mut fixed = source.to_owned();
    fixed.replace_range(
        edit.span.start.offset..edit.span.end.offset,
        &edit.replacement,
    );
    assert_eq!(
        Session::default()
            .check("guards_bad.clj", &fixed)
            .diagnostics
            .len(),
        1
    );
    let argument = &report.diagnostics[1];
    assert_eq!(argument.actual.as_deref(), Some("false"));
    assert_eq!(
        &source[argument.span.start.offset..argument.span.end.offset],
        "value"
    );
}

#[test]
fn bindings_are_scoped_and_the_initializer_is_checked_once() {
    for body in [
        "(defn f {:t/sig '[:-> number?]} [] (let [x 1] (if-let [x false] x x)))",
        "(defn f {:t/sig '[(s/nilable string?) :-> string?]} [x] (if-some [_ x] (str/upper-case x) \"\"))",
        "(defn f {:t/sig '[(s/nilable string?) :-> string?]} [x] (if-let [_ (string? x)] (str/upper-case x) \"\"))",
        "(defn f {:t/sig '[:-> number?]} [] (if-let [x nil] x 1))",
        "(defn if-let {:t/sig '[number? :-> number?]} [x] x) (defn f {:t/sig '[:-> number?]} [] (if-let 7))",
    ] {
        let report = check(body);
        assert!(report.ok && report.complete, "{body}: {report:#?}");
    }
    let report = check("(defn f {:t/sig '[:-> number?]} [] (if-let [x (inc \"bad\")] (+ x x) 0))");
    assert_eq!(report.diagnostics.len(), 1, "{report:#?}");

    for body in [
        "(defn f {:t/sig '[:-> number?]} [] (if-let [x 1] x x))",
        "(defn f {:t/sig '[:-> nil?]} [] (when-some [x nil] unbound))",
    ] {
        let report = check(body);
        assert!(
            report.diagnostics.iter().any(|d| d.code == "PNX2304"),
            "{report:#?}"
        );
    }
}

#[test]
fn binding_guards_validate_arity_and_binding_shape() {
    for (form, code) in [
        ("(if-let [x 1])", "PNX2554"),
        ("(if-some [x 1] x x x)", "PNX2554"),
        ("(when-let (x 1) x)", "PNX1001"),
        ("(when-some [x 1 y 2] x)", "PNX1001"),
        ("(if-let [demo/x 1] 1)", "PNX1001"),
    ] {
        let report = check(&format!("(defn f {{:t/sig '[:-> number?]}} [] {form})"));
        assert_eq!(report.diagnostics[0].code, code, "{report:#?}");
    }
}

#[test]
fn negated_conditionals_invert_guards_and_preserve_branch_tail_position() {
    for body in [
        r#"(defn f {:t/sig '[(s/nilable string?) :-> string?]} [name]
             (if-not (nil? name) (str/upper-case name) "UNKNOWN"))"#,
        r#"(defn f {:t/sig '[(s/nilable string?) :-> (s/nilable string?)]} [name]
             (when-not (nil? name) (str/upper-case name)))"#,
        r#"(defn f {:t/sig '[:-> nil?]} [] (if-not true (inc "unreachable")))"#,
        r#"(defn f {:t/sig '[:-> string?]} [] (when-not false 42 "last"))"#,
        "(defn f {:t/sig '[:-> nil?]} [] (when-not false))",
        "(defn f {:t/sig '[:-> #{:done}]} [] (loop [ready true] (if-not ready :done (recur false))))",
        "(defn f {:t/sig '[:-> #{:done}]} [] (loop [again true] (if again (when-not false (recur false)) :done)))",
        r#"(defn if-not {:t/sig '[number? :-> number?]} [n] n)
           (defn f {:t/sig '[:-> number?]} [] (if-not 3))"#,
    ] {
        let report = check(body);
        assert!(report.ok && report.complete, "{body}: {report:#?}");
    }
    for (form, code) in [
        ("(if-not false)", "PNX2554"),
        ("(when-not)", "PNX2554"),
        ("(when-not true unresolved)", "PNX2304"),
        (
            "(loop [again true] (when-not again (recur false)) 1)",
            "PNX1001",
        ),
        ("(clojure.core/if true 1 2)", "PNX9003"),
        ("(clojure.core/do 1)", "PNX9003"),
    ] {
        let report = check(&format!("(defn f {{:t/sig '[:-> number?]}} [] {form})"));
        assert!(
            report
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == code),
            "{report:#?}"
        );
    }
}
