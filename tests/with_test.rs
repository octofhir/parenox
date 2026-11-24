use parenox::{Session, Source};

#[test]
fn with_test_exposes_the_wrapped_function_to_local_and_qualified_callers() {
    let mut session = Session::default();
    let report = session.check_project(
        &[
            Source {
                path: "demo/library.clj",
                source: r#"
                (ns demo.library (:require [clojure.test :refer [with-test is]]))
                (with-test
                  (defn answer {:t/sig '[int? :-> int?]} [n] n)
                  (is (= 42 (answer 42))))
                (defn local {:t/sig '[:-> int?]} [] (answer 42))
            "#,
            },
            Source {
                path: "demo/client.clj",
                source: r#"
                (ns demo.client (:require [demo.library :as lib]))
                (defn good {:t/sig '[:-> int?]} [] (lib/answer 42))
                (defn bad {:t/sig '[:-> int?]} [] (lib/answer))
            "#,
            },
        ],
        None,
    );
    assert!(
        report
            .files
            .iter()
            .find(|file| file.path == "demo/client.clj")
            .unwrap()
            .diagnostics
            .iter()
            .any(|finding| finding.code == "PNX2554"),
        "{report:#?}"
    );
    assert!(
        !report
            .files
            .iter()
            .flat_map(|file| &file.diagnostics)
            .any(|finding| matches!(finding.code.as_str(), "PNX2304" | "PNX2305" | "PNX2322")),
        "{report:#?}"
    );
}

#[test]
fn with_test_checks_the_wrapped_definition_body() {
    let report = Session::default().check(
        "demo.clj",
        r#"
            (ns demo (:require [clojure.test :as test]))
            (test/with-test
              (defn wrong {:t/sig '[:-> string?]} [] 42)
              (test/is true))
        "#,
    );
    assert!(
        report
            .diagnostics
            .iter()
            .any(|finding| finding.code == "PNX2322"),
        "{report:#?}"
    );
    assert!(
        report
            .diagnostics
            .iter()
            .any(|finding| finding.code == "PNX9001"
                && finding.message.contains("with-test test body")),
        "{report:#?}"
    );
}

#[test]
fn with_test_does_not_treat_a_shadowing_macro_as_the_standard_macro() {
    let report = Session::default().check(
        "demo.clj",
        r#"
            (ns demo (:require [clojure.test :refer [with-test]]))
            (defmacro with-test [& args] nil)
            (with-test (defn hidden {:t/sig '[:-> string?]} [] 42))
        "#,
    );
    assert!(
        !report
            .diagnostics
            .iter()
            .any(|finding| finding.code == "PNX2322"),
        "{report:#?}"
    );
    assert!(
        report
            .diagnostics
            .iter()
            .any(|finding| finding.code == "PNX9001"
                && finding
                    .message
                    .contains("Unsupported top-level declaration with-test")),
        "{report:#?}"
    );
}

#[test]
fn a_suggested_fdef_is_inserted_after_with_test() {
    let source = r#"
        (ns demo (:require [clojure.test :as test] [clojure.spec.alpha :as s]))
        (test/with-test
          (defn answer [] 42)
          (test/is (= 42 (answer))))
    "#;
    let mut session = Session::default();
    session.check("demo.clj", source);
    let suggestion = session
        .suggest_fdef("demo.clj", source.find("defn answer").unwrap())
        .unwrap();
    let edit = suggestion
        .edits
        .iter()
        .find(|edit| edit.new_text.contains("(s/fdef answer"))
        .unwrap();
    assert_eq!(source[..edit.offset].trim_end(), source.trim_end());
    assert!(
        session
            .suggest_fdef("demo.clj", source.find("test/is").unwrap())
            .is_none()
    );
}
