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
        ("((fn [x & {:keys [y]}] y) 1 :y 2)", "PNX9001"),
        ("((fn [x & more] (recur x)) 1)", "PNX2554"),
    ] {
        let source = format!("(ns demo) (defn f {{:t/sig '[:-> number?]}} [] {expr})");
        let report = Session::default().check("variadic.clj", &source);
        assert_eq!(report.diagnostics.len(), 1, "{expr}: {report:#?}");
        assert_eq!(report.diagnostics[0].code, code);
    }
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
