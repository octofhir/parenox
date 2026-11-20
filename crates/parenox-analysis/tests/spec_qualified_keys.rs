use parenox_analysis::{Report, Session, Source};

const FIXTURE: &str = include_str!("../../../tests/fixtures/spec-qualified-keys.clj");

fn specs() -> &'static str {
    FIXTURE.split_once(";; JVM-only").unwrap().0
}

fn check(body: &str) -> Report {
    Session::default().check("qualified_keys.clj", &format!("{}{body}", specs()))
}

fn assert_presence_warnings(report: &Report) {
    assert!(!report.complete, "{report:#?}");
    assert_eq!(report.diagnostics.len(), 2, "{report:#?}");
    for key in [
        ":demo.qualified-keys/raw",
        ":demo.qualified-keys/annotation",
    ] {
        assert!(report.diagnostics.iter().any(|finding| {
            finding.code == "PNX9001"
                && finding.message.contains(key)
                && finding.message.contains("checks presence only")
        }));
    }
}

#[test]
fn conforms_known_mixed_keys_and_preserves_unregistered_values() {
    let report = check(
        r#"
(defn mixed {:t/sig '[:-> (s/tuple int? string? #{::s/invalid} #{::s/invalid} #{:region} #{false})]} []
  (let [output (s/conform ::shape {::line 1N ::raw ::s/invalid :label "item"
                                  ::annotation ::s/invalid ::kind :region :flag false})]
    [(::line output) (:label output) (::raw output) (::annotation output) (::kind output) (:flag output)]))
(defn absent {:t/sig '[:-> (s/tuple nil? nil? #{::s/invalid})]} []
  (let [output (s/conform ::shape {::line 1N ::raw nil :label "item" :kind ::s/invalid})]
    [(::kind output) (::annotation output) (:kind output)]))
(defn wrong-required-spelling {:t/sig '[:-> #{::s/invalid}]} []
  (s/conform ::shape {:line 1N ::raw nil :label "item"}))
(defn wrong-unqualified-spelling {:t/sig '[:-> #{::s/invalid}]} []
  (s/conform ::shape {::line 1N ::raw nil ::label "item"}))
(defn bad-registered-optional {:t/sig '[:-> #{::s/invalid}]} []
  (s/conform ::shape {::line 1N ::raw nil :label "item" ::kind ::s/invalid}))
(defn both-spellings {:t/sig '[:-> (s/tuple int? int?)]} []
  (let [both (s/conform ::two-spellings {::line 1N :line 2N})]
    [(::line both) (:line both)]))
(defn bad-second-spelling {:t/sig '[:-> #{::s/invalid}]} []
  (s/conform ::two-spellings {::line 1N :line nil}))
"#,
    );
    assert_presence_warnings(&report);
}

#[test]
fn a_qualified_replacement_changes_the_outer_map_entry_shape() {
    let fixtures = r#"
(def row {::line 1N ::raw nil :label "item"})
(defn unchanged-entry {:t/sig '[:-> int?]} []
  (let [normalized (s/conform ::shape row)
        entry (clojure.lang.MapEntry/create normalized normalized)]
    (::line (key (s/conform ::shapes entry)))))
(defn changed-entry {:t/sig '[:-> (s/tuple int? int?)]} []
  (let [normalized (s/conform ::shape row)
        entry (clojure.lang.MapEntry/create normalized row)
        result (s/conform ::shapes entry)]
    [(::line (first result)) (::line (first (rest result)))]))
"#;
    assert_presence_warnings(&check(fixtures));

    let report = check(&format!(
        r#"{fixtures}
(defn no-entry-key {{:t/sig '[:-> any?]}} []
  (key (s/conform ::shapes (clojure.lang.MapEntry/create row row))))"#
    ));
    let errors: Vec<_> = report
        .diagnostics
        .iter()
        .filter(|finding| finding.severity == "error")
        .collect();
    let [finding] = errors.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(finding.code, "PNX2322", "{finding:#?}");
    assert!(finding.message.contains("key"), "{finding:#?}");
}

#[test]
fn missing_required_keys_preempt_foreign_qualified_fields() {
    let base = r#"
(s/def :external/extra (s/and integer? (s/conformer int)))
(defn missing {:t/sig '[:-> #{::s/invalid}]} []
  (try
    (s/conform ::shape {::raw nil :label "item" :external/extra 2147483648})
    (catch ArithmeticException _ :unreachable)))
"#;
    assert_presence_warnings(&check(base));

    let report = check(&format!(
        r#"{base}
(defn foreign {{:t/sig '[:-> int?]}} []
  (:external/extra
   (s/conform ::shape {{::line 1N ::raw nil :label "item" :external/extra 7N}})))
(defn other-spelling {{:t/sig '[:-> string?]}} []
  (::label (s/conform ::shape {{::line 1N ::raw nil :label "item" ::label "qualified label"}})))
"#
    ));
    assert_presence_warnings(&report);
}

#[test]
fn registering_a_declared_qualified_key_rechecks_its_consumer() {
    let provider = Source {
        path: "qualified_keys.clj",
        source: specs(),
    };
    let consumer = Source {
        path: "consumer.clj",
        source: r#"
(ns consumer (:require [clojure.spec.alpha :as s] [demo.qualified-keys :as qualified]))
(defn converted {:t/sig '[:-> int?]} []
  (::qualified/line
   (s/conform ::qualified/shape {::qualified/line 1N ::qualified/raw ::s/invalid :label "item"})))
"#,
    };
    let registered = Source {
        path: "registration.clj",
        source: r#"
(ns registration (:require [clojure.spec.alpha :as s]))
(s/def :demo.qualified-keys/raw keyword?)
"#,
    };
    let sources = [provider, consumer, registered];
    let mut session = Session::default();
    let mut check_consumer = |count| {
        session
            .check_project(&sources[..count], None)
            .files
            .into_iter()
            .find(|file| file.path == "consumer.clj")
            .unwrap()
    };
    let original = check_consumer(2);
    assert!(original.complete && original.ok, "{original:#?}");

    let changed = check_consumer(3);
    let [finding] = changed.diagnostics.as_slice() else {
        panic!("{changed:#?}");
    };
    assert_eq!(finding.code, "PNX2322", "{finding:#?}");

    let without = check_consumer(2);
    assert!(without.complete && without.ok, "{without:#?}");
}

#[test]
fn imported_map_contracts_keep_the_qualified_key_spelling() {
    let provider = Source {
        path: "provider.clj",
        source: r#"
(ns demo.wire (:require [clojure.spec.alpha :as s]))
(s/def ::line (s/and integer? (s/conformer int)))
(s/def ::label string?)
(s/def ::kind keyword?)
(s/def ::shape (s/keys :req [::line] :req-un [::label] :opt [::kind]))
(defn accept {:t/sig '[::shape :-> keyword?]} [_] :accepted)
"#,
    };
    let export = Session::default().export_externs(&[provider], &[]);
    let edn = export.edn.unwrap_or_else(|| panic!("{:#?}", export.report));
    let check = |argument| {
        let consumer = format!(
            "(ns demo.wire-user (:require [demo.wire :as wire]))\n\
             (defn send {{:t/sig '[:-> keyword?]}} [] (wire/accept {argument}))"
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
    let exact = check("{::wire/line 1N :label \"item\" ::wire/kind :region}");
    assert!(exact.ok && exact.complete, "{exact:#?}");

    let plain = check("{:line 1N :label \"item\"}");
    let [finding] = plain.files[0].diagnostics.as_slice() else {
        panic!("{plain:#?}");
    };
    assert_eq!(finding.code, "PNX2322", "{finding:#?}");
    assert!(finding.message.contains(":demo.wire/line"));
    assert_eq!(
        finding.contract.as_ref().unwrap().path.as_deref(),
        Some("provider.edn")
    );
}
