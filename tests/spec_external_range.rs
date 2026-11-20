use parenox::{Session, Source};

const LIBRARY: Source<'_> = Source {
    path: "wire.clj",
    source: r#"
(ns wire (:require [clojure.spec.alpha :as s]))
(s/def ::line (s/and integer? (s/conformer int)))
(s/def ::character (s/and integer? (s/conformer int)))
(s/def ::position (s/keys :req-un [::line ::character]))
(s/def ::alternate (s/keys :req-un [::line ::character]))
(s/def ::start ::position)
(s/def ::end ::position)
(s/def ::range (s/keys :req-un [::start ::end]))
"#,
};

const ROOT: Source<'_> = Source {
    path: "range.edn",
    source: r#"
{:wire/range {:spec (clojure.spec.alpha/keys :req-un [:wire/start :wire/end]) :ns wire}
 :wire/start {:spec :wire/position :ns wire}}
"#,
};

const POSITIONS: Source<'_> = Source {
    path: "positions.edn",
    source: r#"
{:wire/end {:spec :wire/position :ns wire}
 :wire/position {:spec (clojure.spec.alpha/keys :req-un [:wire/line :wire/character]) :ns wire}
 :wire/alternate {:spec (clojure.spec.alpha/keys :req-un [:wire/line :wire/character]) :ns wire}}
"#,
};

const LEAVES: Source<'_> = Source {
    path: "leaves.edn",
    source: r#"
{:wire/line {:spec (clojure.spec.alpha/and clojure.core/integer?
                    (clojure.spec.alpha/conformer clojure.core/int)) :ns wire}
 :wire/character {:spec (clojure.spec.alpha/and clojure.core/integer?
                         (clojure.spec.alpha/conformer clojure.core/int)) :ns wire}}
"#,
};

const SIDECARS: [Source<'_>; 3] = [ROOT, POSITIONS, LEAVES];
const CLIENT: Source<'_> = Source {
    path: "client.clj",
    source: r#"
(ns client (:require [clojure.spec.alpha :as s] [wire :as w]))
(defn direct {:t/sig '[:-> {:start {:line int? :character int? :extra #{false}}
                            :end {:line int? :character int?}}]} []
  (s/conform ::w/range {:start {:line 1N :character 2 :extra false}
                        :end {:line 3 :character 4N}}))
