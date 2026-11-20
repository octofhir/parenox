use parenox_analysis::{Report, Session, Source};

const FIXTURE: &str = include_str!("../../../tests/fixtures/spec-optional-keys.clj");

fn specs() -> &'static str {
    FIXTURE.split_once(";; JVM-only").unwrap().0
}

fn check(body: &str) -> Report {
    Session::default().check("optional_keys.clj", &format!("{}{body}", specs()))
}

fn assert_presence_warnings(report: &Report) {
    assert!(!report.complete, "{report:#?}");
    assert_eq!(report.diagnostics.len(), 2, "{report:#?}");
    for field in ["required-unknown", "optional-unknown"] {
        assert!(report.diagnostics.iter().any(|finding| {
            finding.code == "PNX9001"
                && finding.message.contains(field)
                && finding.message.contains("checks presence only")
        }));
    }
}

#[test]
fn conforms_optional_values_only_when_present_and_registered() {
    let report = check(
        r#"
(defn absent {:t/sig '[:-> (s/tuple #{::s/invalid} #{false})]} []
  (let [output (s/conform ::shape {:required-unknown ::s/invalid})]
    [(:required-unknown output) (contains? output :optional-any)]))
(defn present {:t/sig '[:-> (s/tuple #{true} nil? #{::s/invalid} #{:region} int?)]} []
  (let [output (s/conform ::shape {:required-unknown false
                                  :optional-unknown ::s/invalid
                                  :optional-any nil
                                  :optional-keyword :region
                                  :optional-int 7N})]
    [(contains? output :optional-any) (:optional-any output) (:optional-unknown output)
     (:optional-keyword output) (:optional-int output)]))
(defn opaque-value {:t/sig '[:-> #{false}]} []
  (:optional-any (s/conform ::shape {:required-unknown nil :optional-any false})))
(defn bad-any {:t/sig '[:-> #{::s/invalid}]} []
  (s/conform ::shape {:required-unknown nil :optional-any ::s/invalid}))
(defn bad-keyword {:t/sig '[:-> #{::s/invalid}]} []
  (s/conform ::shape {:required-unknown true :optional-keyword ::s/invalid}))
(defn bad-int {:t/sig '[:-> #{::s/invalid}]} []
  (s/conform ::shape {:required-unknown true :optional-int 1.5}))
"#,
    );
    assert_presence_warnings(&report);
}

#[test]
fn checks_required_presence_before_converting_and_preserves_collection_order() {
    let report = check(
        r#"
(defn key-first {:t/sig '[:-> #{::s/invalid}]} []
  (try
    (s/conform ::shape {:optional-int 2147483648})
    (catch ArithmeticException _ :unreachable)))
(defn thrown {:t/sig '[:-> #{:overflow}]} []
  (try
    (s/conform ::shape {:required-unknown false :optional-int 2147483648})
    (catch ArithmeticException _ :overflow)))
(defn order-not-known {:t/sig '[:-> #{::s/invalid :overflow}]} []
  (try
    (s/conform ::shape {:required-unknown false :optional-keyword nil :optional-int 2147483648})
    (catch ArithmeticException _ :overflow)))
(defn stop-before-overflow {:t/sig '[:-> #{::s/invalid}]} []
  (try
    (s/conform ::shapes (list {} {:required-unknown false :optional-int 2147483648}))
    (catch ArithmeticException _ :unreachable)))
(defn overflow-first {:t/sig '[:-> #{:overflow}]} []
  (try
    (s/conform ::shapes (list {:required-unknown false :optional-int 2147483648} {}))
    (catch ArithmeticException _ :overflow)))
"#,
    );
    assert_presence_warnings(&report);
}

#[test]
fn optional_conversions_reconstruct_set_members_and_map_entries() {
    let report = check(
        r#"
(defn set-member {:t/sig '[:-> (s/tuple (s/coll-of map? :kind set?) (s/nilable int?))]} []
  (let [output (s/conform ::shapes #{{:required-unknown nil :optional-int 7N}})]
    [output (:optional-int (first output))]))
(defn keeps-entry {:t/sig '[:-> #{false}]} []
  (let [left {:required-unknown nil :optional-any false}
        right {:required-unknown true :optional-keyword :region}]
    (:optional-any (key (s/conform ::shapes (clojure.lang.MapEntry/create left right))))))
(defn reboxes-entry {:t/sig '[:-> int?]} []
  (let [left {:required-unknown nil :optional-int 7N}
        right {:required-unknown true}]
    (:optional-int (first (s/conform ::shapes (clojure.lang.MapEntry/create left right))))))
"#,
    );
    assert_presence_warnings(&report);

    let report = check(
        r#"
(defn cannot-take-key {:t/sig '[:-> any?]} []
  (let [left {:required-unknown nil :optional-int 7N}
        right {:required-unknown true}]
    (key (s/conform ::shapes (clojure.lang.MapEntry/create left right)))))
"#,
    );
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
fn keeps_qualified_fields_and_unsupported_registered_conversions_as_gaps() {
    let report = check(
        r#"
(s/def ::opaque (s/or :text string?))
(s/def ::opaque-shape (s/keys :opt-un [::opaque]))
(defn qualified {:t/sig '[:-> any?]} []
  (s/conform ::shape {:required-unknown nil :foreign/unregistered 7N}))
(defn opaque {:t/sig '[:-> any?]} []
  (s/conform ::opaque-shape {:opaque "text"}))
"#,
    );
    assert!(!report.complete, "{report:#?}");
    for explanation in ["undeclared qualified key", "supported conversion"] {
        assert!(
            report.diagnostics.iter().any(|finding| {
                finding.code == "PNX9001" && finding.message.contains(explanation)
            }),
            "{report:#?}"
        );
    }
}

#[test]
fn late_optional_registration_rechecks_an_unchanged_collection_consumer() {
    let consumer = r#"
(ns consumer (:require [clojure.spec.alpha :as s] [demo.optional-keys :as optional]))
(defn present {:t/sig '[:-> string?]} []
  (:optional-any (first (s/conform ::optional/shapes
                      [{:required-unknown false :optional-unknown ::s/invalid :optional-any "kept"}]))))
(defn absent {:t/sig '[:-> string?]} []
  (:optional-any (first (s/conform ::optional/shapes [{:required-unknown false :optional-any "kept"}]))))
"#;
    let registry = r#"
(ns registry (:require [clojure.spec.alpha :as s]))
(s/def :demo.optional-keys/optional-unknown any?)
"#;
    let mut session = Session::default();
    let sources = [
        Source {
            path: "optional_keys.clj",
            source: specs(),
        },
        Source {
            path: "consumer.clj",
            source: consumer,
        },
        Source {
            path: "registry.clj",
            source: registry,
        },
    ];
    let check_consumer = |session: &mut Session, count| {
        let report = session.check_project(&sources[..count], None);
        report
            .files
            .into_iter()
            .find(|file| file.path == "consumer.clj")
            .unwrap()
    };
    let report = check_consumer(&mut session, 2);
    assert!(report.complete && report.ok, "{report:#?}");

    let report = check_consumer(&mut session, 3);
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(finding.code, "PNX2322", "{finding:#?}");

    let report = check_consumer(&mut session, 2);
    assert!(report.complete && report.ok, "{report:#?}");
}
