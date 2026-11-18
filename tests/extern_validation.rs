use parenox::{ProjectReport, Session, Source};

fn validate(source: &str, externs: &str) -> ProjectReport {
    Session::default().validate_externs(
        &[Source {
            path: "api.clj",
            source,
        }],
        &[Source {
            path: "api.edn",
            source: externs,
        }],
    )
}

#[test]
fn source_can_accept_more_inputs_return_a_narrower_type_and_offer_extra_arities() {
    let report = validate(
        "(ns api (:require [clojure.spec.alpha :as s]))
         (defn label {:t/sig '[[:-> string?] [(s/nilable string?) :-> string?]]}
           ([] \"default\") ([value] (or value \"default\")))",
        "{api/label [string? :-> (clojure.spec.alpha/nilable string?)]}",
    );
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn an_extern_that_promises_a_wider_input_is_reported_at_the_edn_declaration() {
    let source = Source {
        path: "api.clj",
        source: "(ns api) (defn label {:t/sig '[string? :-> string?]} [value] value)",
    };
    let externs = Source {
        path: "api.edn",
        source: "{api/label [number? :-> string?]}",
    };
    let mut session = Session::default();
    assert!(session.check_project_with_externs(&[source], &[externs]).ok);
    let report = session.validate_externs(&[source], &[externs]);
    assert!(report.files[0].ok && report.complete);
    let [diagnostic] = report.files[1].diagnostics.as_slice() else {
        panic!("{report:#?}")
    };
    assert_eq!(diagnostic.code, "PNX2322");
    assert_eq!(
        diagnostic.contract.as_ref().unwrap().path.as_deref(),
        Some(externs.path)
    );
    assert!(
        diagnostic
            .notes
            .iter()
            .any(|note| note.contains(source.path))
    );
    assert_eq!(
        &externs.source[diagnostic.span.start.offset..diagnostic.span.end.offset],
        "api/label"
    );
}

#[test]
fn validation_points_to_the_same_nested_field_without_trusting_the_extern() {
    let source = "(ns api)
        (defn label {:t/sig '[{:request {:title string?}} :-> string?]} [input]
          (get-in input [:request :title]))";
    let report = validate(
        source,
        "{api/label [{:request {:title keyword?}} :-> string?]}",
    );
    assert!(report.files[0].ok && report.complete, "{report:#?}");
    let [error] = report.files[1].diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(error.code, "PNX2322");
    assert!(error.notes.iter().any(|note| {
        note == "argument 1 → field :request → field :title: expected String, got Keyword"
    }));
}

#[test]
fn exact_source_arity_overrides_variadic_source_for_a_promised_call() {
    let source = "(ns api)
        (defn feed {:t/sig '[[string? :-> number?] [number? & string? :-> number?]]}
          ([x] (count x)) ([x & _] x))";
    let bad = validate(source, "{api/feed [number? & string? :-> number?]}");
    let [diagnostic] = bad.files[1].diagnostics.as_slice() else {
        panic!("{bad:#?}")
    };
    assert_eq!(diagnostic.code, "PNX2322");
    assert!(diagnostic.message.contains("1 argument:"));
    let good = validate(
        source,
        "{api/feed [[string? :-> number?] [number? & string? :-> number?]]}",
    );
    assert!(good.ok && good.complete, "{good:#?}");
}

#[test]
fn a_variadic_source_can_implement_a_fixed_extern_but_must_cover_a_promised_tail() {
    let source = "(ns api)
        (defn pick {:t/sig '[string? & number? :-> string?]} [label & _] label)";
    let fixed = validate(source, "{api/pick [string? number? :-> string?]}");
    assert!(fixed.ok && fixed.complete, "{fixed:#?}");
    let shifted_prefix = validate(source, "{api/pick [string? number? & number? :-> string?]}");
    assert!(
        shifted_prefix.ok && shifted_prefix.complete,
        "{shifted_prefix:#?}"
    );
    let wrong_rest = validate(source, "{api/pick [string? & string? :-> string?]}");
    assert!(
        wrong_rest.files[1].diagnostics[0]
            .message
            .contains("2 arguments:")
    );

    let fixed_only = validate(
        "(ns api) (defn pick {:t/sig '[string? :-> string?]} [label] label)",
        "{api/pick [string? & number? :-> string?]}",
    );
    assert_eq!(fixed_only.files[1].diagnostics[0].code, "PNX2322");
    assert!(
        fixed_only.files[1].diagnostics[0]
            .message
            .contains("2 arguments:")
    );
}

#[test]
fn validation_does_not_trust_the_promises_to_check_another_source_body() {
    let source = Source {
        path: "api.clj",
        source: "(ns api) (defn forward {:t/sig '[:-> string?]} [] (api/impl))",
    };
    let externs = Source {
        path: "api.edn",
        source: "{api/forward [:-> string?] api/impl [:-> string?]}",
    };
    let mut session = Session::default();
    assert!(session.check_project_with_externs(&[source], &[externs]).ok);
    let audited = session.validate_externs(&[source], &[externs]);
    assert!(!audited.ok && !audited.complete, "{audited:#?}");
    assert_eq!(audited.stats.checked_functions, 1);
    assert_eq!(audited.files[0].diagnostics[0].code, "PNX2304");
    let [diagnostic] = audited.files[1].diagnostics.as_slice() else {
        panic!("{audited:#?}")
    };
    assert_eq!(diagnostic.code, "PNX9011");
    assert!(diagnostic.message.contains("api/impl"));
    let ordinary = session.check_project_with_externs(&[source], &[externs]);
    assert!(ordinary.ok && ordinary.complete, "{ordinary:#?}");
    assert_eq!(ordinary.stats.checked_functions, 1);
}

#[test]
fn trusted_dependency_externs_check_bodies_but_cannot_supply_a_validated_name() {
    let source = Source {
        path: "api.clj",
        source: "(ns api (:require [vendor :as v]))
          (defn render {:t/sig '[:-> string?]} [] (v/render))",
    };
    let promises = Source {
        path: "api.edn",
        source: "{api/render [:-> string?]}",
    };
    let dependencies = Source {
        path: "vendor.edn",
        source: "{vendor/render [:-> string?]}",
    };
    let mut session = Session::default();
    let unresolved = session.validate_externs(&[source], &[promises]);
    assert!(!unresolved.ok && !unresolved.complete, "{unresolved:#?}");
    let with_dependencies =
        session.validate_externs_with_dependencies(&[source], &[promises], &[dependencies]);
    assert!(
        with_dependencies.ok && with_dependencies.complete,
        "{with_dependencies:#?}"
    );

    let overlap = Source {
        source: "{vendor/render [:-> string?]}",
        ..promises
    };
    let report = session.validate_externs_with_dependencies(&[source], &[overlap], &[dependencies]);
    assert!(!report.ok, "{report:#?}");
    assert!(
        report.files[0]
            .diagnostics
            .iter()
            .any(|d| d.code == "PNX9003")
    );
    for externs in &report.files[1..] {
        let [diag] = externs.diagnostics.as_slice() else {
            panic!("{externs:#?}")
        };
        assert_eq!(diag.code, "PNX2790");
    }
}

#[test]
fn body_errors_still_fail_validation_when_the_contracts_match() {
    let report = validate(
        "(ns api) (defn find-label {:t/sig '[:-> string?]} [] 404)",
        "{api/find-label [:-> string?]}",
    );
    assert!(!report.ok, "{report:#?}");
    assert_eq!(report.files[0].diagnostics[0].code, "PNX2322");
    assert!(report.files[1].diagnostics.is_empty());
}

#[test]
fn generic_equivalence_is_proven_but_not_arbitrary_generic_subtyping() {
    let source = "(ns api) (defn same {:t/sig '(t/all [a] [a :-> a])} [value] value)";
    let equivalent = validate(source, "{api/same (t/all [value] [value :-> value])}");
    assert!(equivalent.ok && equivalent.complete, "{equivalent:#?}");
    let distinct = validate(
        source,
        "{api/same (t/all [(value :< number?)] [value :-> value])}",
    );
    assert!(!distinct.complete, "{distinct:#?}");
    assert_eq!(distinct.files[1].diagnostics[0].code, "PNX9011");
}

#[test]
fn private_vars_literals_and_macro_scopes_cannot_prove_externs() {
    let report = validate(
        "(ns api) (defn- hidden {:t/sig '[:-> number?]} [] 1) (def data 2)",
        "{api/hidden [:-> number?] api/data [:-> number?] api/bind {:macro :let-like}}",
    );
    assert!(!report.complete, "{report:#?}");
    let diagnostics = &report.files[1].diagnostics;
    assert_eq!(diagnostics.len(), 3, "{report:#?}");
    assert!(diagnostics.iter().all(|d| d.code == "PNX9011"));
    assert!(
        diagnostics
            .iter()
            .any(|d| d.message.contains("macro scope"))
    );
}
