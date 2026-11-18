use parenox::{Session, Source, reader};

#[test]
fn validates_namespaced_keywords_even_in_discarded_and_inactive_forms() {
    let bad = include_str!("../examples/keyword_aliases_bad.clj");
    let inactive = r#"(ns demo)
        (defn f {:t/sig '[:-> keyword?]} [] #?(:clj :ok :cljs ::missing/status))"#;
    let before_ns = r#"(def early ::missing/status)
        (ns demo (:require [demo.schema :as-alias missing]))"#;
    for (path, source) in [
        ("discard.clj", bad),
        ("inactive.cljc", inactive),
        ("before-ns.clj", before_ns),
    ] {
        let report = Session::default().check(path, source);
        assert!(!report.complete, "{report:#?}");
        let [diagnostic] = report.diagnostics.as_slice() else {
            panic!("{report:#?}");
        };
        assert_eq!(diagnostic.code, "PNX9000");
        assert_eq!(
            &source[diagnostic.span.start.offset..diagnostic.span.end.offset],
            "::missing/status"
        );
        assert!(diagnostic.message.contains("namespace alias missing"));
        assert_eq!(reader::parse(source).syntax().text().to_string(), source);
    }
}

#[test]
fn adding_a_keyword_alias_fixes_the_next_snapshot() {
    let mut session = Session::default();
    let source = include_str!("../examples/keyword_aliases.clj");
    let without_alias = source.replace(":as-alias schema", ":as-alias renamed");
    let bad = session.check("keyword_aliases.clj", &without_alias);
    assert_eq!(bad.diagnostics[0].code, "PNX9000");
    let fixed = session.check("keyword_aliases.clj", source);
    assert!(fixed.ok && fixed.complete, "{fixed:#?}");
    assert_eq!(fixed.stats.checked_functions, 2);
    assert_eq!(
        session
            .check("keyword_aliases.clj", source)
            .stats
            .reused_functions,
        2
    );
}

#[test]
fn externs_require_edn_keywords_without_auto_resolution() {
    for token in ["::keyword", "::alias/keyword"] {
        let source = format!("{{db/run [{token} :-> string?]}}");
        let report = Session::default().check_project(
            &[],
            Some(Source {
                path: "externs.edn",
                source: &source,
            }),
        );
        let [diagnostic] = report.files[0].diagnostics.as_slice() else {
            panic!("{report:#?}");
        };
        assert_eq!(diagnostic.code, "PNX9000");
        assert_eq!(
            &source[diagnostic.span.start.offset..diagnostic.span.end.offset],
            token
        );
        assert!(diagnostic.message.contains("EDN"));
    }
}
