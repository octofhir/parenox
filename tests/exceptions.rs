use parenox::{Report, Session};

fn check(expression: &str, expected: &str) -> Report {
    let source = format!("(ns demo) (defn f {{:t/sig '[:-> {expected}]}} [] {expression})");
    Session::default().check("exceptions.clj", &source)
}

#[test]
fn equality_and_case_check_handlers_when_jvm_dispatch_can_throw() {
    for (input, expression, reaches_handler) in [
        ("t/any", r#"(= value "tag")"#, true),
        ("t/any", r#"(= "tag" value)"#, true),
        ("t/any", "(= nil value)", false),
        ("t/any", "(= value)", false),
        ("t/any", "(= 1 2 value)", false),
        ("string?", r#"(= value "tag")"#, false),
        ("t/any", r#"(case value "tag" :hit :miss)"#, true),
        ("number?", "(case value 42 :hit :miss)", true),
        ("t/any", "(case value :fallback)", false),
        ("string?", r#"(case value "tag" :hit :miss)"#, false),
    ] {
        let source = format!(
            r#"(ns demo)
               (defn f {{:t/sig '[{input} :-> t/any]}} [value]
                 (try {expression}
                   (catch clojure.lang.ExceptionInfo _ (inc "wrong"))))"#
        );
        let report = Session::default().check("exceptions.clj", &source);
        assert!(report.complete, "{expression}: {report:#?}");
        if reaches_handler {
            let [finding] = report.diagnostics.as_slice() else {
                panic!("{expression}: {report:#?}");
            };
            assert_eq!(finding.code, "PNX2322");
            assert_eq!(
                &source[finding.span.start.offset..finding.span.end.offset],
                "\"wrong\""
            );
        } else {
            assert!(report.ok && report.diagnostics.is_empty(), "{report:#?}");
        }
    }
}

#[test]
fn a_literal_nil_call_operator_is_rejected_before_its_operands() {
    let source = "(ns demo) (defn bad {:t/sig '[:-> t/any]} [] (nil missing-name))";
    let report = Session::default().check("exceptions.clj", source);
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(finding.code, "PNX1001");
    assert_eq!(
        &source[finding.span.start.offset..finding.span.end.offset],
        "nil"
    );
}

#[test]
fn calling_a_value_known_to_be_nil_throws_only_when_invoked() {
    for (expression, target) in [
        ("(let [callback nil] (callback 1))", "callback"),
        ("((quote nil) 1)", "(quote nil)"),
        ("((if false inc nil) 1)", "nil"),
        ("(apply nil [1])", "nil"),
        ("(mapv nil [1])", "nil"),
    ] {
        let source = format!(
            r#"(ns demo) (defn f {{:t/sig '[:-> #{{:caught}}]}} []
                 (try {expression}
                   (catch IllegalArgumentException _ (inc "unreachable"))
                   (catch NullPointerException _ :caught)))"#,
        );
        let report = Session::default().check("exceptions.clj", &source);
        let [finding] = report.diagnostics.as_slice() else {
            panic!("{report:#?}");
        };
        assert!(report.complete);
        assert_eq!(finding.code, "PNX2322");
        assert_eq!(finding.expected.as_deref(), Some("Callable"));
        assert_eq!(finding.actual.as_deref(), Some("Nil"));
        assert_eq!(
            &source[finding.span.start.offset..finding.span.end.offset],
            target
        );
    }

    let not_invoked = check("(empty? (mapv nil []))", "#{true}");
    assert!(not_invoked.ok && not_invoked.complete, "{not_invoked:#?}");

    let blocked = check(
        r#"(try
             (let [callback nil] (callback (throw (ex-info "operand" {}))))
             (catch NullPointerException _ (inc "unreachable"))
             (catch clojure.lang.ExceptionInfo _ :operand))"#,
        "#{:operand}",
    );
    assert!(blocked.ok && blocked.complete, "{blocked:#?}");
}

#[test]
fn direct_calls_cast_known_scalar_targets_before_evaluating_arguments() {
    let operand = r#"(throw (ex-info "argument" {}))"#;
    for operator in [
        "123",
        "123N",
        r#"#"tag""#,
        "(if true 123 inc)",
        "(let [local 123] local)",
    ] {
        let expression = format!(
            "(try ({operator} {operand})
               (catch clojure.lang.ExceptionInfo _ :argument)
               (catch ClassCastException _ :target))",
        );
        let report = check(&expression, "#{:target}");
        let [finding] = report.diagnostics.as_slice() else {
            panic!("{operator}: {report:#?}");
        };
        assert!(report.complete);
        assert_eq!(finding.code, "PNX2322");
        assert_eq!(finding.expected.as_deref(), Some("Callable"));
    }

    let still_compiles_arguments = check("(123 missing-name)", "t/any");
    assert!(
        still_compiles_arguments.diagnostics.iter().any(|finding| {
            finding.code == "PNX2304" && finding.message.contains("missing-name")
        })
    );

    let broad_number = "(ns demo) (defn f {:t/sig '[number? :-> t/any]} [target] (target 1))";
    let report = Session::default().check("exceptions.clj", broad_number);
    let [gap] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(gap.code, "PNX9001");
}

#[test]
fn collection_calls_evaluate_arguments_before_casting_the_callback() {
    let blocked = r#"(throw (ex-info "argument" {}))"#;
    for expression in [
        format!("(apply 123 [{blocked}])"),
        format!("(mapv 123 [{blocked}])"),
    ] {
        let report = check(
            &format!(
                "(try {expression}
                   (catch clojure.lang.ExceptionInfo _ :argument)
                   (catch ClassCastException _ :target))",
            ),
            "#{:argument}",
        );
        assert!(report.ok && report.complete, "{expression}: {report:#?}");
    }
    for expression in ["(apply 123 [1])", "(mapv 123 [1])"] {
        let report = check(
            &format!("(try {expression} (catch ClassCastException _ :target))"),
            "#{:target}",
        );
        let [finding] = report.diagnostics.as_slice() else {
            panic!("{expression}: {report:#?}");
        };
        assert!(report.complete);
        assert_eq!(finding.code, "PNX2322");
        assert_eq!(finding.expected.as_deref(), Some("Callable"));
    }
    let empty = check("(empty? (mapv 123 []))", "#{true}");
    assert!(empty.ok && empty.complete, "{empty:#?}");
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
fn evaluated_set_collisions_and_member_failures_stop_construction() {
    let source = r#"
(defn duplicate {:t/sig '[:-> #{:caught}]} []
  (let [member :ready]
    (try
      (do #{member :ready} (inc "dead"))
      (catch IllegalArgumentException _ :caught))))
(defn member-failed {:t/sig '[:-> #{:caught}]} []
  (let [member :ready]
    (try
      (do #{member :ready (throw nil)} (inc "dead"))
      (catch NullPointerException _ :caught))))
"#;
    let report = Session::default().check("exceptions.clj", source);
    let [error] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(error.code, "PNX2790");
    assert_eq!(error.span.start.offset, source.find(":ready}").unwrap());
    assert_eq!(
        &source[error.span.start.offset..error.span.end.offset],
        ":ready"
    );

    let valid = check("(let [member :ready] #{member :pending})", "t/any");
    assert!(valid.ok && valid.complete, "{valid:#?}");
    let rejected = check("#{(inc \"invalid\")}", "string?");
    assert_eq!(rejected.diagnostics.len(), 1, "{rejected:#?}");
    assert_eq!(rejected.diagnostics[0].code, "PNX2322");
}

#[test]
fn nonreturning_arguments_keep_compiler_checks_in_later_forms() {
    for expression in [
        "(throw nil)",
        "(+ 1 (throw nil) (inc \"dead\"))",
        "(do (throw nil) (name))",
        "(name (throw nil) :extra)",
        "(do (throw nil) (apply count []))",
        "(do (throw nil) (:label))",
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
        ("(do (throw nil) (count))", "PNX2554"),
        (
            "(try (count) (catch clojure.lang.ArityException _ :caught))",
            "PNX2554",
        ),
        (
            "(do (throw nil) (clojure.lang.MapEntry/create :key))",
            "PNX2554",
        ),
        ("(apply count [])", "PNX2554"),
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

#[test]
fn a_known_function_reports_wrong_arity_only_when_invoked() {
    let source = r#"(ns demo)
(defn typed {:t/sig '[number? :-> number?]} [n] n)
(defn dead {:t/sig '[:-> number?]} [] (do (throw nil) (typed)))
(defn reached {:t/sig '[:-> number?]} [] (typed))
"#;
    let report = Session::default().check("exceptions.clj", source);
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert!(report.complete, "{report:#?}");
    assert_eq!(finding.code, "PNX2554");
    assert_eq!(finding.span.start.line, 4);
    assert_eq!(
        &source[finding.span.start.offset..finding.span.end.offset],
        "(typed)"
    );
}

#[test]
fn inferred_helpers_reject_arity_only_when_reached_and_raise_only_arity_errors() {
    let source = r#"(ns demo)
(defn- helper [n] n)
(defn inferred {:t/sig '[:-> number?]} [] (helper 7))
(defn dead {:t/sig '[:-> number?]} [] (do (throw nil) (helper)))
(defn eager {:t/sig '[:-> number?]} [] (helper (throw nil) unresolved))
(defn caught {:t/sig '[:-> #{:arity}]} []
  (try (helper)
       (catch NullPointerException _ :wrong)
       (catch clojure.lang.ArityException _ :arity)))
(defn stops {:t/sig '[:-> number?]} [] (do (helper) (inc "unreached")))
"#;
    let report = Session::default().check("exceptions.clj", source);
    let findings: Vec<_> = report
        .diagnostics
        .iter()
        .map(|d| {
            (
                d.code.as_str(),
                &source[d.span.start.offset..d.span.end.offset],
            )
        })
        .collect();
    assert!(report.complete, "{report:#?}");
    assert_eq!(
        findings,
        [
            ("PNX2304", "unresolved"),
            ("PNX2554", "(helper)"),
            ("PNX2554", "(helper)"),
        ],
        "{report:#?}"
    );
}

#[test]
fn unreached_function_literals_keep_their_lexical_compiler_checks() {
    for expression in [
        "(do (throw nil) {:run (fn again [x] (again x))})",
        "(do (throw nil) {:run #(reduce)})",
        "(do (throw nil) {:run #(inc %)})",
        "(do (throw nil) {:run (fn* [x] x)})",
        "(do (throw nil) (reduce))",
    ] {
        let report = check(expression, "nil?");
        assert!(report.ok && report.complete, "{expression}: {report:#?}");
    }

    let declared = Session::default().check(
        "closures.clj",
        "(defn- helper [] :ok) (defn f {:t/sig '[:-> nil?]} []
           (do (throw nil) (apply helper [])))",
    );
    assert!(declared.ok && declared.complete, "{declared:#?}");

    for (literal, operand, code) in [
        ("(fn [x] (missing x))", "missing", "PNX2304"),
        ("#(missing %)", "missing", "PNX2304"),
        ("(fn* [x] (missing x))", "missing", "PNX2304"),
        ("(fn [] (count))", "(count)", "PNX2554"),
        ("(fn [] (reduce missing))", "missing", "PNX2304"),
    ] {
        let source =
            format!("(defn f {{:t/sig '[:-> nil?]}} [] (do (throw nil) {{:run {literal}}}))");
        let report = Session::default().check("closures.clj", &source);
        let [diagnostic] = report.diagnostics.as_slice() else {
            panic!("{literal}: {report:#?}");
        };
        assert_eq!(diagnostic.code, code);
        assert_eq!(
            &source[diagnostic.span.start.offset..diagnostic.span.end.offset],
            operand
        );
    }
}

#[test]
fn function_literals_used_as_data_check_names_before_reporting_type_gaps() {
    for (literal, operand, code) in [
        ("(fn [x] (missing x))", "missing", "PNX2304"),
        ("#(missing %)", "missing", "PNX2304"),
        ("(fn [] (count))", "(count)", "PNX2554"),
        ("(fn [] (get))", "(fn [] (get))", "PNX9001"),
        ("#(inc \"text\")", "#(inc \"text\")", "PNX9001"),
    ] {
        let source = format!("(defn f {{:t/sig '[:-> map?]}} [] {{:run {literal}}})");
        let report = Session::default().check("closures.clj", &source);
        let [diagnostic] = report.diagnostics.as_slice() else {
            panic!("{literal}: {report:#?}");
        };
        assert_eq!(diagnostic.code, code, "{report:#?}");
        assert_eq!(
            &source[diagnostic.span.start.offset..diagnostic.span.end.offset],
            operand
        );
    }
}
