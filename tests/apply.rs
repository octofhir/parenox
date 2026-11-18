use parenox::{Session, Source};

#[test]
fn spreads_nil_vectors_and_exact_seqs_across_project_namespaces() {
    let input = [
        Source {
            path: "apply.clj",
            source: include_str!("../examples/apply.clj"),
        },
        Source {
            path: "provider.clj",
            source: include_str!("../examples/arities/provider.clj"),
        },
        Source {
            path: "variadic.clj",
            source: include_str!("../examples/variadic.clj"),
        },
        Source {
            path: "apply_bad.clj",
            source: include_str!("../examples/apply_bad.clj"),
        },
    ];
    let report = Session::default().check_project(&input, None);
    assert!(report.complete, "{report:#?}");
    assert!(report.files[..3].iter().all(|file| file.ok), "{report:#?}");
    let [element, arity] = report.files[3].diagnostics.as_slice() else {
        panic!("{report:#?}")
    };
    assert_eq!(element.code, "PNX2322");
    assert_eq!(
        &input[3].source[element.span.start.offset..element.span.end.offset],
        "\"3\""
    );
    assert_eq!(element.suggestions[0].replacement, "3");
    assert_eq!(arity.code, "PNX2554");
    assert!(arity.message.contains("0 or 1"), "{arity:#?}");
}

#[test]
fn checks_each_possible_exact_shape_and_protects_projected_spans() {
    let source = r#"(ns demo)
        (defn f {:t/sig '[boolean? :-> number?]} [missing?]
          (let [args (if missing? [] ["3"])]
            (apply inc args)))"#;
    let report = Session::default().check("apply.clj", source);
    assert!(report.complete, "{report:#?}");
    let [arity, element] = report.diagnostics.as_slice() else {
        panic!("{report:#?}")
    };
    assert_eq!(arity.code, "PNX2554");
    assert_eq!(element.code, "PNX2322");
    assert!(element.suggestions.is_empty(), "{element:#?}");
    assert!(element.help.as_deref().unwrap().contains("elements"));
}

#[test]
fn diagnoses_nonseqable_inputs_and_marks_open_spread_as_incomplete() {
    let source = r#"(ns demo)
        (defn bad {:t/sig '[:-> number?]} []
          (apply + true))
        (defn open {:t/sig '[(clojure.spec.alpha/coll-of number? :kind vector?) :-> number?]} [xs]
          (apply (fn [& values] (first values)) xs))"#;
    let report = Session::default().check("apply.clj", source);
    assert!(!report.complete, "{report:#?}");
    let [nonseqable, open] = report.diagnostics.as_slice() else {
        panic!("{report:#?}")
    };
    assert_eq!(nonseqable.code, "PNX2322");
    assert_eq!(open.code, "PNX9001");
    assert!(open.message.contains("cannot infer every argument count"));
}

#[test]
fn resolves_apply_and_evaluates_the_target_before_other_inputs() {
    let source = r#"(ns demo)
        (defn f {:t/sig '[:-> #{:target}]} []
          (try
            (apply (throw (ex-info "target" {:which :target}))
                   (throw (ex-info "prefix" {:which :prefix}))
                   (throw (ex-info "spread" {:which :spread})))
            (catch clojure.lang.ExceptionInfo e (:which (ex-data e)))))
        (defn g {:t/sig '[:-> number?]} []
          (let [apply (fn [a b] (+ a b))]
            (apply 1 2)))"#;
    let report = Session::default().check("apply.clj", source);
    // Inline fn as a stored value is a known gap. A local variable still shadows core apply.
    assert_eq!(report.diagnostics.len(), 1, "{report:#?}");
    assert_eq!(report.diagnostics[0].code, "PNX9001");
    assert!(report.diagnostics[0].message.starts_with("fn is currently"));
}

#[test]
fn forwards_typed_rest_arguments_and_checks_every_reachable_overload() {
    let source = r#"
        (ns demo (:require [clojure.spec.alpha :as s]))
        (defn sum {:t/sig '[number? & number? :-> number?]} [base & more]
          (apply + base more))
        (defn total {:t/sig '[(s/coll-of number? :kind vector?) :-> number?]} [values]
          (apply sum 0 values))
        (defn describe {:t/sig '[[number? :-> string?] [number? & number? :-> number?]]}
          ([base] (str base))
          ([base & more] (apply + base more)))
        (defn from-nonempty {:t/sig '[(s/coll-of number? :kind vector?)
                                     :-> (s/nilable (s/or :label string? :total number?))]}
          [values]
          (when-let [items (seq values)] (apply describe items)))
    "#;
    let report = Session::default().check("forward.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");

    let app = r#"(ns app (:require [clojure.spec.alpha :as s]))
        (defn total {:t/sig '[(s/coll-of string? :kind vector?) :-> number?]}
          [values] (apply db/sum 0 values))"#;
    let report = Session::default().check_project(
        &[Source {
            path: "app.clj",
            source: app,
        }],
        Some(Source {
            path: "externs.edn",
            source: "{db/sum [number? & number? :-> number?]}",
        }),
    );
    assert!(report.complete, "{report:#?}");
    let [diagnostic] = report.files[0].diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(diagnostic.code, "PNX2322");
    assert_eq!(
        diagnostic.message,
        "apply collection elements: expected Number, got String"
    );
    assert_eq!(
        &app[diagnostic.span.start.offset..diagnostic.span.end.offset],
        "values"
    );
    assert!(diagnostic.suggestions.is_empty());
}

#[test]
fn open_spread_reports_a_possible_arity_and_exact_literals_stay_exact() {
    let source = r#"(ns demo (:require [clojure.spec.alpha :as s]))
        (defn fixed {:t/sig '[number? :-> number?]} [x] x)
        (defn maybe {:t/sig '[(s/coll-of number? :kind vector?) :-> number?]} [values]
          (apply fixed values))
        (defn nonempty {:t/sig '[(s/coll-of number? :kind vector?) :-> number?]} [values]
          (if-let [items (seq values)] (apply fixed items) 0))
        (defn minus {:t/sig '[(s/coll-of number? :kind vector?) :-> number?]} [values]
          (apply - values))"#;
    let report = Session::default().check("arity.clj", source);
    assert!(report.complete, "{report:#?}");
    let counts: Vec<_> = report
        .diagnostics
        .iter()
        .map(|d| (d.code.as_str(), d.message.as_str()))
        .collect();
    assert_eq!(
        counts,
        [
            (
                "PNX2554",
                "apply may pass 0 arguments, but the function expects 1"
            ),
            (
                "PNX2554",
                "apply may pass 2 arguments, but the function expects 1"
            ),
            (
                "PNX2554",
                "apply may pass 0 arguments, but the function expects at least 1"
            ),
        ]
    );

    let source = format!(
        "(ns demo) (defn f {{:t/sig '[:-> number?]}} []
           (apply + [{} \"3\"]))",
        "1 ".repeat(40)
    );
    let report = Session::default().check("literal.clj", &source);
    let [diagnostic] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(diagnostic.code, "PNX2322");
    assert_eq!(
        &source[diagnostic.span.start.offset..diagnostic.span.end.offset],
        "\"3\""
    );
    assert_eq!(diagnostic.suggestions[0].replacement, "3");
}
