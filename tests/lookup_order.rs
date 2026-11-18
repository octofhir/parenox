use parenox::{Report, Session};

fn check(expression: &str, expected: &str) -> Report {
    Session::default().check(
        "lookup.clj",
        &format!("(defn f {{:t/sig '[:-> {expected}]}} [] {expression})"),
    )
}

#[test]
fn lookup_examples_track_exact_recovery_and_default_result() {
    let mut session = Session::default();
    let report = session.check(
        "lookup_order.clj",
        include_str!("../examples/lookup_order.clj"),
    );
    assert!(report.ok && report.complete, "{report:#?}");
    let report = session.check(
        "lookup_order_bad.clj",
        include_str!("../examples/lookup_order_bad.clj"),
    );
    let [diagnostic] = &report.diagnostics[..] else {
        panic!("{report:#?}");
    };
    assert_eq!(diagnostic.code, "PNX2322");
    assert_eq!(diagnostic.actual.as_deref(), Some(":default"));
}

#[test]
fn argument_failures_take_precedence_over_lookup_and_later_arguments() {
    for expression in [
        r#"(get (throw (ex-info "target" {:stage :target})) :status (inc "dead"))"#,
        r#"(get (throw (ex-info "target" {:stage :target})) (throw (ex-info "key" {:stage :key})))"#,
        r#"(req! (throw (ex-info "target" {:stage :target})) :missing)"#,
        r#"(contains? (throw (ex-info "target" {:stage :target})) (throw (ex-info "key" {:stage :key})))"#,
        r#"(get-in (throw (ex-info "target" {:stage :target})) [(throw (ex-info "key" {:stage :key}))])"#,
    ] {
        let form = format!("(try {expression} (catch ExceptionInfo e (:stage (ex-data e))))");
        let report = check(&form, "#{:target}");
        assert!(report.ok && report.complete, "{expression}: {report:#?}");
    }
    let report = check("(get (throw nil) unresolved :default)", "nil?");
    let [diagnostic] = &report.diagnostics[..] else {
        panic!("{report:#?}");
    };
    assert_eq!(diagnostic.code, "PNX2304");
}

#[test]
fn successful_lookups_do_not_add_impossible_exception_paths() {
    for (expression, expected) in [
        ("(get {} :status :missing)", "#{:missing}"),
        ("(get #{:ready} :missing)", "nil?"),
        ("(get-in {:status :ready} [:status])", "#{:ready}"),
        ("(contains? {:status nil} :status)", "#{true}"),
        ("(req! {:status nil} :status)", "nil?"),
    ] {
        let form = format!("(try {expression} (catch Throwable _ 42))");
        let report = check(&form, expected);
        assert!(report.ok && report.complete, "{expression}: {report:#?}");
    }
    let report = check(
        "(try (req! 1 :missing) (catch IllegalArgumentException _ :missing))",
        "#{:missing}",
    );
    let [diagnostic] = &report.diagnostics[..] else {
        panic!("{report:#?}");
    };
    assert_eq!(diagnostic.code, "PNX2741");
}
