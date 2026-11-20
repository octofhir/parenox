use parenox_analysis::{Report, Session, Source};

const FIXTURE: &str = include_str!("../../../tests/fixtures/spec-location-collection.clj");

fn specs() -> &'static str {
    FIXTURE.split_once(";; JVM-only").unwrap().0
}

fn check(body: &str) -> Report {
    Session::default().check(
        "locations.clj",
        &format!(
            r#"{}
(def location
  {{:uri "file:///home.clj"
    :range {{:start {{:line 1N :character 2}}
             :end {{:line 3 :character 4}}}}}})
(defn accept {{:t/sig '[::locations :-> keyword?]}} [input] :accepted)
{body}
"#,
            specs(),
        ),
    )
}

#[test]
fn checks_the_members_of_location_collections_and_distinguishes_map_entries() {
    let report = check(
        r#"
(defn send {:t/sig '[:-> keyword?]} []
  (accept {})
  (accept [])
  (accept #{})
  (accept '())
  (accept [location])
  (accept #{location})
  (accept (list location)))
(defn send-entry {:t/sig '[(s/map-of ::location ::location) :-> keyword?]} [pairs]
  (if-let [entry (first pairs)]
    (accept entry)
    :empty))
"#,
    );
    assert!(report.ok && report.complete, "{report:#?}");

    for input in [
        "nil",
        "\"not a collection\"",
        "[::s/invalid]",
        "[{:uri \"missing range\"}]",
        "{:entry location}",
        "(first {:entry location})",
        "[{:uri \"overflow\" :range {:start {:line 2147483648 :character 2} :end {:line 3 :character 4}}}]",
    ] {
        let report = check(&format!(
            "(defn send {{:t/sig '[:-> keyword?]}} [] (accept {input}))"
        ));
        let [finding] = report.diagnostics.as_slice() else {
            panic!("{input}: {report:#?}");
        };
        assert_eq!(finding.code, "PNX2322", "{input}: {finding:#?}");
    }

    let report = check(r#"(defn forward {:t/sig '[map? :-> keyword?]} [input] (accept input))"#);
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(finding.code, "PNX9004", "{finding:#?}");
    assert!(!report.complete);
}

#[test]
fn collection_contracts_keep_their_element_shape_when_imported_without_source() {
    let provider = format!(
        "{}\n(defn accept {{:t/sig '[::locations :-> keyword?]}} [input] :accepted)",
        specs(),
    );
    let export = Session::default().export_externs(
        &[Source {
            path: "locations.clj",
            source: &provider,
        }],
        &[],
    );
    let edn = export.edn.unwrap_or_else(|| panic!("{:#?}", export.report));
    let check = |argument| {
        let consumer = format!(
            "(ns consumer (:require [demo.location-collection :as location]))
             (defn send {{:t/sig '[:-> keyword?]}} [] (location/accept {argument}))"
        );
        Session::default().check_project_with_externs(
            &[Source {
                path: "consumer.clj",
                source: &consumer,
            }],
            &[Source {
                path: "locations.edn",
                source: &edn,
            }],
        )
    };
    let report = check(
        "[{:uri \"url\" :range {:start {:line 1N :character 1} :end {:line 2 :character 2}}}]",
    );
    assert!(report.ok && report.complete, "{report:#?}");
    let report = check("[{:uri \"missing range\"}]");
    let [finding] = report.files[0].diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(finding.code, "PNX2322", "{finding:#?}");
}

#[test]
fn validation_projects_open_element_shapes_with_a_gap_for_conformance() {
    let report = check(
        r#"
(defn guarded {:t/sig '[any? :-> (s/nilable string?)]} [input]
  (when (s/valid? ::locations input)
    (:uri (first input))))
"#,
    );
    assert!(report.ok && report.complete, "{report:#?}");

    for (body, explanation) in [
        (
            r#"(defn uncertain {:t/sig '[::location :-> any?]} [input]
                 (s/conform ::locations [input]))"#,
            "requires a closed map with known keyword fields",
        ),
        (
            r#"(defn qualified-key {:t/sig '[:-> any?]} []
                 (s/conform ::locations [(assoc location ::extra false)]))"#,
            "undeclared qualified key",
        ),
        (
            r#"(defn reconstruct {:t/sig '[(s/coll-of ::location :kind set?) :-> any?]} [input]
                 (s/conform ::locations input))"#,
            "cannot reconstruct this collection of maps",
        ),
    ] {
        let report = check(body);
        assert!(!report.complete, "{report:#?}");
        assert!(
            report.diagnostics.iter().any(|finding| {
                finding.code == "PNX9001" && finding.message.contains(explanation)
            }),
            "{report:#?}"
        );
    }
}

#[test]
fn conforms_closed_location_maps_inside_vectors() {
    let report = check(
        r#"
(defn converted {:t/sig '[:-> int?]} []
  (get-in (first (s/conform ::locations [location])) [:range :start :line]))
(defn extra {:t/sig '[:-> #{::s/invalid}]} []
  (:annotation (first (s/conform ::locations [(assoc location :annotation ::s/invalid)]))))
(defn empty-vector {:t/sig '[:-> (s/coll-of string? :kind vector?)]} []
  (s/conform ::locations []))
(defn empty-map {:t/sig '[:-> map?]} []
  (s/conform ::locations {}))
(defn invalid {:t/sig '[:-> #{::s/invalid}]} []
  (s/conform ::locations [location {:uri "missing range"}]))
(defn invalid-field {:t/sig '[:-> #{::s/invalid}]} []
  (s/conform ::locations [(assoc location :uri nil)]))
(defn entry-is-not-a-location {:t/sig '[:-> #{::s/invalid}]} []
  (s/conform ::locations {:entry location}))
(defn overflow {:t/sig '[:-> #{:overflow}]} []
  (try
    (s/conform ::locations [(assoc-in location [:range :end :line] 2147483648)])
    (catch ArithmeticException _ :overflow)))
(defn left-to-right {:t/sig '[:-> #{::s/invalid}]} []
  (try
    (s/conform ::locations [nil (assoc-in location [:range :end :line] 2147483648)])
    (catch ArithmeticException _ :unreachable)))"#,
    );
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn reconstructs_location_map_entries_only_when_elements_change() {
    let report = check(
        r#"
(defn reused-entry {:t/sig '[:-> ::location]} []
  (let [converted (s/conform ::location location)
        entry (clojure.lang.MapEntry/create converted converted)]
    (key (s/conform ::locations entry))))
(defn rebuilt-vector {:t/sig '[:-> int?]} []
  (let [entry (clojure.lang.MapEntry/create location location)]
    (get-in (first (s/conform ::locations entry)) [:range :start :line])))
(defn invalid-entry {:t/sig '[:-> #{::s/invalid}]} []
  (s/conform ::locations (clojure.lang.MapEntry/create :wrong location)))
(defn throwing-entry {:t/sig '[:-> #{:overflow}]} []
  (let [wide (assoc-in location [:range :end :line] 2147483648)]
    (try
      (s/conform ::locations (clojure.lang.MapEntry/create location wide))
      (catch ArithmeticException _ :overflow))))
(defn rejected-before-overflow {:t/sig '[:-> #{::s/invalid}]} []
  (let [wide (assoc-in location [:range :end :line] 2147483648)]
    (try
      (s/conform ::locations (clojure.lang.MapEntry/create nil wide))
      (catch ArithmeticException _ :unreachable))))"#,
    );
    assert!(report.ok && report.complete, "{report:#?}");

    for (name, value) in [
        ("original", "location"),
        (
            "replaced",
            "(assoc-in (s/conform ::location location) [:range :start :line] 1N)",
        ),
    ] {
        let report = check(&format!(
            r#"(defn {name} {{:t/sig '[:-> any?]}} []
                 (let [item {value}]
                   (key (s/conform ::locations (clojure.lang.MapEntry/create item item)))))"#,
        ));
        let [finding] = report.diagnostics.as_slice() else {
            panic!("{report:#?}");
        };
        assert_eq!(finding.code, "PNX2322", "{report:#?}");
        assert!(finding.message.contains("key"), "{finding:#?}");
    }
}

#[test]
fn map_entry_conformance_reports_unproven_box_identity() {
    let report = check(
        r#"(defn uncertain {:t/sig '[:-> any?]} []
              (let [wide (assoc-in location [:range :start :line] 128)
                    converted (s/conform ::location wide)]
                (s/conform ::locations (clojure.lang.MapEntry/create converted converted))))"#,
    );
    assert!(!report.complete, "{report:#?}");
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(finding.code, "PNX9001", "{report:#?}");
    assert!(
        finding.message.contains("identity of both elements"),
        "{finding:#?}"
    );
}

#[test]
fn rechecks_a_consumer_when_a_nested_conversion_changes_without_a_type_change() {
    let original = specs();
    let edited = original.replace("(s/def ::uri string?)", "(s/def ::uri (s/or :uri string?))");
    let consumer = r#"
(ns consumer (:require [clojure.spec.alpha :as s] [demo.location-collection :as loc]))
(defn uri {:t/sig '[:-> string?]} []
  (let [location {:uri "url" :range {:start {:line 1N :character 2} :end {:line 3 :character 4}}}]
    (:uri (first (s/conform ::loc/locations [location])))))"#;
    let mut session = Session::default();
    let mut check = |declarations| {
        session.check_project(
            &[
                Source {
                    path: "location.clj",
                    source: declarations,
                },
                Source {
                    path: "consumer.clj",
                    source: consumer,
                },
            ],
            None,
        )
    };
    let report = check(original);
    assert!(report.ok && report.complete, "{report:#?}");
    let report = check(&edited);
    assert!(!report.complete, "{report:#?}");
    assert!(
        report.files[1].diagnostics.iter().any(|finding| {
            finding.code == "PNX9001" && finding.message.contains("supported conversion")
        }),
        "{report:#?}"
    );
    let report = check(original);
    assert!(report.ok && report.complete, "{report:#?}");
}
