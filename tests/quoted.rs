use parenox::{Session, Source};

#[test]
fn checks_quoted_data_as_values_without_resolving_or_calling_their_symbols() {
    let report = Session::default().check("quoted.clj", include_str!("../examples/quoted.clj"));
    assert!(report.ok && report.complete, "{report:#?}");
    for (body, ret) in [
        ("(first '(missing definitely-missing))", "symbol?"),
        ("(nth '(missing \"literal\") 1)", "#{\"literal\"}"),
        ("(first ''command)", "symbol?"),
        ("(first (quote (other.ns/unresolved)))", "symbol?"),
        ("(if (symbol? 'flag) 7 (inc \"skipped\"))", "number?"),
        ("(first (:names '(:names (\"Ada\") :values [])))", "nil?"),
    ] {
        let source = format!("(ns demo) (defn f {{:t/sig '[:-> {ret}]}} [] {body})");
        let report = Session::default().check("quoted.clj", &source);
        assert!(report.ok && report.complete, "{body}: {report:#?}");
    }

    let bad =
        Session::default().check("quoted_bad.clj", include_str!("../examples/quoted_bad.clj"));
    let [diagnostic] = bad.diagnostics.as_slice() else {
        panic!("{bad:#?}")
    };
    assert_eq!(diagnostic.code, "PNX2322");
    assert_eq!(diagnostic.actual.as_deref(), Some("String"));
}

#[test]
fn quotes_do_not_create_a_cycle_or_mark_an_inferred_helper_as_called() {
    let app = Source {
        path: "alpha.clj",
        source: "(ns alpha) (defn action {:t/sig '[:-> symbol?]} [] 'beta/run)",
    };
    let provider = Source {
        path: "beta.clj",
        source: "(ns beta (:require alpha)) (defn run {:t/sig '[:-> symbol?]} [] (alpha/action))",
    };
    let report = Session::default().check_project(&[provider, app], None);
    assert!(report.ok && report.complete, "{report:#?}");

    let report = Session::default().check(
        "quoted.clj",
        "(ns demo) (defn- helper [] 1)
                   (defn f {:t/sig '[:-> symbol?]} [] 'helper)",
    );
    let [diagnostic] = report.diagnostics.as_slice() else {
        panic!("{report:#?}")
    };
    assert_eq!(diagnostic.code, "PNX9002");
}

#[test]
fn quoted_map_data_keeps_fixed_fields_alongside_other_keys() {
    let source = r#"
(ns demo)
(def bindings '{local :payload [head & tail] :remaining :version 1})
(defn version {:t/sig '[:-> number?]} [] (:version bindings))
(defn priority {:t/sig '[:-> string?]} [] (get '{2 "high"} 2 "default"))
"#;
    let report = Session::default().check("quoted.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");

    let bad = source.replace("(:version bindings)", "(inc (:missing bindings))");
    let report = Session::default().check("quoted.clj", &bad);
    let [diagnostic] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(diagnostic.actual.as_deref(), Some("Nil"));
}

#[test]
fn an_unmodeled_evaluated_key_does_not_prove_absence() {
    let source = "(ns demo)
        (defn read-id {:t/sig '[keyword? :-> number?]} [key]
          (inc (:id {key 42})))";
    let report = Session::default().check("quoted.clj", source);
    assert!(!report.complete, "{report:#?}");
    assert_eq!(report.diagnostics.len(), 1, "{report:#?}");
    assert_eq!(report.diagnostics[0].severity, "warning");
}
