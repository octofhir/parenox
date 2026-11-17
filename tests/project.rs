use parenox::{ProjectReport, Session, Source};
use std::collections::BTreeMap;

const EXTERNS: &str = include_str!("../examples/clinic/externs.edn");

fn clinic() -> BTreeMap<&'static str, String> {
    [
        (
            "clinic/model.clj",
            include_str!("../examples/clinic/model.clj"),
        ),
        (
            "clinic/store.clj",
            include_str!("../examples/clinic/store.clj"),
        ),
        (
            "clinic/service.clj",
            include_str!("../examples/clinic/service.clj"),
        ),
        (
            "clinic/http.clj",
            include_str!("../examples/clinic/http.clj"),
        ),
    ]
    .into_iter()
    .map(|(path, source)| (path, source.into()))
    .collect()
}

fn run(
    session: &mut Session,
    files: &BTreeMap<&str, String>,
    externs: Option<&str>,
) -> ProjectReport {
    let sources: Vec<_> = files
        .iter()
        .map(|(path, source)| Source { path, source })
        .collect();
    session.check_project(
        &sources,
        externs.map(|source| Source {
            path: "externs.edn",
            source,
        }),
    )
}

fn fix(files: &mut BTreeMap<&str, String>) {
    let service = files.get_mut("clinic/service.clj").unwrap();
    *service = service.replace(
        "(str/upper-case (:name patient))",
        "(str/upper-case (or (:name patient) \"Unknown\"))",
    );
    let http = files.get_mut("clinic/http.clj").unwrap();
    *http = http
        .replace("(store/find-patient 42)", "(store/find-patient id)")
        .replace("(str/starts-with? 42", "(str/starts-with? id");
}

#[test]
fn clinic_errors_fixes_and_warm_checks_preserve_cross_file_types() {
    let mut files = clinic();
    let mut session = Session::default();
    let initial = run(&mut session, &files, Some(EXTERNS));
    assert!(!initial.ok && initial.complete, "{initial:#?}");
    assert_eq!(
        initial
            .files
            .iter()
            .map(|f| f.diagnostics.len())
            .sum::<usize>(),
        3
    );
    assert_eq!(
        initial
            .files
            .iter()
            .find(|f| f.path.ends_with("http.clj"))
            .unwrap()
            .diagnostics
            .len(),
        2
    );
    assert_eq!(initial.stats.parsed_files, 5);
    fix(&mut files);
    let fixed = run(&mut session, &files, Some(EXTERNS));
    assert!(fixed.ok && fixed.complete, "{fixed:#?}");
    assert_eq!(fixed.stats.parsed_files, 2);
    assert_eq!(fixed.stats.checked_functions, 2);
    assert_eq!(fixed.stats.reused_functions, 1);
    let warm = run(&mut session, &files, Some(EXTERNS));
    assert!(warm.ok);
    assert_eq!(warm.stats.parsed_files, 0);
    assert_eq!(warm.stats.checked_functions, 0);
    assert_eq!(warm.stats.reused_functions, 3);
}

#[test]
fn body_edits_reuse_consumers_but_signature_and_extern_changes_recheck_them() {
    let mut files = clinic();
    fix(&mut files);
    let mut session = Session::default();
    assert!(run(&mut session, &files, Some(EXTERNS)).ok);
    let service = files.get_mut("clinic/service.clj").unwrap();
    *service = service.replace("Unknown", "Anonymous");
    let body = run(&mut session, &files, Some(EXTERNS));
    assert!(body.ok);
    assert_eq!(body.stats.checked_functions, 1);
    assert_eq!(body.stats.reused_functions, 2);
    let service = files.get_mut("clinic/service.clj").unwrap();
    *service = service.replace(
        "'[::model/patient :-> string?]",
        "'[::model/patient :-> number?]",
    );
    let changed = run(&mut session, &files, Some(EXTERNS));
    assert!(!changed.ok && changed.complete);
    assert_eq!(changed.stats.checked_functions, 2);
    assert_eq!(changed.stats.reused_functions, 1);
    assert!(
        !changed
            .files
            .iter()
            .find(|f| f.path.ends_with("http.clj"))
            .unwrap()
            .ok
    );
    let mut files = clinic();
    fix(&mut files);
    let external = EXTERNS.replace("[string? string?", "[number? string?");
    let changed = run(&mut session, &files, Some(&external));
    assert!(!changed.ok && changed.complete);
    assert!(
        !changed
            .files
            .iter()
            .find(|f| f.path.ends_with("http.clj"))
            .unwrap()
            .ok
    );
}

