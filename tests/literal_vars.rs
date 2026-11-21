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

#[test]
fn folds_stock_str_initializers_in_definition_order() {
    let source = r#"(ns demo.text (:require [wire.tags :as-alias wire]))
(def jiofile-key (str "prefix😀" ::wire/file nil))
(defn str {:t/sig '[t/any :-> keyword?]} [_] :shadow)
(def qualified (clojure.core/str ::file))
(def shadowed (str ::file))
(defn invalid {:t/sig '[:-> number?]} [] (inc jiofile-key))
"#;
    let mut session = Session::default();
    let report = session.check("text.clj", source);
    assert_eq!(report.diagnostics.len(), 2, "{report:#?}");
    let [unsupported, mismatch] = report.diagnostics.as_slice() else {
        unreachable!();
    };
    assert_eq!(unsupported.code, "PNX9001");
    assert_eq!(
        &source[unsupported.span.start.offset..unsupported.span.end.offset],
        "(str ::file)"
    );
    assert_eq!(mismatch.code, "PNX2322");
    assert_eq!(
        &source[mismatch.span.start.offset..mismatch.span.end.offset],
        "jiofile-key"
    );
    for (var, value) in [
        ("jiofile-key", "\"prefix😀:wire.tags/file\""),
        ("qualified (", "\":demo.text/file\""),
    ] {
        let offset = source.find(var).unwrap();
        let hover = session.hover_at("text.clj", offset).unwrap();
        assert_eq!(hover.value_type.as_deref(), Some(value));
    }

    let declared_twice = "(ns demo.text)\n(declare str)\n(def key (str ::file))\n(declare str)";
    let report = session.check("text.clj", declared_twice);
    assert!(report.diagnostics.iter().any(|finding| {
        finding.code == "PNX9001"
            && &declared_twice[finding.span.start.offset..finding.span.end.offset] == "(str ::file)"
    }));
}

#[test]
fn does_not_fold_str_after_unchecked_top_level_effects() {
    let mut session = Session::default();
    for prefix in [
        "",
        "(ns demo.generated) (defmacro install [] '(def str (fn [& _] \"override\"))) (install)",
        "(ns demo.generated) (do (defn str [& _] \"override\"))",
        "(ns demo.generated) (def replacement (do (defn str [& _] \"override\")))",
        "(ns demo.generated) (defn replacement [& _] \"override\")
         (def ^{:on-load (.refer *ns* 'str #'replacement)} marker 0)",
        "(ns demo.generated) (defn replacement [& _] \"override\")
         (defn marker {:on-load (.refer *ns* 'str #'replacement)} [] nil)",
    ] {
        let source = format!("{prefix}\n(def token (str ::file))");
        let report = session.check("effects.clj", &source);
        assert!(
            report.diagnostics.iter().any(|finding| {
                finding.code == "PNX9001"
                    && &source[finding.span.start.offset..finding.span.end.offset] == "(str ::file)"
            }),
            "{source}\n{report:#?}"
        );
        let hover = session.hover_at("effects.clj", source.find("token").unwrap());
        assert!(
            hover.and_then(|hover| hover.value_type).is_none(),
            "{source}"
        );
    }
}

#[test]
fn imported_macro_changes_invalidate_a_later_str_root() {
    let source = Source {
        path: "consumer.clj",
        source: r#"(ns demo.consumer (:require [demo.provider :as p]))
            (defn marker {:t/sig '[:-> nil?]} [] (p/install))
            (def token (str ::file))"#,
    };
    let provider = Source {
        path: "provider.clj",
        source: "(ns demo.provider) (defn install {:t/sig '[:-> nil?]} [] nil)",
    };
    let expanded = Source {
        source: r#"(ns demo.provider)
            (defn replacement [& _] "override")
            (defmacro install [] (.refer *ns* 'str #'replacement) nil)"#,
        ..provider
    };
    let extern_macro = Source {
        path: "vendor.edn",
        source: "{demo.provider/install {:macro :let-like}}",
    };
    let mut session = Session::default();
    let offset = source.source.find("token").unwrap();
    for (inputs, externs, expected) in [
        (
            &[source, provider][..],
            &[][..],
            Some("\":demo.consumer/file\""),
        ),
        (&[source, expanded][..], &[][..], None),
        (
            &[source, provider][..],
            &[][..],
            Some("\":demo.consumer/file\""),
        ),
        (&[source][..], &[extern_macro][..], None),
    ] {
        let report = session.check_project_with_externs(inputs, externs);
        let hover = session.hover_at(source.path, offset).unwrap();
        assert_eq!(hover.value_type.as_deref(), expected, "{report:#?}");
    }
}

#[test]
fn folded_str_initializers_round_trip_as_literal_externs() {
    let provider = Source {
        path: "pod.clj",
        source: "(ns demo.pod)\n(def jiofile-key (str ::file))\n(def empty-text (str))\n",
    };
    let generated = Session::default().export_externs(&[provider], &[]);
    let encoded = generated
        .edn
        .unwrap_or_else(|| panic!("{:#?}", generated.report));
    assert!(encoded.contains("\":demo.pod/file\""), "{encoded}");
    let externs = Source {
        path: "pod.edn",
        source: &encoded,
    };
    let report = Session::default().validate_externs(&[provider], &[externs]);
    assert!(report.ok && report.complete, "{report:#?}");
}
