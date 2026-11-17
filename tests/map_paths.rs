use parenox::{Report, Session};

fn check(expression: &str, result: &str) -> Report {
    let source = format!(
        "(ns demo (:require [clojure.spec.alpha :as s]))\n(defn f {{:t/sig '[:-> {result}]}} [] {expression})"
    );
    Session::default().check("paths.clj", &source)
}

#[test]
fn nested_lookup_separates_absence_from_present_nil_and_false() {
    for (expression, expected) in [
        (r#"(get-in {:p {:x nil}} [:p :x] "fallback")"#, "nil?"),
        (r#"(get-in {:p {:x false}} [:p :x] "fallback")"#, "boolean?"),
        (r#"(get-in {} [:p :x] "fallback")"#, "string?"),
        (r#"(get-in {:p nil} [:p :x] "fallback")"#, "string?"),
        (r#"(get-in {:p 42} [:p :x] "fallback")"#, "string?"),
        (r#"(get-in {:p #{:other}} [:p :x] "fallback")"#, "string?"),
        (r#"(get-in :seed [] "fallback")"#, "keyword?"),
        (r#"(get-in {:p {::name "Ada"}} [:p ::name])"#, "string?"),
    ] {
        let report = check(expression, expected);
        assert!(report.ok && report.complete, "{expression}: {report:#?}");
    }

    let report = check(r#"(get-in {:p {:x 1}} [:p :x] (inc "bad"))"#, "number?");
    assert_eq!(report.diagnostics.len(), 1, "{report:#?}");
    assert_eq!(report.diagnostics[0].actual.as_deref(), Some("String"));
}

#[test]
fn nested_assoc_creates_maps_and_preserves_unmodified_fields() {
    let report = Session::default().check("paths.clj", include_str!("../examples/paths.clj"));
    assert!(report.ok && report.complete, "{report:#?}");
    for (expression, expected) in [
        (
            r#"(get-in (assoc-in nil [:p :x] "Ada") [:p :x])"#,
            "string?",
        ),
        (
            r#"(get-in (assoc-in {:p nil} [:p :x] false) [:p :x])"#,
            "boolean?",
        ),
        (
            r#"(get-in (assoc-in {:p {:sibling 42}} [:p :x] "Ada") [:p :sibling])"#,
            "number?",
        ),
        (r#"(get-in (assoc-in {:p 42} [:p] "Ada") [:p])"#, "string?"),
    ] {
        let report = check(expression, expected);
        assert!(report.ok && report.complete, "{expression}: {report:#?}");
    }

    let source = include_str!("../examples/paths_bad.clj");
    let report = Session::default().check("paths_bad.clj", source);
    assert!(!report.ok && report.complete, "{report:#?}");
    assert_eq!(report.diagnostics.len(), 2, "{report:#?}");
    assert_eq!(report.diagnostics[0].actual.as_deref(), Some("Nil"));
    let span = report.diagnostics[1].span;
    assert_eq!(&source[span.start.offset..span.end.offset], ":patient");
}

#[test]
fn nested_paths_report_unsupported_shapes_and_open_map_intermediates() {
    for expression in ["(get-in {} [0])", "(assoc-in {} [] 1)"] {
        let report = check(expression, "number?");
        assert!(!report.ok && !report.complete, "{report:#?}");
        assert_eq!(report.diagnostics[0].code, "PNX9001");
    }
    let path = ":p ".repeat(33);
    let report = check(&format!("(assoc-in {{}} [{path}] 1)"), "number?");
    assert_eq!(report.diagnostics[0].code, "PNX9001");

    let source = "(ns demo (:require [clojure.spec.alpha :as s]))
        (defn f {:t/sig '[(s/keys) :-> string?]} [p]
          (get-in (assoc-in p [:unknown :x] \"Ada\") [:unknown :x]))";
    let report = Session::default().check("open.clj", source);
    assert!(!report.ok && !report.complete, "{report:#?}");
    assert_eq!(report.diagnostics[0].code, "PNX9003");
}
