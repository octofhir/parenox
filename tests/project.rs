use parenox::{ProjectReport, Session, Source};
use std::collections::BTreeMap;

const EXTERNS: &str = include_str!("../examples/clinic/externs.edn");

#[test]
fn checks_aliased_calls_in_unannotated_bodies_when_source_exports_are_known() {
    let app = r#"(ns app (:require [dep :as d]))
      (defn f [x] (let [a x]
        [(if a (d/titl a) (d/title a))
         '(d/quoted) (opaque (d/macro-data))]))"#;
    let base = "(ns dep) (defn title [x] x)";
    let mut session = Session::default();
    let mut check = |provider| {
        session.check_project(
            &[
                Source {
                    path: "app.clj",
                    source: app,
                },
                Source {
                    path: "dep.clj",
                    source: provider,
                },
            ],
            None,
        )
    };
    let broken = check(base);
    assert!(!broken.complete);
    let unresolved: Vec<_> = broken.files[0]
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == "PNX2304")
        .collect();
    assert_eq!(unresolved.len(), 1, "{:?}", broken.files[0].diagnostics);
    let span = unresolved[0].span;
    assert_eq!(&app[span.start.offset..span.end.offset], "d/titl");

    for provider in [
        "(ns dep) (defn title [x] x) (defn titl [x] x)",
        "(ns dep) (defn title [x] x) (def generated (intern *ns* 'titl identity))",
        "(ns dep) (defn title [x] x) (make-defs)",
        "(ns dep) (defn title [x] x",
    ] {
        let report = check(provider);
        assert!(
            !report.files[0]
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "PNX2304")
        );
    }
}

#[test]
fn checks_source_arities_and_record_factories_without_contracts() {
    let app = r#"(ns shop (:require [catalog :as c]))
      (defn products []
        [(c/->Item "first" 1)
         (c/map->Item {:name "second" :count 2})
         (c/->Item "too few")
         (c/map->Item {} :unexpected)
         (c/label (c/laebl "typo"))
         (c/label)
         (c/label "one" "two" "three")
         (c/dynamic)
         (c/redefined)])"#;
    let provider = r#"(ns catalog)
        (defrecord ^:private Item [name count])
        (defmethod print-method Item [item writer] (.write writer (:name item)))
        (defn label ([x] x) ([x y & more] (list x y more)))
        (defn ^:dynamic dynamic [x] x)
        (defn redefined [x] x)
        (defn redefined [] :last)"#;
    let mut session = Session::default();
    let sources = |provider| {
        [
            Source {
                path: "shop.clj",
                source: app,
            },
            Source {
                path: "catalog.clj",
                source: provider,
            },
        ]
    };
    let report = session.check_project(&sources(provider), None);
    let findings: Vec<_> = report.files[0]
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.severity == "error")
        .map(|diagnostic| {
            let span = diagnostic.span;
            (
                diagnostic.code.as_str(),
                &app[span.start.offset..span.end.offset],
            )
        })
        .collect();
    assert_eq!(
        findings,
        [
            ("PNX2554", "c/->Item"),
            ("PNX2554", "c/map->Item"),
            ("PNX2304", "c/laebl"),
            ("PNX2554", "c/label"),
        ],
        "{report:#?}"
    );
    assert!(!report.complete);

    for name in ["c/->Item", "c/map->Item"] {
        let definition = session
            .definition("shop.clj", app.find(name).unwrap())
            .unwrap();
        assert_eq!(definition.path, "catalog.clj");
        assert_eq!(
            &provider[definition.span.start.offset..definition.span.end.offset],
            "Item"
        );
    }

    let edited_provider = provider.replace("([x] x)", "([] :none) ([x] x)");
    let warm = session.check_project(&sources(&edited_provider), None);
    let cold = Session::default().check_project(&sources(&edited_provider), None);
    let messages = |report: &ProjectReport| {
        report.files[0]
            .diagnostics
            .iter()
            .map(|d| (&d.code, &d.message))
            .map(|(code, message)| (code.clone(), message.clone()))
            .collect::<Vec<_>>()
    };
    assert_eq!(messages(&warm), messages(&cold));
    assert!(
        !messages(&warm)
            .iter()
            .any(|(_, message)| message.contains("c/label expects"))
    );
}

