use parenox::Session;

#[test]
fn import_groups_and_qualified_classes_resolve_in_exception_handlers() {
    let source = include_str!("../examples/imports.clj");
    let mut session = Session::default();
    let report = session.check("imports.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");
    assert_eq!(
        session.check("imports.clj", source).stats.reused_functions,
        1
    );

    let changed = source.replace("IOException FileNotFoundException", "IOException");
    let report = session.check("imports.clj", &changed);
    assert!(!report.complete, "{report:#?}");
    assert_eq!(report.stats.checked_functions, 1);
    assert!(report.diagnostics.iter().any(|d| {
        d.code == "PNX9001" && d.message.contains("Catch class FileNotFoundException")
    }));
    assert!(session.check("imports.clj", source).ok);
}

#[test]
fn imports_jvm_classes_from_the_default_package() {
    let source = "(ns demo (:import SampleParser))";
    let mut session = Session::default();
    let report = session.check("imports.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");
    assert_eq!(
        session.completion_java_class("imports.clj", "SampleParser"),
        Some("SampleParser")
    );
    assert_eq!(
        session.definition_java_class("imports.clj", source.find("SampleParser").unwrap()),
        Some("SampleParser")
    );

    let conflicting = "(ns demo (:import SampleParser [example SampleParser]))";
    let report = session.check("imports.clj", conflicting);
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(finding.code, "PNX2790", "{finding:#?}");
}

#[test]
fn validates_import_shape_conflicts_and_known_nonthrowable_classes() {
    for (source, code) in [
        (include_str!("../examples/imports_bad.clj"), "PNX2790"),
        ("(ns bad (:import [java.io]))", "PNX1001"),
        ("(ns bad (:import java/String))", "PNX1001"),
        ("(ns bad (:import java..String))", "PNX1001"),
        (
            "(ns bad (:import java.lang.String))
           (defn f {:t/sig '[:-> number?]} [] (try 1 (catch String e 2)))",
            "PNX1001",
        ),
        (
            "(ns bad (:import java.io.IOException))
           (def IOException 3)",
            "PNX2790",
        ),
    ] {
        let report = Session::default().check("imports.clj", source);
        assert_eq!(report.diagnostics[0].code, code, "{report:#?}");
    }
    let report = Session::default().check(
        "imports.clj",
        "(ns unknown (:import java.time.Instant))
         (defn run {:t/sig '[:-> number?]} [] (try 1 (catch Instant e 2)))",
    );
    assert!(!report.complete, "{report:#?}");
    assert!(
        report.diagnostics[0]
            .message
            .contains("supported JVM exception classes")
    );
}
