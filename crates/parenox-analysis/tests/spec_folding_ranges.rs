use parenox_analysis::{Report, Session, Source};

const FIXTURE: &str = include_str!("../../../tests/fixtures/spec-folding-ranges.clj");

fn specs() -> &'static str {
    FIXTURE.split_once(";; JVM-only").unwrap().0
}

fn check(cases: &str) -> Report {
    Session::default().check("folding_ranges.clj", &format!("{}{cases}", specs()))
}

#[test]
fn keeps_folding_range_keys_and_known_optional_fields_inside_collections() {
    let accepts = r#"
(defn accept {:t/sig '[::folding-ranges :-> keyword?]} [input] :accepted)
"#;
    let report = check(&format!(
        r#"{accepts}
(defn sends {{:t/sig '[:-> keyword?]}} []
  (accept {{}})
  (accept #{{}})
  (accept [{{:start-line ::s/invalid :end-line nil :kind :region}}])
  (accept (list {{:start-line false :end-line "unregistered"}}))
  (let [fold {{:start-line nil :end-line false}}]
    (accept (clojure.lang.MapEntry/create fold fold))))
(defn kind {{:t/sig '[any? :-> (s/nilable keyword?)]}} [input]
  (when (s/valid? ::folding-ranges input)
    (:kind (first input))))
"#
    ));
    assert!(!report.complete, "{report:#?}");
    assert_eq!(report.diagnostics.len(), 5, "{report:#?}");
    assert!(report.diagnostics.iter().all(|finding| {
        finding.code == "PNX9001" && finding.message.contains("checks presence only")
    }));

    for argument in [
        "[{:end-line 3}]",
        "[{:start-line 1 :end-line 3 :kind 4}]",
        "{:entry {:start-line 1 :end-line 3}}",
    ] {
        let report = check(&format!(
            "{accepts}\n(defn rejects {{:t/sig '[:-> keyword?]}} [] (accept {argument}))"
        ));
        assert!(!report.ok && !report.complete, "{argument}: {report:#?}");
        assert_eq!(
            report
                .diagnostics
                .iter()
                .filter(|finding| finding.code == "PNX2322")
                .count(),
            1,
            "{argument}: {report:#?}"
        );
    }

    let report = check(
        r#"
(defn result {:t/sig '[:-> any?]} []
  (s/conform ::folding-ranges [{:start-line nil :end-line false ::extra 1}]))
"#,
    );
    assert!(
        report.diagnostics.iter().any(|finding| {
            finding.code == "PNX9001" && finding.message.contains("undeclared qualified key")
        }),
        "{report:#?}"
    );
}

#[test]
fn conforms_folding_ranges_without_treating_unregistered_fields_as_specs() {
    let report = check(
        r#"
(defn presence-only {:t/sig '[:-> (s/tuple #{::s/invalid} #{false} #{::s/invalid} nil?)]} []
  (let [fold (first (s/conform ::folding-ranges
                            [{:start-line ::s/invalid :end-line false :collapsed-text ::s/invalid}]))]
    [(:start-line fold) (:end-line fold) (:collapsed-text fold) (:kind fold)]))
(defn known-kind {:t/sig '[:-> #{:region}]} []
  (:kind (first (s/conform ::folding-ranges [{:start-line nil :end-line false :kind :region}]))))
(defn rejected-kind {:t/sig '[:-> #{::s/invalid}]} []
  (s/conform ::folding-ranges [{:start-line nil :end-line false :kind ::s/invalid}]))
"#,
    );
    assert!(!report.complete, "{report:#?}");
    assert_eq!(report.diagnostics.len(), 5, "{report:#?}");
    assert!(report.diagnostics.iter().all(|finding| {
        finding.code == "PNX9001" && finding.message.contains("checks presence only")
    }));
}

#[test]
fn late_registry_type_changes_recheck_a_folding_range_consumer() {
    let provider = specs();
    let consumer = r#"
(ns consumer (:require [demo.folding-ranges :as folding]))
(defn input {:t/sig '[:-> keyword?]} []
  (folding/accept [{:start-line nil :end-line false}]))
"#;
    let registry = r#"
(ns registry (:require [clojure.spec.alpha :as s]))
(s/def :demo.folding-ranges/start-line int?)
"#;
    let provider = format!(
        "{provider}\n(defn accept {{:t/sig '[::folding-ranges :-> keyword?]}} [_] :accepted)"
    );
    let mut session = Session::default();
    let mut sources = vec![
        Source {
            path: "folding_ranges.clj",
            source: &provider,
        },
        Source {
            path: "consumer.clj",
            source: consumer,
        },
    ];
    let original = session.check_project(&sources, None);
    assert!(original.files[1].diagnostics.is_empty(), "{original:#?}");
    sources.push(Source {
        path: "registry.clj",
        source: registry,
    });
    let changed = session.check_project(&sources, None);
    let [error] = changed.files[1].diagnostics.as_slice() else {
        panic!("{changed:#?}");
    };
    assert_eq!(error.code, "PNX2322", "{error:#?}");
}
