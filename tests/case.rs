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
