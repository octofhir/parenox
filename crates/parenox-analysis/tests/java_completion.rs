use parenox_analysis::{Session, Source};

#[test]
fn resolves_explicit_imports_and_verified_implicit_classes() {
    let source = r#"(ns demo
  (:import [java.time Instant LocalDate]
           (java.util UUID)
           java.util.concurrent.TimeUnit))
(defn unfinished [] (Instant/"#;
    let mut session = Session::default();
    session.check("demo.clj", source);
    for (qualifier, class) in [
        ("Instant", "java.time.Instant"),
        ("LocalDate", "java.time.LocalDate"),
        ("UUID", "java.util.UUID"),
        ("TimeUnit", "java.util.concurrent.TimeUnit"),
        ("Math", "java.lang.Math"),
        ("BigInteger", "java.math.BigInteger"),
        ("Thread$State", "java.lang.Thread$State"),
        ("java.nio.file.Files", "java.nio.file.Files"),
        ("java.util.Map$Entry", "java.util.Map$Entry"),
    ] {
        assert_eq!(
            session.completion_java_class("demo.clj", qualifier),
            Some(class)
        );
    }
    for qualifier in [
        "URI",
        "Record",
        "Files",
        "java..lang.Foo",
        "a.3Class",
        "a.b/C",
    ] {
        assert_eq!(session.completion_java_class("demo.clj", qualifier), None);
    }
}

#[test]
fn keeps_namespace_aliases_and_source_namespaces_ahead_of_java() {
    let caller = Source {
        path: "caller.clj",
        source: r#"(ns caller
  (:require [clojure.string :as String]
            [external.Tools :as external.Tools]
            [external.Unindexed]
            [unloaded.ns :as java.lang.Math])
  (:import [java.util UUID]))"#,
    };
    let source = Source {
        path: "source.clj",
        source: "(ns java.util.UUID)",
    };
    let mut session = Session::default();
    session.check_project(&[caller, source], None);
    for qualifier in [
        "String",
        "external.Tools",
        "external.Unindexed",
        "java.lang.Math",
        "java.util.UUID",
    ] {
        assert_eq!(session.completion_java_class("caller.clj", qualifier), None);
    }
    assert_eq!(
        session.completion_java_class("caller.clj", "UUID"),
        Some("java.util.UUID")
    );
    assert_eq!(
        session.completion_namespace("caller.clj", "String"),
        Some("clojure.string")
    );
}
