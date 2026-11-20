use parenox_analysis::{Report, Session, Source};

const FIXTURE: &str = include_str!("../../../tests/fixtures/spec-extra-registry.clj");
const STATES: &str = r#"
(def states {1 :ready 2 :failed})
(s/def :external/status (s/and int? states (s/conformer states)))
"#;
const ABSENT: &str = "s/conform cannot check an undeclared qualified key against the spec registry";
const OPAQUE: &str = "s/conform cannot apply a supported conversion to an undeclared qualified key's registered spec";
const NOT_LOADED: &str =
    "s/conform cannot prove the registered spec for an undeclared qualified key is loaded";

fn specs() -> &'static str {
    FIXTURE.split_once(";; JVM-only").unwrap().0
}

fn check(body: &str) -> Report {
    Session::default().check("extra_registry.clj", &format!("{}{body}", specs()))
}

#[test]
fn finite_source_specs_conform_actual_undeclared_qualified_values() {
    let report = check(&format!(
        r#"{STATES}
(defn ready {{:t/sig '[:-> #{{:ready}}]}} []
  (:external/status (s/conform ::row {{:label "result" :external/status 1}})))
(defn rejected {{:t/sig '[:-> (s/tuple #{{::s/invalid}} #{{::s/invalid}})]}} []
  [(s/conform ::row {{:label "result" :external/status 3}})
   (s/conform ::row {{:label "result" :external/status 1N}})])
"#
    ));
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn registered_extra_conversions_determine_map_entry_identity() {
    let inputs = format!(
        r#"{STATES}
(def coordinate {{:label "coordinate" :external/line 7N}})
(def status {{:label "status" :external/status 1}})
(defn kept-entry {{:t/sig '[:-> int?]}} []
  (let [boxed (s/conform ::row coordinate)
        entry (clojure.lang.MapEntry/create boxed boxed)]
    (:external/line (key (s/conform ::rows entry)))))
(defn reboxed-result {{:t/sig '[:-> int?]}} []
  (:external/line (first (s/conform ::rows (clojure.lang.MapEntry/create coordinate coordinate)))))
(defn enum-result {{:t/sig '[:-> #{{:ready}}]}} []
  (:external/status (first (s/conform ::rows (clojure.lang.MapEntry/create status status)))))
"#
    );
    let report = check(&inputs);
    assert!(report.ok && report.complete, "{report:#?}");

    for changed in ["coordinate", "status"] {
        let report = check(&format!(
            r#"{inputs}
(defn rebuilt {{:t/sig '[:-> any?]}} []
  (key (s/conform ::rows (clojure.lang.MapEntry/create {changed} {changed}))))"#
        ));
        let [finding] = report.diagnostics.as_slice() else {
            panic!("{report:#?}");
        };
        assert_eq!(finding.code, "PNX2322", "{finding:#?}");
        assert!(finding.message.contains("key"), "{finding:#?}");
    }
}

#[test]
fn required_keys_are_checked_before_the_extra_registry() {
    let report = check(
        r#"
(defn missing {:t/sig '[:-> #{::s/invalid}]} []
  (try (s/conform ::row {:external/line 2147483648 :external/unregistered false})
       (catch ArithmeticException _ :unreachable)))
(defn overflow {:t/sig '[:-> #{:overflow}]} []
  (try (s/conform ::row {:label "coordinate" :external/line 2147483648})
       (catch ArithmeticException _ :overflow)))
"#,
    );
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn missing_and_opaque_source_specs_remain_explicit_gaps() {
    let report = check(
        r#"
(s/def :external/opaque (s/or :text string?))
(defn missing {:t/sig '[:-> any?]} []
  (s/conform ::row {:label "result" :external/unregistered 1}))
(defn opaque {:t/sig '[:-> any?]} []
  (s/conform ::row {:label "result" :external/opaque "text"}))
"#,
    );
    assert!(!report.complete, "{report:#?}");
    assert_eq!(report.diagnostics.len(), 2, "{report:#?}");
    for message in [ABSENT, OPAQUE] {
        assert!(
            report
                .diagnostics
                .iter()
                .any(|finding| { finding.code == "PNX9001" && finding.message.contains(message) })
        );
    }
}

#[test]
fn registered_extra_specs_cannot_inherit_an_unproven_conversion() {
    let sources = [
        Source {
            path: "consumer.clj",
            source: r#"
(ns consumer
  (:require [clojure.spec.alpha :as s] [demo.extra-registry :as rows] [demo.registry]))
(defn nested {:t/sig '[:-> #{:ready}]} []
  (get-in (s/conform ::rows/row {:label "result" :external/detail {:state 1}})
          [:external/detail :state]))
(defn collection {:t/sig '[:-> int?]} []
  (:line (first (:external/items (s/conform ::rows/row {:label "result" :external/items [{:line 1N}]})))))
(defn alias {:t/sig '[:-> #{:ready}]} []
  (:external/alias (s/conform ::rows/row {:label "result" :external/alias 1})))
(defn direct-any-collection {:t/sig '[:-> (s/tuple int?)]} []
  (:external/any-items (s/conform ::rows/row {:label "result" :external/any-items [1]})))
"#,
        },
        Source {
            path: "demo/extra_registry.clj",
            source: specs(),
        },
        Source {
            path: "demo/registry.clj",
            source: r#"
(ns demo.registry (:require [clojure.spec.alpha :as s]))
(s/def :external/detail (s/keys :req-un [:demo.hidden/state]))
(s/def :external/items (s/coll-of :demo.hidden/item))
(s/def :external/alias :demo.hidden/state)
(s/def :external/any-items (s/coll-of any?))
"#,
        },
        Source {
            path: "demo/hidden.clj",
            source: r#"
(ns demo.hidden (:require [clojure.spec.alpha :as s]))
(def states {1 :ready 2 :failed})
(s/def ::state (s/and int? states (s/conformer states)))
(s/def ::line (s/and integer? (s/conformer int)))
(s/def ::item (s/keys :req-un [::line]))
"#,
        },
    ];
    let report = Session::default().check_project(&sources, None);
    let consumer = &report.files[0];
    assert!(!consumer.complete, "{report:#?}");
    assert_eq!(consumer.diagnostics.len(), 3, "{report:#?}");
    assert!(
        consumer
            .diagnostics
            .iter()
            .all(|finding| { finding.code == "PNX9001" && finding.message.contains(OPAQUE) })
    );
}

#[test]
fn an_unchanged_consumer_observes_extra_registration_and_conversion_edits() {
    let rows = Source {
        path: "rows.clj",
        source: specs(),
    };
    let consumer = Source {
        path: "consumer.clj",
        source: r#"
(ns consumer (:require [clojure.spec.alpha :as s] [demo.extra-registry :as rows] [registry]))
(defn ready {:t/sig '[:-> #{:ready}]} []
  (:external/status (s/conform ::rows/row {:label "result" :external/status 1})))
(defn independent {:t/sig '[:-> #{:constant}]} [] :constant)
"#,
    };
    let ready = format!("(ns registry (:require [clojure.spec.alpha :as s]))\n{STATES}");
    let retry = ready.replace(":ready", ":retry");
    let check_consumer = |session: &mut Session, registry: Option<&str>| {
        let mut sources = vec![rows, consumer];
        if let Some(source) = registry {
            sources.push(Source {
                path: "registry.clj",
                source,
            });
        }
        session
            .check_project(&sources, None)
            .files
            .into_iter()
            .find(|file| file.path == consumer.path)
            .unwrap()
    };
    let assert_absent = |report: &Report| {
        let [finding] = report.diagnostics.as_slice() else {
            panic!("{report:#?}");
        };
        assert_eq!(finding.code, "PNX9001", "{finding:#?}");
        assert!(finding.message.contains(ABSENT), "{finding:#?}");
    };
    let mut session = Session::default();
    assert_absent(&check_consumer(&mut session, None));

    let added = check_consumer(&mut session, Some(&ready));
    assert!(added.ok && added.complete, "{added:#?}");
    assert_eq!(added.stats.checked_functions, 1, "{added:#?}");
    assert_eq!(added.stats.reused_functions, 1, "{added:#?}");
    let warm = check_consumer(&mut session, Some(&ready));
    assert_eq!(warm.stats.reused_functions, 2, "{warm:#?}");

    let changed = check_consumer(&mut session, Some(&retry));
    let fresh = check_consumer(&mut Session::default(), Some(&retry));
    let [finding] = changed.diagnostics.as_slice() else {
        panic!("{changed:#?}");
    };
    assert_eq!(finding.code, "PNX2322", "{finding:#?}");
    assert!(finding.message.contains(":retry"), "{finding:#?}");
    assert_eq!(fresh.diagnostics.len(), 1, "{fresh:#?}");
    assert_eq!(fresh.diagnostics[0].message, finding.message);
    assert_eq!(changed.stats.checked_functions, 1, "{changed:#?}");
    assert_eq!(changed.stats.reused_functions, 1, "{changed:#?}");

    assert_absent(&check_consumer(&mut session, None));
}

#[test]
fn extra_registry_conformance_requires_a_direct_namespace_load() {
    let rows = Source {
        path: "demo/extra_registry.clj",
        source: specs(),
    };
    let registry = format!("(ns demo.registry (:require [clojure.spec.alpha :as s]))\n{STATES}");
    let registry = Source {
        path: "demo/registry.clj",
        source: &registry,
    };
    let bridge = Source {
        path: "demo/bridge.clj",
        source: "(ns demo.bridge (:require [demo.registry]))",
    };

    for (libspec, clause, proven) in [
        ("", "", false),
        ("[demo.registry :as-alias reg]", "", false),
        ("[demo.bridge]", "", false),
        ("[demo.registry]", "", true),
        ("[demo.registry :as reg]", "", true),
        ("", "(:use [demo.registry])", true),
    ] {
        let consumer = format!(
            r#"
(ns consumer (:require [clojure.spec.alpha :as s] [demo.extra-registry :as rows] {libspec}) {clause})
(defn ready {{:t/sig '[:-> #{{:ready}}]}} []
  (:external/status (s/conform ::rows/row {{:label "result" :external/status 1}})))
"#
        );
        let report = Session::default().check_project(
            &[
                Source {
                    path: "consumer.clj",
                    source: &consumer,
                },
                rows,
                registry,
                bridge,
            ],
            None,
        );
        let consumer = &report.files[0];
        if proven {
            assert!(
                consumer.ok && consumer.complete,
                "{libspec} {clause}: {report:#?}"
            );
        } else {
            let [finding] = consumer.diagnostics.as_slice() else {
                panic!("{libspec} {clause}: {report:#?}");
            };
            assert_eq!(finding.code, "PNX9001", "{libspec} {clause}: {finding:#?}");
            assert!(finding.message.contains(NOT_LOADED), "{finding:#?}");
        }
    }
}

#[test]
fn moving_an_equal_registration_between_namespaces_invalidates_the_consumer() {
    let rows = Source {
        path: "demo/extra_registry.clj",
        source: specs(),
    };
    let consumer = Source {
        path: "consumer.clj",
        source: r#"
(ns consumer
  (:require [clojure.spec.alpha :as s] [demo.extra-registry :as rows] [demo.loaded]))
(defn ready {:t/sig '[:-> #{:ready}]} []
  (:external/status (s/conform ::rows/row {:label "result" :external/status 1})))
"#,
    };
    let check_consumer = |session: &mut Session, in_loaded_namespace| {
        let loaded = if in_loaded_namespace { STATES } else { "" };
        let unloaded = if in_loaded_namespace { "" } else { STATES };
        let loaded = format!("(ns demo.loaded (:require [clojure.spec.alpha :as s]))\n{loaded}");
        let unloaded =
            format!("(ns demo.unloaded (:require [clojure.spec.alpha :as s]))\n{unloaded}");
        session
            .check_project(
                &[
                    consumer,
                    rows,
                    Source {
                        path: "demo/loaded.clj",
                        source: &loaded,
                    },
                    Source {
                        path: "demo/unloaded.clj",
                        source: &unloaded,
                    },
                ],
                None,
            )
            .files
            .into_iter()
            .find(|file| file.path == consumer.path)
            .unwrap()
    };

    let mut session = Session::default();
    let original = check_consumer(&mut session, true);
    assert!(original.complete && original.ok, "{original:#?}");
    let warm = check_consumer(&mut session, true);
    assert!(warm.complete && warm.ok, "{warm:#?}");
    assert_eq!(warm.stats.reused_functions, 1, "{warm:#?}");

    let moved = check_consumer(&mut session, false);
    let fresh = check_consumer(&mut Session::default(), false);
    let [finding] = moved.diagnostics.as_slice() else {
        panic!("{moved:#?}");
    };
    let [fresh_finding] = fresh.diagnostics.as_slice() else {
        panic!("{fresh:#?}");
    };
    assert_eq!(finding.code, "PNX9001", "{finding:#?}");
    assert!(finding.message.contains(NOT_LOADED), "{finding:#?}");
    assert_eq!(finding.message, fresh_finding.message);
    assert_eq!(moved.stats.checked_functions, 1, "{moved:#?}");
    let warm_gap = check_consumer(&mut session, false);
    assert_eq!(warm_gap.diagnostics[0].message, finding.message);
    assert_eq!(warm_gap.stats.reused_functions, 1, "{warm_gap:#?}");

    let restored = check_consumer(&mut session, true);
    assert!(restored.complete && restored.ok, "{restored:#?}");
    assert_eq!(restored.stats.checked_functions, 1, "{restored:#?}");
}
