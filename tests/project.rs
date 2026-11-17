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
