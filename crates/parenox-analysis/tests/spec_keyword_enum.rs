use parenox_analysis::{Session, Source};

fn specs() -> &'static str {
    include_str!("../../../tests/fixtures/spec-keyword-enum.clj")
        .split_once(";; JVM-only")
        .unwrap()
        .0
}

#[test]
fn literal_keywords_validate_and_conform_to_exact_fixed_integers() {
    let uses = r#"
(defn item-kind {:t/sig '[:-> (s/and int? #{25})]} []
  (s/conform :completion-item/kind :typeparameter))
(defn insert-format {:t/sig '[:-> (s/and int? #{2})]} []
  (s/conform :completion-item/insert-text-format :snippet))
(defn zero {:t/sig '[:-> (s/and int? #{0})]} []
  (s/conform ::edge :zero))
(defn zero-valid {:t/sig '[:-> #{true}]} []
  (s/valid? ::edge :zero))
(defn qualified-key {:t/sig '[:-> (s/and int? #{7})]} []
  (s/conform ::edge-alias :foreign/known))
(defn guarded {:t/sig '[keyword? :-> (s/and int? #{0 7})]} [input]
  (if (s/valid? ::edge input) (s/conform ::edge input) 0))
"#;
    let mut source = format!("{}{uses}", specs());
    for (index, input) in [
        ":false",
        ":nil",
        ":marker",
        "::s/invalid",
        ":unlisted",
        ":known",
        ":foreign/unknown",
        "1",
        "\"zero\"",
        "false",
        "nil",
    ]
    .into_iter()
    .enumerate()
    {
        source.push_str(&format!(
            r#"
(defn invalid-{index} {{:t/sig '[:-> #{{::s/invalid}}]}} [] (s/conform ::edge {input}))
(defn invalid-{index}? {{:t/sig '[:-> #{{false}}]}} [] (s/valid? ::edge {input}))
"#
        ));
    }
    let report = Session::default().check("keyword_enum.clj", &source);
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn keyword_enum_calls_keep_overlapping_inputs_and_narrow_guards() {
    let uses = r#"
(s/def ::keyword keyword?)
(defn validated {:t/sig '[#{:zero :unlisted} :-> #{:zero}]} [input]
  (if (s/valid? ::edge input) input :zero))
(defn narrowed-code {:t/sig '[#{:zero :unlisted} :-> (s/and int? #{0})]} [input]
  (if (s/valid? ::edge input) (s/conform ::edge input) 0))
"#;
    let mut source = format!("{}{uses}", specs());
    for (suffix, input_type) in [("core", "keyword?"), ("named", "::keyword")] {
        source.push_str(&format!(
            r#"
(defn valid-{suffix} {{:t/sig '[{input_type} :-> boolean?]}} [input]
  (s/valid? ::edge input))
(defn conform-{suffix}
  {{:t/sig '[{input_type} :-> (s/or :code (s/and int? #{{0 7}}) :invalid #{{::s/invalid}})]}}
  [input] (s/conform ::edge input))
"#
        ));
    }
    let mut session = Session::default();
    let report = session.check("keyword_enum.clj", &source);
    assert!(report.ok && report.complete, "{report:#?}");

    for input_type in ["keyword?", "::keyword"] {
        let source = format!(
            r#"{source}
(defn false-only {{:t/sig '[{input_type} :-> #{{false}}]}} [input]
  (s/valid? ::edge input))
(defn invalid-only {{:t/sig '[{input_type} :-> #{{::s/invalid}}]}} [input]
  (s/conform ::edge input))
(defn success-only {{:t/sig '[{input_type} :-> (s/and int? #{{0 7}})]}} [input]
  (s/conform ::edge input))
(defn missing-zero
  {{:t/sig '[{input_type} :-> (s/or :code (s/and int? #{{7}}) :invalid #{{::s/invalid}})]}}
  [input] (s/conform ::edge input))
(defn missing-known
  {{:t/sig '[{input_type} :-> (s/or :code (s/and int? #{{0}}) :invalid #{{::s/invalid}})]}}
  [input] (s/conform ::edge input))
"#
        );
        let report = session.check("keyword_enum.clj", &source);
        let expected = [
            "false-only",
            "invalid-only",
            "success-only",
            "missing-zero",
            "missing-known",
        ];
        assert_eq!(report.diagnostics.len(), expected.len(), "{report:#?}");
        for (finding, name) in report.diagnostics.iter().zip(expected) {
            assert_eq!(finding.code, "PNX2322", "{input_type}: {finding:#?}");
            assert_eq!(
                finding.contract.as_ref().unwrap().name,
                format!("demo.keyword-enum/{name}"),
                "{input_type}: {finding:#?}"
            );
        }
    }
}

#[test]
fn enum_conversions_rebuild_entries_only_when_a_value_changes() {
    let source = format!(
        r#"{}
(def item {{:label "complete" :kind :method :insert-text-format :snippet}})
(def untouched {{:label "complete"}})
(defn conformed-kind {{:t/sig '[:-> (s/and int? #{{2}})]}} []
  (:kind (s/conform ::completion-item item)))
(defn conformed-format {{:t/sig '[:-> (s/and int? #{{2}})]}} []
  (:insert-text-format (first (s/conform ::completion-items (clojure.lang.MapEntry/create item item)))))
(defn preserved {{:t/sig '[:-> string?]}} []
  (:label (key (s/conform ::completion-items (clojure.lang.MapEntry/create untouched untouched)))))
"#,
        specs()
    );
    let mut session = Session::default();
    let report = session.check("keyword_enum.clj", &source);
    assert!(report.ok && report.complete, "{report:#?}");

    let source = format!(
        r#"{source}
(defn rebuilt {{:t/sig '[:-> any?]}} []
  (key (s/conform ::completion-items (clojure.lang.MapEntry/create item item))))
"#
    );
    let report = session.check("keyword_enum.clj", &source);
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(finding.code, "PNX2322", "{finding:#?}");
    assert!(finding.message.contains("key"), "{finding:#?}");
}

#[test]
fn exact_keyword_enum_domains_round_trip_without_provider_sources() {
    let provider = format!(
        r#"{}
(defn accept {{:t/sig '[::edge :-> keyword?]}} [_] :accepted)
"#,
        specs()
    );
    let export = Session::default().export_externs(
        &[Source {
            path: "provider.clj",
            source: &provider,
        }],
        &[],
    );
    assert!(export.report.ok && export.report.complete, "{export:#?}");
    let edn = export.edn.unwrap();
    let check = |body: &str| {
        let consumer = format!(
            "(ns consumer (:require [clojure.spec.alpha :as s] [demo.keyword-enum :as wire]))\n{body}"
        );
        Session::default().check_project_with_externs(
            &[Source {
                path: "consumer.clj",
                source: &consumer,
            }],
            &[Source {
                path: "provider.edn",
                source: &edn,
            }],
        )
    };
    let accepted = check(
        r#"(defn send {:t/sig '[:-> keyword?]} []
             (wire/accept :zero)
             (wire/accept :foreign/known))"#,
    );
    assert!(accepted.ok && accepted.complete, "{accepted:#?}");

    for input in [":unlisted", ":known", ":marker", "::s/invalid", "1"] {
        let report = check(&format!(
            r#"(defn send {{:t/sig '[:-> keyword?]}} [] (wire/accept {input}))"#
        ));
        let [finding] = report.files[0].diagnostics.as_slice() else {
            panic!("{input}: {report:#?}");
        };
        assert_eq!(finding.code, "PNX2322", "{input}: {finding:#?}");
        assert_eq!(
            finding.contract.as_ref().unwrap().path.as_deref(),
            Some("provider.edn")
        );
    }

    let report = check(r#"(defn send {:t/sig '[any? :-> keyword?]} [x] (wire/accept x))"#);
    let [finding] = report.files[0].diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(finding.code, "PNX9004", "{finding:#?}");
}

#[test]
fn changing_a_code_rechecks_the_unchanged_conformance_consumer() {
    let consumer = Source {
        path: "consumer.clj",
        source: r#"
(ns consumer (:require [clojure.spec.alpha :as s] [demo.keyword-enum]))
(defn code {:t/sig '[:-> (s/and int? #{1})]} []
  (s/conform :completion-item/kind :text))
(defn independent {:t/sig '[:-> keyword?]} [] :stable)
"#,
    };
    let check = |session: &mut Session, provider| {
        session
            .check_project(
                &[
                    Source {
                        path: "provider.clj",
                        source: provider,
                    },
                    consumer,
                ],
                None,
            )
            .files
            .into_iter()
            .find(|file| file.path == consumer.path)
            .unwrap()
    };

    let mut session = Session::default();
    let first = check(&mut session, specs());
    assert!(first.ok && first.complete, "{first:#?}");
    let warm = check(&mut session, specs());
    assert_eq!(warm.stats.reused_functions, 2, "{warm:#?}");

    let updated = specs().replace(":text 1", ":text 17");
    let changed = check(&mut session, &updated);
    let [finding] = changed.diagnostics.as_slice() else {
        panic!("{changed:#?}");
    };
    assert_eq!(finding.code, "PNX2322", "{finding:#?}");
    assert_eq!(finding.actual.as_deref(), Some("17"), "{finding:#?}");
    let fresh = check(&mut Session::default(), &updated);
    assert_eq!(fresh.diagnostics.len(), 1, "{fresh:#?}");
    assert_eq!(fresh.diagnostics[0].message, finding.message);
    assert_eq!(changed.stats.checked_functions, 1, "{changed:#?}");
    assert_eq!(changed.stats.reused_functions, 1, "{changed:#?}");

    let restored = check(&mut session, specs());
    assert!(restored.ok && restored.complete, "{restored:#?}");
}
