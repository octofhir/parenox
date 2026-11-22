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
fn an_unavailable_external_operator_keeps_the_operand_scope_opaque() {
    let app = Source {
        path: "app.clj",
        source: r#"(ns app (:require [fixtures.macros :as m :refer [with-value]]))
(defn qualified {:t/sig '[:-> string?]} []
  (m/with-value [local "abc"] (subs local 1) (subs false 0)))
(defn referred {:t/sig '[:-> string?]} []
  (with-value [local "abc"] (subs local 1) (subs false 0)))"#,
    };
    let provider = Source {
        path: "fixtures/macros.clj",
        source: r#"(ns fixtures.macros)
(defn with-value {:t/sig '[t/any t/any t/any :-> string?]} [binding body extra] "ok")"#,
    };
    let mut session = Session::default();
    let diagnostics = |report: &parenox::ProjectReport| {
        report.files[0]
            .diagnostics
            .iter()
            .map(|finding| {
                (
                    finding.code.clone(),
                    &app.source[finding.span.start.offset..finding.span.end.offset],
                )
            })
            .collect::<Vec<_>>()
    };

    let missing = session.check_project(&[app], None);
    assert!(!missing.complete);
    assert_eq!(
        diagnostics(&missing),
        [
            ("PNX9003".into(), "m/with-value"),
            ("PNX9003".into(), "with-value"),
        ],
        "{missing:#?}"
    );

    let provided = session.check_project(&[app, provider], None);
    let cold = Session::default().check_project(&[app, provider], None);
    assert_eq!(
        serde_json::to_value(&provided.files[0].diagnostics).unwrap(),
        serde_json::to_value(&cold.files[0].diagnostics).unwrap()
    );
    let eager_findings = [
        ("PNX2304".into(), "local"),
        ("PNX2304".into(), "local"),
        ("PNX2322".into(), "false"),
    ];
    assert_eq!(
        diagnostics(&provided),
        [eager_findings.clone(), eager_findings].concat(),
        "{provided:#?}"
    );
    assert_eq!(
        serde_json::to_value(&session.check_project(&[app], None).files[0].diagnostics).unwrap(),
        serde_json::to_value(&missing.files[0].diagnostics).unwrap()
    );
}

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
fn source_macros_keep_their_payload_opaque_through_referrals_and_externs() {
    let provider = Source {
        path: "vendor/bindings.clj",
        source: r#"(ns vendor.bindings)
          (defmacro notice [_] nil)
          (defmacro ^:private hidden [_] nil)"#,
    };
    let externs = Source {
        path: "types/vendor.edn",
        source: "{vendor.bindings/notice [number? :-> nil?] vendor.bindings/hidden [number? :-> nil?]}",
    };
    let cases = [
        ("[vendor.bindings :as v]", "v/notice", "PNX9001"),
        (
            "[vendor.bindings :refer [notice] :rename {notice warn}]",
            "warn",
            "PNX9001",
        ),
        ("[vendor.bindings :refer :all]", "notice", "PNX9001"),
        ("[vendor.bindings :as v]", "v/hidden", "PNX2304"),
    ];
    for (require, call, code) in cases {
        let source = format!(
            "(ns app (:require {require}))\n(defn label {{:t/sig '[string? :-> string?]}} [text] ({call} (inc text)) text)"
        );
        let consumer = Source {
            path: "app.clj",
            source: &source,
        };
        let report = Session::default().check_project(&[consumer, provider], Some(externs));
        let [diagnostic] = report.files[0].diagnostics.as_slice() else {
            panic!("{report:#?}")
        };
        assert_eq!(diagnostic.code, code, "{report:#?}");
    }
}

#[test]
fn replacing_a_macro_groups_calls_by_compilation_order() {
    let provider = Source {
        path: "vendor/bindings.clj",
        source: r#"(ns vendor.bindings)
          (defmacro notice [_] nil)
          (defn before {:t/sig '[string? :-> nil?]} [text] (notice (inc text)))
          (defn notice {:t/sig '[number? :-> number?]} [n] (notice (inc "syntax")) n)
          (defn after {:t/sig '[string? :-> number?]} [text] (notice text))"#,
    };
    let consumer = Source {
        path: "app.clj",
        source: r#"(ns app (:require [vendor.bindings :refer :all]))
          (defn call {:t/sig '[string? :-> number?]} [label] (notice label))"#,
    };
    let report = Session::default().check_project(&[consumer, provider], None);
    let findings = |file: usize, source: &str| {
        report.files[file]
            .diagnostics
            .iter()
            .map(|d| {
                let range = d.span.start.offset..d.span.end.offset;
                (d.code.clone(), source[range].to_owned())
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(
        findings(0, consumer.source),
        [("PNX2322".into(), "label".into())]
    );
    assert_eq!(
        findings(1, provider.source),
        [
            ("PNX9001".into(), "defmacro".into()),
            ("PNX9001".into(), "notice".into()),
            ("PNX9001".into(), "notice".into()),
            ("PNX2322".into(), "text".into()),
        ],
    );
}

#[test]
fn a_public_source_macro_can_use_its_declared_lexical_scope() {
    let provider = Source {
        path: "vendor/bindings.clj",
        source: "(ns vendor.bindings) (defmacro with-bindings [bindings & body] (list* 'let bindings body))",
    };
    let report = Session::default().check_project(&[USE, provider], Some(EXTERNS));
    let [diagnostic] = report.files[0].diagnostics.as_slice() else {
        panic!("{report:#?}")
    };
    assert_eq!(diagnostic.code, "PNX9009");
    assert_eq!(
        diagnostic.contract.as_ref().unwrap().kind,
        ContractKind::MacroScope
    );
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
