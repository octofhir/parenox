use parenox::{Session, Source};

const SOURCE: &str = include_str!("../examples/regex_split.cljc");

#[test]
fn split_produces_string_vectors_and_nullable_destructuring() {
    let mut session = Session::default();
    let report = session.check("regex_split.cljc", SOURCE);
    assert!(report.ok && report.complete, "{report:#?}");

    let source = SOURCE.replace("    value))", "    (str/upper-case value)))");
    let report = session.check("regex_split.cljc", &source);
    assert!(!report.ok && report.complete, "{report:#?}");
    let mismatch = &report.diagnostics[0];
    assert_eq!(mismatch.code, "PNX2322");
    assert_eq!(mismatch.expected.as_deref(), Some("String"));
    assert_eq!(mismatch.actual.as_deref(), Some("Nil | String"));
    assert_eq!(
        &source[mismatch.span.start.offset..mismatch.span.end.offset],
        "value"
    );

    let source = SOURCE.replace(r##"(re-pattern #"=")"##, r#""=""#);
    let report = session.check("regex_split.cljc", &source);
    let mismatch = &report.diagnostics[0];
    assert_eq!(mismatch.expected.as_deref(), Some("Pattern"));
    assert_eq!(
        mismatch.contract.as_ref().unwrap().name,
        "clojure.string/split"
    );
}

#[test]
fn pattern_contract_survives_export_and_checks_consumers() {
    let export = Session::default().export_externs(
        &[Source {
            path: "regex_split.cljc",
            source: SOURCE,
        }],
        &[],
    );
    let edn = export.edn.unwrap_or_else(|| panic!("{:#?}", export.report));
    let consumer = Source {
        path: "client.clj",
        source: r##"(ns client (:require [clojure.spec.alpha :as s]))
            (defn read-fields {:t/sig '[string? :-> (s/coll-of string? :kind vector?)]} [s]
              (demo.query-string/fields s #"="))"##,
    };
    let report = Session::default().check_project_with_externs(
        &[consumer],
        &[Source {
            path: "api.edn",
            source: &edn,
        }],
    );
    assert!(report.ok && report.complete, "{report:#?}");
}