#[test]
fn checks_project_calls_in_standard_test_assertions() {
    let source = r#"
        (ns shop-test (:require [clojure.test :refer :all] [catalog :as catalog]))
        (deftest validates-item
          (testing "look up an item"
            (is (= "Ada" (catalog/label "Ada")))
            (is (nil? (catalog/lable "Ada")))
            (is (= nil (catalog/label)))
            (is (custom-assertion (catalog/quoted-data)))
            '(is (= nil (catalog/quoted-data)))))
    "#;
    let report = Session::default().check_project(
        &[
            Source {
                path: "shop_test.clj",
                source,
            },
            Source {
                path: "catalog.clj",
                source: "(ns catalog) (defn label [name] name)",
            },
        ],
        None,
    );
    assert!(!report.complete);
    let found: Vec<_> = report.files[0]
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.severity == "error")
        .map(|diagnostic| {
            let span = diagnostic.span;
            (
                diagnostic.code.as_str(),
                &source[span.start.offset..span.end.offset],
            )
        })
        .collect();
    assert_eq!(
        found,
        [("PNX2304", "catalog/lable"), ("PNX2554", "catalog/label")],
        "{report:#?}"
    );
}

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

#[test]
fn equivalent_rebuilt_interfaces_reuse_the_callers_check() {
    let app = Source {
        path: "app.clj",
        source: "(ns app (:require [rates :as rates] [fees :as fees]))
                 (defn total {:t/sig '[:-> number?]} [] (+ (rates/base) (fees/current)))",
    };
    let provider = Source {
        path: "rates.clj",
        source: "(ns rates) (defn base {:t/sig '[:-> number?]} [] 5)",
    };
    let externs = Source {
        path: "externs.edn",
        source: "{fees/current [:-> number?] fees/other [:-> string?]}",
    };
    let mut session = Session::default();
    let first = session.check_project(&[app, provider], Some(externs));
    assert!(first.ok && first.complete, "{first:#?}");
    let reordered = Source {
        source: "{fees/other [:-> string?]\n fees/current [:-> number?]}",
        ..externs
    };
    let edited = Source {
        source: "(ns rates) (defn base {:t/sig '[:-> number?]} [] 6)",
        ..provider
    };
    let same_interface = session.check_project(&[app, edited], Some(reordered));
    assert!(
        same_interface.ok && same_interface.complete,
        "{same_interface:#?}"
    );
    assert_eq!(same_interface.stats.parsed_files, 2);
    assert_eq!(same_interface.stats.checked_functions, 1);
    assert_eq!(same_interface.stats.reused_functions, 1);
    let changed = Source {
        source: "{fees/current [:-> string?]}",
        ..externs
    };
    let new_interface = session.check_project(&[app, edited], Some(changed));
    assert!(
        !new_interface.ok && new_interface.complete,
        "{new_interface:#?}"
    );
    assert_eq!(new_interface.stats.checked_functions, 1);
    assert_eq!(
        new_interface.files[0].diagnostics[0].actual.as_deref(),
        Some("String")
    );
}

#[test]
fn source_contracts_win_and_private_vars_do_not_resolve_through_externs() {
    let caller = r#"(ns caller (:require [library :as lib]))
        (defn convert {:t/sig '[:-> string?]} [] (lib/label))
        (defn hidden {:t/sig '[:-> string?]} [] (lib/hidden))
        (defn secret {:t/sig '[:-> string?]} [] (lib/secret))
        (defn external {:t/sig '[:-> string?]} [] (lib/external))"#;
    let library = r#"(ns library)
        (defn label {:t/sig '[:-> number?]} [] 120)
        (defn- hidden {:t/sig '[:-> string?]} [] "internal")
        (def ^:private secret "42")"#;
    let externs = Some(
        "{library/label [:-> string?] library/hidden [:-> string?]
          library/secret [:-> string?] library/external [:-> string?]}",
    );
    let mut files = BTreeMap::from([
        ("caller.clj", caller.to_owned()),
        ("library.clj", library.to_owned()),
    ]);
    let mut session = Session::default();
    let source_wins = run(&mut session, &files, externs);
    assert!(source_wins.complete, "{source_wins:#?}");
    let [result, function, literal] = source_wins.files[0].diagnostics.as_slice() else {
        panic!("{source_wins:#?}")
    };
    assert_eq!(result.code, "PNX2322");
    assert_eq!(result.actual.as_deref(), Some("Number"));
    assert_eq!(function.code, "PNX2304");
    assert_eq!(literal.code, "PNX2304");
    assert!(function.message.contains("lib/hidden"));
    assert!(literal.message.contains("lib/secret"));

    let warm = run(&mut session, &files, externs);
    assert_eq!(warm.files[0].stats.reused_functions, 4);
    assert!(files.remove("library.clj").is_some());
    let fallback = run(&mut session, &files, externs);
    assert!(fallback.ok && fallback.complete, "{fallback:#?}");

    files.insert("library.clj", library.to_owned());
    let restored = run(&mut session, &files, externs);
    assert_eq!(restored.files[0].stats.checked_functions, 4);
    assert_eq!(restored.files[0].diagnostics.len(), 3);
}
