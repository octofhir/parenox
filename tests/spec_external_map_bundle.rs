use parenox::{Session, Source};

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
fn incomplete_or_conflicting_bundles_never_supply_children() {
    let only_root = Source {
        path: "root.edn",
        source: "{:wire/capabilities {:spec (clojure.spec.alpha/keys :req-un [:wire/label]) :ns wire}}",
    };
    let label = Source {
        path: "label.edn",
        source: "{:wire/label {:spec clojure.core/string? :ns wire}}",
    };
    let broken = Session::default().check_project_with_externs(&[CLIENT], &[only_root, label]);
    assert!(
        broken.files[1]
            .diagnostics
            .iter()
            .any(|d| d.message.contains("child specs in this file"))
    );

    let edn = bundle();
    let sidecar = Source {
        path: "bundle.edn",
        source: &edn,
    };
    let duplicated = Session::default().check_project_with_externs(&[CLIENT], &[sidecar, label]);
    assert!(
        duplicated.files[0]
            .diagnostics
            .iter()
            .any(|d| d.message.contains("supported conversion"))
    );
    assert!(
        duplicated.files[2]
            .diagnostics
            .iter()
            .any(|d| d.code == "PNX2790")
    );

    let mut session = Session::default();
    let proposed = session
        .propose_contracts(&[LIBRARY], &[label], &[ROOT.into()])
        .unwrap();
    assert_eq!(proposed.externs.as_deref(), Some("{}\n"));
    assert!(
        proposed
            .edn
            .unwrap()
            .contains("child already supplied by an extern")
    );
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
