use parenox_analysis::{Session, Source};

#[test]
fn an_extern_edit_rechecks_a_function_that_only_uses_it_in_a_precondition() {
    let caller = Source {
        path: "caller.clj",
        source: r#"
(ns demo.precondition (:require [vendor :as v]))
(defn guarded {:t/sig '[:-> #{:ready}]} []
  {:pre [(v/accepted? "value")]}
  :ready)
(defn independent {:t/sig '[:-> #{:other}]} [] :other)
"#,
    };
    let matching = Source {
        path: "vendor.edn",
        source: "{vendor/accepted? [string? :-> boolean?]}",
    };
    let mismatching = Source {
        source: "{vendor/accepted? [number? :-> boolean?]}",
        ..matching
    };

    let mut session = Session::default();
    let first = session.check_project_with_externs(&[caller], &[matching]);
    assert!(first.ok && first.complete, "{first:#?}");
    let warm = session.check_project_with_externs(&[caller], &[matching]);
    assert_eq!(warm.files[0].stats.reused_functions, 2, "{warm:#?}");

    let changed = session.check_project_with_externs(&[caller], &[mismatching]);
    let report = &changed.files[0];
    assert!(!report.ok && report.complete, "{changed:#?}");
    assert_eq!(report.stats.checked_functions, 1, "{changed:#?}");
    assert_eq!(report.stats.reused_functions, 1, "{changed:#?}");
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{changed:#?}");
    };
    assert_eq!(finding.code, "PNX2322");
    assert_eq!(finding.contract.as_ref().unwrap().name, "vendor/accepted?");
    assert_eq!(
        &caller.source[finding.span.start.offset..finding.span.end.offset],
        r#""value""#
    );

    let cold = Session::default().check_project_with_externs(&[caller], &[mismatching]);
    assert_eq!(
        serde_json::to_value(&report.diagnostics).unwrap(),
        serde_json::to_value(&cold.files[0].diagnostics).unwrap()
    );
    let restored = session.check_project_with_externs(&[caller], &[matching]);
    assert!(restored.ok && restored.complete, "{restored:#?}");
}

#[test]
fn an_inline_fn_precondition_keeps_its_private_helper_in_the_cache_dependencies() {
    let source = r#"
(ns demo.inline-precondition)
(defn- accepted? [value] (string? value))
(defn guarded {:t/sig '[:-> #{:ready}]} []
  ((fn ^{:pre [(accepted? value)]} [value] :ready) "text"))
(defn independent {:t/sig '[:-> #{:other}]} [] :other)
"#;
    let mut session = Session::default();
    let first = session.check("inline.clj", source);
    assert!(first.ok && first.complete, "{first:#?}");
    let warm = session.check("inline.clj", source);
    assert_eq!(warm.stats.reused_functions, 2, "{warm:#?}");

    let changed = source.replace("(string? value)", "(inc value)");
    let report = session.check("inline.clj", &changed);
    assert!(!report.ok && report.complete, "{report:#?}");
    assert_eq!(report.stats.checked_functions, 1, "{report:#?}");
    assert_eq!(report.stats.reused_functions, 1, "{report:#?}");
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(finding.code, "PNX2322");
    assert_eq!(finding.contract.as_ref().unwrap().name, "clojure.core/inc");
    assert_eq!(
        finding.span.start.offset,
        changed.find("(inc value)").unwrap() + "(inc ".len()
    );
}

#[test]
fn a_parameter_use_in_a_precondition_resolves_to_the_lexical_binding() {
    let path = "navigation.clj";
    let source = r#"
(ns demo.precondition-navigation)
(defn guarded {:t/sig '[string? :-> string?]} [value]
  {:pre [(some? value)]}
  value)
"#;
    let mut session = Session::default();
    let report = session.check(path, source);
    assert!(report.ok && report.complete, "{report:#?}");

    let binding = source.find("[value]").unwrap() + 1;
    let condition = source.find("(some? value)").unwrap() + "(some? ".len();
    let body = source.rfind("value").unwrap();
    let definition = session.definition(path, condition).unwrap();
    assert_eq!(definition.path, path);
    assert_eq!(definition.span.start.offset, binding);
    assert_eq!(
        &source[definition.span.start.offset..definition.span.end.offset],
        "value"
    );

    let references = session.references(path, condition, true);
    assert!(references.iter().all(|reference| reference.path == path));
    let offsets: Vec<_> = references
        .iter()
        .map(|reference| reference.span.start.offset)
        .collect();
    assert_eq!(offsets, [binding, condition, body]);
}

#[test]
fn adding_and_removing_project_core_rechecks_a_literal_precondition() {
    let caller = Source {
        path: "caller.clj",
        source: r#"
(ns demo.precondition)
(defn guarded {:t/sig '[:-> #{:ready}]} [] {:pre [true]} :ready)
"#,
    };
    let replacement = Source {
        path: "core.clj",
        source: "(ns clojure.core)",
    };
    let mut session = Session::default();
    let first = session.check_project(&[caller], None);
    assert!(first.ok && first.complete, "{first:#?}");
    let warm = session.check_project(&[caller], None);
    assert_eq!(warm.files[0].stats.reused_functions, 1, "{warm:#?}");

    let blocked = session.check_project(&[caller, replacement], None);
    let report = &blocked.files[0];
    assert!(!report.complete, "{blocked:#?}");
    assert_eq!(report.stats.checked_functions, 1, "{blocked:#?}");
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{blocked:#?}");
    };
    assert_eq!(finding.code, "PNX9001");
    assert!(
        finding.message.contains("clojure.core/assert"),
        "{finding:#?}"
    );
    assert_eq!(
        &caller.source[finding.span.start.offset..finding.span.end.offset],
        "true"
    );

    let restored = session.check_project(&[caller], None);
    assert!(restored.ok && restored.complete, "{restored:#?}");
    assert_eq!(
        restored.files[0].stats.checked_functions, 1,
        "{restored:#?}"
    );
}
