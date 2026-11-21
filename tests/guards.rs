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

#[test]
fn when_first_binds_the_first_value_even_when_it_is_nil_or_false() {
    let source = r#"(ns demo (:require [clojure.spec.alpha :as s]))
      (defn nil-item {:t/sig '[:-> #{:ran}]} []
        (when-first [item [nil :next]] (if (nil? item) :ran :unexpected)))
      (defn false-item {:t/sig '[:-> #{:ran}]} []
        (when-first [item [false :next]] (if item :unexpected :ran)))
      (defn empty-input {:t/sig '[:-> nil?]} []
        (when-first [item []] (inc "unreachable")))
      (defn entry {:t/sig '[:-> number?]} []
        (when-first [[_ value] {:cost 5}] (inc value)))
      (defn tail-recur {:t/sig '[:-> #{:next}]} []
        (loop [xs ["skip" :next]]
          (when-first [x xs] (if (string? x) (recur (next xs)) x))))
      (defn never-throws {:t/sig '[:-> #{:done}]} []
        (try (when-first [item [:done]] item)
             (catch ExceptionInfo _ :unexpected)))"#;
    let report = Session::default().check("when_first.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn when_first_reports_invalid_syntax_inputs_and_unresolved_skipped_forms() {
    for (expression, code) in [
        ("(when-first [x] x)", "PNX1001"),
        ("(when-first [x 42] (inc \"unreachable\"))", "PNX2322"),
        ("(when-first [x nil] unresolved)", "PNX2304"),
        (
            "(loop [xs [1]] (inc (when-first [x xs] (recur []))))",
            "PNX1001",
        ),
    ] {
        let expected = if code == "PNX2304" { "nil?" } else { "number?" };
        let source = format!("(defn f {{:t/sig '[:-> {expected}]}} [] {expression})");
        let report = Session::default().check("when_first.clj", &source);
        let [diagnostic] = &report.diagnostics[..] else {
            panic!("{expression}: {report:#?}");
        };
        assert_eq!(diagnostic.code, code);
    }
}

#[test]
fn scalar_equality_keeps_only_possible_literal_branches() {
    let source = r#"(ns demo)
      (defn same {:t/sig '[:-> #{true}]} [] (= :ready :ready :ready))
      (defn distinct {:t/sig '[:-> #{false}]} [] (= :ready :pending))
      (defn nil-vs-false {:t/sig '[:-> #{false}]} [] (= nil false))
      (defn char-vs-string {:t/sig '[:-> #{false}]} [] (= \a "a"))
      (defn singleton {:t/sig '[number? :-> #{true}]} [n] (= n))
      (defn all-distinct {:t/sig '[#{:stopped :paused} :-> #{false}]} [status]
        (= :ready status))
      (defn possibly-equal {:t/sig '[#{:ready :paused} :-> #{:ok :wait}]} [status]
        (if (= status :ready) :ok :wait))
      (defn opaque-values {:t/sig '[:-> #{:same :different}]} []
        (if (= 1 1N) :same :different))"#;
    let report = Session::default().check("status.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn seq_tests_keep_only_the_possible_values_in_each_branch() {
    let source = r#"(ns demo (:require [clojure.spec.alpha :as s]))
      (defn ensure-empty
        {:t/sig '[(s/nilable #{""}) :-> (s/nilable #{""})]}
        [candidate] candidate)
      (defn read-path
        {:t/sig '[(s/nilable string?) :-> (s/nilable string?)]}
        [path]
        (if (seq path) (subs path 0) (ensure-empty path)))"#;
    let mut session = Session::default();
    let report = session.check("seq_guard.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");
    let offset = source.find("path 0").unwrap();
    let Some(parenox::DocumentHover::Local(hover)) =
        session.document_hover_at("seq_guard.clj", offset)
    else {
        panic!("missing hover in seq guard");
    };
    assert_eq!(hover.inferred_type, "String");
}

#[test]
fn a_shadowed_seq_does_not_prove_emptiness() {
    let report = check(
        r#"(defn read-path
             {:t/sig '[(s/nilable string?) :-> (s/nilable string?)]}
             [path]
             (let [seq (fn [ignored] true)]
               (when (seq path) (subs path 0))))"#,
    );
    let [diagnostic] = &report.diagnostics[..] else {
        panic!("{report:#?}");
    };
    assert_eq!(diagnostic.code, "PNX2322");
    assert_eq!(diagnostic.actual.as_deref(), Some("Nil | String"));
}

#[test]
fn when_first_preserves_presence_of_the_saved_selector_and_the_falsey_head() {
    let source = r#"(ns demo (:require [clojure.spec.alpha :as s]))
      (s/def ::items (s/tuple (s/nilable (s/or :false #{false} :text string?))))
      (defn assert-present {:t/sig '[::items :-> ::items]} [value] value)
      (defn read-items
        {:t/sig '[{:items (s/nilable ::items)} :-> any?]}
        [row]
        (let [saved row]
          (when-first [first-item (get row :items)]
            [(assert-present (:items saved)) first-item])))"#;
    let mut session = Session::default();
    let report = session.check("seq_guard.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");
    let offset = source.rfind("first-item").unwrap();
    let Some(parenox::DocumentHover::Local(hover)) =
        session.document_hover_at("seq_guard.clj", offset)
    else {
        panic!("missing hover on head");
    };
    assert_eq!(hover.inferred_type, "Nil | false | String");
}
