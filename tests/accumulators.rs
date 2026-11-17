use parenox::{Report, Session};

fn check(expression: &str, expected: &str) -> Report {
    let form = format!("(ns demo) (defn f {{:t/sig '[:-> {expected}]}} [] {expression})");
    Session::default().check("accum.clj", &form)
}

#[test]
fn accumulating_vectors_can_grow_without_losing_their_element_types() {
    let good = include_str!("../examples/accumulators.clj");
    let report = Session::default().check("accumulators.clj", good);
    assert!(report.ok && report.complete, "{report:#?}");

    let bad = include_str!("../examples/accumulators_bad.clj");
    let report = Session::default().check("accumulators_bad.clj", bad);
    assert!(!report.ok && report.complete, "{report:#?}");
    assert_eq!(report.diagnostics.len(), 1, "{report:#?}");
    let diagnostic = &report.diagnostics[0];
    assert_eq!(diagnostic.actual.as_deref(), Some("Nil | String"));
    assert_eq!(
        &bad[diagnostic.span.start.offset..diagnostic.span.end.offset],
        "(first remaining)"
    );
}

#[test]
fn conj_preserves_vector_order_and_prepends_to_seqs() {
    for (expression, expected) in [
        ("(count (conj))", "number?"),
        ("(conj \"same\")", "#{\"same\"}"),
        ("(nth (conj [9] \"second\") 1)", "#{\"second\"}"),
        ("(first (conj nil 1 \"first\"))", "#{\"first\"}"),
        (
            "(nth (conj (seq [1 2]) \"third\" \"fourth\") 1)",
            "#{\"third\"}",
        ),
        ("(count (conj #{:ready} :done))", "number?"),
        (
            "(loop [output []] (case (first output) nil (recur (conj output 1)) 1 (nth output 0)))",
            "number?",
        ),
    ] {
        let report = check(expression, expected);
        assert!(report.ok && report.complete, "{expression}: {report:#?}");
    }

    for (expression, code) in [
        ("(conj false 1)", "PNX2322"),
        ("(conj {} [:status :open])", "PNX9001"),
        ("(nth (conj [1] 2) 2)", "PNX2801"),
    ] {
        let report = check(expression, "number?");
        assert_eq!(report.diagnostics.len(), 1, "{expression}: {report:#?}");
        assert_eq!(
            report.diagnostics[0].code, code,
            "{expression}: {report:#?}"
        );
    }
}
