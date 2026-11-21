use parenox::{DocumentHover, Report, Session};

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
fn a_literal_get_in_lookup_can_throw_before_the_next_key_expression() {
    let source = r#"(ns demo)
        (defn inlined {:t/sig '[t/any :-> #{:later}]} [target]
          (try
            (get-in target [:key (throw (ex-info "late key" {}))])
            (catch IllegalArgumentException _ :earlier)
            (catch ExceptionInfo _ :later)))
        (defn with-default {:t/sig '[t/any :-> #{:later}]} [target]
          (try
            (get-in target [:key (throw (ex-info "late key" {}))] :fallback)
            (catch IllegalArgumentException _ :earlier)
            (catch ExceptionInfo _ :later)))
        (defn computed-path {:t/sig '[t/any :-> #{:later}]} [target]
          (try
            (let [path [:key (throw (ex-info "late key" {}))]] (get-in target path))
            (catch IllegalArgumentException _ :earlier)
            (catch ExceptionInfo _ :later)))"#;
    let report = Session::default().check("lookup.clj", source);
    let [diagnostic] = report.diagnostics.as_slice() else {
        panic!("{report:#?}")
    };
    assert_eq!(diagnostic.code, "PNX2322");
    assert_eq!(diagnostic.expected.as_deref(), Some(":later"));
    assert_eq!(diagnostic.actual.as_deref(), Some(":earlier | :later"));
    assert!(diagnostic.message.contains("demo/inlined"));
}

#[test]
fn an_opaque_macro_shadowing_get_in_has_no_standard_lookup_proof() {
    let source = r#"(ns demo)
        (defmacro get-in [& _] :real)
        (defn opaque {:t/sig '[:-> #{:real}]} [] (get-in {} [:missing] :fallback))
        (defn explicit {:t/sig '[:-> #{:fallback}]} [] (clojure.core/get-in {} [:missing] :fallback))"#;
    let report = Session::default().check("lookup.clj", source);
    let [declaration, use_site] = report.diagnostics.as_slice() else {
        panic!("{report:#?}")
    };
    assert_eq!(declaration.code, "PNX9001");
    assert_eq!(use_site.code, "PNX9001");
    assert_eq!(
        &source[use_site.span.start.offset..use_site.span.end.offset],
        "get-in"
    );
}

#[test]
fn a_shadowed_get_in_cannot_narrow_the_map_in_a_branch() {
    let source = r#"(ns demo (:require [clojure.spec.alpha :as s]))
        (defmacro get-in [& _] true)
        (defn read-path {:t/sig '[{:outer {:leaf (s/nilable string?)}} :-> string?]} [m]
          (if (get-in m [:outer :leaf])
            (let [leaf (:leaf (:outer m))] (subs leaf 0))
            "fallback"))"#;
    let mut session = Session::default();
    let report = session.check("lookup.clj", source);
    let mismatch = report
        .diagnostics
        .iter()
        .find(|finding| finding.code == "PNX2322")
        .unwrap();
    assert_eq!(mismatch.actual.as_deref(), Some("Nil | String"));
    let offset = source.rfind("leaf 0").unwrap();
    let Some(DocumentHover::Local(local)) = session.document_hover_at("lookup.clj", offset) else {
        panic!("expected a type for the leaf read");
    };
    assert_eq!(local.inferred_type, "Nil | String");
}

#[test]
fn a_source_macro_shadows_core_lookups_after_its_definition() {
    let source = r#"(ns demo (:require [clojure.spec.alpha :as s]))
        (defn early {:t/sig '[:-> #{:present}]} [] (get {:path :present} :path))
        (defmacro get [& _] true)
        (defn late {:t/sig '[{:path (s/nilable string?)} :-> string?]} [m]
          (if (get m :path) (subs (:path m) 0) "fallback"))
        (defn qualified {:t/sig '[:-> #{:present}]} [] (clojure.core/get {:path :present} :path))"#;
    let mut session = Session::default();
    let report = session.check("lookup.clj", source);
    let [declaration, macro_use, mismatch] = report.diagnostics.as_slice() else {
        panic!("{report:#?}")
    };
    assert_eq!(declaration.code, "PNX9001");
    assert_eq!(macro_use.code, "PNX9001");
    assert_eq!(mismatch.code, "PNX2322");
    assert_eq!(mismatch.actual.as_deref(), Some("Nil | String"));
    let opaque = source.find("get m :path").unwrap() + "get m ".len();
    assert!(session.document_hover_at("lookup.clj", opaque).is_none());
    let early = source.find(":present} :path").unwrap() + ":present} ".len();
    let Some(DocumentHover::Lookup { inferred_type, .. }) =
        session.document_hover_at("lookup.clj", early)
    else {
        panic!("expected a type before the macro declaration");
    };
    assert_eq!(inferred_type, ":present");
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
