use parenox::{Report, Session};

fn check(expression: &str, contract: &str) -> Report {
    let source = format!("(ns demo) (defn f {{:t/sig '[:-> {contract}]}} [] {expression})");
    Session::default().check("sequences.clj", &source)
}

#[test]
fn nonempty_seq_guards_support_running_totals_and_precise_positions() {
    let report =
        Session::default().check("sequences.clj", include_str!("../examples/sequences.clj"));
    assert!(report.ok && report.complete, "{report:#?}");
    let report = Session::default().check(
        "sequences_bad.clj",
        include_str!("../examples/sequences_bad.clj"),
    );
    assert!(!report.ok && report.complete, "{report:#?}");
    let [absence, scalar] = report.diagnostics.as_slice() else {
        panic!("{report:#?}")
    };
    assert_eq!(absence.code, "PNX2322");
    assert_eq!(absence.actual.as_deref(), Some("Nil | Number"));
    assert_eq!(scalar.expected.as_deref(), Some("Seqable or Nil"));
    assert_eq!(scalar.actual.as_deref(), Some("false"));
}

#[test]
fn seq_emptiness_preserves_nil_elements_and_index_errors() {
    for (expression, contract) in [
        ("(seq [])", "nil?"),
        ("(next nil)", "nil?"),
        ("(first nil)", "nil?"),
        ("(next [1])", "nil?"),
        ("(if (seq [nil]) (first (seq [nil])) 42)", "nil?"),
        ("(nth (seq [1 \"name\"]) 1)", "string?"),
        ("(nth (seq [1]) 3 \"fallback\")", "string?"),
        ("(count (seq [1 2]))", "number?"),
    ] {
        let report = check(expression, contract);
        assert!(report.ok && report.complete, "{expression}: {report:#?}");
    }
    for (expression, code) in [
        ("(nth (seq [1]) 1)", "PNX2801"),
        ("(next false)", "PNX2322"),
        ("(seq \"abc\")", "PNX9001"),
        ("(seq [] [])", "PNX2554"),
    ] {
        let report = check(expression, "number?");
        assert_eq!(report.diagnostics.len(), 1, "{expression}: {report:#?}");
        assert_eq!(
            report.diagnostics[0].code, code,
            "{expression}: {report:#?}"
        );
    }
}

#[test]
fn rest_returns_a_truthy_empty_list_and_keeps_remaining_positions() {
    for (expression, contract) in [
        ("(if (rest nil) \"truthy\" 0)", "string?"),
        ("(if (rest [1]) \"truthy\" 0)", "string?"),
        ("(seq (rest []))", "nil?"),
        ("(seq (rest [nil]))", "nil?"),
        ("(first (rest [1 nil]))", "nil?"),
        ("(next (rest [1 2]))", "nil?"),
        ("(nth (rest [1 \"tail\"]) 0)", "string?"),
        ("(nth (rest [1]) 0 \"missing\")", "string?"),
        ("(seq ())", "nil?"),
        ("(seq '())", "nil?"),
        ("(seq (#()))", "nil?"),
        ("(apply + (rest [4]))", "number?"),
        ("(first (conj (rest [4]) \"reset\"))", "string?"),
    ] {
        let report = check(expression, contract);
        assert!(report.ok && report.complete, "{expression}: {report:#?}");
    }
    let report = check("(rest false)", "number?");
    let [diagnostic] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(diagnostic.code, "PNX2322");
    assert_eq!(diagnostic.expected.as_deref(), Some("Seqable or Nil"));

    let report = check("(nth (rest nil) 0)", "number?");
    assert_eq!(report.diagnostics[0].code, "PNX2801", "{report:#?}");
}

#[test]
fn emptiness_distinguishes_nil_empty_and_present_false_values() {
    for (expression, contract) in [
        ("(empty? nil)", "#{true}"),
        ("(empty? (rest [1]))", "#{true}"),
        ("(empty? {})", "#{true}"),
        ("(empty? #{})", "#{true}"),
        ("(empty? \"\")", "#{true}"),
        ("(empty? [nil])", "#{false}"),
        ("(empty? {:x false})", "#{false}"),
        ("(not-empty \"\")", "nil?"),
        ("(not-empty \"Ada\")", "#{\"Ada\"}"),
        (
            "(not-empty [nil])",
            "(clojure.spec.alpha/coll-of nil? :kind vector?)",
        ),
        (
            "(filterv not-empty [\"\" \"Ada\"])",
            "(clojure.spec.alpha/coll-of #{\"Ada\"} :kind vector?)",
        ),
    ] {
        let report = check(expression, contract);
        assert!(report.ok && report.complete, "{expression}: {report:#?}");
    }
    for expression in ["(empty? false)", "(not-empty 0)"] {
        let report = check(expression, "boolean?");
        assert_eq!(report.diagnostics[0].code, "PNX2322", "{report:#?}");
    }
}

#[test]
fn emptiness_guards_refine_sequence_state_and_open_empty_shapes() {
    let report = Session::default().check(
        "sequences.clj",
        r#"(ns demo (:require [clojure.spec.alpha :as s]))
           (defn sum-values
             {:t/sig '[(s/coll-of number? :kind vector?) :-> number?]}
             [values]
             (loop [items (seq values) total 0]
               (if-not (empty? items)
                 (recur (next items) (+ total (first items)))
                 total)))
           (defn empty-first
             {:t/sig '[(s/coll-of number? :kind vector?) :-> nil?]}
             [values]
             (when (empty? values) (first values)))
           (defn fallback
             {:t/sig '[(s/keys) :-> string?]} [configuration]
             (if (empty? configuration)
               (:unexpected configuration "missing")
               "configured"))"#,
    );
    assert!(report.ok && report.complete, "{report:#?}");
}
