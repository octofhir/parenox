use parenox_analysis::{Report, Session, Source};

const SPECS: &str = r#"
(ns demo.marked-string (:require [clojure.spec.alpha :as s]))
(s/def ::marked-string
  (s/and (s/or :string string? :marked-string (s/keys :req-un [::language ::value]))
         (s/conformer second)))
(s/def ::reversed
  (s/and (s/or :marked-string (s/keys :req-un [::language ::value]) :string clojure.core/string?)
         (s/conformer clojure.core/second)))
(s/def :extra/line (s/and integer? (s/conformer int)))
"#;

const STRINGS: &str = r#"
(s/def ::language string?)
(s/def ::value string?)
"#;

fn check(body: &str) -> Report {
    Session::default().check("marked_string.clj", &format!("{SPECS}{body}"))
}

#[test]
fn inline_marked_string_checks_required_presence_and_preserves_the_original_values() {
    let report = check(
        r#"
(defn text {:t/sig '[:-> #{"code"}]} [] (s/conform ::marked-string "code"))
(defn reversed-text {:t/sig '[:-> #{""}]} [] (s/conform ::reversed ""))
(defn marked {:t/sig '[:-> {:language #{false} :value #{::s/invalid}}]} []
  (s/conform ::marked-string {:language false :value ::s/invalid}))
(defn reversed-marked {:t/sig '[:-> {:language nil? :value #{false}}]} []
  (s/conform ::reversed {:language nil :value false}))
(defn valid {:t/sig '[:-> #{true}]} []
  (s/valid? ::marked-string {:language false :value ::s/invalid}))
(defn invalid {:t/sig '[:-> #{false}]} [] (s/valid? ::reversed {:language "clojure"}))
(defn missing-language {:t/sig '[:-> #{::s/invalid}]} [] (s/conform ::marked-string {:value nil}))
(defn missing-value {:t/sig '[:-> #{::s/invalid}]} [] (s/conform ::reversed {:language "clojure"}))
(defn wrong-input {:t/sig '[:-> #{::s/invalid}]} [] (s/conform ::marked-string [:clojure "code"]))
(defn invalid-marker {:t/sig '[:-> #{false}]} [] (s/valid? ::marked-string ::s/invalid))
"#,
    );
    assert!(!report.complete, "{report:#?}");
    assert_eq!(report.diagnostics.len(), 4, "{report:#?}");
    for key in [":demo.marked-string/language", ":demo.marked-string/value"] {
        assert_eq!(
            report
                .diagnostics
                .iter()
                .filter(|finding| {
                    finding.code == "PNX9001"
                        && finding.message.contains(key)
                        && finding.message.contains("s/keys checks presence only")
                })
                .count(),
            2,
            "{report:#?}"
        );
    }
}

#[test]
fn optional_fields_in_the_inline_branch_validate_only_when_present() {
    let report = check(&format!(
        r#"{STRINGS}
(s/def ::optional
  (s/and (s/or :text string? :marked (s/keys :req-un [::language] :opt-un [::value]))
         (s/conformer second)))
(defn absent {{:t/sig '[:-> {{:language #{{"clojure"}}}}]}} []
  (s/conform ::optional {{:language "clojure"}}))
(defn present {{:t/sig '[:-> #{{"code"}}]}} []
  (:value (s/conform ::optional {{:language "clojure" :value "code"}})))
(defn invalid {{:t/sig '[:-> #{{false}}]}} []
  (s/valid? ::optional {{:language "clojure" :value nil}}))
(defn required {{:t/sig '[:-> #{{::s/invalid}}]}} []
  (s/conform ::optional {{:value "code"}}))
"#
    ));
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn inline_branch_preserves_entries_until_a_registered_extra_converts() {
    let uses = format!(
        r#"{STRINGS}
(s/def ::wrapper (s/keys :req-un [::marked-string]))
(s/def ::items (s/coll-of ::wrapper))
(def text {{:marked-string "code"}})
(def marked {{:marked-string {{:language "clojure" :value "form"}}}})
(def boxed {{:marked-string {{:language "clojure" :value "form" :extra/line 1N}}}})
(defn unchanged {{:t/sig '[:-> #{{"form"}}]}} []
  (let [entry (s/conform ::items (clojure.lang.MapEntry/create text marked))]
    (:value (:marked-string (val entry)))))
(defn changed {{:t/sig '[:-> int?]}} []
  (let [items (s/conform ::items (clojure.lang.MapEntry/create text boxed))]
    (:extra/line (:marked-string (first (rest items))))))
"#
    );
    let report = check(&uses);
    assert!(report.ok && report.complete, "{report:#?}");

    let report = check(&format!(
        r#"{uses}
(defn cannot-see-map-entry {{:t/sig '[:-> any?]}} []
  (key (s/conform ::items (clojure.lang.MapEntry/create text boxed))))
"#
    ));
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(finding.code, "PNX2322", "{finding:#?}");
    assert!(finding.message.contains("key"), "{finding:#?}");
}

#[test]
fn marked_string_input_exports_with_registered_fields_and_imports_without_source() {
    let source = format!(
        r#"{SPECS}{STRINGS}
(defn send {{:t/sig '[::marked-string :-> keyword?]}} [_] :accepted)
"#
    );
    let provider = Source {
        path: "provider.clj",
        source: &source,
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
    let check = |input| {
        let source = format!(
            r#"(ns consumer (:require [clojure.spec.alpha :as s] [demo.marked-string :as wire]))
(defn forwards {{:t/sig '[:-> keyword?]}} [] (wire/send {input}))"#
        );
        Session::default().check_project_with_externs(
            &[Source {
                path: "consumer.clj",
                source: &source,
            }],
            &[externs],
        )
    };
    for input in [r#""code""#, r#"{:language "clojure" :value "code"}"#] {
        let report = check(input);
        assert!(report.ok && report.complete, "{input}: {report:#?}");
    }
    for input in [
        "::s/invalid",
        r#"{:language "clojure"}"#,
        r#"{:language nil :value "code"}"#,
    ] {
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
fn newly_registered_fields_recheck_the_unchanged_marked_string_consumer() {
    let consumer = Source {
        path: "consumer.clj",
        source: r#"
(ns consumer (:require [clojure.spec.alpha :as s] [demo.marked-string :as wire]))
(defn formerly-valid {:t/sig '[:-> #{true}]} []
  (s/valid? ::wire/marked-string {:language false :value nil}))
(defn formerly-big {:t/sig '[:-> #{false}]} []
  (int? (:value (s/conform ::wire/marked-string {:language "clojure" :value 1N}))))
(defn independent {:t/sig '[:-> string?]} [] "stable")
"#,
    };
    let registered = format!(
        r#"{SPECS}
(s/def ::language string?)
(s/def ::value (s/and integer? (s/conformer int)))
"#
    );
    let check_consumer = |session: &mut Session, provider: &str| {
        session
            .check_project(
                &[
                    consumer,
                    Source {
                        path: "demo/marked_string.clj",
                        source: provider,
                    },
                ],
                None,
            )
            .files
            .remove(0)
    };
    let mut session = Session::default();
    let unchecked_fields = check_consumer(&mut session, SPECS);
    assert!(
        unchecked_fields.ok && unchecked_fields.complete,
        "{unchecked_fields:#?}"
    );
    let warm = check_consumer(&mut session, SPECS);
    assert_eq!(warm.stats.reused_functions, 3, "{warm:#?}");

    let changed = check_consumer(&mut session, &registered);
    let [valid, numeric] = changed.diagnostics.as_slice() else {
        panic!("{changed:#?}");
    };
    for (finding, function, actual) in [
        (valid, "formerly-valid", "false"),
        (numeric, "formerly-big", "true"),
    ] {
        assert_eq!(finding.code, "PNX2322", "{finding:#?}");
        assert!(finding.message.contains(function), "{finding:#?}");
        assert_eq!(finding.actual.as_deref(), Some(actual), "{finding:#?}");
    }
    assert_eq!(changed.stats.checked_functions, 2, "{changed:#?}");
    assert_eq!(changed.stats.reused_functions, 1, "{changed:#?}");

    let restored = check_consumer(&mut session, SPECS);
    assert!(restored.ok && restored.complete, "{restored:#?}");
    assert_eq!(restored.stats.checked_functions, 2, "{restored:#?}");
    assert_eq!(restored.stats.reused_functions, 1, "{restored:#?}");
}

#[test]
fn inline_map_branch_does_not_claim_exactness_for_foreign_or_opaque_children() {
    let local = r#"
(ns demo.marked-string (:require [clojure.spec.alpha :as s] [demo.foreign :as foreign]))
(s/def ::value string?)
(s/def ::language (s/conformer identity))
(s/def ::foreign
  (s/and (s/or :text string? :marked (s/keys :req-un [::foreign/language ::value]))
         (s/conformer second)))
(s/def ::opaque
  (s/and (s/or :text string? :marked (s/keys :req-un [::language ::value]))
         (s/conformer second)))
(defn foreign-result {:t/sig '[:-> any?]} [] (s/conform ::foreign "text"))
(defn opaque-result {:t/sig '[:-> any?]} [] (s/conform ::opaque "text"))
"#;
    let foreign = r#"
(ns demo.foreign (:require [clojure.spec.alpha :as s]))
(s/def ::language string?)
"#;
    let report = Session::default().check_project(
        &[
            Source {
                path: "demo/marked_string.clj",
                source: local,
            },
            Source {
                path: "demo/foreign.clj",
                source: foreign,
            },
        ],
        None,
    );
    let targeted: Vec<_> = report.files[0]
        .diagnostics
        .iter()
        .filter(|finding| {
            finding.code == "PNX9001" && finding.message.contains("supported conversion")
        })
        .map(|finding| &local[finding.span.start.offset..finding.span.end.offset])
        .collect();
    assert_eq!(targeted, ["::foreign", "::opaque"], "{report:#?}");
}
