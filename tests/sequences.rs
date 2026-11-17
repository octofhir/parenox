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
