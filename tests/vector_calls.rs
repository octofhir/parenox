use parenox::{Report, Session};

fn check(body: &str) -> Report {
    let source = format!("(ns demo (:require [clojure.spec.alpha :as s]))\n{body}");
    Session::default().check("vectors.clj", &source)
}

#[test]
fn checks_vector_pipelines_and_narrows_elements_with_core_predicates() {
    let report = Session::default().check("vectors.clj", include_str!("../examples/vectors.clj"));
    assert!(report.ok && report.complete, "{report:#?}");

    for (expr, element) in [
        (r#"(filterv string? [nil "Ada" 1])"#, r#"#{ "Ada" }"#),
        (r#"(filterv nil? [nil "Ada"])"#, "nil?"),
        (r#"(filterv identity [false nil ""])"#, "string?"),
        ("(filterv some? [false nil])", "boolean?"),
        ("(mapv identity #{:active})", "keyword?"),
        (r#"(mapv identity "Ada")"#, "char?"),
        ("(mapv inc [] [1])", "string?"),
        ("(filterv (fn [x] false) [1])", "string?"),
    ] {
        let report = check(&format!(
            "(defn f {{:t/sig '[:-> (s/coll-of {element} :kind vector?)]}} [] {expr})"
        ));
        assert!(report.ok && report.complete, "{expr}: {report:#?}");
    }
}

#[test]
fn points_at_elements_and_callback_bodies_without_suggesting_collection_replacements() {
    let source = include_str!("../examples/vectors_bad.clj");
    let report = Session::default().check("vectors_bad.clj", source);
    assert!(!report.ok && report.complete, "{report:#?}");
    let [direct, inline] = &report.diagnostics[..] else {
        panic!("{report:#?}");
    };
    let fragment = |d: &parenox::Diagnostic| &source[d.span.start.offset..d.span.end.offset];
    assert_eq!(fragment(direct), r#"["2"]"#);
    assert_eq!(direct.actual.as_deref(), Some("String"));
    assert!(direct.suggestions.is_empty());
    assert!(
        direct
            .help
            .as_deref()
            .is_some_and(|help| help.contains("elements"))
    );
    assert_eq!(fragment(inline), "name");
    assert_eq!(inline.expected.as_deref(), Some("String"));
    assert_eq!(inline.suggestions[0].replacement, r#"(or name "")"#);
}

#[test]
fn empty_inputs_skip_invocation_but_still_check_callback_expressions() {
    for (expr, code) in [
        (r#"(mapv (fnil inc (inc "bad")) [])"#, "PNX2322"),
        ("(mapv (fn [x y] missing) [])", "PNX2304"),
        ("(mapv inc [1] [2])", "PNX2554"),
        ("(mapv inc 42)", "PNX2322"),
        ("(mapv inc {:x 1})", "PNX9001"),
        ("(filterv number? [1] [2])", "PNX2554"),
    ] {
        let report = check(&format!(
            "(defn f {{:t/sig '[:-> (s/coll-of number? :kind vector?)]}} [] {expr})"
        ));
        assert_eq!(report.diagnostics.len(), 1, "{expr}: {report:#?}");
        assert_eq!(report.diagnostics[0].code, code, "{expr}: {report:#?}");
    }
}

#[test]
fn some_predicate_preserves_false_and_respects_shadowing() {
    for body in [
        r#"(defn f {:t/sig '[:-> (s/coll-of string? :kind vector?)]}
             [] (filterv some? [nil false "Ada"]))"#,
        r#"(defn- some? [value] true)
           (defn f {:t/sig '[:-> (s/coll-of string? :kind vector?)]}
             [] (filterv some? [nil "Ada"]))"#,
    ] {
        let report = check(body);
        assert!(report.complete && !report.ok, "{report:#?}");
        assert_eq!(report.diagnostics.len(), 1, "{report:#?}");
        assert_eq!(report.diagnostics[0].code, "PNX2322", "{report:#?}");
    }
}
