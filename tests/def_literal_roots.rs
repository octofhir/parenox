use parenox::{Session, Source};

const PROVIDER: Source<'_> = Source {
    path: "config.clj",
    source: r#"
(ns demo.root-config)
(def ^:private defaults {:mode :safe
                         :options [nil false]
                         :targets #{:local :remote}
                         :label "mesh"})
(def setup {:layers [defaults {:copy defaults}]
            :quoted '(configure defaults)})
"#,
};

const CONSUMER: Source<'_> = Source {
    path: "consumer.clj",
    source: r#"
(ns demo.root-consumer (:require [demo.root-config :as config]))
(defn mode {:t/sig '[:-> #{:safe}]} [] (get-in config/setup [:layers 1 :copy :mode]))
(defn disabled {:t/sig '[:-> #{false}]} [] (get-in config/setup [:layers 0 :options 1]))
(defn absent {:t/sig '[:-> nil?]} [] (get-in config/setup [:layers 1 :copy :options 0]))
(defn quoted-name {:t/sig '[:-> symbol?]} [] (nth (:quoted config/setup) 1))
(defn independent {:t/sig '[:-> number?]} [] 1)
"#,
};

#[test]
fn captures_exact_nil_and_false_from_adjacent_roots() {
    let source = r#"
(ns demo.roots)
(def ^:private nothing nil)
;; Reader discards with inert payloads are allowed between the declarations.
#_(unresolved)
(comment (alter-var-root #'nothing (constantly :unused)))
(def missing nothing)
(def that-text "literal #_ and #=")
(def preserved-text that-text)
(def disabled false)
(def ^:private settings {:nested [disabled {:again disabled}]})
(defn absent {:t/sig '[:-> nil?]} [] missing)
(defn preserved {:t/sig '[:-> #{"literal #_ and #="}]} [] preserved-text)
(defn disabled-copy {:t/sig '[:-> #{false}]} [] (get-in settings [:nested 1 :again]))
"#;
    let report = Session::default().check("roots.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn captures_several_roots_through_consecutive_inert_definitions() {
    let provider = Source {
        path: "settings.clj",
        source: r#"(ns reports.settings)
            (def ^:private limit 80)
            (def ^:private layout :text)
            (def ^:private unrelated 'example)
            (comment (alter-var-root #'limit (constantly "unexecuted")))
            (def options {:layout layout :page [limit {:size limit}] :source 'limit})"#,
    };
    let consumer = Source {
        path: "client.clj",
        source: r#"(ns reports.client (:require [reports.settings :as settings]))
            (defn limit {:t/sig '[:-> number?]} [] (get-in settings/options [:page 1 :size]))
            (defn layout {:t/sig '[:-> #{:text}]} [] (:layout settings/options))
            (defn stable {:t/sig '[:-> boolean?]} [] true)"#,
    };
    let mut session = Session::default();
    let original = session.check_project(&[consumer, provider], None);
    assert!(original.ok && original.complete, "{original:#?}");

    let changed = provider.source.replace("limit 80", "limit \"many\"");
    let edited = [
        consumer,
        Source {
            source: &changed,
            ..provider
        },
    ];
    let warm = session.check_project(&edited, None);
    let [finding] = warm.files[0].diagnostics.as_slice() else {
        panic!("{warm:#?}");
    };
    assert!(warm.complete);
    assert_eq!(finding.code, "PNX2322");
    assert_eq!(finding.actual.as_deref(), Some("String"));
    assert_eq!(
        &consumer.source[finding.span.start.offset..finding.span.end.offset],
        "(get-in settings/options [:page 1 :size])"
    );
    let cold = Session::default().check_project(&edited, None);
    assert_eq!(
        serde_json::to_value(&warm.files[0].diagnostics).unwrap(),
        serde_json::to_value(&cold.files[0].diagnostics).unwrap()
    );
    let restored = session.check_project(&[consumer, provider], None);
    assert!(restored.ok && restored.complete, "{restored:#?}");

    let export = session.export_externs(&[provider], &[]);
    let edn = export.edn.unwrap_or_else(|| panic!("{:#?}", export.report));
    let externs = Source {
        path: "settings.edn",
        source: &edn,
    };
    let isolated = Session::default().check_project_with_externs(&[consumer], &[externs]);
    assert!(isolated.ok && isolated.complete, "{isolated:#?}");
}

#[test]
fn a_root_capture_stops_at_evaluated_forms_and_discarded_reader_effects() {
    let source = r#"(ns reports.settings)
        (def ^:private limit 80)
        BARRIER
        (def ^:private layout :text)
        (def options {:layout layout :limit limit})"#;
    for barrier in [
        r#"(def changed (alter-var-root #'limit (constantly "updated")))"#,
        "#_ #custom/reader :can-change-root",
    ] {
        let input = source.replace("BARRIER", barrier);
        let report = Session::default().check("settings.clj", &input);
        assert!(!report.complete, "{report:#?}");
        assert!(
            report.diagnostics.iter().any(|finding| {
                finding.code == "PNX9001"
                    && &input[finding.span.start.offset..finding.span.end.offset]
                        == "{:layout layout :limit limit}"
            }),
            "{report:#?}"
        );
    }
}

#[test]
fn exports_a_public_value_captured_from_a_private_root() {
    let exported = Session::default().export_externs(&[PROVIDER], &[]);
    let snapshot = exported
        .edn
        .unwrap_or_else(|| panic!("{:#?}", exported.report));
    assert!(snapshot.contains("demo.root-config/setup"), "{snapshot}");
    assert!(
        !snapshot.contains("demo.root-config/defaults"),
        "{snapshot}"
    );
    assert!(snapshot.contains("(configure defaults)"), "{snapshot}");
    assert!(snapshot.contains("#{"), "{snapshot}");

    let externs = Source {
        path: "config.edn",
        source: &snapshot,
    };
    let validation = Session::default().validate_externs(&[PROVIDER], &[externs]);
    assert!(validation.ok && validation.complete, "{validation:#?}");

    let mut client = Session::default();
    let valid = client.check_project_with_externs(&[CONSUMER], &[externs]);
    assert!(valid.ok && valid.complete, "{valid:#?}");
    let incompatible = CONSUMER.source.replace("#{:safe}", "#{:other}");
    let rejected = client.check_project_with_externs(
        &[Source {
            source: &incompatible,
            ..CONSUMER
        }],
        &[externs],
    );
    assert!(!rejected.ok && rejected.complete, "{rejected:#?}");
    let [finding] = rejected.files[0].diagnostics.as_slice() else {
        panic!("{rejected:#?}");
    };
    assert_eq!(finding.code, "PNX2322");
    assert_eq!(finding.actual.as_deref(), Some(":safe"));
}

#[test]
fn editing_the_captured_root_rechecks_only_consumers_of_its_value() {
    let mut session = Session::default();
    let before = session.check_project(&[CONSUMER, PROVIDER], None);
    assert!(before.ok && before.complete, "{before:#?}");
    let cached = session.check_project(&[CONSUMER, PROVIDER], None);
    assert_eq!(cached.files[0].stats.reused_functions, 5, "{cached:#?}");

    let changed = PROVIDER.source.replace(":safe", ":fast");
    let after = session.check_project(
        &[
            CONSUMER,
            Source {
                source: &changed,
                ..PROVIDER
            },
        ],
        None,
    );
    assert!(!after.ok && after.complete, "{after:#?}");
    let report = &after.files[0];
    assert_eq!(report.stats.checked_functions, 4, "{after:#?}");
    assert_eq!(report.stats.reused_functions, 1, "{after:#?}");
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{after:#?}");
    };
    assert_eq!(finding.code, "PNX2322");
    assert_eq!(finding.actual.as_deref(), Some(":fast"));

    let restored = session.check_project(&[CONSUMER, PROVIDER], None);
    assert!(restored.ok && restored.complete, "{restored:#?}");
}

#[test]
fn a_later_data_reader_invalidates_an_earlier_root_for_consumers_and_externs() {
    let provider = Source {
        path: "provider.cljc",
        source: r#"(ns demo.reader-roots)
            (def root "before")
            #?(:clj (def stable [#_:change 7]) :cljs nil)
            #?(:cljs #probe/change :inactive)"#,
    };
    let consumer = Source {
        path: "consumer.clj",
        source: r#"(ns demo.consumer (:require [demo.reader-roots :as p]))
            (defn value {:t/sig '[:-> number?]} [] p/root)
            (defn current {:t/sig '[:-> number?]} [] (first p/stable))"#,
    };
    let mut session = Session::default();
    let before = session.check_project(&[consumer, provider], None);
    let [expectation] = before.files[0].diagnostics.as_slice() else {
        panic!("{before:#?}");
    };
    assert!(before.complete);
    assert_eq!(expectation.code, "PNX2322");
    assert_eq!(
        &consumer.source[expectation.span.start.offset..expectation.span.end.offset],
        "p/root"
    );

    let changed = provider
        .source
        .replace("#_:change", "#_#probe/change :change");
    let edited = Source {
        source: &changed,
        ..provider
    };
    let inputs = [consumer, edited];
    let warm = session.check_project(&inputs, None);
    let cold = Session::default().check_project(&inputs, None);
    let [finding] = warm.files[0].diagnostics.as_slice() else {
        panic!("{warm:#?}");
    };
    assert!(!warm.complete);
    assert_eq!(finding.code, "PNX9004", "{warm:#?}");
    assert_eq!(finding.span, expectation.span);
    assert_eq!(warm.files[0].stats.checked_functions, 1, "{warm:#?}");
    assert_eq!(warm.files[0].stats.reused_functions, 1, "{warm:#?}");
    let diagnostics = |report: &parenox::ProjectReport| {
        report
            .files
            .iter()
            .map(|file| serde_json::to_value(&file.diagnostics).unwrap())
            .collect::<Vec<_>>()
    };
    assert_eq!(diagnostics(&warm), diagnostics(&cold));
    let restored = session.check_project(&[consumer, provider], None);
    assert_eq!(diagnostics(&restored), diagnostics(&before));

    let externs = Source {
        path: "literal-roots.edn",
        source: r#"{demo.reader-roots/root {:value "before"}
                    demo.reader-roots/stable {:value [7]}}"#,
    };
    let good = session.validate_externs(&[provider], &[externs]);
    assert!(good.ok && good.complete, "{good:#?}");
    let unverified = session.validate_externs(&[edited], &[externs]);
    let extern_report = unverified
        .files
        .iter()
        .find(|file| file.path == externs.path)
        .unwrap();
    let [finding] = extern_report.diagnostics.as_slice() else {
        panic!("{unverified:#?}");
    };
    assert_eq!(finding.code, "PNX9011");
    assert!(
        finding.message.contains("later data reader"),
        "{finding:#?}"
    );
}

#[test]
fn captures_literal_name_vectors_and_refreshes_their_consumers() {
    let provider = Source {
        path: "tokens.clj",
        source: r#"(ns demo.tokens (:require [wire.tags :as-alias wire]))
            (def ^:private tokens [::wire/first "keep/slash" :wire//])
            (def names (->> tokens (map name) vec))
            (def modifiers [:definition :implementation])
            (def modifier-names (clojure.core/->> modifiers (clojure.core/map clojure.core/name) clojure.core/vec))"#,
    };
    let consumer = Source {
        path: "consumer.clj",
        source: r#"(ns demo.token-consumer (:require [demo.tokens :as t]))
            (defn check-name {:t/sig '[:-> #{"first"}]} [] (nth t/names 0))
            (defn check-slash {:t/sig '[:-> #{"keep/slash"}]} [] (nth t/names 1))
            (defn check-keyword {:t/sig '[:-> #{"/"}]} [] (nth t/names 2))
            (defn check-modifier {:t/sig '[:-> #{"implementation"}]} [] (nth t/modifier-names 1))"#,
    };
    let mut session = Session::default();
    let baseline = session.check_project(&[consumer, provider], None);
    assert!(baseline.ok && baseline.complete, "{baseline:#?}");

    let changed = provider.source.replace("::wire/first", "::wire/second");
    let inputs = [
        consumer,
        Source {
            source: &changed,
            ..provider
        },
    ];
    let warm = session.check_project(&inputs, None);
    let cold = Session::default().check_project(&inputs, None);
    assert_eq!(
        serde_json::to_value(&warm.files[0].diagnostics).unwrap(),
        serde_json::to_value(&cold.files[0].diagnostics).unwrap(),
    );
    let [finding] = warm.files[0].diagnostics.as_slice() else {
        panic!("{warm:#?}");
    };
    assert_eq!(finding.code, "PNX2322");
    assert_eq!(finding.actual.as_deref(), Some("\"second\""));
    let restored = session.check_project(&[consumer, provider], None);
    assert!(restored.ok && restored.complete, "{restored:#?}");

    let exported = session.export_externs(&[provider], &[]);
    let edn = exported
        .edn
        .unwrap_or_else(|| panic!("{:#?}", exported.report));
    assert!(edn.contains(r#""first" "keep/slash" "/""#), "{edn}");
}

#[test]
fn name_vector_capture_requires_proven_core_operators_and_input() {
    for source in [
        "(ns demo.tokens) (defn name [_] \"replaced\") (def names (->> [:original] (map name) vec))",
        "(ns demo.tokens) (defn vec [_] []) (def names (->> [:original] (map name) vec))",
        "(ns demo.tokens) (defmacro ->> [& _] []) (def names (->> [:original] (map name) vec))",
        "(ns demo.tokens) (defn map [& _] []) (def names (->> [:original] (map name) vec))",
        "(ns demo.tokens) (def input [:original]) (def other 0) (def names (->> input (map name) vec))",
        "(ns demo.tokens) (def input [:original]) #_#custom/tag nil (def names (->> input (map name) vec))",
        "(ns demo.tokens) (def names (->> [nil] (map name) vec))",
    ] {
        assert_gap_at_initializer(source, source.rfind("(->>").unwrap());
    }
}

#[test]
fn unsupported_initializer_paths_keep_a_gap_at_the_captured_definition() {
    for (prefix, initializer) in [
        ("(ns root-cases) (def root :ready)", "root-cases/root"),
        (
            "(ns root-cases) (def root :ready) (alter-var-root #'root (constantly :changed))",
            "root",
        ),
        ("(ns root-cases) (def root :ready)", "{root :value}"),
        ("(ns root-cases) (def root :ready)", "#{root}"),
        ("(ns root-cases) (def root :ready)", "(identity root)"),
        ("(ns root-cases) (def root (identity :ready))", "root"),
        ("(ns root-cases) (def ^:dynamic root :ready)", "root"),
        (
            "(ns root-cases) (def ^{:doc (str \"metadata\")} root :ready)",
            "root",
        ),
        (
            "(ns root-cases) (def root :ready) #_#test/rebind :payload",
            "root",
        ),
        ("(def root :ready)", "root"),
    ] {
        let source = format!("{prefix}\n(def snapshot {initializer})");
        let start = source.len() - initializer.len() - 1;
        assert_gap_at_initializer(&source, start);
    }
}

#[test]
fn forward_references_do_not_capture_later_roots() {
    let source = "(ns root-cases)\n(def snapshot root)\n(def root :ready)";
    let start = source.find("(def snapshot root)").unwrap() + "(def snapshot ".len();
    assert_gap_at_initializer(source, start);
}

#[test]
fn a_capture_with_evaluated_metadata_remains_unchecked() {
    let source = r#"
(ns root-cases)
(def root :ready)
(def ^{:doc (str "metadata")} snapshot root)
"#;
    assert_gap_at_initializer(source, source.rfind("root").unwrap());
}

fn assert_gap_at_initializer(source: &str, start: usize) {
    let report = Session::default().check("root-cases.clj", source);
    assert!(!report.complete, "{source}\n{report:#?}");
    assert!(
        report.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "PNX9001" && diagnostic.span.start.offset == start
        }),
        "{source}\n{report:#?}"
    );
}

#[test]
fn ordinary_source_maps_keep_numeric_lookup_proofs_through_edits() {
    let provider = Source {
        path: "status.clj",
        source: r#"
(ns demo.literal-status)
(def status {1 :hit})
(def nested {:status {1 :hit}})
"#,
    };
    let consumer = Source {
        path: "client.clj",
        source: r#"
(ns demo.literal-client (:require [demo.literal-status :as p]))
(defn direct {:t/sig '[:-> #{:hit}]} [] (get p/status 1N :missing))
(defn nested {:t/sig '[:-> #{:hit}]} [] (get-in p/nested [:status 1N] :missing))
(defn independent {:t/sig '[:-> number?]} [] 7)
"#,
    };
    let mut session = Session::default();
    let before = session.check_project(&[consumer, provider], None);
    assert!(before.ok && before.complete, "{before:#?}");

    let changed = provider.source.replace(":hit", ":changed");
    let inputs = [
        consumer,
        Source {
            source: &changed,
            ..provider
        },
    ];
    let warm = session.check_project(&inputs, None);
    let cold = Session::default().check_project(&inputs, None);
    assert!(!warm.ok && warm.complete, "{warm:#?}");
    assert_eq!(warm.files[0].stats.checked_functions, 2);
    assert_eq!(warm.files[0].stats.reused_functions, 1);
    assert_eq!(
        serde_json::to_value(&warm.files[0].diagnostics).unwrap(),
        serde_json::to_value(&cold.files[0].diagnostics).unwrap()
    );
    let findings = &warm.files[0].diagnostics;
    assert_eq!(findings.len(), 2, "{warm:#?}");
    assert!(findings.iter().all(|finding| {
        finding.code == "PNX2322" && finding.actual.as_deref() == Some(":changed")
    }));
    assert_eq!(
        findings
            .iter()
            .map(|finding| &consumer.source[finding.span.start.offset..finding.span.end.offset])
            .collect::<Vec<_>>(),
        [
            "(get p/status 1N :missing)",
            "(get-in p/nested [:status 1N] :missing)",
        ]
    );

    let restored = session.check_project(&[consumer, provider], None);
    assert!(restored.ok && restored.complete, "{restored:#?}");
}

#[test]
fn sequence_destructuring_keeps_literal_proofs_in_source_and_exported_edn() {
    let provider = Source {
        path: "provider.clj",
        source: r#"
(ns probe.provider)
(def single '({:nested {1 :hit}}))
(def trailing '(:previous false {:nested {1 :hit}}))
"#,
    };
    let consumer = Source {
        path: "consumer.clj",
        source: r#"
(ns probe.consumer (:require [probe.provider :as provider]))
(defn single {:t/sig '[:-> #{:hit}]} []
  (let [{:keys [nested]} provider/single] (get nested 1N)))
(defn trailing {:t/sig '[:-> #{:hit}]} []
  (let [{:all opts} provider/trailing] (get (:nested opts) 1N)))
(defn independent {:t/sig '[:-> number?]} [] 7)
"#,
    };
    let mut session = Session::default();
    let initial = session.check_project(&[consumer, provider], None);
    assert!(initial.ok && initial.complete, "{initial:#?}");

    let edited = provider.source.replace(":hit", ":changed");
    let changed = Source {
        source: &edited,
        ..provider
    };
    let warm = session.check_project(&[consumer, changed], None);
    let cold = Session::default().check_project(&[consumer, changed], None);
    assert!(!warm.ok && warm.complete, "{warm:#?}");
    assert_eq!(warm.files[0].stats.checked_functions, 2);
    assert_eq!(warm.files[0].stats.reused_functions, 1);
    let diagnostics = &warm.files[0].diagnostics;
    assert_eq!(diagnostics.len(), 2, "{warm:#?}");
    assert!(diagnostics.iter().all(|finding| {
        finding.code == "PNX2322" && finding.actual.as_deref() == Some(":changed")
    }));
    assert_eq!(
        serde_json::to_value(diagnostics).unwrap(),
        serde_json::to_value(&cold.files[0].diagnostics).unwrap()
    );
    assert!(session.check_project(&[consumer, provider], None).ok);

    let exported = Session::default().export_externs(&[provider], &[]);
    let externs = Source {
        path: "provider.edn",
        source: exported.edn.as_deref().unwrap(),
    };
    let without_source = Session::default().check_project_with_externs(&[consumer], &[externs]);
    assert!(
        without_source.ok && without_source.complete,
        "{without_source:#?}"
    );
}

#[test]
fn explicit_quote_can_initialize_an_adjacent_literal_root() {
    let source = Source {
        path: "explicit-root-quote.clj",
        source: r#"
(ns reader.explicit-root-quote)
(def options (quote [{:nested {1 :hit}}]))
(def copy options)
(defn sample {:t/sig '[:-> #{:hit}]} []
  (get-in copy [0 :nested 1N]))
"#,
    };
    let mut session = Session::default();
    let report = session.check(source.path, source.source);
    assert!(report.ok && report.complete, "{report:#?}");

    let exported = session.export_externs(&[source], &[]);
    let externs = exported
        .edn
        .as_deref()
        .expect("the literal snapshot must export");
    assert!(
        externs.contains("reader.explicit-root-quote/copy {:value [{:nested {1 :hit}}]}"),
        "{externs}"
    );
}

#[test]
fn platform_long_initializers_track_competing_namespaces_and_export_as_values() {
    let reader = Source {
        path: "platform-root.clj",
        source: r#"
(ns demo.platform-root)
(def ^:const max-size Long/MAX_VALUE)
(def ^:const min-size java.lang.Long/MIN_VALUE)
(defn bounds {:t/sig '[:-> #{true}]} []
  (and (= max-size 9223372036854775807)
       (= min-size -9223372036854775808)))
"#,
    };
    let mut session = Session::default();
    let initial = session.check_project(&[reader], None);
    assert!(initial.ok && initial.complete, "{initial:#?}");

    let competing = Source {
        path: "long.clj",
        source: "(ns Long) (def MAX_VALUE :shadowing-var)",
    };
    let edited_inputs = [reader, competing];
    let warm = session.check_project(&edited_inputs, None);
    let cold = Session::default().check_project(&edited_inputs, None);
    assert!(!warm.ok && !warm.complete, "{warm:#?}");
    let diagnostics = &warm.files[0].diagnostics;
    let gap = diagnostics
        .iter()
        .find(|finding| finding.code == "PNX9001")
        .unwrap();
    assert_eq!(
        &reader.source[gap.span.start.offset..gap.span.end.offset],
        "Long/MAX_VALUE"
    );
    assert_eq!(
        serde_json::to_value(diagnostics).unwrap(),
        serde_json::to_value(&cold.files[0].diagnostics).unwrap()
    );
    let restored = session.check_project(&[reader], None);
    assert!(restored.ok && restored.complete, "{restored:#?}");

    let exported = session.export_externs(&[reader], &[]);
    let externs = exported.edn.as_deref().expect("constant roots must export");
    assert!(externs.contains("demo.platform-root/max-size {:value 9223372036854775807}"));
    assert!(externs.contains("demo.platform-root/min-size {:value -9223372036854775808}"));
}

#[test]
fn adjacent_aliases_keep_folded_platform_roots_and_export_their_values() {
    let source = Source {
        path: "adjacent-folded-root.clj",
        source: r#"
(ns reader.adjacent-folded-root)
(def ^:const earliest Long/MIN_VALUE)
(def saved earliest)
(def forwarded saved)
(defn matches-jvm {:t/sig '[:-> #{true}]} []
  (= forwarded -9223372036854775808))
"#,
    };
    let mut session = Session::default();
    let checked = session.check(source.path, source.source);
    assert!(checked.ok && checked.complete, "{checked:#?}");
    let exported = session.export_externs(&[source], &[]);
    let edn = exported
        .edn
        .as_deref()
        .unwrap_or_else(|| panic!("{exported:#?}"));
    for name in ["earliest", "saved", "forwarded"] {
        assert!(
            edn.contains(&format!(
                "reader.adjacent-folded-root/{name} {{:value -9223372036854775808}}"
            )),
            "{edn}"
        );
    }
}

#[test]
fn a_proven_platform_alias_keeps_later_initializer_folds_available() {
    let source = Source {
        path: "folded-prefix.clj",
        source: r#"
(ns reader.folded-prefix)
(def ^:const min-value Long/MIN_VALUE)
(def saved-min min-value)
(def ^:const max-value Long/MAX_VALUE)
(def label (str "min" "max"))
(defn matches-jvm {:t/sig '[:-> #{true}]} []
  (= max-value 9223372036854775807))
(defn folded-label {:t/sig '[:-> #{"minmax"}]} [] label)"#,
    };
    let mut session = Session::default();
    let original = session.check(source.path, source.source);
    assert!(original.ok && original.complete, "{original:#?}");

    let opaque = source.source.replace(
        "(def saved-min min-value)",
        "(def saved-min (identity min-value))",
    );
    let warm = session.check(source.path, &opaque);
    let cold = Session::default().check(source.path, &opaque);
    assert_eq!(
        serde_json::to_value(&warm.diagnostics).unwrap(),
        serde_json::to_value(&cold.diagnostics).unwrap()
    );
    assert!(!warm.complete, "{warm:#?}");
    assert!(
        warm.diagnostics.iter().any(|finding| {
            finding.code == "PNX9001"
                && &opaque[finding.span.start.offset..finding.span.end.offset] == "Long/MAX_VALUE"
        }),
        "{warm:#?}"
    );

    let reverted = session.check(source.path, source.source);
    assert!(reverted.ok && reverted.complete, "{reverted:#?}");
    let exported = session.export_externs(&[source], &[]);
    let edn = exported
        .edn
        .unwrap_or_else(|| panic!("{:#?}", exported.report));
    assert!(edn.contains("reader.folded-prefix/max-value {:value 9223372036854775807}"));
    assert!(edn.contains("reader.folded-prefix/label {:value \"minmax\"}"));
}

#[test]
fn bare_alias_chains_preserve_a_shared_literal_value_and_its_edn() {
    let provider = Source {
        path: "threading.clj",
        source: r#"(ns demo.threading)
(def ^:private first-ops #{'-> 'some->})
(def saved first-ops)
(def exported saved)"#,
    };
    let consumer = Source {
        path: "use_threading.clj",
        source: r#"(ns demo.use-threading (:require [demo.threading :as t]))
(defn accepts-first {:t/sig '[:-> #{true}]} [] (contains? t/exported '->))
(defn unchanged {:t/sig '[:-> string?]} [] "fixed")"#,
    };
    let mut session = Session::default();
    let first = session.check_project(&[consumer, provider], None);
    assert!(first.ok && first.complete, "{first:#?}");

    let changed = provider.source.replace("'-> ", "'->> ");
    let edited = Source {
        source: &changed,
        ..provider
    };
    let warm = session.check_project(&[consumer, edited], None);
    let cold = Session::default().check_project(&[consumer, edited], None);
    assert!(!warm.ok && warm.complete, "{warm:#?}");
    assert_eq!(warm.stats.checked_functions, 1);
    assert_eq!(warm.stats.reused_functions, 1);
    assert_eq!(
        serde_json::to_value(&warm.files[0].diagnostics).unwrap(),
        serde_json::to_value(&cold.files[0].diagnostics).unwrap()
    );
    let [finding] = warm.files[0].diagnostics.as_slice() else {
        panic!("{warm:#?}");
    };
    assert_eq!(finding.actual.as_deref(), Some("false"));
    let reverted = session.check_project(&[consumer, provider], None);
    assert!(reverted.ok && reverted.complete, "{reverted:#?}");

    let exported = session.export_externs(&[provider], &[]);
    let edn = exported
        .edn
        .unwrap_or_else(|| panic!("{:#?}", exported.report));
    assert!(edn.contains("demo.threading/exported {:value #{-> some->}}"));
    let consumed = Session::default().check_project_with_externs(
        &[consumer],
        &[Source {
            path: "threading.edn",
            source: &edn,
        }],
    );
    assert!(consumed.ok && consumed.complete, "{consumed:#?}");
}

#[test]
fn literal_set_unions_use_the_standard_namespace_and_export_as_data() {
    let source = Source {
        path: "ops.clj",
        source: r#"(ns demo.ops (:require [clojure.set :as set]))
(def common '#{defn def deftest})
(def forbidden (set/union common '#{if deftest :import}))
(def first-ops #{'-> 'some->})
(def last-ops #{'->> 'some->>})
(def operators (set/union first-ops last-ops))
(defn has-import? {:t/sig '[:-> #{true}]} [] (contains? forbidden :import))
(defn has-last? {:t/sig '[:-> #{true}]} [] (contains? operators 'some->>))"#,
    };
    let mut session = Session::default();
    let ready = session.check_project(&[source], None);
    assert!(ready.ok && ready.complete, "{ready:#?}");
    let edn = session.export_externs(&[source], &[]).edn.unwrap();
    assert!(edn.contains("demo.ops/forbidden {:value #{:import def defn deftest if}}"));
    assert!(edn.contains("demo.ops/operators {:value #{-> ->> some-> some->>}}"));

    let competing = Source {
        path: "set.clj",
        source: "(ns clojure.set) (defn union {:t/sig '[any? any? :-> any?]} [x y] #{})",
    };
    let warm = session.check_project(&[source, competing], None);
    let cold = Session::default().check_project(&[source, competing], None);
    assert!(!warm.complete, "{warm:#?}");
    assert_eq!(
        serde_json::to_value(&warm.files[0].diagnostics).unwrap(),
        serde_json::to_value(&cold.files[0].diagnostics).unwrap()
    );
    assert!(
        warm.files[0].diagnostics.iter().any(|finding| {
            finding.code == "PNX9001"
                && &source.source[finding.span.start.offset..finding.span.end.offset]
                    == "(set/union common '#{if deftest :import})"
        }),
        "{warm:#?}"
    );
    let restored = session.check_project(&[source], None);
    assert!(restored.ok && restored.complete, "{restored:#?}");

    let changed = source.source.replace(
        "(def operators",
        "(alter-var-root #'last-ops (constantly #{}))\n(def operators",
    );
    let report = session.check(source.path, &changed);
    assert!(
        report.diagnostics.iter().any(|finding| {
            finding.code == "PNX9001"
                && &changed[finding.span.start.offset..finding.span.end.offset]
                    == "(set/union first-ops last-ops)"
        }),
        "{report:#?}"
    );

    let numeric =
        "(ns demo.ops (:require [clojure.set :as set])) (def values (set/union #{1/2} #{1}))";
    let report = session.check("numeric.clj", numeric);
    assert!(
        report.diagnostics.iter().any(|finding| {
            finding.code == "PNX9001"
                && &numeric[finding.span.start.offset..finding.span.end.offset]
                    == "(set/union #{1/2} #{1})"
        }),
        "{report:#?}"
    );
}

#[test]
fn numeric_set_unions_keep_the_stored_object_from_the_selected_input() {
    let provider = Source {
        path: "numeric.clj",
        source: r#"(ns demo.numeric (:require [clojure.set :as set]))
(def larger-right (set/union #{1N} #{1 :extra}))
(def larger-left (set/union #{1N :extra} #{1}))
(def left-tie (set/union #{1N} #{1}))"#,
    };
    let consumer = Source {
        path: "uses.clj",
        source: r#"(ns demo.uses (:require [demo.numeric :as n] [clojure.spec.alpha :as s]))
(defn chosen {:t/sig '[:-> #{true}]} [] (int? (get n/larger-right 1N)))
(defn unchanged {:t/sig '[:-> (s/tuple #{false} #{false})]} []
  [(int? (get n/larger-left 1)) (int? (get n/left-tie 1))])"#,
    };
    let mut session = Session::default();
    let original = session.check_project(&[consumer, provider], None);
    assert!(original.ok && original.complete, "{original:#?}");

    let changed = provider
        .source
        .replace("#{1N} #{1 :extra}", "#{1N :extra} #{1}");
    let edited = Source {
        source: &changed,
        ..provider
    };
    let warm = session.check_project(&[consumer, edited], None);
    let cold = Session::default().check_project(&[consumer, edited], None);
    assert!(!warm.ok && warm.complete, "{warm:#?}");
    assert_eq!(warm.stats.checked_functions, 1);
    assert_eq!(warm.stats.reused_functions, 1);
    assert_eq!(
        serde_json::to_value(&warm.files[0].diagnostics).unwrap(),
        serde_json::to_value(&cold.files[0].diagnostics).unwrap()
    );
    let [finding] = warm.files[0].diagnostics.as_slice() else {
        panic!("{warm:#?}");
    };
    assert_eq!(finding.code, "PNX2322");
    assert_eq!(finding.actual.as_deref(), Some("false"));
    assert_eq!(
        &consumer.source[finding.span.start.offset..finding.span.end.offset],
        "(int? (get n/larger-right 1N))"
    );

    let restored = session.check_project(&[consumer, provider], None);
    assert!(restored.ok && restored.complete, "{restored:#?}");
    let edn = session.export_externs(&[provider], &[]).edn.unwrap();
    assert!(edn.contains("demo.numeric/larger-right {:value #{1 :extra}}"));
    assert!(edn.contains("demo.numeric/larger-left {:value #{1N :extra}}"));
    assert!(edn.contains("demo.numeric/left-tie {:value #{1N}}"));
    let report = Session::default().check_project_with_externs(
        &[consumer],
        &[Source {
            path: "numeric.edn",
            source: &edn,
        }],
    );
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn an_inert_bare_alias_keeps_later_source_folds_available() {
    let source = r#"(ns demo.folded-alias (:require [clojure.set :as set]))
(def base #{:base})
(def copied base)
(def combined (set/union base #{:more}))
(def label (str "base" "more"))
(defn required-entry {:t/sig '[:-> #{true}]} [] (contains? combined :more))
(defn folded-name {:t/sig '[:-> #{"basemore"}]} [] label)"#;
    let mut session = Session::default();
    let report = session.check("folded-alias.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");

    let changed = source.replace("(def copied base)", "(def copied (identity base))");
    let report = session.check("folded-alias.clj", &changed);
    assert!(
        report.diagnostics.iter().any(|finding| {
            finding.code == "PNX9001"
                && &changed[finding.span.start.offset..finding.span.end.offset]
                    == "(set/union base #{:more})"
        }),
        "{report:#?}"
    );
}

#[test]
fn literal_set_evidence_is_cached_and_transferred_through_edn() {
    let provider = Source {
        path: "provider.clj",
        source: r#"
(ns proof.provider)
(def members #{1 2})
"#,
    };
    let consumer = Source {
        path: "consumer.clj",
        source: r#"
(ns proof.consumer (:require [proof.provider :as provider]))
(defn seen? {:t/sig '[:-> #{true}]} [] (contains? provider/members 1N))
(defn unrelated {:t/sig '[:-> number?]} [] 7)
"#,
    };
    let mut session = Session::default();
    let initial = session.check_project(&[consumer, provider], None);
    assert!(initial.ok && initial.complete, "{initial:#?}");

    let reordered = provider.source.replace("#{1 2}", "#{2 1}");
    let equivalent = Source {
        source: &reordered,
        ..provider
    };
    let same = session.check_project(&[consumer, equivalent], None);
    assert!(same.ok && same.complete, "{same:#?}");
    assert_eq!(same.files[0].stats.reused_functions, 2);

    let shifted = provider.source.replace("#{1 2}", "#{2 3}");
    let changed = Source {
        source: &shifted,
        ..provider
    };
    let edited = [consumer, changed];
    let warm = session.check_project(&edited, None);
    let cold = Session::default().check_project(&edited, None);
    assert!(!warm.ok && warm.complete, "{warm:#?}");
    assert_eq!(warm.files[0].stats.checked_functions, 1);
    assert_eq!(warm.files[0].stats.reused_functions, 1);
    let [finding] = warm.files[0].diagnostics.as_slice() else {
        panic!("{warm:#?}");
    };
    assert_eq!(finding.code, "PNX2322");
    assert_eq!(finding.actual.as_deref(), Some("false"));
    assert_eq!(
        &consumer.source[finding.span.start.offset..finding.span.end.offset],
        "(contains? provider/members 1N)"
    );
    assert_eq!(
        serde_json::to_value(&warm.files[0].diagnostics).unwrap(),
        serde_json::to_value(&cold.files[0].diagnostics).unwrap()
    );
    assert!(session.check_project(&[consumer, provider], None).ok);

    let exported = session.export_externs(&[provider], &[]);
    let externs = Source {
        path: "provider.edn",
        source: exported.edn.as_deref().unwrap(),
    };
    let mut imported = Session::default();
    let initial = imported.check_project_with_externs(&[consumer], &[externs]);
    assert!(initial.ok && initial.complete, "{initial:#?}");
    let shifted = externs.source.replace("#{1 2}", "#{2 3}");
    let changed = Source {
        source: &shifted,
        ..externs
    };
    let warm = imported.check_project_with_externs(&[consumer], &[changed]);
    let cold = Session::default().check_project_with_externs(&[consumer], &[changed]);
    assert!(!warm.ok && warm.complete, "{warm:#?}");
    assert_eq!(warm.files[0].stats.checked_functions, 1);
    assert_eq!(warm.files[0].stats.reused_functions, 1);
    assert_eq!(
        serde_json::to_value(&warm.files[0].diagnostics).unwrap(),
        serde_json::to_value(&cold.files[0].diagnostics).unwrap()
    );
    assert!(
        imported
            .check_project_with_externs(&[consumer], &[externs])
            .ok
    );
}
