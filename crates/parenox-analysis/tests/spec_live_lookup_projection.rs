use parenox_analysis::{Session, Source};

const SOURCE: &str = r#"
(ns demo.sync (:require [clojure.spec.alpha :as s]))
(def modes {:off 0 :default 1 :incremental 2})
(s/def ::sync
  (s/conformer (fn [input]
                 {:active true
                  :mode (or (get modes input) (get modes :default))
                  :write {:content true}})))
(s/def ::options (s/keys :opt-un [::sync]))
"#;

#[test]
fn live_table_conformance_keeps_fixed_fields_and_possible_exceptions() {
    let body = r#"
(defn fixed
  {:t/sig '[:-> {:active #{true} :mode t/any :write {:content #{true}}}]} []
  (s/conform ::sync :off))
(defn nested
  {:t/sig '[:-> {:sync {:active #{true} :mode t/any :write {:content #{true}}}}]} []
  (s/conform ::options {:sync false}))
(defn valid {:t/sig '[t/any :-> #{true}]} [input]
  (s/valid? ::sync input))
"#;
    let check = |body| Session::default().check("sync.clj", &format!("{SOURCE}{body}"));
    let report = check(body);
    assert!(report.ok && report.complete, "{report:#?}");

    let narrowed = r#"
(defn assume-code {:t/sig '[:-> int?]} []
  (:mode (s/conform ::sync :off)))
(defn uncaught {:t/sig '[:-> #{true}]} []
  (try (s/valid? ::sync :off) (catch Throwable _ :thrown)))
"#;
    let report = check(narrowed);
    let [domain, effect] = report.diagnostics.as_slice() else {
        panic!("{report:#?}")
    };
    assert_eq!(domain.code, "PNX9004");
    assert!(domain.message.contains("cannot verify Any"));
    assert_eq!(effect.code, "PNX2322");
    assert!(effect.actual.as_deref().unwrap().contains(":thrown"));
}

#[test]
fn editing_the_initializer_changes_the_example_and_keeps_the_live_extern() {
    let mut session = Session::default();
    let original = Source {
        path: "sync.clj",
        source: SOURCE,
    };
    let names = [":demo.sync/sync".into()];
    let first = session.propose_contracts(&[original], &[], &names).unwrap();
    let edn = first.externs.unwrap();
    assert_eq!(edn.matches("demo.sync/modes").count(), 2);
    assert!(
        first
            .edn
            .unwrap()
            .contains(r#":initial-table "{:off 0 :default 1 :incremental 2}""#)
    );

    let edited = SOURCE.replace(":default 1", ":default 9");
    let current = Source {
        source: &edited,
        ..original
    };
    let next = session.propose_contracts(&[current], &[], &names).unwrap();
    assert_eq!(next.externs.as_deref(), Some(edn.as_str()));
    assert!(
        next.edn
            .unwrap()
            .contains(r#":initial-table "{:off 0 :default 9 :incremental 2}""#)
    );

    let external = Source {
        path: "sync.edn",
        source: &edn,
    };
    let checked = session.validate_externs(&[current], &[external]);
    assert!(checked.ok && checked.complete, "{checked:#?}");
    let consumer = Source {
        path: "consumer.clj",
        source: r#"(ns consumer (:require [demo.sync] [clojure.spec.alpha :as s]))
                   (defn f {:t/sig '[:demo.sync/sync :-> nil?]} [_] nil)"#,
    };
    let rejected = session.check_project_with_externs(&[consumer], &[external]);
    assert!(
        !rejected.complete
            && rejected.files[0]
                .diagnostics
                .iter()
                .any(|finding| { finding.message.contains("unchecked input type") }),
        "{rejected:#?}"
    );
}

#[test]
fn the_source_proof_requires_an_adjacent_ordinary_table_in_the_current_namespace() {
    let table = "(def modes {:off 0 :default 1 :incremental 2})";
    for source in [
        SOURCE.replace("(def modes", "(def ^:const modes"),
        SOURCE.replace("(def modes", "(defn get [& _] nil)\n(def modes"),
        SOURCE.replace("(get modes :default)", "(get other :default)"),
        SOURCE
            .replace("(fn [input]", "(fn [modes]")
            .replace("modes input", "modes modes"),
        format!("{table}\n{}", SOURCE.replace(table, "")),
        SOURCE.replace(
            "(s/def ::sync",
            "(def changed-meta (alter-meta! #'modes assoc :const true))\n(s/def ::sync",
        ),
        SOURCE.replace("(s/def ::sync", "#_ignored\n(s/def ::sync"),
    ] {
        let report = Session::default().check("sync.clj", &source);
        assert!(
            !report.complete
                && report.diagnostics.iter().any(|finding| {
                    finding.code == "PNX9001" && finding.message.contains("s/conformer")
                }),
            "{report:#?}"
        );
    }
}
