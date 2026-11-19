use parenox::{Report, Session};

fn check(expression: &str, expected: &str) -> (Report, String) {
    let source = format!(
        "(ns demo (:require [clojure.spec.alpha :as s]))\n(defn f {{:t/sig '[:-> {expected}]}} [] {expression})"
    );
    (
        Session::default().check("comprehension.clj", &source),
        source,
    )
}

#[test]
fn for_keeps_the_element_type_with_sequential_scopes() {
    // Adapted from clojure-lsp's feature.command: iterate a map and
    // destructure the document ID and the results together.
    let (report, _) = check(
        r#"(let [changes-by-uri {:a [{:uri nil} {:uri "file:///a"}]}]
             (for [[doc-id sub-results] changes-by-uri
                   {:keys [uri]} sub-results
                   :let [uri (if (= doc-id :a) uri nil) path uri]
                   :when (some? path)
                   :while (string? path)]
               path))"#,
        "(s/coll-of string? :kind seq?)",
    );
    assert!(report.ok && report.complete, "{report:#?}");

    let (report, _) = check(
        "(let [xs [[1]]] (first (for [xs xs x xs :let [x (+ x 1)]] x)))",
        "number?",
    );
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn for_reports_a_wrong_element_type_at_the_consumer() {
    let (report, _) = check(
        r#"(for [{:keys [uri]} [{:uri "file:///a"}]] uri)"#,
        "(s/coll-of number? :kind seq?)",
    );
    assert!(!report.ok, "{report:#?}");
    let [diagnostic] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(diagnostic.code, "PNX2322");
}

#[test]
fn doseq_checks_the_body_and_discards_the_result() {
    // Adapted from clojure-lsp's feature.file-management doseq over uris.
    let (report, _) = check(r#"(doseq [uri ["file:///a"]] (subs uri 0 4) uri)"#, "nil?");
    assert!(report.ok && report.complete, "{report:#?}");

    let (report, source) = check(r#"(doseq [uri [17]] (subs uri 0 4))"#, "nil?");
    let [diagnostic] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(diagnostic.code, "PNX2322");
    assert_eq!(
        &source[diagnostic.span.start.offset..diagnostic.span.end.offset],
        "uri"
    );
}

#[test]
fn comprehensions_reject_non_seqable_sources_and_later_locals_are_not_visible() {
    let (report, source) = check("(for [x 42] x)", "(s/coll-of number? :kind seq?)");
    let [diagnostic] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(diagnostic.code, "PNX2322");
    assert_eq!(
        &source[diagnostic.span.start.offset..diagnostic.span.end.offset],
        "42"
    );

    let (report, _) = check(
        "(for [x [later] later [1]] x)",
        "(s/coll-of number? :kind seq?)",
    );
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.code == "PNX2304" && d.message.contains("later")),
        "{report:#?}"
    );

    let source = r#"(ns demo)
      (defn f {:t/sig '[any? :-> nil?]} [input] (doseq [item input] item))"#;
    let report = Session::default().check("unknown.clj", source);
    assert!(!report.complete, "{report:#?}");
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.code == "PNX9004" && d.message.contains("seqable")),
        "{report:#?}"
    );
}

#[test]
fn doseq_with_only_let_modifiers_preserves_the_body_result() {
    let (report, _) = check("(doseq [:let [x 1 y (inc x)]] y)", "number?");
    assert!(report.ok && report.complete, "{report:#?}");
    let (report, _) = check("(doseq [:let [x 1]] x)", "nil?");
    assert!(
        report.diagnostics.iter().any(|d| d.code == "PNX2322"),
        "{report:#?}"
    );
}

#[test]
fn for_defers_iteration_effects_but_not_evaluation_of_the_first_source() {
    for (expression, expected) in [
        (
            r#"(try (do (for [x [1] y (throw (ex-info "later" {}))] y) :created)
                  (catch ExceptionInfo _ :not-created))"#,
            "#{:created}",
        ),
        (
            r#"(try (do (for [x [1]] (throw (ex-info "later" {}))) :created)
                  (catch ExceptionInfo _ :not-created))"#,
            "#{:created}",
        ),
        (
            r#"(try (for [x (throw (ex-info "now" {}))] x)
                  (catch ExceptionInfo _ :caught))"#,
            "#{:caught}",
        ),
        (
            r#"(try (doseq [x [1]] (throw (ex-info "now" {})))
                  (catch ExceptionInfo _ :caught))"#,
            "#{:caught}",
        ),
        (
            r#"(try (doseq [x []] (throw (ex-info "unreachable" {})))
                  (catch ExceptionInfo _ :not-caught))"#,
            "nil?",
        ),
        (
            r#"(try (doseq [x [1] :while false] (throw (ex-info "unreachable" {})))
                  (catch ExceptionInfo _ :not-caught))"#,
            "nil?",
        ),
    ] {
        let (report, _) = check(expression, expected);
        assert!(report.ok && report.complete, "{expression}: {report:#?}");
    }
}
