use parenox::{Session, Source};

const APP: Source<'_> = Source {
    path: "app.clj",
    source: include_str!("../examples/namespace_specs/app.clj"),
};
const CONFIG: Source<'_> = Source {
    path: "config.clj",
    source: include_str!("../examples/namespace_specs/config.clj"),
};
const HANDLER: Source<'_> = Source {
    path: "handler.clj",
    source: include_str!("../examples/namespace_specs/handler.clj"),
};

#[test]
fn alias_only_requires_do_not_create_cycles_and_spec_keys_follow_their_owner() {
    let mut session = Session::default();
    let first = session.check_project(&[APP, CONFIG, HANDLER], None);
    assert!(first.ok && first.complete, "{first:#?}");
    let reordered = session.check_project(&[HANDLER, CONFIG, APP], None);
    assert!(reordered.ok && reordered.complete, "{reordered:#?}");
    assert_eq!(reordered.stats.parsed_files, 0);
    assert_eq!(reordered.stats.checked_functions, 0);
    assert_eq!(reordered.stats.reused_functions, 4);

    let edited = HANDLER
        .source
        .replace("(s/def ::id string?)", "(s/def ::id number?)");
    let changed = session.check_project(
        &[
            APP,
            CONFIG,
            Source {
                source: &edited,
                ..HANDLER
            },
        ],
        None,
    );
    assert!(!changed.ok && changed.complete, "{changed:#?}");
    assert_eq!(changed.stats.parsed_files, 1);
    let [app, config, handler] = changed.files.as_slice() else {
        panic!("{changed:#?}");
    };
    assert!(handler.ok && config.ok);
    assert_eq!(app.diagnostics[0].expected.as_deref(), Some("Number"));
    assert_eq!(app.diagnostics[0].actual.as_deref(), Some("String"));

    let removed = session.check_project(&[APP, CONFIG], None);
    assert!(!removed.ok && !removed.complete, "{removed:#?}");
    assert_eq!(removed.stats.parsed_files, 0);
    for (file, key) in removed
        .files
        .iter()
        .zip([":wire/id", ":demo.namespace-specs.handler/id"])
    {
        assert!(
            file.diagnostics
                .iter()
                .any(|d| d.message == format!("No declaration for spec {key}")),
            "{file:#?}"
        );
    }
    let restored = session.check_project(&[APP, CONFIG, HANDLER], None);
    assert!(restored.ok && restored.complete, "{restored:#?}");
}

#[test]
fn recursive_and_ambiguous_project_specs_report_the_declaration() {
    let one = Source {
        path: "one.clj",
        source: "(ns one (:require [clojure.spec.alpha :as s]))
                 (s/def :wire/item :wire/other)",
    };
    let two = Source {
        path: "two.clj",
        source: "(ns two (:require [clojure.spec.alpha :as s]))
                 (s/def :wire/other :wire/item)",
    };
    let mut session = Session::default();
    let recursive = session.check_project(&[one, two], None);
    assert!(!recursive.complete);
    assert!(recursive.files[0].diagnostics.is_empty());
    let diagnostic = &recursive.files[1].diagnostics[0];
    assert_eq!(diagnostic.code, "PNX9001");
    assert_eq!(diagnostic.severity, "warning");
    assert!(diagnostic.message.contains("Recursive spec"));
    assert_eq!(
        &two.source[diagnostic.span.start.offset..diagnostic.span.end.offset],
        ":wire/item"
    );

    let duplicate = [
        Source {
            source: "(ns one) (clojure.spec.alpha/def :wire/item string?)",
            ..one
        },
        Source {
            source: "(ns two) (clojure.spec.alpha/def :wire/item number?)",
            ..two
        },
    ];
    for input in [duplicate, [duplicate[1], duplicate[0]]] {
        let ambiguous = session.check_project(&input, None);
        assert!(!ambiguous.ok && !ambiguous.complete, "{ambiguous:#?}");
        for file in ambiguous.files {
            assert_eq!(file.diagnostics.len(), 1);
            assert!(
                file.diagnostics[0]
                    .message
                    .contains("declared in more than one source file")
            );
        }
    }
}

