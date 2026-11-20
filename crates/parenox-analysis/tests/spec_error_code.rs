use parenox_analysis::{Report, Session, Source};

fn specs() -> &'static str {
    include_str!("../../../tests/fixtures/spec-error-code.clj")
        .split_once(";; JVM-only")
        .unwrap()
        .0
}

fn check(body: &str) -> Report {
    Session::default().check("error_code.clj", &format!("{}{body}", specs()))
}

#[test]
fn error_codes_conform_to_the_fixed_integer_or_keyword_input() {
    let uses = r#"
(defn raw-kind {:t/sig '[:-> #{:server/retry}]} []
  (s/conform :error/code :server/retry))
(defn raw-number {:t/sig '[:-> (s/and int? #{-17})]} []
  (s/conform ::code -17))
(defn wide-number {:t/sig '[:-> int?]} []
  (s/conform ::reversed 2147483648))
(defn valid-wide {:t/sig '[:-> #{true}]} []
  (s/valid? :error/code 2147483648))
(defn valid-keyword {:t/sig '[:-> #{true}]} []
  (s/valid? ::reversed :ready))
(defn keyword-only {:t/sig '[:-> #{:ready}]} []
  (s/conform ::keyword-only :ready))
(defn excluded-branch {:t/sig '[:-> #{::s/invalid}]} []
  (s/conform ::keyword-only 7))
"#;
    let mut source = uses.to_owned();
    for (index, input) in ["::s/invalid", "7N", "1.0", "nil", "false", "\"ready\""]
        .into_iter()
        .enumerate()
    {
        source.push_str(&format!(
            r#"
(defn invalid-{index} {{:t/sig '[:-> #{{::s/invalid}}]}} [] (s/conform :error/code {input}))
(defn invalid-{index}? {{:t/sig '[:-> #{{false}}]}} [] (s/valid? ::code {input}))
"#
        ));
    }
    let report = check(&source);
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn broad_error_codes_and_guards_keep_only_possible_outcomes() {
    let uses = r#"
(defn exact-fixed {:t/sig '[int? :-> int?]} [x] (s/conform :error/code x))
(defn valid-fixed {:t/sig '[int? :-> #{true}]} [x] (s/valid? :error/code x))
(defn exact-keyword {:t/sig '[::keyword :-> ::keyword]} [x] (s/conform :error/code x))
(defn valid-keyword {:t/sig '[::keyword :-> #{true}]} [x] (s/valid? :error/code x))
(defn broad-keyword {:t/sig '[keyword? :-> keyword?]} [x] (s/conform :error/code x))
(defn broad-integer {:t/sig '[integer? :-> (s/or :int int? :invalid #{::s/invalid})]} [x]
  (s/conform :error/code x))
(defn broad-value {:t/sig '[any? :-> (s/or :kw keyword? :int int?)]} [x]
  (s/conform :error/code x))
(defn narrow-fixed {:t/sig '[integer? :-> int?]} [x]
  (if (s/valid? :error/code x) x 0))
(defn narrow-keyword {:t/sig '[#{:ready ::s/invalid nil false} :-> #{:ready}]} [x]
  (if (s/valid? ::code x) (s/conform :error/code x) :ready))
"#;
    let report = check(uses);
    assert!(report.ok && report.complete, "{report:#?}");

    let report = check(&format!(
        r#"{uses}
(defn keyword-always-valid {{:t/sig '[keyword? :-> #{{true}}]}} [x] (s/valid? :error/code x))
(defn integer-always-valid {{:t/sig '[integer? :-> #{{true}}]}} [x] (s/valid? :error/code x))
(defn broad-never-fixed {{:t/sig '[any? :-> keyword?]}} [x] (s/conform :error/code x))
(defn broad-never-keyword {{:t/sig '[any? :-> int?]}} [x] (s/conform :error/code x))
"#
    ));
    let expected = [
        "keyword-always-valid",
        "integer-always-valid",
        "broad-never-fixed",
        "broad-never-keyword",
    ];
    assert_eq!(report.diagnostics.len(), expected.len(), "{report:#?}");
    for (finding, name) in report.diagnostics.iter().zip(expected) {
        assert_eq!(finding.code, "PNX2322", "{finding:#?}");
        assert_eq!(
            finding.contract.as_ref().unwrap().name,
            format!("demo.error-code/{name}"),
            "{finding:#?}"
        );
    }
}

#[test]
fn exact_error_code_projection_preserves_conformed_map_entries() {
    let report = check(
        r#"
(def left {:code :ready :message "prepared"})
(def right {:code 2147483648 :message "wide code"})
(defn key-code {:t/sig '[:-> #{:ready}]} []
  (:code (key (s/conform ::errors (clojure.lang.MapEntry/create left right)))))
(defn val-code {:t/sig '[:-> int?]} []
  (:code (val (s/conform ::errors (clojure.lang.MapEntry/create left right)))))
(defn bad-code {:t/sig '[:-> #{::s/invalid}]} []
  (s/conform ::error {:code ::s/invalid :message "bad code"}))
(defn bad-entry {:t/sig '[:-> #{::s/invalid}]} []
  (s/conform ::errors (clojure.lang.MapEntry/create left {:code 7N :message "not fixed"})))
"#,
    );
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn exact_projection_requires_identity_branches_and_the_core_second() {
    let source = r#"
(ns demo.error-code-opaque
  (:refer-clojure :exclude [second])
  (:require [clojure.spec.alpha :as s]))
(defn second {:t/sig '[any? :-> #{:shadowed}]} [_] :shadowed)
(s/def ::shadowed-second
  (s/and (s/or :kw keyword? :int int?) (s/conformer second)))
(s/def ::converting-branch
  (s/and (s/or :kw keyword? :int (s/and integer? (s/conformer int)))
         (s/conformer clojure.core/second)))
(s/def ::canonical
  (s/and (s/or :kw keyword? :int int?) (s/conformer clojure.core/second)))
(defn local-second {:t/sig '[:-> any?]} [] (s/conform ::shadowed-second :ready))
(defn converting-branch {:t/sig '[:-> any?]} [] (s/conform ::converting-branch 1000))
(defn core-second {:t/sig '[:-> #{:ready}]} [] (s/conform ::canonical :ready))
"#;
    let report = Session::default().check("error_code_opaque.clj", source);
    assert!(!report.complete, "{report:#?}");
    assert!(
        report
            .diagnostics
            .iter()
            .all(|finding| finding.code == "PNX9001")
    );
    let opaque: Vec<_> = report
        .diagnostics
        .iter()
        .filter(|finding| finding.message.contains("supported conversion"))
        .map(|finding| &source[finding.span.start.offset..finding.span.end.offset])
        .collect();
    assert_eq!(
        opaque,
        ["::shadowed-second", "::converting-branch"],
        "{report:#?}"
    );
}

#[test]
fn the_error_code_input_contract_round_trips_without_sources() {
    let provider = format!(
        r#"{}
(defn emit {{:t/sig '[:error/code :-> string?]}} [_] "done")
"#,
        specs()
    );
    let exported = Session::default().export_externs(
        &[Source {
            path: "provider.clj",
            source: &provider,
        }],
        &[],
    );
    assert!(
        exported.report.ok && exported.report.complete,
        "{exported:#?}"
    );
    let edn = exported.edn.unwrap();
    let check = |input| {
        let consumer = format!(
            r#"(ns consumer (:require [clojure.spec.alpha :as s] [demo.error-code :as wire]))
(defn sends {{:t/sig '[:-> string?]}} [] (wire/emit {input}))
"#
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
    for input in [":server/retry", "0", "2147483648"] {
        let report = check(input);
        assert!(report.ok && report.complete, "{input}: {report:#?}");
    }
    for input in ["::s/invalid", "0N", "1.0", "nil", "\"ready\""] {
        let report = check(input);
        let [finding] = report.files[0].diagnostics.as_slice() else {
            panic!("{input}: {report:#?}");
        };
        assert_eq!(finding.code, "PNX2322", "{input}: {finding:#?}");
        assert_eq!(
            finding.contract.as_ref().unwrap().path.as_deref(),
            Some("provider.edn")
        );
    }
}

#[test]
fn error_code_runtime_registration_requires_loading_its_owners() {
    let registration = r#"
(s/def :demo.code-owner/code (s/and (s/or :kw keyword? :int int?) (s/conformer second)))
"#;
    let rows = Source {
        path: "demo/rows.clj",
        source: r#"
(ns demo.rows (:require [clojure.spec.alpha :as s]))
(s/def ::code :demo.code-owner/code)
(s/def ::row (s/keys :req-un [::code]))
(defn ready {:t/sig '[#{:ready} :-> #{:ready}]} [x] x)
"#,
    };
    let consumer = r#"
(ns consumer
  (:require [clojure.spec.alpha :as s] [demo.code-owner :as codes] [demo.rows :as rows]))
(defn direct {:t/sig '[:-> #{:ready}]} [] (s/conform ::codes/code :ready))
(defn guarded {:t/sig '[#{:ready ::s/invalid nil false} :-> any?]} [x]
  (if (s/valid? ::rows/code x) (rows/ready x) :invalid))
(defn nested {:t/sig '[:-> #{:ready}]} []
  (:code (s/conform ::rows/row {:code :ready})))
(defn independent {:t/sig '[:-> keyword?]} [] :stable)
"#;
    let check_consumer = |session: &mut Session, source: &str, moved: bool| {
        let owner = format!(
            "(ns demo.code-owner (:require [clojure.spec.alpha :as s])){}",
            if moved { "" } else { registration }
        );
        let other = format!(
            "(ns demo.unloaded (:require [clojure.spec.alpha :as s])){}",
            if moved { registration } else { "" }
        );
        session
            .check_project(
                &[
                    Source {
                        path: "consumer.clj",
                        source,
                    },
                    rows,
                    Source {
                        path: "demo/code_owner.clj",
                        source: &owner,
                    },
                    Source {
                        path: "demo/unloaded.clj",
                        source: &other,
                    },
                ],
                None,
            )
            .files
            .into_iter()
            .find(|file| file.path == "consumer.clj")
            .unwrap()
    };
    let assert_no_load = |report: &Report, source: &str| {
        assert!(!report.complete, "{report:#?}");
        let missing_loads: Vec<_> = report
            .diagnostics
            .iter()
            .filter(|finding| {
                finding.code == "PNX9001"
                    && finding
                        .message
                        .contains("Cannot prove the named spec and its registrations are loaded")
            })
            .map(|finding| &source[finding.span.start.offset..finding.span.end.offset])
            .collect();
        assert_eq!(
            missing_loads,
            ["::codes/code", "::rows/code", "::rows/row"],
            "{report:#?}"
        );
        assert!(
            report.diagnostics.iter().any(|finding| {
                finding.code == "PNX2322"
                    && finding
                        .contract
                        .as_ref()
                        .is_some_and(|contract| contract.name == "demo.rows/ready")
            }),
            "the unchecked s/valid? guard must not narrow x: {report:#?}"
        );
    };

    let mut session = Session::default();
    let alias_only = consumer.replace(":as codes", ":as-alias codes");
    let unchecked = check_consumer(&mut session, &alias_only, false);
    assert_no_load(&unchecked, &alias_only);

    let loaded = check_consumer(&mut session, consumer, false);
    assert!(loaded.ok && loaded.complete, "{loaded:#?}");
    let warm = check_consumer(&mut session, consumer, false);
    assert_eq!(warm.stats.reused_functions, 4, "{warm:#?}");

    // The key still names demo.code-owner, but its registration has moved to an unloaded file.
    let moved = check_consumer(&mut session, consumer, true);
    assert_no_load(&moved, consumer);
    assert_eq!(moved.stats.checked_functions, 3, "{moved:#?}");
    assert_eq!(moved.stats.reused_functions, 1, "{moved:#?}");
    let fresh = check_consumer(&mut Session::default(), consumer, true);
    let findings = |report: &Report| serde_json::to_value(&report.diagnostics).unwrap();
    assert_eq!(findings(&moved), findings(&fresh));
    let warm_gap = check_consumer(&mut session, consumer, true);
    assert_eq!(warm_gap.stats.reused_functions, 4, "{warm_gap:#?}");
    assert_eq!(findings(&warm_gap), findings(&fresh));

    let restored = check_consumer(&mut session, consumer, false);
    assert!(restored.ok && restored.complete, "{restored:#?}");
    assert_eq!(restored.stats.checked_functions, 3, "{restored:#?}");
    assert_eq!(restored.stats.reused_functions, 1, "{restored:#?}");
}
