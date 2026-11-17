use parenox::{Report, Session};

fn check(source: &str) -> Report {
    Session::default().check("cond.clj", source)
}

#[test]
fn cond_narrows_selected_and_remaining_paths_and_preserves_recur() {
    let report = check(include_str!("../examples/cond.clj"));
    assert!(report.ok && report.complete, "{report:#?}");
    let bad = include_str!("../examples/cond_bad.clj");
    let report = check(bad);
    assert!(!report.ok && report.complete, "{report:#?}");
    assert_eq!(report.diagnostics.len(), 2);
    let missing = &report.diagnostics[0];
    assert_eq!(missing.code, "PNX2322");
    assert_eq!(missing.actual.as_deref(), Some("Nil | String"));
    let branch = &report.diagnostics[1];
    assert_eq!(branch.code, "PNX2322");
    assert_eq!(branch.actual.as_deref(), Some("String"));
    assert_eq!(
        &bad[branch.span.start.offset..branch.span.end.offset],
        "input"
    );
}

#[test]
fn cond_respects_compiler_checks_and_clojure_truthiness() {
    for body in [
        "(defn f {:t/sig '[:-> nil?]} [] (cond))",
        "(defn f {:t/sig '[:-> number?]} [] (cond 0 5 :else (inc \"bad\")))",
        "(defn f {:t/sig '[:-> number?]} [] (cond false 0 :anything 7))",
        "(defn f {:t/sig '[:-> number?]} [] (loop [x 0] (cond true x :else (recur \"dead\"))))",
        "(defn cond {:t/sig '[number? :-> number?]} [x] x)
         (defn f {:t/sig '[:-> number?]} [] (cond 9))",
    ] {
        let report = check(body);
        assert!(report.ok && report.complete, "{body}: {report:#?}");
    }

    for (expression, code) in [
        ("(cond true)", "PNX2554"),
        ("(cond :else 1 true missing)", "PNX2304"),
        ("(loop [x 0] (cond (recur x) 1 :else x))", "PNX1001"),
        ("(loop [x 0] (cond true x :else (recur)))", "PNX2554"),
        ("(+ 1 (cond :else (recur)))", "PNX1001"),
    ] {
        let report = check(&format!(
            "(defn f {{:t/sig '[:-> number?]}} [] {expression})"
        ));
        assert_eq!(report.diagnostics.len(), 1, "{expression}: {report:#?}");
        assert_eq!(
            report.diagnostics[0].code, code,
            "{expression}: {report:#?}"
        );
        assert!(report.complete);
    }
}
