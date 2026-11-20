use parenox::{Session, Source};

#[test]
fn resolves_clojure_test_refer_all_and_renamed_test_macros() {
    let source = r#"
        (ns app-test (:require [clojure.test :refer :all :rename {deftest scenario}]))
        (scenario finds-a-label (is (= "label" "label")))
    "#;
    let report = Session::default().check("app_test.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");

    let excluded = source.replace(":rename {deftest scenario}", ":exclude [deftest]");
    let excluded = excluded.replace("(scenario ", "(deftest ");
    let report = Session::default().check("app_test.clj", &excluded);
    assert_eq!(report.diagnostics.len(), 1);
    assert_eq!(report.diagnostics[0].severity, "error");
    assert!(
        report.diagnostics[0]
            .message
            .contains("Unsupported top-level declaration deftest")
    );
}

#[test]
fn legacy_use_supports_bare_namespaces_and_renamed_explicit_referrals() {
    let app = r#"(ns app-test
      (:use clojure.test
            [demo.all-refers.library :only (label) :rename {label caption}]))
      (defn text {:t/sig '[:-> string?]} [] (caption))
      (deftest output (is (= "ready" (text))))"#;
    let report = check(&mut Session::default(), LIBRARY, app);
    assert!(report.ok && report.complete, "{report:#?}");
}

const LIBRARY: &str = include_str!("../examples/all_refers/library.clj");
const APP: &str = include_str!("../examples/all_refers/app.clj");

fn check(session: &mut Session, library: &str, app: &str) -> parenox::ProjectReport {
    session.check_project(
        &[
            Source {
                path: "app.clj",
                source: app,
            },
            Source {
                path: "library.clj",
                source: library,
            },
        ],
        None,
    )
}

#[test]
fn refers_public_functions_and_values_with_exclusions_and_renames() {
    let mut session = Session::default();
    let good = check(&mut session, LIBRARY, APP);
    assert!(good.ok && good.complete, "{good:#?}");
    assert_eq!(good.files[0].stats.checked_functions, 3);
    let bad = check(
        &mut session,
        LIBRARY,
        include_str!("../examples/all_refers/bad.clj"),
    );
    let diagnostics = &bad.files[0].diagnostics;
    assert_eq!(diagnostics.len(), 2, "{bad:#?}");
    assert!(diagnostics.iter().all(|d| d.code == "PNX2304"));
    assert!(diagnostics[0].message.contains("skipped"));
    assert!(diagnostics[1].message.contains("hidden"));
}

#[test]
fn refreshes_referrals_when_exported_names_change_or_are_restored() {
    let mut session = Session::default();
    assert!(check(&mut session, LIBRARY, APP).ok);
    let warm = check(&mut session, LIBRARY, APP);
    assert_eq!(warm.files[0].stats.reused_functions, 3);
    let no_caption = LIBRARY.replace("(defn label", "(defn title");
    let removed = check(&mut session, &no_caption, APP);
    assert!(removed.complete, "{removed:#?}");
    assert_eq!(removed.stats.parsed_files, 1);
    assert_eq!(removed.files[0].diagnostics[0].code, "PNX2304");
    assert!(removed.files[0].diagnostics[0].message.contains("caption"));
    assert!(check(&mut session, LIBRARY, APP).ok);

    let changed_type = LIBRARY.replace(
        "'[:-> string?]} []\n  \"ready\"",
        "'[:-> number?]} []\n  42",
    );
    let changed = check(&mut session, &changed_type, APP);
    let [result] = changed.files[0].diagnostics.as_slice() else {
        panic!("{changed:#?}");
    };
    assert_eq!(result.code, "PNX2322");
    assert_eq!(result.actual.as_deref(), Some("Number"));
}

#[test]
fn preserves_namespace_order_for_core_and_reports_unavailable_sources() {
    let mut session = Session::default();
    let core = r#"(ns app
        (:require [demo.all-refers.library :refer :all])
        (:refer-clojure :only [inc]))
        (clojure.core/defn count-up {:t/sig '[:-> clojure.core/number?]} [] (inc 2))"#;
    // Arithmetic result types are broad. A string-only library replacement
    // makes it possible to distinguish the callee using its return contract.
    let lib = LIBRARY.replace(
        "'[number? :-> number?]} [n]\n  (+ n 10)",
        "'[number? :-> string?]} [n]\n  (str n)",
    );
    let report = check(&mut session, &lib, core);
    assert!(report.ok && report.complete, "{report:#?}");

    let external = r#"(ns app (:require [some.library :refer :all]))
        (defn f {:t/sig '[:-> number?]} [] 1)"#;
    let report = session.check_project(
        &[Source {
            path: "app.clj",
            source: external,
        }],
        Some(Source {
            path: "externs.edn",
            source: "{some.library/adjust [number? :-> number?]}",
        }),
    );
    assert!(!report.complete, "{report:#?}");
    let [diagnostic] = report.files[0].diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(diagnostic.code, "PNX9001");
    assert!(
        diagnostic
            .message
            .contains("requires the source of namespace some.library")
    );
}
