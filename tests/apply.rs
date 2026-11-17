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
          (apply + xs))"#;
    let report = Session::default().check("apply.clj", source);
    assert!(!report.complete, "{report:#?}");
    let [nonseqable, open] = report.diagnostics.as_slice() else {
        panic!("{report:#?}")
    };
    assert_eq!(nonseqable.code, "PNX2322");
    assert_eq!(open.code, "PNX9001");
    assert!(open.message.contains("known argument count"));
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