(defn local {:t/sig '[:-> #{true}]} []
  (let [start {:line 5 :character 6}
        end (:end {:end {:line 7 :character 8}})]
    (s/valid? ::w/range {:start start :end end})))
(defn missing {:t/sig '[:-> #{false}]} []
  (try (s/valid? ::w/range {:start {:line 2147483648} :end {:line 0 :character 1}})
       (catch ArithmeticException _ true)))
(defn overflow {:t/sig '[:-> #{:overflow}]} []
  (try (s/conform ::w/range {:start {:line 0 :character 1}
                            :end {:line 2147483648 :character 3}})
       (catch ArithmeticException _ :overflow)))
"#,
};

#[test]
fn source_free_ranges_check_literal_endpoints_and_keep_opaque_maps_unchecked() {
    let mut session = Session::default();
    let checked = session.check_project_with_externs(&[CLIENT], &SIDECARS);
    assert!(checked.ok && checked.complete, "{checked:#?}");

    let source = format!(
        "{}\n{}",
        CLIENT.source,
        r#"(defn opaque {:t/sig '[{:line integer? :character integer?} :-> boolean?]} [start]
             (s/valid? ::w/range {:start start :end {:line 0 :character 1}}))"#,
    );
    let checked = session.check_project_with_externs(
        &[Source {
            source: &source,
            ..CLIENT
        }],
        &SIDECARS,
    );
    let [finding] = checked.files[0].diagnostics.as_slice() else {
        panic!("{checked:#?}");
    };
    assert_eq!(finding.code, "PNX9001");
    assert!(finding.message.contains("nested map"), "{finding:#?}");
    assert!(!checked.complete);
}

#[test]
fn source_conversion_accepts_an_endpoint_with_a_nil_alternative() {
    let consumer = Source {
        source: r#"(ns client (:require [clojure.spec.alpha :as s] [wire :as w]))
(defn maybe-range {:t/sig '[boolean? :-> t/any]} [b]
  (s/conform ::w/range {:start (if b {:line 0 :character 0} nil)
                        :end {:line 1 :character 1}}))"#,
        ..CLIENT
    };
    let checked = Session::default().check_project(&[LIBRARY, consumer], None);
    assert!(checked.ok && checked.complete, "{checked:#?}");
}

#[test]
fn validating_an_imported_range_does_not_bound_the_original_integers() {
    let consumer = Source {
        source: r#"(ns client (:require [clojure.spec.alpha :as s] [wire :as w]))
(defn guarded {:t/sig '[map? :-> ::w/range]} [input]
  (if (s/valid? ::w/range input) input (throw (ex-info "invalid range" {}))))"#,
        ..CLIENT
    };
    let checked = Session::default().check_project_with_externs(&[consumer], &SIDECARS);
    assert!(
        checked.files[0].diagnostics.iter().any(|finding| {
            finding.code == "PNX2322" && finding.message.contains("client/guarded")
        }),
        "{checked:#?}"
    );
}

#[test]
fn forward_source_aliases_and_cyclic_extern_aliases_stay_unresolved() {
    let forward = LIBRARY
        .source
        .replace("(s/def ::start ::position)\n", "")
        .replace(
            "(s/def ::position",
            "(s/def ::start ::position)\n(s/def ::position",
        );
    let mut session = Session::default();
    let proposed = session
        .propose_contracts(
            &[Source {
                source: &forward,
                ..LIBRARY
            }],
            &[],
            &[":wire/range".into()],
        )
        .unwrap();
    assert_eq!(proposed.externs.as_deref(), Some("{}\n"));
    assert!(
        proposed.report.files[0].diagnostics.iter().any(|finding| {
            finding.code == "PNX9001"
                && finding
                    .message
                    .contains("requires :wire/position to be registered first")
        }),
        "{:#?}",
        proposed.report
    );

    let cycle = Source {
        source: "{:wire/end {:spec :wire/start :ns wire}}",
        ..POSITIONS
    };
    let root = ROOT
        .source
        .replace(":spec :wire/position", ":spec :wire/end");
    let checked = session.check_project_with_externs(
        &[CLIENT],
        &[
            Source {
                source: &root,
                ..ROOT
            },
            cycle,
            LEAVES,
        ],
    );
    assert!(!checked.complete, "{checked:#?}");
    for target in [":wire/start", ":wire/end"] {
        assert!(
            checked
                .files
                .iter()
                .skip(1)
                .flat_map(|file| &file.diagnostics)
                .any(|finding| {
                    finding.code == "PNX9001"
                        && finding
                            .message
                            .contains(&format!("alias needs a supported target {target}"))
                }),
            "{checked:#?}"
        );
    }
}

#[test]
fn a_manual_alias_to_an_equivalent_spec_still_drifts_from_source() {
    let mut session = Session::default();
    let valid = session.validate_externs(&[LIBRARY], &SIDECARS);
    assert!(valid.ok && valid.complete, "{valid:#?}");

    let custom_spec = Source {
        path: "clojure/spec/alpha.clj",
        source: "(ns clojure.spec.alpha)",
    };
    let shadowed = session.validate_externs(&[LIBRARY, custom_spec], &SIDECARS);
    assert!(
        shadowed.files[2].diagnostics.iter().any(|finding| {
            finding.code == "PNX9011" && finding.message.contains(":wire/start")
        }),
        "{shadowed:#?}"
    );

    let alternate = ROOT
        .source
        .replace(":spec :wire/position", ":spec :wire/alternate");
    let drift = session.validate_externs(
        &[LIBRARY],
        &[
            Source {
                source: &alternate,
                ..ROOT
            },
            POSITIONS,
            LEAVES,
        ],
    );
    assert!(
        drift.files[1].diagnostics.iter().any(|finding| {
            finding.code == "PNX9011" && finding.message.contains(":wire/start")
        }),
        "{drift:#?}"
    );
}

#[test]
fn a_warm_range_tracks_transitive_child_edits_and_trust() {
    let mut session = Session::default();
    let initial = session.check_project_with_externs(&[CLIENT], &SIDECARS);
    assert!(initial.ok && initial.complete, "{initial:#?}");
    let warm = session.check_project_with_externs(&[CLIENT], &SIDECARS);
    assert!(
        warm.ok && warm.complete && warm.stats.reused_functions > 0,
        "{warm:#?}"
    );

    let changed = LEAVES.source.replacen(
        "(clojure.spec.alpha/and clojure.core/integer?\n                    (clojure.spec.alpha/conformer clojure.core/int))",
        "clojure.core/string?",
        1,
    );
    let updated = session.check_project_with_externs(
        &[CLIENT],
        &[
            ROOT,
            POSITIONS,
            Source {
                source: &changed,
                ..LEAVES
            },
        ],
    );
    assert!(
        updated.files[0]
            .diagnostics
            .iter()
            .any(|finding| { finding.code == "PNX2322" && finding.message.contains("direct") }),
        "{updated:#?}"
    );

    let trusted = session.validate_externs_with_dependencies(&[CLIENT], &[], &SIDECARS);
    assert!(trusted.ok && trusted.complete, "{trusted:#?}");
    let mixed =
        session.validate_externs_with_dependencies(&[CLIENT], &[LEAVES], &[ROOT, POSITIONS]);
    assert!(
        mixed.files[0].diagnostics.iter().any(|finding| {
            finding.code == "PNX9001" && finding.message.contains("supported conversion")
        }),
        "{mixed:#?}"
    );
}
