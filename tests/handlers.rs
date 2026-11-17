use parenox::{Report, Session};

fn check(expression: &str, expected: &str) -> Report {
    let source = format!("(ns demo) (defn f {{:t/sig '[:-> {expected}]}} [] {expression})");
    Session::default().check("handlers.clj", &source)
}

#[test]
fn try_joins_recovery_and_success_but_ignores_cleanup_values() {
    let report = Session::default().check("handlers.clj", include_str!("../examples/handlers.clj"));
    assert!(report.ok && report.complete, "{report:#?}");
    let bad = include_str!("../examples/handlers_bad.clj");
    let report = Session::default().check("handlers_bad.clj", bad);
    assert!(!report.ok && report.complete, "{report:#?}");
    let [wrong_type, recur] = report.diagnostics.as_slice() else {
        panic!("{report:#?}")
    };
    assert_eq!(wrong_type.code, "PNX2322");
    assert_eq!(wrong_type.expected.as_deref(), Some("Number"));
    assert_eq!(
        &bad[wrong_type.span.start.offset..wrong_type.span.end.offset],
        "(:field (ex-data e))"
    );
    assert_eq!(wrong_type.actual.as_deref(), Some(":id"));
    assert_eq!(recur.code, "PNX1001");
    assert_eq!(recur.message, "Cannot recur across try");
}

#[test]
fn handlers_keep_clojure_scope_and_tail_rules() {
    for (expression, expected) in [
        ("(try)", "nil?"),
        ("(try 1)", "number?"),
        ("(try 1 (catch Throwable e (inc \"dead\")))", "number?"),
        ("(try (throw nil) (catch Throwable e))", "nil?"),
        (
            "(try (throw nil) (catch RuntimeException e 1) (catch NullPointerException e \"dead\"))",
            "number?",
        ),
        (
            "(try 1 (catch Throwable e (loop [i 0] (recur i))))",
            "number?",
        ),
        ("(try 1 (finally (loop [i 0] (recur i))))", "number?"),
        (
            "(loop [once? true] (if once? (do (try 1) (recur false)) 2))",
            "number?",
        ),
    ] {
        let report = check(expression, expected);
        assert!(report.ok && report.complete, "{expression}: {report:#?}");
    }

    for (expression, code, message) in [
        (
            "(loop [x 0] (try (recur x)))",
            "PNX1001",
            "Cannot recur across try",
        ),
        (
            "(loop [x 0] (try 1 (catch Throwable e (recur x))))",
            "PNX1001",
            "recur is only allowed in tail position",
        ),
        (
            "(loop [x 0] (try 1 (finally (recur x))))",
            "PNX1001",
            "recur is only allowed in tail position",
        ),
        (
            "(try 1 (finally 2) (catch Throwable e 3))",
            "PNX1001",
            "finally must be the last clause",
        ),
        (
            "(try 1 (catch Throwable e 2) 3)",
            "PNX1001",
            "Only catch or finally",
        ),
        (
            "(try 1 (catch Throwable ns/e 2))",
            "PNX1001",
            "unqualified binding",
        ),
        (
            "(try 1 (catch String e 2))",
            "PNX1001",
            "must extend Throwable",
        ),
        (
            "(try 1 (catch vendor.CustomException e 2))",
            "PNX9001",
            "outside the supported JVM exception classes",
        ),
        (
            "(try 1 (catch Throwable e 1) (finally e))",
            "PNX2304",
            "Unresolved or private",
        ),
        (
            "(try 1 (catch Throwable e unresolved))",
            "PNX2304",
            "Unresolved or private",
        ),
    ] {
        let report = check(expression, "number?");
        assert_eq!(report.diagnostics.len(), 1, "{expression}: {report:#?}");
        assert_eq!(
            report.diagnostics[0].code, code,
            "{expression}: {report:#?}"
        );
        assert!(
            report.diagnostics[0].message.contains(message),
            "{expression}: {report:#?}"
        );
    }
}

#[test]
fn known_throws_select_handlers_and_preserve_structured_data() {
    for (expression, expected) in [
        (
            r#"(try (throw (ex-info "missing" {:code "MISSING"}))
                 (catch NullPointerException e (inc "unreachable"))
                 (catch RuntimeException e (:code (ex-data e))))"#,
            r#"#{"MISSING"}"#,
        ),
        (
            r#"(try (throw nil)
                 (catch ExceptionInfo e (:code (ex-data e)))
                 (catch NullPointerException e "fallback"))"#,
            r#"#{"fallback"}"#,
        ),
        (
            r#"(try 1 (catch ExceptionInfo e (:code (ex-data e))))"#,
            "number?",
        ),
        (
            r#"(try (try (throw (ex-info "missing" {:code "INNER"}))
                       (catch ExceptionInfo e (throw e)))
                 (catch ExceptionInfo e (:code (ex-data e))))"#,
            r#"#{"INNER"}"#,
        ),
        (
            r#"(try (inc (throw (ex-info "missing" {:code "DIRECT"})))
                 (catch ExceptionInfo e (:code (ex-data e))))"#,
            r#"#{"DIRECT"}"#,
        ),
        (
            r#"(try (loop [] (recur)) (finally (inc "unreachable")))"#,
            "number?",
        ),
    ] {
        let report = check(expression, expected);
        assert!(report.ok && report.complete, "{expression}: {report:#?}");
    }
}

#[test]
fn an_ordinary_function_call_keeps_catch_results_possible() {
    let source = r#"
        (ns demo)
        (defn- fail {:t/sig '[:-> string?]} []
          (throw (ex-info "missing" {:code "MISSING"})))
        (defn handle {:t/sig '[:-> string?]} []
          (try (fail)
               (catch ExceptionInfo e (count (ex-data e)))))
    "#;
    let report = Session::default().check("handlers.clj", source);
    assert!(!report.ok && report.complete, "{report:#?}");
    assert_eq!(report.diagnostics.len(), 1, "{report:#?}");
    assert_eq!(
        report.diagnostics[0].actual.as_deref(),
        Some("Number | String")
    );
}
