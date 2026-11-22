use parenox::{Session, Source};

#[test]
fn known_core_vars_without_a_model_report_a_coverage_gap() {
    let source = r#"
        (ns demo (:refer-clojure :exclude [keyword] :rename {map-indexed indexed}))
        (defn use-core {:t/sig '[:-> any?]} []
          [clojure.core/keyword (indexed :x [{:x 1}])])
        (defn excluded {:t/sig '[:-> any?]} [] (keyword "a"))
        (defn shadowed {:t/sig '[:-> number?]} []
          (let [indexed (fn [x] (inc x))] (indexed 10)))
    "#;
    let report = Session::default().check("core.clj", source);
    assert!(!report.complete);
    let findings: Vec<_> = report
        .diagnostics
        .iter()
        .map(|diagnostic| {
            let span = diagnostic.span;
            (
                diagnostic.code.as_str(),
                &source[span.start.offset..span.end.offset],
            )
        })
        .collect();
    assert_eq!(
        findings,
        [
            ("PNX9001", "clojure.core/keyword"),
            ("PNX9001", "indexed"),
            ("PNX2304", "keyword"),
        ],
        "{report:#?}"
    );
}

#[test]
fn the_core_catalog_distinguishes_opaque_macros_vars_and_missing_names() {
    let source = r#"(ns demo (:refer-clojure :exclude [symbol]))
        (defn resource {:t/sig '[:-> t/any]} []
          (with-open [input (java.io.StringReader. "text")] (slurp input)))
        (defn known {:t/sig '[:-> t/any]} [] (clojure.core/symbol "text"))
        (defn excluded {:t/sig '[:-> t/any]} [] (symbol "text"))
        (defn misspelled {:t/sig '[:-> t/any]} [] (misspelld "text"))
        (defn local {:t/sig '[:-> number?]} []
          (let [with-open (fn [x] (inc x))] (with-open 3)))
        (defn macro-value {:t/sig '[:-> t/any]} [] with-open)
    "#;
    let report = Session::default().check("core.clj", source);
    let findings: Vec<_> = report
        .diagnostics
        .iter()
        .map(|diagnostic| {
            (
                diagnostic.code.as_str(),
                &source[diagnostic.span.start.offset..diagnostic.span.end.offset],
            )
        })
        .collect();
    assert_eq!(
        findings,
        [
            ("PNX9001", "with-open"),
            ("PNX9001", "clojure.core/symbol"),
            ("PNX2304", "symbol"),
            ("PNX2304", "misspelld"),
            ("PNX2304", "with-open"),
        ],
        "{report:#?}"
    );
    assert!(
        report.diagnostics[0]
            .message
            .contains("has no checked scope")
    );
}

#[test]
fn clojure_core_referrals_respect_exclusions_renames_and_local_vars() {
    let good = include_str!("../examples/core_imports.clj");
    let report = Session::default().check("core_imports.clj", good);
    assert!(report.ok && report.complete, "{report:#?}");
    let bad = include_str!("../examples/core_imports_bad.clj");
    let report = Session::default().check("core_imports_bad.clj", bad);
    assert!(!report.ok && report.complete, "{report:#?}");
    let [unbound] = report.diagnostics.as_slice() else {
        panic!("{report:#?}")
    };
    assert_eq!(unbound.code, "PNX2304");
    assert_eq!(
        &bad[unbound.span.start.offset..unbound.span.end.offset],
        "inc"
    );

    let source = r#"
        (ns demo (:refer-clojure :only []))
        (def answer 42)
        (clojure.core/defn run {:t/sig '[:-> clojure.core/number?]} []
          (do (if true answer 0)))
    "#;
    let report = Session::default().check("only.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn core_renames_do_not_refer_excluded_names_and_change_cached_resolution() {
    let mut session = Session::default();
    let source = r#"
        (ns demo (:refer-clojure :rename {inc advance}))
        (defn run {:t/sig '[:-> number?]} [] (advance 1))
    "#;
    let initial = session.check("core.clj", source);
    assert!(initial.ok && initial.complete, "{initial:#?}");
    assert_eq!(session.check("core.clj", source).stats.reused_functions, 1);

    let edited = source.replace(
        ":rename {inc advance}",
        ":exclude [inc] :rename {inc advance}",
    );
    let excluded = session.check("core.clj", &edited);
    assert_eq!(excluded.stats.checked_functions, 1);
    assert_eq!(excluded.diagnostics[0].code, "PNX2304", "{excluded:#?}");
    assert!(excluded.diagnostics[0].message.contains("advance"));

    let original = "(ns demo (:refer-clojure :exclude []))
                    (defn run {:t/sig '[:-> number?]} [] (inc 1))";
    assert!(session.check("core.clj", original).ok);
    let edited = original.replace(":exclude []", ":exclude [inc]");
    let filtered = session.check("core.clj", &edited);
    assert_eq!(filtered.stats.checked_functions, 1);
    assert_eq!(filtered.diagnostics[0].code, "PNX2304", "{filtered:#?}");
}

#[test]
fn a_later_core_referral_can_replace_an_earlier_library_referral() {
    let provider = Source {
        path: "provider.clj",
        source: include_str!("../examples/core_imports.clj"),
    };
    let app = Source {
        path: "app.clj",
        source: r#"
            (ns app
              (:require [demo.core-imports :refer [inc]])
              (:refer-clojure :only [inc defn number?]))
            (defn run {:t/sig '[:-> number?]} [] (inc 1))
        "#,
    };
    let report = Session::default().check_project(&[app, provider], None);
    assert!(report.ok && report.complete, "{report:#?}");
}
