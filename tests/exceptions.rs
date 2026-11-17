use parenox::{Report, Session};

fn check(expression: &str, expected: &str) -> Report {
    let source = format!("(ns demo) (defn f {{:t/sig '[:-> {expected}]}} [] {expression})");
    Session::default().check("exceptions.clj", &source)
}

#[test]
fn explicit_exceptions_preserve_data_and_terminate_the_path() {
    let good = include_str!("../examples/exceptions.clj");
    let report = Session::default().check("exceptions.clj", good);
    assert!(report.ok && report.complete, "{report:#?}");

    let bad = include_str!("../examples/exceptions_bad.clj");
    let report = Session::default().check("exceptions_bad.clj", bad);
    assert!(!report.ok && report.complete, "{report:#?}");
    assert_eq!(report.diagnostics.len(), 2, "{report:#?}");
    for (diagnostic, text) in report
        .diagnostics
        .iter()
        .zip(["\"not an exception\"", "\"not a cause\""])
    {
        assert_eq!(diagnostic.code, "PNX2322");
        assert_eq!(diagnostic.expected.as_deref(), Some("Nil | Throwable"));
        assert_eq!(
            &bad[diagnostic.span.start.offset..diagnostic.span.end.offset],
            text
        );
    }
}

#[test]
fn nonreturning_arguments_keep_compiler_checks_in_later_forms() {
    for expression in [
        "(throw nil)",
        "(+ 1 (throw nil) (inc \"dead\"))",
        "(:id (throw nil) (inc \"dead\"))",
        "((throw nil) (inc \"dead\"))",
        "((fn [x y] (inc \"dead\")) (throw nil) 1)",
        "[(throw nil) (inc \"dead\")]",
        "#{(throw nil)}",
        "{:id (throw nil)}",
        "(loop [n 0 total 0] (recur (throw nil) (inc \"dead\")))",
    ] {
        let report = check(expression, "string?");
        assert!(report.ok && report.complete, "{expression}: {report:#?}");
    }

    for (expression, code) in [
        ("(throw)", "PNX2554"),
        ("(ex-info \"missing\")", "PNX2554"),
        ("(+ (throw nil) unresolved)", "PNX2304"),
        ("[(throw nil) unresolved]", "PNX2304"),
        ("(loop [x 0] (throw (recur x)))", "PNX1001"),
    ] {
        let report = check(expression, "string?");
        assert_eq!(report.diagnostics.len(), 1, "{expression}: {report:#?}");
        assert_eq!(
            report.diagnostics[0].code, code,
            "{expression}: {report:#?}"
        );
    }

    for expression in ["(ex-data nil)", "(ex-data 1)"] {
        let report = check(expression, "nil?");
        assert!(report.ok && report.complete, "{expression}: {report:#?}");
    }
}
