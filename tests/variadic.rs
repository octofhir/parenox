use parenox::{Session, Source};

#[test]
fn rest_contracts_check_each_argument_and_select_fixed_clauses_first() {
    let provider = Source {
        path: "variadic.clj",
        source: include_str!("../examples/variadic.clj"),
    };
    let good = Session::default().check(provider.path, provider.source);
    assert!(good.ok && good.complete, "{good:#?}");
    let consumer = Source {
        path: "variadic_bad.clj",
        source: include_str!("../examples/variadic_bad.clj"),
    };
    let wrong = Session::default().check_project(&[consumer, provider], None);
    let [element, arity] = wrong.files[0].diagnostics.as_slice() else {
        panic!("{wrong:#?}")
    };
    assert_eq!(element.code, "PNX2322");
    assert_eq!(element.message, "Argument 2: expected Number, got String");
    assert_eq!(
        &consumer.source[element.span.start.offset..element.span.end.offset],
        "\"bad\""
    );
    assert_eq!(arity.code, "PNX2554");
    assert!(arity.message.contains("at least 1"), "{arity:#?}");
}

#[test]
fn inline_rest_is_nil_when_empty_and_recur_receives_one_rest_slot() {
    for (expr, result) in [
        ("((fn [n & more] more) 1)", "nil?"),
        ("((fn ([n] n) ([n & more] \"varargs\")) 1)", "number?"),
        (
            "((fn ([n] n) ([n & more] (first more))) 1 \"last\")",
            "string?",
        ),
        (
            "((fn [go & more] (if go (recur false [\"replacement\"]) (first more))) true)",
            "string?",
        ),
    ] {
        let source = format!("(ns demo) (defn f {{:t/sig '[:-> {result}]}} [] {expr})");
        let report = Session::default().check("variadic.clj", &source);
        assert!(report.ok && report.complete, "{expr}: {report:#?}");
    }
    for (expr, code) in [
        ("((fn ([x y] x) ([x & xs] x)) 1)", "PNX2554"),
        ("((fn ([x & more] x) ([x y & more] x)) 1)", "PNX2554"),
        ("((fn [x & more extra] x) 1)", "PNX1001"),
        ("((fn [x & more] (recur x)) 1)", "PNX2554"),
    ] {
        let source = format!("(ns demo) (defn f {{:t/sig '[:-> number?]}} [] {expr})");
        let report = Session::default().check("variadic.clj", &source);
        assert_eq!(report.diagnostics.len(), 1, "{expr}: {report:#?}");
        assert_eq!(report.diagnostics[0].code, code);
    }
}

#[test]
fn variadic_map_bindings_read_pairs_and_a_trailing_options_map() {
    let source = r#"
(ns demo)
(defn- size [& {:keys [n] :or {n :default}}] n)
(defn empty-options {:t/sig '[:-> #{:default}]} [] (size))
(defn keyword-pairs {:t/sig '[:-> #{:pair}]} [] (size :n :pair))
(defn single-map {:t/sig '[:-> #{:single}]} [] (size {:n :single}))
(defn trailing-map {:t/sig '[:-> #{:override}]} [] (size :n :pair {:n :override}))
(defn via-apply {:t/sig '[:-> #{:spread}]} []
  (apply (fn [& {:as opts}] (:n opts)) [:ignored 1 {:n :spread}]))
"#;
    let report = Session::default().check("variadic.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");

    let bad = source.replace("(size :n :pair)", "(size :n :wrong)");
    let report = Session::default().check("variadic.clj", &bad);
    let [error] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(error.code, "PNX2322");
    assert_eq!(error.actual.as_deref(), Some(":wrong"));
}

#[test]
fn variadic_map_binding_reports_a_gap_for_unknown_rest_positions() {
    let source = r#"
(defn options {:t/sig '[& t/any :-> t/any]} [& {:as opts}] opts)
"#;
    let report = Session::default().check("variadic.clj", source);
    assert!(!report.complete, "{report:#?}");
    assert!(report.diagnostics.iter().any(|finding| {
        finding.code == "PNX9001"
            && finding
                .message
                .contains("requires fixed sequence positions")
    }));
}

#[test]
fn extern_variadic_contracts_validate_the_rest_arguments() {
    let externs = Source {
        path: "externs.edn",
        source: "{db/labels [string? & string? :-> number?]}",
    };
    let app = Source {
        path: "app.clj",
        source: "(ns app) (defn read-all {:t/sig '[:-> number?]} [] (db/labels \"A\" \"B\" 3))",
    };
    let report = Session::default().check_project(&[app], Some(externs));
    assert!(report.complete, "{report:#?}");
    assert_eq!(report.files[0].diagnostics.len(), 1);
    assert_eq!(
        report.files[0].diagnostics[0].message,
        "Argument 3: expected String, got Number"
    );
}
