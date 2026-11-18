use parenox::{Report, Session};

fn check(expression: &str, expected: &str) -> Report {
    Session::default().check(
        "list.clj",
        &format!("(ns demo) (defn f {{:t/sig '[:-> {expected}]}} [] {expression})"),
    )
}

#[test]
fn lists_keep_order_and_accumulate_across_recur_states() {
    let good = include_str!("../examples/list_accumulators.clj");
    let report = Session::default().check("list_accumulators.clj", good);
    assert!(report.ok && report.complete, "{report:#?}");

    let bad = include_str!("../examples/list_accumulators_bad.clj");
    let report = Session::default().check("list_accumulators_bad.clj", bad);
    let [diagnostic] = &report.diagnostics[..] else {
        panic!("{report:#?}");
    };
    assert_eq!(diagnostic.code, "PNX2322");
    assert_eq!(
        &bad[diagnostic.span.start.offset..diagnostic.span.end.offset],
        "42"
    );
}

#[test]
fn list_builders_preserve_nil_elements_and_take_seqable_tails() {
    for (expression, result) in [
        ("(if (list) :truthy :unexpected)", "#{:truthy}"),
        ("(first (cons nil '(:next)))", "nil?"),
        ("(first (next (cons :head nil)))", "nil?"),
        (
            "(if (contains? (first (next (cons :head {:key :value}))) 1) :yes :no)",
            "#{:yes}",
        ),
        ("(first (next (apply cons [:x [:y]])))", "#{:y}"),
        (
            r#"(try (list (throw (ex-info "first" {:stage :first})) (throw (ex-info "second" {:stage :second}))) (catch ExceptionInfo e (:stage (ex-data e))))"#,
            "#{:first}",
        ),
    ] {
        let report = check(expression, result);
        assert!(report.ok && report.complete, "{expression}: {report:#?}");
    }
}

#[test]
fn spreading_an_open_vector_into_list_does_not_invent_a_length() {
    let source = r#"(ns demo (:require [clojure.spec.alpha :as s]))
      (defn f {:t/sig '[(s/coll-of string? :kind vector?) :-> string?]} [names]
        (first (apply list names)))"#;
    let report = Session::default().check("spread.clj", source);
    let [diagnostic] = &report.diagnostics[..] else {
        panic!("{report:#?}");
    };
    assert_eq!(diagnostic.actual.as_deref(), Some("Nil | String"));
}
