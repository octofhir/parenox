use parenox::{Session, Source};

const CONFIG: Source<'_> = Source {
    path: "config.clj",
    source: include_str!("../examples/literal_vars/config.clj"),
};
const APP: Source<'_> = Source {
    path: "app.clj",
    source: include_str!("../examples/literal_vars/app.clj"),
};

#[test]
fn forward_var_declarations_resolve_to_the_definition_when_available() {
    let consumer = Source {
        path: "consumer.cljc",
        source: r#"(ns consumer (:require [provider :as p]))
            (defn status {:t/sig '[:-> string?]} [] (p/label "ready"))"#,
    };
    let mut session = Session::default();
    let provider = Source {
        path: "provider.clj",
        source: "(ns provider)\n(declare label)",
    };
    let before = session.check_project(&[consumer, provider], None);
    assert!(!before.complete, "{before:#?}");
    assert!(before.files[1].diagnostics.iter().any(|diagnostic| {
        diagnostic.message.contains("Declared var provider/label")
            && diagnostic.severity == "warning"
    }));
    let offset = consumer.source.find("p/label").unwrap();
    let target = session.definition(consumer.path, offset).unwrap();
    assert_eq!(target.path, provider.path);
    assert_eq!(target.span.start.line, 2);

    let defined = r#"(ns provider)
       (declare label)
       (defn label {:t/sig '[string? :-> string?]} [value] value)"#;
    let after = session.check_project(
        &[
            consumer,
            Source {
                source: defined,
                ..provider
            },
        ],
        None,
    );
    assert!(after.ok && after.complete, "{after:#?}");
    assert_eq!(
        session
            .definition(consumer.path, offset)
            .unwrap()
            .span
            .start
            .line,
        3
    );
}

#[test]
fn literal_vars_preserve_typed_values_in_their_own_and_importing_namespaces() {
    let mut session = Session::default();
    let first = session.check_project(&[APP, CONFIG], None);
    assert!(first.ok && first.complete, "{first:#?}");
    let warm = session.check_project(&[APP, CONFIG], None);
    assert_eq!(warm.stats.reused_functions, 4);

    let edited = CONFIG.source.replace(":limit 4", ":limit \"unbounded\"");
    let changed = session.check_project(
        &[
            APP,
            Source {
                source: &edited,
                ..CONFIG
            },
        ],
        None,
    );
    assert!(!changed.ok && changed.complete, "{changed:#?}");
    assert_eq!(changed.stats.parsed_files, 1);
    let wrong = &changed.files[0].diagnostics[0];
    assert_eq!(wrong.code, "PNX2322");
    assert_eq!(wrong.actual.as_deref(), Some("String"));
    assert!(changed.files[1].ok);

    let added = Source {
        path: "bad.clj",
        source: include_str!("../examples/literal_vars/bad.clj"),
    };
    let private = session.check_project(&[added, CONFIG], None);
    assert!(!private.ok && private.complete, "{private:#?}");
    assert_eq!(private.files[0].diagnostics[0].code, "PNX2304");
}

#[test]
fn literal_vars_shadow_core_builtins_and_unsupported_initializers_are_explicit() {
    let mut session = Session::default();
    let shadowing = r#"
        (ns demo)
        (def inc #{:ready})
        (defn ready {:t/sig '[:-> (clojure.spec.alpha/nilable keyword?)]} []
          (inc :ready))
    "#;
    let report = session.check("shadowing.clj", shadowing);
    assert!(report.ok && report.complete, "{report:#?}");

    let unsupported = r#"
        (ns demo)
        (def ^:dynamic *limit* 1)
        (def computed (inc 1))
        (def unbound)
        (defn unsupported {:t/sig '[:-> number?]} []
          (+ *limit* computed unbound))
    "#;
    let report = session.check("unsupported.clj", unsupported);
    assert!(!report.ok && !report.complete, "{report:#?}");
    assert_eq!(report.diagnostics.len(), 3, "{report:#?}");
    assert!(report.diagnostics.iter().all(|d| d.code == "PNX9001"));
}
