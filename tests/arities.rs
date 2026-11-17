use parenox::{Session, Source};

const PROVIDER: Source<'_> = Source {
    path: "provider.clj",
    source: include_str!("../examples/arities/provider.clj"),
};

const APP: Source<'_> = Source {
    path: "app.clj",
    source: include_str!("../examples/arities/app.clj"),
};

#[test]
fn typed_overloads_use_the_selected_contract_and_recur_arity() {
    let mut session = Session::default();
    let project = session.check_project(&[APP, PROVIDER], None);
    assert!(project.ok && project.complete, "{project:#?}");
    assert_eq!(project.stats.checked_functions, 5);
    let warm = session.check_project(&[APP, PROVIDER], None);
    assert_eq!(warm.stats.reused_functions, 5);

    let bad = Source {
        path: "bad.clj",
        source: include_str!("../examples/arities/bad.clj"),
    };
    let report = session.check_project(&[bad, PROVIDER], None);
    let [length, phase] = report.files[0].diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(length.code, "PNX2554");
    assert_eq!(length.message, "Expected 1 or 2 arguments, got 3");
    assert_eq!(phase.code, "PNX2322");
    assert_eq!(
        &bad.source[phase.span.start.offset..phase.span.end.offset],
        ":other"
    );
}

#[test]
fn updating_one_arity_rechecks_a_previous_caller() {
    let mut session = Session::default();
    assert!(session.check_project(&[APP, PROVIDER], None).ok);
    let edited = PROVIDER
        .source
        .replace("[string? :-> number?]", "[number? :-> number?]")
        .replace("([label] (count label))", "([label] (inc label))");
    let report = session.check_project(
        &[
            APP,
            Source {
                source: &edited,
                ..PROVIDER
            },
        ],
        None,
    );
    assert!(!report.ok && report.complete, "{report:#?}");
    assert_eq!(report.stats.parsed_files, 1);
    let [diagnostic] = report.files[0].diagnostics.as_slice() else {
        panic!("{report:#?}")
    };
    assert_eq!(diagnostic.expected.as_deref(), Some("Number"));
    assert_eq!(diagnostic.actual.as_deref(), Some("String"));
}

#[test]
fn extern_overloads_support_multiple_signatures() {
    let app = Source {
        path: "app.clj",
        source: "(ns app) (defn f {:t/sig '[:-> number?]} [] (rates/latest 1))",
    };
    let external = Source {
        path: "externs.edn",
        source: "{rates/latest [[:-> string?] [number? :-> number?]]}",
    };
    let report = Session::default().check_project(&[app], Some(external));
    assert!(report.complete && report.ok, "{report:#?}");
}

#[test]
fn invalid_overloads_have_precise_declaration_diagnostics() {
    for (definition, code, text) in [
        (
            "(defn f {:t/sig '[[number? :-> number?] [number? number? :-> number?]]} ([x] x) ([x] x))",
            "PNX2554",
            "Duplicate function arity 1",
        ),
        (
            "(defn f {:t/sig '[[number? :-> number?] [string? :-> string?]]} ([x] x))",
            "PNX2554",
            "Duplicate signature arity 1",
        ),
        (
            "(defn f {:t/sig '[number? :-> number?]} ([x] x) ([x y] y))",
            "PNX2554",
            "Signature arities",
        ),
        (
            "(defn f {:t/sig '[:-> number?]} ([] (recur 1)))",
            "PNX2554",
            "expects 0 arguments",
        ),
        ("(defn- f ([x] x) ([x y] y))", "PNX9002", "needs a :t/sig"),
    ] {
        let source = format!("(ns demo) {definition}");
        let report = Session::default().check("f.clj", &source);
        assert_eq!(report.diagnostics.len(), 1, "{report:#?}");
        assert_eq!(report.diagnostics[0].code, code, "{report:#?}");
        assert!(report.diagnostics[0].message.contains(text), "{report:#?}");
    }
}
