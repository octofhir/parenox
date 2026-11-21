use parenox::{Report, Session};

fn check(expression: &str, expected: &str) -> Report {
    let source = format!("(ns demo) (defn f {{:t/sig '[:-> {expected}]}} [] {expression})");
    Session::default().check("case.clj", &source)
}

#[test]
fn dispatch_refines_grouped_literals_and_loop_arguments() {
    let good = include_str!("../examples/case.clj");
    let report = Session::default().check("case.clj", good);
    assert!(report.ok && report.complete, "{report:#?}");

    let bad = include_str!("../examples/case_bad.clj");
    let report = Session::default().check("case_bad.clj", bad);
    assert!(!report.ok && report.complete, "{report:#?}");
    let [wrong_type, no_match] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(wrong_type.code, "PNX2322");
    assert_eq!(wrong_type.actual.as_deref(), Some("true"));
    assert_eq!(no_match.code, "PNX2802");
    assert_eq!(
        &bad[no_match.span.start.offset..no_match.span.end.offset],
        ":unknown"
    );
}

#[test]
fn dispatch_distinguishes_runtime_paths_and_compiled_branches() {
    for body in [
        "(case :ready () missing :ready 1)",
        "(case true true 1 false (inc \"bad\"))",
        "(case nil nil 1 0)",
        "(case \\a \\a 1 0)",
        "(case :ready 1)",
        "(if true 1 (case :missing :present 0))",
        "(loop [x 1] (case x 1 (recur 2) 2 x))",
        "(loop [x \"Aa\"] (case x \"Aa\" (recur \"BB\") \"BB\" 1))",
        "(loop [x 1] (if true 1 (case x (recur x))))",
    ] {
        let report = check(body, "number?");
        assert!(report.ok && report.complete, "{body}: {report:#?}");
    }

    for (body, code) in [
        ("(case)", "PNX2554"),
        ("(case :ready () 1 0)", "PNX1001"),
        ("(case :ready (:ready :ready) 1 0)", "PNX2790"),
        ("(case :ready :ready 1 :ready 2)", "PNX2790"),
        ("(case :ready :ready 1 :missing unresolved)", "PNX2304"),
        ("(loop [x 1] (case x 2 (recur) 1))", "PNX2554"),
        ("(loop [x 1] (case (recur x) 1 1))", "PNX1001"),
        ("(loop [x 1] (inc (case x 1 (recur x))))", "PNX1001"),
        ("(case :ready [1] 1 0)", "PNX9001"),
    ] {
        let report = check(body, "number?");
        assert_eq!(report.diagnostics.len(), 1, "{body}: {report:#?}");
        assert_eq!(report.diagnostics[0].code, code, "{body}: {report:#?}");
    }
}

#[test]
fn integer_dispatch_compares_values_across_boxes_and_literal_spellings() {
    for expression in [
        "(case 1N 1 :expected :miss)",
        "(case 84/2 (1 16r2a) :expected :miss)",
        "(case 9223372036854775807N :other :wrong 9223372036854775807 :expected :miss)",
        "(case -9223372036854775808 -9223372036854775808N :expected :miss)",
        "(case 4294967296 0 :wrong :expected)",
        "(case 42.0 42 :wrong :expected)",
        "(case 42.0M 42 :wrong :expected)",
    ] {
        let report = check(expression, "#{:expected}");
        assert!(report.ok && report.complete, "{expression}: {report:#?}");
    }
}

