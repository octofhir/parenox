use parenox_analysis::{Report, Session, Source};

fn specs() -> &'static str {
    include_str!("../../../tests/fixtures/spec-inverse-enum.clj")
        .split_once(";; JVM-only")
        .unwrap()
        .0
}

fn check(body: &str) -> Report {
    Session::default().check("inverse_enum.clj", &format!("{}{body}", specs()))
}

#[test]
fn inverse_enums_accept_equivalent_integers_and_reject_other_values() {
    let mut uses = r#"
(defn large-kind {:t/sig '[:-> #{:typeparameter}]} []
  (s/conform :input.completion-item/kind 25N))
(defn boxed-format {:t/sig '[:-> #{:plaintext}]} []
  (s/conform ::format (s/conform ::boxed-code 1N)))
(defn zero {:t/sig '[:-> #{:zero}]} [] (s/conform ::edge 0N))
(defn zero-valid {:t/sig '[:-> #{true}]} [] (s/valid? ::edge 0))
(defn negative {:t/sig '[:-> #{:operator/negative}]} [] (s/conform ::edge -2N))
(defn invalid-output {:t/sig '[:-> #{false}]} [] (s/valid? ::edge 9))
(defn ordinary-marker {:t/sig '[:-> #{::s/invalid}]} [] (get edge-by-code 9))
(defn fixed-format {:t/sig '[:-> #{:plaintext}]} [] (s/conform ::fixed-format 1))
(defn fixed-rejects-bigint {:t/sig '[:-> #{::s/invalid}]} [] (s/conform ::fixed-format 1N))
"#
    .to_owned();
    for (index, input) in [
        "1",
        "1N",
        "2/2",
        "9223372036854775808/9223372036854775808",
        "0x1",
        "01",
        "2r1",
    ]
    .into_iter()
    .enumerate()
    {
        uses.push_str(&format!(
            r#"
(defn valid-{index} {{:t/sig '[:-> #{{true}}]}} [] (s/valid? :input.completion-item/kind {input}))
(defn conformed-{index} {{:t/sig '[:-> #{{:plaintext}}]}} [] (s/conform ::format {input}))
"#
        ));
    }
    for (index, input) in [
        "::s/invalid",
        "26N",
        "2147483648",
        "9223372036854775808N",
        "0",
        "-1",
        "1.0",
        "1M",
        "3/2",
        "nil",
        "false",
        "\"1\"",
    ]
    .into_iter()
    .enumerate()
    {
        uses.push_str(&format!(
            r#"
(defn invalid-{index} {{:t/sig '[:-> #{{::s/invalid}}]}} []
  (s/conform :input.completion-item/kind {input}))
(defn invalid-{index}? {{:t/sig '[:-> #{{false}}]}} []
  (s/valid? :input.completion-item/kind {input}))
"#
        ));
    }
    let report = check(&uses);
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn broad_inverse_enums_keep_possible_outputs_and_narrow_valid_input() {
    let uses = r#"
(defn broad-integer {:t/sig '[integer? :-> #{:plaintext :snippet ::s/invalid}]} [x]
  (s/conform ::format x))
(defn broad-any {:t/sig '[any? :-> #{:plaintext :snippet ::s/invalid}]} [x]
  (s/conform ::format x))
(defn broad-keyword {:t/sig '[keyword? :-> #{::s/invalid}]} [x]
  (s/conform ::format x))
(defn guarded {:t/sig '[number? :-> (s/and integer? #{1 2})]} [x]
  (if (s/valid? ::format x) x 1N))
(defn finite-guarded {:t/sig '[(s/and integer? #{1 9}) :-> #{:plaintext}]} [x]
  (if (s/valid? ::format x) (s/conform ::format x) :plaintext))
"#;
    let report = check(uses);
    assert!(report.ok && report.complete, "{report:#?}");

    let report = check(&format!(
        r#"{uses}
(defn always-invalid {{:t/sig '[integer? :-> #{{false}}]}} [x] (s/valid? ::format x))
(defn always-valid {{:t/sig '[integer? :-> #{{true}}]}} [x] (s/valid? ::format x))
(defn only-failure {{:t/sig '[any? :-> #{{::s/invalid}}]}} [x] (s/conform ::format x))
(defn only-success {{:t/sig '[any? :-> #{{:plaintext :snippet}}]}} [x] (s/conform ::format x))
(defn no-snippet {{:t/sig '[integer? :-> #{{:plaintext ::s/invalid}}]}} [x] (s/conform ::format x))
(defn no-plaintext {{:t/sig '[integer? :-> #{{:snippet ::s/invalid}}]}} [x] (s/conform ::format x))
"#
    ));
    let expected = [
        "always-invalid",
        "always-valid",
        "only-failure",
        "only-success",
        "no-snippet",
        "no-plaintext",
    ];
    assert_eq!(report.diagnostics.len(), expected.len(), "{report:#?}");
    for (finding, name) in report.diagnostics.iter().zip(expected) {
        assert_eq!(finding.code, "PNX2322", "{finding:#?}");
        assert_eq!(
            finding.contract.as_ref().unwrap().name,
            format!("demo.inverse-enum/{name}"),
            "{finding:#?}"
        );
    }
}

#[test]
fn inverse_enums_convert_the_fields_and_rebuild_changed_map_entries() {
    let uses = r#"
(def item {:kind 1N :insert-text-format 4/2 :extra true})
(def unchanged {:extra true})
(defn result {:t/sig '[:-> (s/tuple #{:text} #{:snippet} #{true})]} []
  (let [row (s/conform ::input-item item)]
    [(:kind row) (:insert-text-format row) (:extra row)]))
(defn rejected {:t/sig '[:-> #{::s/invalid}]} [] (s/conform ::input-item {:kind 1.0}))
(defn changed {:t/sig '[:-> #{:text}]} []
  (:kind (first (s/conform ::input-items (clojure.lang.MapEntry/create item item)))))
(defn unchanged-entry {:t/sig '[:-> #{true}]} []
  (:extra (key (s/conform ::input-items (clojure.lang.MapEntry/create unchanged {})))))
"#;
    let report = check(uses);
    assert!(report.ok && report.complete, "{report:#?}");

    let report = check(&format!(
        r#"{uses}
(defn changed-entry {{:t/sig '[:-> any?]}} []
  (key (s/conform ::input-items (clojure.lang.MapEntry/create item item))))
"#
    ));
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(finding.code, "PNX2322", "{finding:#?}");
    assert!(finding.message.contains("key"), "{finding:#?}");
}

#[test]
fn source_free_inverse_and_fixed_domains_keep_distinct_integer_rules() {
    let provider = format!(
        r#"{}
(defn inverse {{:t/sig '[::format :-> keyword?]}} [_] :accepted)
(defn explicit {{:t/sig '[(s/and integer? #{{1 2}}) :-> keyword?]}} [_] :accepted)
(defn fixed {{:t/sig '[::fixed-format :-> keyword?]}} [_] :accepted)
"#,
        specs()
    );
    let provider = Source {
        path: "provider.clj",
        source: &provider,
    };
    let mut session = Session::default();
    let exported = session.export_externs(&[provider], &[]);
    assert!(
        exported.report.ok && exported.report.complete,
        "{exported:#?}"
    );
    let edn = exported.edn.unwrap();
    let externs = Source {
        path: "provider.edn",
        source: &edn,
    };
    let validated = session.validate_externs(&[provider], &[externs]);
    assert!(validated.ok && validated.complete, "{validated:#?}");
    let check = |name, input| {
        let consumer = format!(
            r#"(ns consumer (:require [clojure.spec.alpha :as s] [demo.inverse-enum :as wire]))
(defn sends {{:t/sig '[:-> keyword?]}} [] (wire/{name} {input}))
(defn marker {{:t/sig '[:-> #{{::s/invalid}}]}} [] (get wire/edge-by-code 9))
"#
        );
        Session::default().check_project_with_externs(
            &[Source {
                path: "consumer.clj",
                source: &consumer,
            }],
            &[externs],
        )
    };
    for (name, accepted) in [
        ("inverse", &["1", "1N", "2/2", "2N"][..]),
        ("explicit", &["1", "1N", "2/2", "2N"][..]),
        ("fixed", &["1", "2"][..]),
    ] {
        for &input in accepted {
            let report = check(name, input);
            assert!(report.ok && report.complete, "{name} {input}: {report:#?}");
        }
    }
    for (name, input) in [
        ("inverse", "1M"),
        ("inverse", "3N"),
        ("explicit", "3/2"),
        ("explicit", "3"),
        ("fixed", "1N"),
    ] {
        let report = check(name, input);
        let [finding] = report.files[0].diagnostics.as_slice() else {
            panic!("{name} {input}: {report:#?}");
        };
        assert_eq!(finding.code, "PNX2322", "{finding:#?}");
        assert_eq!(
            finding.contract.as_ref().unwrap().path.as_deref(),
            Some("provider.edn")
        );
    }
}

#[test]
fn inverse_enums_require_an_earlier_unambiguous_loaded_table() {
    let forward = "(def formats {:plaintext 1 :snippet 2})\n";
    let uses = r#"
(defn formatted {:t/sig '[:-> any?]} []
  (s/conform :input.completion-item/insert-text-format 1))
"#;
    for source in [
        specs().replace(":plaintext 1 :snippet 2", ":plaintext 1 :snippet 1"),
        specs().replace("(def formats {", "(def ^:dynamic formats {"),
        specs().replace("(def formats-by-code ", "(def ^:dynamic formats-by-code "),
        specs().replace("[clojure.set :as set]", "[clojure.set :as-alias set]"),
        specs().replace(
            "(def formats {:plaintext 1 :snippet 2})\n(def formats-by-code (clojure.set/map-invert formats))",
            "(def formats-by-code (clojure.set/map-invert formats))\n(def formats {:plaintext 1 :snippet 2})",
        ),
        format!("{forward}{}", specs().replace(forward, "")),
        format!("{}\n(def formats {{:other 1}})", specs()),
    ] {
        let source = format!("{source}{uses}");
        let report = Session::default().check("inverse_enum.clj", &source);
        assert!(!report.complete, "{report:#?}");
        let targeted: Vec<_> = report
            .diagnostics
            .iter()
            .filter(|finding| finding.message.contains("supported conversion"))
            .collect();
        let [finding] = targeted.as_slice() else {
            panic!("{report:#?}");
        };
        assert_eq!(finding.code, "PNX9001", "{finding:#?}");
        assert_eq!(
            &source[finding.span.start.offset..finding.span.end.offset],
            ":input.completion-item/insert-text-format"
        );
    }
}

#[test]
fn literal_inversion_requires_a_checked_reader_prefix() {
    let provider = Source {
        path: "levels.clj",
        source: r#"(ns demo.levels (:require [clojure.set :as set]))
(def level->severity {:error 1 :warning 2 :info 3})
#_:inert
(def severity->level (set/map-invert level->severity))"#,
    };
    let consumer = Source {
        path: "consumer.clj",
        source: r#"(ns consumer (:require [demo.levels :as levels]))
(defn level {:t/sig '[:-> #{:trace}]} [] (get levels/severity->level 1))"#,
    };
    let mut session = Session::default();
    let baseline = session.check_project(&[consumer, provider], None);
    let [finding] = baseline.files[0].diagnostics.as_slice() else {
        panic!("{baseline:#?}");
    };
    assert!(baseline.complete);
    assert_eq!(finding.code, "PNX2322");
    assert_eq!(finding.actual.as_deref(), Some(":error"));

    let changed = provider
        .source
        .replace("#_:inert", "#_#probe/change :inert");
    let inputs = [
        consumer,
        Source {
            source: &changed,
            ..provider
        },
    ];
    let warm = session.check_project(&inputs, None);
    let cold = Session::default().check_project(&inputs, None);
    assert!(!warm.complete);
    assert_eq!(warm.files[0].diagnostics.len(), 1, "{warm:#?}");
    assert_eq!(warm.files[0].diagnostics[0].code, "PNX9004", "{warm:#?}");
    let diagnostics = |report: &parenox_analysis::ProjectReport| {
        report
            .files
            .iter()
            .map(|file| serde_json::to_value(&file.diagnostics).unwrap())
            .collect::<Vec<_>>()
    };
    assert_eq!(diagnostics(&warm), diagnostics(&cold));
    let restored = session.check_project(&[consumer, provider], None);
    assert_eq!(diagnostics(&restored), diagnostics(&baseline));

    let exported = session.export_externs(&[provider], &[]);
    let edn = exported
        .edn
        .unwrap_or_else(|| panic!("{:#?}", exported.report));
    assert!(
        edn.contains("demo.levels/severity->level {:value {1 :error 2 :warning 3 :info}}"),
        "{edn}"
    );
    let imported = Session::default().check_project_with_externs(
        &[consumer],
        &[Source {
            path: "levels.edn",
            source: &edn,
        }],
    );
    let [imported] = imported.files[0].diagnostics.as_slice() else {
        panic!("{imported:#?}");
    };
    assert_eq!(imported.code, finding.code);
    assert_eq!(imported.actual, finding.actual);
    assert_eq!(imported.span, finding.span);
}

#[test]
fn inverse_enum_runtime_conformance_requires_the_registration_owner() {
    let provider = Source {
        path: "demo/inverse_enum.clj",
        source: specs(),
    };
    for (libspec, loaded) in [(":as-alias", false), (":as", true)] {
        let consumer = format!(
            r#"(ns consumer (:require [clojure.spec.alpha :as s] [demo.inverse-enum {libspec} wire]))
(defn format-code {{:t/sig '[:-> #{{:plaintext}}]}} []
  (s/conform ::wire/format 1N))
"#
        );
        let report = Session::default().check_project(
            &[
                Source {
                    path: "consumer.clj",
                    source: &consumer,
                },
                provider,
            ],
            None,
        );
        if loaded {
            assert!(report.ok && report.complete, "{report:#?}");
        } else {
            let [finding] = report.files[0].diagnostics.as_slice() else {
                panic!("{report:#?}");
            };
            assert_eq!(finding.code, "PNX9001", "{finding:#?}");
            assert!(
                finding.message.contains("registrations are loaded"),
                "{finding:#?}"
            );
        }
    }
}

#[test]
fn a_project_clojure_set_namespace_invalidates_cached_inverse_conformance() {
    let source = r#"
(ns consumer (:require [clojure.spec.alpha :as s] [demo.inverse-enum :as wire]))
(defn format-code {:t/sig '[:-> #{:plaintext}]} [] (s/conform ::wire/format 1N))
(defn independent {:t/sig '[:-> keyword?]} [] :stable)
"#;
    let consumer = Source {
        path: "consumer.clj",
        source,
    };
    let provider = Source {
        path: "demo/inverse_enum.clj",
        source: specs(),
    };
    let replacement = Source {
        path: "clojure/set.clj",
        source: r#"
(ns clojure.set)
(defn map-invert {:t/sig '[map? :-> map?]} [_] {1 :different})
"#,
    };
    let check = |session: &mut Session, replace_dependency| {
        let mut sources = vec![consumer, provider];
        if replace_dependency {
            sources.push(replacement);
        }
        session.check_project(&sources, None).files.remove(0)
    };

    let mut session = Session::default();
    let standard = check(&mut session, false);
    assert!(standard.ok && standard.complete, "{standard:#?}");
    let warm = check(&mut session, false);
    assert_eq!(warm.stats.reused_functions, 2, "{warm:#?}");

    let replaced = check(&mut session, true);
    assert!(!replaced.complete, "{replaced:#?}");
    assert!(replaced.diagnostics.iter().any(|finding| {
        finding.code == "PNX9001"
            && finding.message.contains("supported conversion")
            && &source[finding.span.start.offset..finding.span.end.offset] == "::wire/format"
    }));
    assert_eq!(replaced.stats.checked_functions, 1, "{replaced:#?}");
    assert_eq!(replaced.stats.reused_functions, 1, "{replaced:#?}");

    let restored = check(&mut session, false);
    assert!(restored.ok && restored.complete, "{restored:#?}");
    assert_eq!(restored.stats.checked_functions, 1, "{restored:#?}");
    assert_eq!(restored.stats.reused_functions, 1, "{restored:#?}");
}
