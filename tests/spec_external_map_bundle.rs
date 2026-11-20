use parenox::{ExternChangeKind, ExternSnapshot, Session, Source, compare_externs};

const LIBRARY: Source<'_> = Source {
    path: "wire.clj",
    source: r#"
(ns wire (:require [clojure.spec.alpha :as s]))
(s/def ::label string?)
(s/def ::lens (s/conformer (fn [input] {:resolve input})))
(s/def ::capabilities (s/keys :req-un [::label] :opt-un [::lens]))
"#,
};
const ROOT: &str = ":wire/capabilities";
const ROOT_SIDECAR: Source<'_> = Source {
    path: "root.edn",
    source: r#"{:wire/capabilities {:spec (clojure.spec.alpha/keys :req-un [:wire/label] :opt-un [:wire/lens]) :ns wire}}"#,
};
const LABEL_SIDECAR: Source<'_> = Source {
    path: "label.edn",
    source: "{:wire/label {:spec clojure.core/string? :ns wire}}",
};
const LENS_SIDECAR: Source<'_> = Source {
    path: "lens.edn",
    source: r#"{:wire/lens {:spec (clojure.spec.alpha/conformer (clojure.core/fn [input] {:resolve input})) :ns wire}}"#,
};
const CLIENT: Source<'_> = Source {
    path: "client.clj",
    source: r#"