#[test]
fn numeric_branches_retain_the_subjects_class_and_refine_known_map_fields() {
    let source = r#"
(ns demo (:require [clojure.spec.alpha :as s]))
(defn narrowed {:t/sig '[int? :-> (s/and int? #{9223372036854775807})]} [value]
  (case value 9223372036854775807N value 9223372036854775807))
(defn exact-bigint {:t/sig '[:-> #{false}]} []
  (let [entry {:id 2147483648N}]
    (case (:id entry) 2147483648 (int? (:id entry)) true)))
(defn finite-domain {:t/sig '[(s/and integer? #{2147483648 42}) :-> #{:wide :small}]} [value]
  (case value 2147483648 :wide 42N :small))
"#;
    let report = Session::default().check("case.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn equivalent_integer_labels_conflict_and_missing_labels_are_reported() {
    for (expression, code) in [
        ("(case 42 42 :first 42N :duplicate)", "PNX2790"),
        ("(case 42 (1 84/2) :first 16r2a :duplicate)", "PNX2790"),
        (
            "(case 2147483648 2147483648 :first 2147483648N :duplicate)",
            "PNX2790",
        ),
        ("(case 2147483648N 2147483649 :miss)", "PNX2802"),
        ("(case 1 9223372036854775808N :large :fallback)", "PNX9001"),
    ] {
        let report = check(expression, "keyword?");
        let [finding] = report.diagnostics.as_slice() else {
            panic!("{expression}: {report:#?}");
        };
        assert_eq!(finding.code, code, "{expression}: {finding:#?}");
    }
}

#[test]
fn symbolic_case_labels_are_data_and_narrow_the_tested_value() {
    let source = r#"
(ns thread-get)
(defn use-get-in {:t/sig '[#{get-in} :-> #{:nested}]} [_] :nested)
(defn dispatch {:t/sig '[symbol? :-> #{:nested :other}]} [operator]
  (let [get-in :shadow]
    (case operator get-in (use-get-in operator) :other)))
(defn guard-dispatch {:t/sig '[symbol? :-> #{:nested :other}]} [operator]
  (if (= operator (quote get-in)) (use-get-in operator) :other))
"#;
    let report = Session::default().check("thread_get.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");

    for expression in [
        "(case 'get-in (get get-in) :nested :other)",
        "(case 'quote 'get-in :nested :other)",
        "(case 'get-in 'get-in :nested :other)",
        "(case 'clojure.core/get-in get-in :other :nested)",
    ] {
        let report = check(expression, "#{:nested}");
        assert!(report.ok && report.complete, "{expression}: {report:#?}");
    }

    let report = check("(case 'get 'get :first ^:meta get :duplicate)", "keyword?");
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(finding.code, "PNX2790");
}

#[test]
fn equality_condp_narrows_map_selectors_and_only_checks_reached_results() {
    let source = r#"
(ns demo (:require [clojure.spec.alpha :as s]))
(defn render {:t/sig '[(s/or :label {:kind #{:label} :label string?}
                            :count {:kind #{:count} :count number?})
                       :-> string?]} [row]
  (condp = (:kind row)
    :label (:label row)
    :count (str (inc (:count row)))
    (subs false 0)))
(defn repeated {:t/sig '[keyword? :-> string?]} [status]
  (condp = status
    :ready "first"
    :ready (subs false 0)
    "other"))
"#;
    let report = Session::default().check("demo.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");

    for (expression, expected) in [
        ("(condp = :on :on :first :on (subs false 0))", "#{:first}"),
        ("(condp clojure.core/= nil false :off nil :nil)", "#{:nil}"),
        ("(condp = 'get 'other :other 'get :symbol)", "#{:symbol}"),
        ("(condp = \\x \\x :char :fallback)", "#{:char}"),
        (
            "(loop [state :start] (condp = state :start (recur :done) :done 7))",
            "number?",
        ),
    ] {
        let report = check(expression, expected);
        assert!(report.ok && report.complete, "{expression}: {report:#?}");
    }
}

#[test]
fn equality_condp_keeps_compiler_findings_and_reports_unchecked_predicates() {
    for (expression, code) in [
        ("(condp = :none :one 1)", "PNX2802"),
        ("(condp = :one :one 1 :two unresolved)", "PNX2304"),
        ("(condp = (subs false 0) 1)", "PNX2322"),
        ("(condp = [] :one 1 2)", "PNX9001"),
        ("(condp = :one :one :>> (fn [_] 1) 2)", "PNX9001"),
        (
            "(let [= (fn [_ _] true)] (condp = :none :one 1 2))",
            "PNX9001",
        ),
    ] {
        let report = check(expression, "number?");
        let [finding] = report.diagnostics.as_slice() else {
            panic!("{expression}: {report:#?}");
        };
        assert_eq!(finding.code, code, "{expression}: {finding:#?}");
    }
    let report = check("(condp = (subs false 0) (identity :key) 1 2)", "number?");
    let codes: Vec<_> = report
        .diagnostics
        .iter()
        .map(|finding| finding.code.as_str())
        .collect();
    assert_eq!(codes, ["PNX2322", "PNX9001"], "{report:#?}");
}

#[test]
fn empty_condp_can_throw_while_printing_an_arbitrary_subject() {
    let source = r#"
(ns demo)
(defn explain {:t/sig '[t/any :-> #{:argument}]} [value]
  (try (condp = value)
       (catch clojure.lang.ExceptionInfo _ :printing)
       (catch IllegalArgumentException _ :argument)))
"#;
    let mut session = Session::default();
    let report = session.check("demo.clj", source);
    assert!(
        report
            .diagnostics
            .iter()
            .any(|finding| finding.code == "PNX2802")
    );
    assert!(
        report.diagnostics.iter().any(|finding| {
            finding.code == "PNX2322"
                && finding
                    .actual
                    .as_deref()
                    .is_some_and(|actual| actual.contains(":printing"))
        }),
        "{report:#?}"
    );

    let scalar = session.check("demo.clj", &source.replace("t/any", "keyword?"));
    let [finding] = scalar.diagnostics.as_slice() else {
        panic!("{scalar:#?}");
    };
    assert_eq!(finding.code, "PNX2802");
}
