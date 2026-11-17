use parenox::{Session, Source};

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