#[test]
fn generation_settings_preserve_the_checked_collection_and_element_types() {
    let source = r#"(ns catalogs (:require [clojure.spec.alpha :as s]))
        (s/def ::label (s/with-gen string? (fn [] (throw (Exception. "generator")))))
        (s/def ::catalog (s/map-of keyword? ::label :gen-max 1))
        (s/def ::labels (s/coll-of ::label :gen-max 1 :kind vector?))
        (defn catalog {:t/sig '[:-> ::catalog]} [] {:a "Ada" :b "Grace"})
        (defn labels {:t/sig '[:-> ::labels]} [] ["Ada" "Grace"])"#;
    let mut session = Session::default();
    let good = session.check("catalogs.clj", source);
    assert!(good.ok && good.complete, "{good:#?}");
    let bad = session.check("catalogs.clj", &source.replace(":a \"Ada\"", ":a nil"));
    let [mismatch] = bad.diagnostics.as_slice() else {
        panic!("{bad:#?}");
    };
    assert_eq!(mismatch.code, "PNX2322");
    assert_eq!(mismatch.severity, "error");
}

#[test]
fn external_fdefs_and_specs_without_a_collection_kind_report_incomplete_coverage() {
    let source = r#"(ns catalogs (:require [clojure.spec.alpha :as s]))
        (s/def ::labels (s/coll-of string? :gen-max 1))
        (s/fdef clojure.core/str :args (s/cat :name string?) :ret string?)"#;
    let report = Session::default().check("catalogs.clj", source);
    assert!(!report.complete);
    assert_eq!(report.diagnostics.len(), 2, "{report:#?}");
    assert!(
        report
            .diagnostics
            .iter()
            .all(|diagnostic| diagnostic.severity == "warning"),
        "{report:#?}"
    );
}

#[test]
fn keys_check_required_presence_without_a_registered_value_spec() {
    let source = r#"(ns wire (:require [clojure.spec.alpha :as s]))
        (s/def ::payload (s/keys :req [::id] :req-un [::name]))
        (s/def ::extras (s/keys :opt [::comment] :opt-un [::note]))
        (defn payload {:t/sig '[:-> ::payload]} [] {::id nil :name 42})
        (defn extras {:t/sig '[map? :-> ::extras]} [m] m)"#;
    let mut session = Session::default();
    let report = session.check("wire.clj", source);
    assert!(!report.complete, "{report:#?}");
    assert_eq!(report.diagnostics.len(), 4, "{report:#?}");
    assert!(
        report.diagnostics.iter().all(|d| {
            d.severity == "warning" && d.message.contains("s/keys checks presence only")
        }),
        "{report:#?}"
    );

    for map in ["{::id nil}", "{:id nil :name 42}", "{::id nil ::name 42}"] {
        let missing = source.replace("{::id nil :name 42}", map);
        let report = session.check("wire.clj", &missing);
        let errors: Vec<_> = report
            .diagnostics
            .iter()
            .filter(|d| d.severity == "error")
            .collect();
        let [diagnostic] = errors.as_slice() else {
            panic!("{report:#?}");
        };
        assert_eq!(diagnostic.code, "PNX2322");
        assert_eq!(
            &missing[diagnostic.span.start.offset..diagnostic.span.end.offset],
            map
        );
    }
}

#[test]
fn registering_a_value_spec_checks_previously_unconstrained_keys() {
    let app = Source {
        path: "app.clj",
        source: r#"(ns app (:require [clojure.spec.alpha :as s]))
            (s/def ::payload (s/keys :req-un [:wire/name]))
            (defn payload {:t/sig '[:-> ::payload]} [] {:name 42})
            (defn read-name {:t/sig '[::payload :-> string?]} [m] (:name m))"#,
    };
    let wire = Source {
        path: "wire.clj",
        source: "(ns wire (:require [clojure.spec.alpha :as s])) (s/def ::name string?)",
    };
    let mut session = Session::default();
    for (sources, expected) in [
        (&[app][..], "PNX9004"),
        (&[app, wire][..], "PNX2322"),
        (&[app][..], "PNX9004"),
    ] {
        let report = session.check_project(sources, None);
        let errors: Vec<_> = report.files[0]
            .diagnostics
            .iter()
            .filter(|d| d.severity == "error")
            .collect();
        let [diagnostic] = errors.as_slice() else {
            panic!("{report:#?}");
        };
        assert_eq!(diagnostic.code, expected, "{report:#?}");
        let text = &app.source[diagnostic.span.start.offset..diagnostic.span.end.offset];
        assert_eq!(
            text,
            if expected == "PNX9004" {
                "(:name m)"
            } else {
                "{:name 42}"
            }
        );
    }
}

#[test]
fn intersections_keep_only_shared_inputs_and_recheck_callers_after_a_spec_edit() {
    let provider = Source {
        path: "routes.clj",
        source: r#"(ns routes (:require [clojure.spec.alpha :as s]))
            (s/def ::mode (s/and (s/nilable keyword?) #{:read :write} #{:read :admin}))
            (s/fdef authorize :args (s/cat :mode ::mode) :ret ::mode)
            (defn authorize [mode] mode)"#,
    };
    let caller = Source {
        path: "app.clj",
        source: r#"(ns app (:require [routes :as routes]))
            (defn view {:t/sig '[:-> keyword?]} [] (routes/authorize :read))"#,
    };
    let mut session = Session::default();
    let good = session.check_project(&[caller, provider], None);
    assert!(good.ok && good.complete, "{good:#?}");

    let edited = provider
        .source
        .replace("#{:read :admin}", "#{:write :admin}");
    let changed = session.check_project(
        &[
            caller,
            Source {
                source: &edited,
                ..provider
            },
        ],
        None,
    );
    assert!(changed.complete, "{changed:#?}");
    let [diagnostic] = changed.files[0].diagnostics.as_slice() else {
        panic!("{changed:#?}");
    };
    assert_eq!(diagnostic.code, "PNX2322");
    assert_eq!(diagnostic.expected.as_deref(), Some(":write"));
    assert_eq!(diagnostic.actual.as_deref(), Some(":read"));
}

#[test]
fn projecting_a_tagged_choice_checks_the_raw_input_not_the_conformed_tuple() {
    let source = r#"(ns routes (:require [clojure.spec.alpha :as s]))
        (s/def ::route (s/and (s/or :key keyword? :text string?)
                             (s/conformer clojure.core/second)))
        (defn route-id [value] value)
        (s/fdef route-id :args (s/cat :value ::route) :ret ::route)
        (defn key-route {:t/sig '[:-> ::route]} [] (route-id :demo))
        (defn text-route {:t/sig '[:-> ::route]} [] (route-id "demo"))"#;
    let mut session = Session::default();
    let good = session.check("routes.clj", source);
    assert!(good.ok && good.complete, "{good:#?}");
    let unqualified = source.replace("clojure.core/second", "second");
    let scoped = session.check("routes.clj", &unqualified);
    assert!(scoped.ok && scoped.complete, "{scoped:#?}");
    let bad = session.check(
        "routes.clj",
        &source.replace("(route-id :demo)", "(route-id [:key :demo])"),
    );
    let [diagnostic] = bad.diagnostics.as_slice() else {
        panic!("{bad:#?}");
    };
    assert_eq!(diagnostic.code, "PNX2322");
}

#[test]
fn opaque_predicates_and_conforming_stages_are_not_accepted_as_exact_contracts() {
    let source = Source {
        path: "routes.clj",
        source: r#"(ns routes (:require [clojure.spec.alpha :as s]))
            (s/def ::named (s/and string? #(re-matches #"[a-z]+" %)))
            (s/def ::converted (s/and keyword? (s/conformer name)))
            (s/def ::tagged (s/and (s/or :tag keyword?) keyword?))
            (defn second [_] :clojure.spec.alpha/invalid)
            (s/def ::removing-tag (s/and (s/or :tag keyword?) (s/conformer second)))
            (defn string? [_] true)
            (s/def ::custom-string (s/and (s/nilable string?) #{:demo}))
            (defn choose {:t/sig '[::tagged :-> keyword?]} [_] :demo)"#,
    };
    let report = Session::default().export_externs(&[source], &[]);
    assert!(
        !report.report.complete && report.edn.is_none(),
        "{report:?}"
    );
    let diagnostics = &report.report.files[0].diagnostics;
    assert!(
        diagnostics.iter().all(|d| d.severity == "warning"),
        "{diagnostics:#?}"
    );
    for expected in [
        "Custom predicate in s/and",
        "Input and result of this s/conformer",
        "This s/and stage checks a conformed value",
        "Source predicate string? is unchecked",
    ] {
        assert!(
            diagnostics.iter().any(|d| d.message.contains(expected)),
            "{diagnostics:#?}"
        );
    }
    assert_eq!(
        diagnostics
            .iter()
            .filter(|d| d.message.contains("this s/conformer"))
            .count(),
        2
    );
}

#[test]
fn quoted_inline_conjunctions_track_a_shadowing_predicate() {
    let source = r#"(ns routes (:require [clojure.spec.alpha :as s]))
        (defn string? [_] true)
        (defn route {:t/sig '[(s/and string? #{:demo}) :-> keyword?]} [_] :demo)"#;
    let checked = Session::default().check("routes.clj", source);
    assert!(!checked.complete, "{checked:#?}");
    assert!(
        checked.diagnostics.iter().any(|d| {
            d.severity == "warning" && d.message.contains("Custom predicate in s/and")
        }),
        "{checked:#?}"
    );
}