(ns client (:require [clojure.spec.alpha :as s] [wire :as w]))
(defn conform {:t/sig '[:-> {:label #{"ok"}
                             :lens {:resolve #{false}}
                             :wire/lens {:resolve #{::s/invalid}}}]} []
  (s/conform ::w/capabilities {:label "ok" :lens false :wire/lens ::s/invalid}))
(defn missing-required {:t/sig '[:-> #{::s/invalid}]} []
  (s/conform ::w/capabilities {:other/unknown true}))
(defn not-a-map {:t/sig '[:-> #{::s/invalid}]} []
  (s/conform ::w/capabilities nil))
(defn valid {:t/sig '[:-> (s/tuple #{true} #{false} #{false})]} []
  [(s/valid? ::w/capabilities {:label "ok" :lens nil})
   (s/valid? ::w/capabilities {:label false})
   (s/valid? ::w/capabilities {:other/unknown true})])
"#,
};

fn bundle() -> String {
    let proposed = Session::default()
        .propose_contracts(&[LIBRARY], &[], &[ROOT.into()])
        .unwrap();
    let edn = proposed
        .externs
        .unwrap_or_else(|| panic!("{:#?}", proposed.report));
    assert!(
        edn.contains(ROOT) && edn.contains(":wire/lens") && edn.contains(":wire/label"),
        "{edn}"
    );
    edn
}

#[test]
fn a_bundle_checks_named_children_and_qualified_extras_without_sources() {
    let edn = bundle();
    let sidecar = Source {
        path: "bundle.edn",
        source: &edn,
    };
    let mut session = Session::default();
    let validation = session.validate_externs(&[LIBRARY], &[sidecar]);
    assert!(validation.ok && validation.complete, "{validation:#?}");
    let checked = session.check_project_with_externs(&[CLIENT], &[sidecar]);
    assert!(checked.ok && checked.complete, "{checked:#?}");

    let changed = edn.replace(":resolve input", ":other input");
    let checked = session.check_project_with_externs(
        &[CLIENT],
        &[Source {
            source: &changed,
            ..sidecar
        }],
    );
    let [finding] = checked.files[0].diagnostics.as_slice() else {
        panic!("{checked:#?}")
    };
    assert_eq!(finding.code, "PNX2322");
    assert!(finding.actual.as_deref().unwrap().contains(":other"));
}

#[test]
fn a_bundle_resolves_across_sidecars_and_tracks_child_edits() {
    let sidecars = [ROOT_SIDECAR, LENS_SIDECAR, LABEL_SIDECAR];
    let mut session = Session::default();
    let validated = session.validate_externs(&[LIBRARY], &sidecars);
    assert!(validated.ok && validated.complete, "{validated:#?}");

    let checked = session.check_project_with_externs(&[CLIENT], &sidecars);
    assert!(checked.ok && checked.complete, "{checked:#?}");
    let warm = session.check_project_with_externs(&[CLIENT], &sidecars);
    assert!(warm.ok && warm.complete && warm.stats.reused_functions > 0);

    let changed = LENS_SIDECAR
        .source
        .replace(":resolve input", ":other input");
    let updated = [
        ROOT_SIDECAR,
        Source {
            source: &changed,
            ..LENS_SIDECAR
        },
        LABEL_SIDECAR,
    ];
    let checked = session.check_project_with_externs(&[CLIENT], &updated);
    let [finding] = checked.files[0].diagnostics.as_slice() else {
        panic!("{checked:#?}");
    };
    assert_eq!(finding.code, "PNX2322");
    assert!(finding.actual.as_deref().unwrap().contains(":other"));

    let restored = session.check_project_with_externs(&[CLIENT], &sidecars);
    assert!(restored.ok && restored.complete, "{restored:#?}");

    let drift = session.validate_externs(&[LIBRARY], &updated);
    assert!(
        drift.files[1]
            .diagnostics
            .iter()
            .any(|finding| finding.code == "PNX9011" && finding.message.contains(ROOT)),
        "{drift:#?}"
    );

    let diff = compare_externs(
        ExternSnapshot {
            sources: &[],
            externs: &sidecars,
        },
        ExternSnapshot {
            sources: &[],
            externs: &updated,
        },
    );
    assert_eq!(
        diff.changes
            .iter()
            .map(|change| (change.name.as_str(), change.kind))
            .collect::<Vec<_>>(),
        [
            (ROOT, ExternChangeKind::Unproven),
            (":wire/lens", ExternChangeKind::Unproven),
        ],
        "{diff:#?}"
    );
}

#[test]
fn a_bundle_requires_unambiguous_child_specs_from_its_runtime_namespace() {
    let wrong_owner = Source {
        source: "{:wire/label {:spec clojure.core/string? :ns foreign}}",
        ..LABEL_SIDECAR
    };
    let duplicate_in_file = Source {
        source: r#"{:wire/label {:spec clojure.core/string? :ns wire}
                    :wire/label {:spec #{"duplicate"} :ns wire}}"#,
        ..LABEL_SIDECAR
    };
    let duplicate_in_sidecar = Source {
        path: "duplicate.edn",
        ..LABEL_SIDECAR
    };
    let unsupported_duplicate = Source {
        source: "{:wire/label {:spec clojure.core/number? :ns wire}}",
        ..duplicate_in_sidecar
    };
    for (child_sidecars, duplicate) in [
        (vec![], false),
        (vec![wrong_owner], false),
        (vec![duplicate_in_file], true),
        (vec![LABEL_SIDECAR, duplicate_in_sidecar], true),
        (vec![LABEL_SIDECAR, unsupported_duplicate], true),
    ] {
        let mut sidecars = vec![ROOT_SIDECAR, LENS_SIDECAR];
        sidecars.extend(child_sidecars);
        let report = Session::default().check_project_with_externs(&[CLIENT], &sidecars);
        assert!(!report.complete, "{report:#?}");
        assert!(
            report.files[0]
                .diagnostics
                .iter()
                .any(|d| d.message.contains("supported conversion")),
            "{report:#?}"
        );
        assert!(
            report.files[1]
                .diagnostics
                .iter()
                .any(|d| d.message.contains("supported direct child specs")),
            "{report:#?}"
        );
        if duplicate {
            assert!(
                report.files[3..].iter().any(|file| file
                    .diagnostics
                    .iter()
                    .any(|d| { d.message.to_ascii_lowercase().contains("duplicate") })),
                "{report:#?}"
            );
        }
    }
}

#[test]
fn a_bundle_uses_only_visible_and_trusted_children() {
    let mut session = Session::default();
    let sidecars = [ROOT_SIDECAR, LENS_SIDECAR, LABEL_SIDECAR];
    let checked = session.validate_externs_with_dependencies(&[CLIENT], &[], &sidecars);
    assert!(checked.ok && checked.complete, "{checked:#?}");

    let checked = session.validate_externs_with_dependencies(
        &[CLIENT],
        &[LABEL_SIDECAR],
        &[ROOT_SIDECAR, LENS_SIDECAR],
    );
    let shadow = Source {
        path: "shadow.clj",
        source: "(ns shadow (:require [clojure.spec.alpha :as s])) (s/def :wire/label string?)",
    };
    let shadowed = session.check_project_with_externs(&[CLIENT, shadow], &sidecars);
    for report in [checked, shadowed] {
        assert!(
            report.files[0].diagnostics.iter().any(|finding| {
                finding.code == "PNX9001" && finding.message.contains("supported conversion")
            }),
            "{report:#?}"
        );
    }
}

#[test]
fn proposals_reuse_equivalent_manual_children_and_preserve_conflicting_ones() {
    let mut session = Session::default();
    let proposed = session
        .propose_contracts(&[LIBRARY], &[LABEL_SIDECAR], &[ROOT.into()])
        .unwrap();
    let edn = proposed.externs.unwrap();
    assert!(edn.contains(ROOT) && edn.contains(":wire/lens"), "{edn}");
    assert!(!edn.contains(":wire/label {:spec"), "{edn}");
    let generated = Source {
        path: "generated.edn",
        source: &edn,
    };
    let checked = session.check_project_with_externs(&[CLIENT], &[generated, LABEL_SIDECAR]);
    assert!(checked.ok && checked.complete, "{checked:#?}");

    let conflicting = Source {
        source: r#"{:wire/label {:spec #{"different"} :ns wire}}"#,
        ..LABEL_SIDECAR
    };
    let proposed = session
        .propose_contracts(&[LIBRARY], &[conflicting], &[ROOT.into()])
        .unwrap();
    assert_eq!(proposed.externs.as_deref(), Some("{}\n"));
}

#[test]
fn proposals_report_an_unresolved_manual_root_without_replacing_it() {
    let proposed = Session::default()
        .propose_contracts(&[LIBRARY], &[ROOT_SIDECAR], &[ROOT.into()])
        .unwrap();
    assert_eq!(proposed.externs.as_deref(), Some("{}\n"));
    let report = proposed.edn.unwrap();
    assert!(report.contains("Existing extern could not be resolved"));
    assert!(report.contains(":extern nil"));
}

#[test]
fn an_alias_only_require_and_an_unknown_qualified_extension_are_reported() {
    let edn = bundle();
    let sidecar = Source {
        path: "bundle.edn",
        source: &edn,
    };
    let check = |source| {
        Session::default().check_project_with_externs(&[Source { source, ..CLIENT }], &[sidecar])
    };
    let alias = CLIENT.source.replace("[wire :as w]", "[wire :as-alias w]");
    let checked = check(&alias);
    assert!(
        checked.files[0]
            .diagnostics
            .iter()
            .any(|d| d.message.contains("registrations are loaded"))
    );

    let dynamic = CLIENT.source.replace(
        ":label \"ok\" :lens false :wire/lens",
        ":label \"ok\" :lens false :other/unknown",
    );
    let checked = check(&dynamic);
    assert!(
        checked.files[0]
            .diagnostics
            .iter()
            .any(|d| d.message.contains("undeclared qualified key"))
    );

    let unknown_validation = format!(
        "{}
(defn validate-extension {{:t/sig '[:-> boolean?]}} []
  (s/valid? ::w/capabilities {{:label \"ok\" :other/unknown ::s/invalid}}))",
        CLIENT.source
    );
    let checked = check(&unknown_validation);
    let [gap] = checked.files[0].diagnostics.as_slice() else {
        panic!("{checked:#?}");
    };
    assert_eq!(gap.code, "PNX9001");
    assert!(gap.message.contains("undeclared qualified key"));

    let opaque = format!(
        "{}
(defn opaque {{:t/sig '[{{:label string?}} :-> map?]}} [input]
  (s/conform ::w/capabilities input))",
        CLIENT.source
    );
    let checked = check(&opaque);
    assert!(
        checked.files[0]
            .diagnostics
            .iter()
            .any(|d| d.message.contains("requires a literal map"))
    );
}

#[test]
fn a_nested_map_spec_needed_by_an_extra_field_stays_unchecked() {
    let consumer = Source {
        source: r#"
(ns client (:require [wire :as w] [clojure.spec.alpha :as s]))
(defn validate {:t/sig '[{:label string?} :-> boolean?]} [input]
  (s/valid? ::w/capabilities {:label "ok" ::w/capabilities input}))
"#,
        ..CLIENT
    };
    let edn = bundle();
    let sidecar = Source {
        path: "bundle.edn",
        source: &edn,
    };
    let from_source = Session::default().check_project(&[consumer, LIBRARY], None);
    let from_bundle = Session::default().check_project_with_externs(&[consumer], &[sidecar]);
    for checked in [from_source, from_bundle] {
        let [gap] = checked.files[0].diagnostics.as_slice() else {
            panic!("{checked:#?}");
        };
        assert_eq!(gap.code, "PNX9001");
        assert!(gap.message.contains("undeclared qualified key"));
    }
}
