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

#[test]
fn map_writes_wait_for_every_argument_before_validating_the_write() {
    for expression in [
        r#"(assoc (throw (ex-info "target" {:stage :first})) :status (throw (ex-info "later" {:stage :later})))"#,
        r#"(assoc {} (throw (ex-info "key" {:stage :first})) (throw (ex-info "later" {:stage :later})) :status (throw (ex-info "later" {:stage :later})))"#,
        r#"(assoc 42 :status (throw (ex-info "value" {:stage :first})))"#,
        r#"(assoc-in (throw (ex-info "target" {:stage :first})) [(throw (ex-info "later" {:stage :later}))] (throw (ex-info "later" {:stage :later})))"#,
        r#"(assoc-in {} [(throw (ex-info "key" {:stage :first}))] (throw (ex-info "later" {:stage :later})))"#,
    ] {
        let form = format!("(try {expression} (catch ExceptionInfo e (:stage (ex-data e))))");
        let report = check(&form, "#{:first}");
        assert!(report.ok && report.complete, "{expression}: {report:#?}");
    }
    let report = check("(assoc {} :status (throw nil) :later unresolved)", "nil?");
    let [diagnostic] = &report.diagnostics[..] else {
        panic!("{report:#?}");
    };
    assert_eq!(diagnostic.code, "PNX2304");
}

#[test]
fn invalid_scalar_writes_raise_only_class_cast_exceptions() {
    let report = check(
        "(try (assoc-in {:request 7} [:request :status] :ready)
              (catch ExceptionInfo _ :unexpected)
              (catch ClassCastException _ :wrong-parent))",
        "#{:wrong-parent}",
    );
    let [diagnostic] = &report.diagnostics[..] else {
        panic!("{report:#?}");
    };
    assert_eq!(diagnostic.code, "PNX2322");
    assert_eq!(diagnostic.actual.as_deref(), Some("Number"));
}
