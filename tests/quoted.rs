use parenox::{DocumentHover, Session, Source};

#[test]
fn checks_quoted_data_as_values_without_resolving_or_calling_their_symbols() {
    let report = Session::default().check("quoted.clj", include_str!("../examples/quoted.clj"));
    assert!(report.ok && report.complete, "{report:#?}");
    for (body, ret) in [
        ("(first '(missing definitely-missing))", "symbol?"),
        ("(nth '(missing \"literal\") 1)", "#{\"literal\"}"),
        ("(first ''command)", "symbol?"),
        ("(first (quote (other.ns/unresolved)))", "symbol?"),
        (
            "(= 'other.ns/unresolved '^:tag other.ns/unresolved)",
            "#{true}",
        ),
        ("(= 'sample.source 'sample.dest)", "#{false}"),
        ("(name 'clojure.core//)", "#{\"/\"}"),
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
fn symbol_qualification_is_checked_in_literals_guards_and_externs() {
    let library = Source {
        path: "routes.clj",
        source: r#"(ns routes (:require [clojure.spec.alpha :as s]))
            (defn keep-simple [name] name)
            (s/fdef keep-simple :args (s/cat :name simple-symbol?) :ret simple-symbol?)
            (defn keep-qualified {:t/sig '[qualified-symbol? :-> qualified-symbol?]} [name] name)
            (defn resolve-name {:t/sig '[symbol? :-> symbol?]} [name]
              (if (simple-symbol? name) (keep-simple name) (keep-qualified name)))
            (defn divide {:t/sig '[:-> simple-symbol?]} [] (keep-simple '/))
            (defn divide-core {:t/sig '[:-> qualified-symbol?]} [] (keep-qualified 'clojure.core//))"#,
    };
    let mut session = Session::default();
    let exported = session.export_externs(&[library], &[]);
    let externs = exported.edn.expect("expected verified externs");
    assert!(externs.contains("clojure.core/simple-symbol?"));
    assert!(externs.contains("clojure.core/qualified-symbol?"));
    let bad = library
        .source
        .replace("(keep-simple '/)", "(keep-simple 'routes/name)");
    let report = session.check(library.path, &bad);
    let [diagnostic] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(diagnostic.code, "PNX2322");
    assert_eq!(diagnostic.expected.as_deref(), Some("SimpleSymbol"));
    assert_eq!(diagnostic.actual.as_deref(), Some("QualifiedSymbol"));
}

#[test]
fn exact_symbols_export_and_check_a_consumer_without_the_provider_source() {
    let provider = Source {
        path: "symbols.clj",
        source: r#"
(ns symbols (:require [clojure.spec.alpha :as s]))
(s/fdef accept :args (s/cat :operator #{'get-in}) :ret #{'get-in})
(defn accept [operator] operator)
(def get-op 'get-in)
"#,
    };
    let mut session = Session::default();
    let exported = session.export_externs(&[provider], &[]);
    let edn = exported.edn.expect("expected verified symbol contracts");
    assert!(edn.contains("#{(quote get-in)}"), "{edn}");
    let externs = Source {
        path: "symbols.edn",
        source: &edn,
    };
    let consumer = r#"
(ns consumer (:require [symbols :as ops]))
(defn read-op {:t/sig '[:-> #{get-in}]} [] (ops/accept ops/get-op))
"#;
    let good = session.check_project(
        &[Source {
            path: "consumer.clj",
            source: consumer,
        }],
        Some(externs),
    );
    assert!(good.ok && good.complete, "{good:#?}");

    let changed = consumer.replace("ops/get-op", "'get");
    let bad = session.check_project(
        &[Source {
            path: "consumer.clj",
            source: &changed,
        }],
        Some(externs),
    );
    let [finding] = bad.files[0].diagnostics.as_slice() else {
        panic!("{bad:#?}");
    };
    assert_eq!(finding.code, "PNX2322");
    assert_eq!(finding.expected.as_deref(), Some("'get-in"));
    assert_eq!(finding.actual.as_deref(), Some("'get"));
    let span = finding.span.start.offset..finding.span.end.offset;
    assert_eq!(&changed[span], "'get");
}

#[test]
fn large_quoted_collections_group_nested_symbol_keys_by_their_class() {
    let rows = (0..33)
        .map(|index| format!("{{leaf{index} :ok}}"))
        .collect::<Vec<_>>()
        .join(" ");
    let source = format!(
        "(ns inventory)
         (defn row {{:t/sig '[:-> any?]}} []
           (let [sample (first '({rows}))] sample))"
    );
    let mut session = Session::default();
    let report = session.check("inventory.clj", &source);
    assert!(report.ok && report.complete, "{report:#?}");
    let offset = source.find("sample (first").unwrap();
    let Some(DocumentHover::Local(value)) = session.document_hover_at("inventory.clj", offset)
    else {
        panic!("expected hover on the quoted row");
    };
    assert_eq!(value.inferred_type, "Map<SimpleSymbol, :ok>");
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
        "(ns demo) (defn- helper [] (inc 0))
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
