use parenox::{ContractKind, Session, Source};

const EXTERNS: Source<'_> = Source {
    path: "types/macros.edn",
    source: include_str!("../examples/macro_scopes/externs.edn"),
};
const USE: Source<'_> = Source {
    path: "use.clj",
    source: include_str!("../examples/macro_scopes/use.clj"),
};

#[test]
fn extern_scope_resolves_sequential_bindings_without_claiming_let_semantics() {
    let report = Session::default().check_project(&[USE], Some(EXTERNS));
    assert!(!report.ok && !report.complete, "{report:#?}");
    assert!(report.files[1].ok, "{report:#?}");
    let [diagnostic] = report.files[0].diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(diagnostic.code, "PNX9009");
    assert!(
        diagnostic
            .message
            .contains("result and effects are unknown")
    );
    let origin = diagnostic.contract.as_ref().unwrap();
    assert_eq!(origin.name, "vendor.bindings/with-bindings");
    assert_eq!(origin.kind, ContractKind::MacroScope);
    assert_eq!(origin.path.as_deref(), Some(EXTERNS.path));
    assert_eq!(
        &USE.source[diagnostic.span.start.offset..diagnostic.span.end.offset],
        "(vendor/with-bindings [label (str label \"!\") result label]\n    result)"
    );
}

#[test]
fn a_binding_is_not_visible_in_its_own_initializer_or_after_the_macro() {
    let app = Source {
        path: "app.clj",
        source: "(ns app (:require [vendor.bindings :refer [with-bindings] :rename {with-bindings bind}]))
          (defn f {:t/sig '[:-> string?]} []
            (bind [inside inside next inside] next)
            inside)",
    };
    let report = Session::default().check_project(&[app], Some(EXTERNS));
    let diagnostics = &report.files[0].diagnostics;
    assert_eq!(diagnostics.len(), 3, "{report:#?}");
    assert_eq!(diagnostics[0].code, "PNX9009");
    for finding in &diagnostics[1..] {
        assert_eq!(finding.code, "PNX2304");
        assert_eq!(
            &app.source[finding.span.start.offset..finding.span.end.offset],
            "inside"
        );
    }
    assert_eq!(diagnostics[1].span.start.line, 3);
    assert_eq!(diagnostics[2].span.start.line, 4);
}

#[test]
fn lexical_scope_does_not_assume_body_types_or_copy_recur_edges() {
    let app = Source {
        path: "app.clj",
        source: "(ns app (:require [vendor.bindings :as v]))
          (defn f {:t/sig '[:-> number?]} []
            (loop [i 1]
              (v/with-bindings [x (inc \"unevaluated\")]
                (let [name x] (str name))
                (recur \"possibly not the enclosing loop\"))
              i))",
    };
    let report = Session::default().check_project(&[app], Some(EXTERNS));
    let diagnostics = &report.files[0].diagnostics;
    assert_eq!(diagnostics.len(), 1, "{report:#?}");
    assert_eq!(diagnostics[0].code, "PNX9009");
}

#[test]
fn local_and_source_functions_win_over_macro_scope_descriptions() {
    let app = Source {
        path: "app.clj",
        source: "(ns app (:require [vendor.bindings :refer [with-bindings]]))
          (defn render {:t/sig '[string? :-> string?]} [x] x)
          (defn scoped {:t/sig '[:-> string?]} []
            (let [with-bindings render] (with-bindings \"local\")))",
    };
    assert!(Session::default().check_project(&[app], Some(EXTERNS)).ok);
    let caller = Source {
        source: "(ns app (:require [vendor.bindings :as v]))
          (defn scoped {:t/sig '[:-> string?]} [] (v/with-bindings \"source\"))",
        ..app
    };
    let provider = Source {
        path: "vendor/bindings.clj",
        source: "(ns vendor.bindings)
          (defn with-bindings {:t/sig '[string? :-> string?]} [x] x)",
    };
    let report = Session::default().check_project(&[caller, provider], Some(EXTERNS));
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn invalid_descriptions_and_invocations_report_their_own_spans() {
    for (source, code) in [
        ("{vendor/m {:macro :for-like}}", "PNX9001"),
        ("{vendor/m {:macro :let-like :ret string?}}", "PNX9001"),
        ("{clojure.core/let {:macro :let-like}}", "PNX2790"),
        (
            "{vendor/m {:macro :let-like} vendor/m [:-> number?]}",
            "PNX9000",
        ),
    ] {
        let report = Session::default().check_project(&[], Some(Source { source, ..EXTERNS }));
        assert_eq!(report.files[0].diagnostics.len(), 1, "{report:#?}");
        assert_eq!(report.files[0].diagnostics[0].code, code);
    }

    let app = Source {
        path: "app.clj",
        source: "(ns app (:require [vendor.bindings :as v]))
          (defn f {:t/sig '[:-> string?]} [] (v/with-bindings [[x] []] x))
          (defn g {:t/sig '[:-> string?]} [] (v/with-bindings [x] x))
          (defn h {:t/sig '[:-> string?]} [] v/with-bindings)",
    };
    let report = Session::default().check_project(&[app], Some(EXTERNS));
    let diagnostics = &report.files[0].diagnostics;
    assert_eq!(diagnostics.len(), 3, "{report:#?}");
    for (finding, text) in diagnostics.iter().zip(["[x]", "[x]", "v/with-bindings"]) {
        assert_eq!(finding.code, "PNX9009");
        assert_eq!(
            &app.source[finding.span.start.offset..finding.span.end.offset],
            text
        );
    }
}

#[test]
fn removing_a_scope_rechecks_consumers_but_reuses_unrelated_functions() {
    let mut session = Session::default();
    let initial = session.check_project(&[USE], Some(EXTERNS));
    assert_eq!(initial.files[0].stats.checked_functions, 2);
    let warm = session.check_project(&[USE], Some(EXTERNS));
    assert_eq!(warm.files[0].stats.reused_functions, 2);
    let without = Source {
        source: "{}",
        ..EXTERNS
    };
    let updated = session.check_project(&[USE], Some(without));
    let cold = Session::default().check_project(&[USE], Some(without));
    assert_eq!(updated.files[0].stats.checked_functions, 1);
    assert_eq!(updated.files[0].stats.reused_functions, 1);
    assert!(
        updated.files[0]
            .diagnostics
            .iter()
            .any(|d| d.code == "PNX9003")
    );
    assert_eq!(
        serde_json::to_value(&updated.files[0].diagnostics).unwrap(),
        serde_json::to_value(&cold.files[0].diagnostics).unwrap()
    );
}