#[test]
fn removed_sources_or_contracts_do_not_leave_stale_declarations() {
    let mut files = clinic();
    fix(&mut files);
    let mut session = Session::default();
    assert!(run(&mut session, &files, Some(EXTERNS)).ok);
    let missing_contract = run(&mut session, &files, None);
    assert!(!missing_contract.ok && !missing_contract.complete);
    assert!(
        missing_contract
            .files
            .iter()
            .flat_map(|f| &f.diagnostics)
            .any(|d| d.message.contains("starts-with?"))
    );
    files.remove("clinic/store.clj");
    let removed = run(&mut session, &files, Some(EXTERNS));
    assert!(!removed.ok && !removed.complete);
    assert!(
        removed
            .files
            .iter()
            .flat_map(|f| &f.diagnostics)
            .any(|d| d.message.contains("find-patient"))
    );
}

#[test]
fn an_extern_file_preserves_the_interface_of_a_source_named_externs() {
    let mut files = BTreeMap::from([
        (
            "caller.clj",
            "(ns caller (:require [externs :as provider]))
             (defn call {:t/sig '[:-> string?]} [] (provider/value))"
                .into(),
        ),
        (
            "externs.clj",
            "(ns externs)
             (defn value {:t/sig '[:-> string?]} [] \"hello\")"
                .into(),
        ),
    ]);
    // The EDN file contributes a distinct contract in the same namespace.
    let externs = Some("{externs/clock [:-> number?]}");
    let mut session = Session::default();
    assert!(run(&mut session, &files, externs).ok);
    assert_eq!(run(&mut session, &files, externs).stats.reused_functions, 2);

    *files.get_mut("externs.clj").unwrap() = "(ns externs)
         (defn value {:t/sig '[:-> number?]} [] 42)"
        .into();
    let changed = run(&mut session, &files, externs);
    assert!(!changed.ok && changed.complete, "{changed:#?}");
    assert_eq!(changed.stats.checked_functions, 2);
    let [caller, provider, external] = changed.files.as_slice() else {
        panic!("{changed:#?}");
    };
    assert!(provider.ok && external.ok);
    assert_eq!(caller.diagnostics[0].expected.as_deref(), Some("String"));
    assert_eq!(caller.diagnostics[0].actual.as_deref(), Some("Number"));
}

#[test]
fn unchanged_externs_follow_edited_removed_and_restored_spec_declarations() {
    let model = "(ns model (:require [clojure.spec.alpha :as s]))
                 (s/def ::value string?)";
    let mut files = BTreeMap::from([
        (
            "caller.clj",
            "(ns caller (:require [vendor :as v]))
             (defn call {:t/sig '[:-> string?]} [] (v/value))"
                .into(),
        ),
        ("model.clj", model.into()),
    ]);
    let externs = Some("{vendor/value [:-> :model/value]}");
    let mut session = Session::default();
    assert!(run(&mut session, &files, externs).ok);
    let warm = run(&mut session, &files, externs);
    assert_eq!(warm.stats.parsed_files, 0);
    assert_eq!(warm.stats.reused_functions, 1);

    files.insert("model.clj", model.replace("string?", "number?"));
    let changed = run(&mut session, &files, externs);
    assert!(!changed.ok && changed.complete, "{changed:#?}");
    assert_eq!(changed.stats.parsed_files, 1);
    assert_eq!(changed.stats.checked_functions, 1);
    assert_eq!(
        changed.files[0].diagnostics[0].actual.as_deref(),
        Some("Number")
    );

    files.remove("model.clj");
    let removed = run(&mut session, &files, externs);
    assert!(!removed.ok && !removed.complete, "{removed:#?}");
    let external = removed.files.last().unwrap();
    assert!(
        external.diagnostics[0]
            .message
            .contains("No declaration for spec :model/value")
    );

    files.insert("model.clj", model.into());
    let restored = run(&mut session, &files, externs);
    assert!(restored.ok && restored.complete, "{restored:#?}");
    assert_eq!(restored.stats.checked_functions, 1);
}

#[test]
fn a_path_changing_from_externs_to_source_is_checked_in_its_new_role() {
    let mut session = Session::default();
    let externs = Source {
        path: "same.clj",
        source: "{}",
    };
    assert!(session.check_project(&[], Some(externs)).ok);
    let source = session.check(externs.path, externs.source);
    assert!(!source.ok && !source.complete, "{source:#?}");
    assert!(
        source.diagnostics[0]
            .message
            .contains("Unsupported top-level form")
    );
    assert!(session.check_project(&[], Some(externs)).ok);
}

#[test]
fn an_extern_edit_only_invalidates_callers_of_its_namespace() {
    let files = BTreeMap::from([
        (
            "caller.clj",
            "(ns caller (:require [vendor.text :as text]))
             (defn format-value {:t/sig '[:-> string?]} [] (text/label 42))"
                .into(),
        ),
        (
            "clock.clj",
            "(ns clock (:require [vendor.clock :refer [now]]))
             (defn read-clock {:t/sig '[:-> number?]} [] (now))"
                .into(),
        ),
    ]);
    let externs = "{vendor.text/label [number? :-> string?]
                    vendor.clock/now [:-> number?]}";
    let mut session = Session::default();
    assert!(run(&mut session, &files, Some(externs)).ok);
    let changed = externs.replace("clock/now [:-> number?]", "clock/now [:-> string?]");
    let report = run(&mut session, &files, Some(&changed));
    assert!(!report.ok && report.complete, "{report:#?}");
    assert_eq!(report.stats.checked_functions, 1);
    assert_eq!(report.stats.reused_functions, 1);
    assert!(report.files[0].ok);
    assert_eq!(
        report.files[1].diagnostics[0].actual.as_deref(),
        Some("String")
    );

    let restored = run(&mut session, &files, Some(externs));
    assert!(restored.ok && restored.complete, "{restored:#?}");
    assert_eq!(restored.stats.checked_functions, 1);
}

#[test]
fn namespace_graph_rejects_cycles_duplicates_and_private_imports() {
    let cycle = [
        Source {
            path: "a.clj",
            source: "(ns a (:require [b :as b]))",
        },
        Source {
            path: "b.clj",
            source: "(ns b (:require [a :as a]))",
        },
    ];
    let result = Session::default().check_project(&cycle, None);
    assert!(!result.complete);
    assert!(
        result
            .files
            .iter()
            .all(|f| f.diagnostics.iter().any(|d| d.code == "PNX9005"))
    );
    let duplicate = [
        Source {
            path: "a.clj",
            source: "(ns same)",
        },
        Source {
            path: "b.clj",
            source: "(ns same)",
        },
    ];
    assert!(!Session::default().check_project(&duplicate, None).ok);
    let private = [
        Source {
            path: "a.clj",
            source: "(ns a) (defn- hidden {:t/sig '[:-> number?]} [] 1)",
        },
        Source {
            path: "b.clj",
            source: "(ns b (:require [a :as a])) (defn f {:t/sig '[:-> number?]} [] (a/hidden))",
        },
    ];
    assert!(!Session::default().check_project(&private, None).ok);
    let referred = [
        Source {
            path: "a.clj",
            source: "(ns a) (defn f {:t/sig '[:-> number?]} [] 1)",
        },
        Source {
            path: "b.clj",
            source: "(ns b (:require [a :refer [f]])) (defn g {:t/sig '[:-> number?]} [] (f))",
        },
    ];
    assert!(Session::default().check_project(&referred, None).ok);
}

#[test]
fn bare_requires_and_referred_renames_follow_the_provider_signature() {
    let provider = include_str!("../examples/libspecs/provider.clj");
    let mut files = BTreeMap::from([
        (
            "app.clj",
            include_str!("../examples/libspecs/app.clj").into(),
        ),
        ("provider.clj", provider.into()),
    ]);
    let mut session = Session::default();
    let initial = run(&mut session, &files, None);
    assert!(initial.ok && initial.complete, "{initial:#?}");
    assert_eq!(run(&mut session, &files, None).stats.reused_functions, 4);

    files.insert(
        "provider.clj",
        provider.replace(
            "[number? :-> number?]} [count]\n  (+ count 2)",
            "[number? :-> string?]} [count]\n  (str count)",
        ),
    );
    let changed = run(&mut session, &files, None);
    assert!(!changed.ok && changed.complete, "{changed:#?}");
    let diagnostic = &changed.files[0].diagnostics[0];
    assert_eq!(diagnostic.actual.as_deref(), Some("String"));
    assert_eq!(diagnostic.expected.as_deref(), Some("Number"));
    assert!(changed.files[1].ok);
}

#[test]
fn referring_renames_only_selected_names_and_preserves_their_binding() {
    let namespaces = "(ns caller (:require [vendor :refer [label skip] :exclude (skip)
                     :rename {label render skip render-skip nonexport unused}]))";
    let externs = Some("{vendor/label [string? :-> string?] vendor/skip [:-> string?]}");
    for body in ["(render \"Ada\")", "(let [render \"local\"] render)"] {
        let files = BTreeMap::from([(
            "caller.clj",
            format!("{namespaces} (defn f {{:t/sig '[:-> string?]}} [] {body})"),
        )]);
        let report = run(&mut Session::default(), &files, externs);
        assert!(report.ok && report.complete, "{report:#?}");
    }
    for name in ["label", "skip", "render-skip", "unused"] {
        let files = BTreeMap::from([(
            "caller.clj",
            format!("{namespaces} (defn f {{:t/sig '[:-> string?]}} [] ({name}))"),
        )]);
        let report = run(&mut Session::default(), &files, externs);
        assert_eq!(
            report.files[0].diagnostics[0].code, "PNX2304",
            "{report:#?}"
        );
    }
    let files = BTreeMap::from([(
        "caller.clj",
        format!("{namespaces} (defn render {{:t/sig '[:-> string?]}} [] \"conflict\")"),
    )]);
    let report = run(&mut Session::default(), &files, externs);
    assert_eq!(
        report.files[0].diagnostics[0].code, "PNX2790",
        "{report:#?}"
    );
    assert!(
        report.files[0].diagnostics[0]
            .message
            .contains("render already refers to vendor/label")
    );
}

#[test]
fn qualified_slash_names_follow_source_and_extern_interface_edits() {
    let app = Source {
        path: "app.clj",
        source: "(ns app) (defn run {:t/sig '[:-> number?]} [] (rates// 4))",
    };
    let provider = Source {
        path: "rates.clj",
        source: "(ns rates (:refer-clojure :exclude [/]))
                 (defn / {:t/sig '[number? :-> number?]} [n] (inc n))",
    };
    let mut session = Session::default();
    assert!(session.check_project(&[app, provider], None).ok);
    let edited = provider
        .source
        .replace(":-> number?", ":-> string?")
        .replace("(inc n)", "(str n)");
    let changed = session.check_project(
        &[
            app,
            Source {
                source: &edited,
                ..provider
            },
        ],
        None,
    );
    assert!(!changed.ok && changed.complete, "{changed:#?}");
    assert_eq!(changed.stats.checked_functions, 2);
    assert_eq!(
        changed.files[0].diagnostics[0].actual.as_deref(),
        Some("String")
    );

    let externs = Source {
        path: "externs.edn",
        source: "{rates// [number? :-> number?]}",
    };
    assert!(session.check_project(&[app], Some(externs)).ok);
    let changed = session.check_project(
        &[app],
        Some(Source {
            source: "{rates// [number? :-> string?]}",
            ..externs
        }),
    );
    assert!(!changed.ok && changed.complete, "{changed:#?}");
    assert_eq!(changed.stats.checked_functions, 1);
    assert_eq!(
        changed.files[0].diagnostics[0].actual.as_deref(),
        Some("String")
    );
}
