use parenox_analysis::{Session, Source};

#[test]
fn completes_standalone_require_aliases_in_an_incomplete_buffer() {
    let caller = r#"(ns app (:require [library.old :as old]))
(require '[clojure.string :as str])
(clojure.core/require (quote [library.source :as src]))
(comment
  (str/j)
  (src/ex)"#;
    let provider = r#"(ns library.source)
(defn exported "Exported docs." [] nil)
(defn- excluded [] nil)"#;
    let mut session = Session::default();
    session.check_project(
        &[
            Source {
                path: "app.clj",
                source: caller,
            },
            Source {
                path: "library/source.clj",
                source: provider,
            },
        ],
        None,
    );
    let string_position = caller.find("str/j").unwrap() + "str/j".len();
    assert_eq!(
        session.completion_archive_namespace("app.clj", "str", string_position),
        Some("clojure.string")
    );
    assert_eq!(
        session.completion_namespace_at("app.clj", "old", string_position),
        Some("library.old")
    );
    assert_eq!(
        session.complete_qualified_var_at("app.clj", "src/ex", caller.len(), 10),
        ["src/exported"]
    );
    assert!(
        session
            .completion_documentation_at("app.clj", "src/exported", "library.source", caller.len())
            .unwrap()
            .contains("Exported docs.")
    );
}

#[test]
fn ignores_future_nested_dynamic_and_shadowed_require_calls() {
    let source = r#"(ns app (:require [library.old :as old]))
(require '[library.new :as old])
(comment (require '[library.nested :as nested]))
(require [library.evaluated :as evaluated])
(require '[library.dynamic :as dynamic] computed)
(defn require [& args] args)
(require '[library.shadowed :as shadowed])
(clojure.core/require '[library.future :as future])"#;
    let mut session = Session::default();
    session.check("app.clj", source);
    for qualifier in ["nested", "evaluated", "dynamic", "shadowed"] {
        assert_eq!(
            session.completion_require_alias("app.clj", qualifier, source.len()),
            None
        );
    }
    assert_eq!(
        session.completion_require_alias("app.clj", "future", source.len() - 1),
        None
    );
    assert_eq!(
        session.completion_require_alias("app.clj", "future", source.len()),
        Some("library.future")
    );
    assert_eq!(
        session.completion_namespace_at("app.clj", "old", source.len()),
        Some("library.old")
    );
}
