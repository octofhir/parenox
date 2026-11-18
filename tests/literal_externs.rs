use parenox::{ContractKind, ExternChangeKind, ExternSnapshot, Session, Source, compare_externs};

const CONFIG: Source<'_> = Source {
    path: "examples/literal_vars/config.clj",
    source: include_str!("../examples/literal_vars/config.clj"),
};
const APP: Source<'_> = Source {
    path: "examples/literal_vars/app.clj",
    source: include_str!("../examples/literal_vars/app.clj"),
};

fn export(source: Source<'_>) -> String {
    let result = Session::default().export_externs(&[source], &[]);
    result.edn.unwrap_or_else(|| panic!("{:#?}", result.report))
}

fn externs(data: &str) -> Source<'_> {
    Source {
        path: "api.edn",
        source: data,
    }
}

fn compare(before: &str, after: &str) -> parenox::ExternDiff {
    compare_externs(
        ExternSnapshot {
            sources: &[],
            externs: &[externs(before)],
        },
        ExternSnapshot {
            sources: &[],
            externs: &[externs(after)],
        },
    )
}

#[test]
fn exported_literals_check_client_without_the_provider_source() {
    let snapshot = export(CONFIG);
    assert!(snapshot.contains("demo.literal-vars.config/default-plan {:value {:limit 4 :mode :demo.literal-vars.config/online}}"));
    assert!(!snapshot.contains("demo.literal-vars.config/fallback "));
    let mut session = Session::default();
    let client = session.check_project_with_externs(&[APP], &[externs(&snapshot)]);
    assert!(client.ok && client.complete, "{client:#?}");
    let validation = session.validate_externs(&[CONFIG], &[externs(&snapshot)]);
    assert!(validation.ok && validation.complete, "{validation:#?}");
}

#[test]
fn validation_detects_value_changes_even_if_the_inferred_type_is_the_same() {
    let library = Source {
        path: "library.clj",
        source: "(ns lib) (def ratio 2/3) (def ^:private hidden nil)",
    };
    let snapshot = export(library);
    let changed = Source {
        source: "(ns lib) (def ratio 3/4)",
        ..library
    };
    let mut session = Session::default();
    let stale = session.validate_externs(&[changed], &[externs(&snapshot)]);
    assert!(!stale.ok && stale.complete, "{stale:#?}");
    let [finding] = stale.files[1].diagnostics.as_slice() else {
        panic!("{stale:#?}")
    };
    assert_eq!(finding.expected.as_deref(), Some("2/3"));
    assert_eq!(finding.actual.as_deref(), Some("3/4"));
    assert_eq!(
        finding.contract.as_ref().unwrap().kind,
        ContractKind::ExternLiteral
    );

    let private = session.validate_externs(&[library], &[externs("{lib/hidden {:value nil}}")]);
    assert!(!private.ok && !private.complete, "{private:#?}");
    assert_eq!(private.files[1].diagnostics[0].code, "PNX9011");
    let absent = session.validate_externs(&[], &[externs(&snapshot)]);
    assert!(!absent.ok && !absent.complete, "{absent:#?}");
}

#[test]
fn diffs_distinguish_value_changes_from_function_changes_and_ignore_order_of_maps_and_sets() {
    let before = "{lib/data {:value {:enabled #{:a :b} :ratio 0.75}}
                   lib/same {:value [nil \\newline \"café\" (not-a-call x)]}}";
    let reflowed = "{lib/same {:value [nil \\u000a \"café\" (not-a-call x)]}
                    lib/data {:value {:ratio 0.75 :enabled #{:b :a}}}}";
    let unchanged = compare(before, reflowed);
    assert!(
        unchanged.complete && unchanged.compatible && unchanged.changes.is_empty(),
        "{unchanged:#?}"
    );
    let changed = compare(before, &reflowed.replace("0.75", "0.50"));
    assert!(changed.complete && !changed.compatible, "{changed:#?}");
    let [change] = changed.changes.as_slice() else {
        panic!("{changed:#?}")
    };
    assert_eq!(change.kind, ExternChangeKind::ValueChanged);
    assert_eq!(change.name, "lib/data");
    assert_eq!(change.argument_count, None);
    assert_eq!(
        serde_json::to_value(change).unwrap()["kind"],
        "value_changed"
    );
    let replaced = compare("{lib/data {:value nil}}", "{lib/data [:-> nil?]}");
    assert!(replaced.complete && !replaced.compatible, "{replaced:#?}");
    assert_eq!(replaced.changes[0].kind, ExternChangeKind::Breaking);
}

#[test]
fn exported_data_keeps_lists_symbols_and_exact_numbers_without_executing_them() {
    let library = Source {
        path: "data.clj",
        source: r#"(ns literal)
            (def values (quote (not-a-call {:key [::tag \λ "x\n🙂" 16r10 1N]} 'symbol)))
            (def pair [(quote sym) '(:ok nil) 1M])
            (defn fourth {:t/sig '[:-> number?]} [] (nth [0 1 2 3] 3))"#,
    };
    let snapshot = export(library);
    assert!(
        snapshot.contains(
            "(not-a-call {:key [:literal/tag \\u03bb \"x\\n🙂\" 16r10 1N]} (quote symbol))"
        ),
        "{snapshot}"
    );
    assert!(
        snapshot.contains("{:value [sym (:ok nil) 1M]}"),
        "{snapshot}"
    );
    let result = Session::default().validate_externs(&[library], &[externs(&snapshot)]);
    assert!(result.ok && result.complete, "{result:#?}");
}

#[test]
fn changing_or_removing_an_extern_literal_invalidates_only_affected_callers() {
    let app = Source {
        path: "callers.clj",
        source: "(ns consumer (:require [lib :as l]))
            (defn label {:t/sig '[:-> string?]} [] (:name l/defaults))
            (defn same {:t/sig '[:-> number?]} [] 2)",
    };
    let mut session = Session::default();
    let good = externs("{lib/defaults {:value {:name \"Untitled\"}}}");
    let bad = externs("{lib/defaults {:value {:name 1}}}");
    assert!(session.check_project_with_externs(&[app], &[good]).ok);
    let changed = session.check_project_with_externs(&[app], &[bad]);
    assert!(!changed.ok && changed.complete, "{changed:#?}");
    assert_eq!(changed.stats.checked_functions, 1);
    assert_eq!(changed.stats.reused_functions, 1);
    let error = &changed.files[0].diagnostics[0];
    assert_eq!(error.code, "PNX2322");
    assert!(
        error
            .notes
            .iter()
            .any(|note| note.contains("lib/defaults") && note.contains("api.edn"))
    );
    let renamed = session.check_project_with_externs(
        &[app],
        &[Source {
            path: "library.edn",
            ..bad
        }],
    );
    assert_eq!(renamed.stats.checked_functions, 1);
    assert!(
        renamed.files[0].diagnostics[0]
            .notes
            .iter()
            .any(|note| note.contains("library.edn"))
    );
    let removed = session.check_project_with_externs(&[app], &[]);
    assert!(!removed.ok && !removed.complete, "{removed:#?}");
    assert_eq!(removed.files[0].diagnostics[0].code, "PNX9003");
}

#[test]
fn invalid_or_conflicting_literal_externs_are_not_published() {
    let mut session = Session::default();
    let value = externs("{lib/data {:value nil}}");
    let function = Source {
        path: "other.edn",
        source: "{lib/data [:-> number?]}",
    };
    let conflict = session.check_project_with_externs(&[], &[value, function]);
    assert!(!conflict.ok && conflict.complete, "{conflict:#?}");
    assert!(
        conflict
            .files
            .iter()
            .all(|file| file.diagnostics[0].code == "PNX2790")
    );
    for (bad, code) in [
        ("{lib/data {:value #vendor/read [1 2]}}", "PNX9001"),
        ("{lib/data {:value 'x}}", "PNX9001"),
        ("{lib/data {:value #{:x :x}}}", "PNX9000"),
        ("{lib/data {:value {1 :x}}}", "PNX9001"),
        ("{lib/data {:value nil :macro :let-like}}", "PNX9001"),
    ] {
        let invalid = compare(value.source, bad);
        assert!(
            !invalid.complete && invalid.changes.is_empty(),
            "{invalid:#?}"
        );
        assert_eq!(invalid.after.files[0].diagnostics[0].code, code);
    }
}

#[test]
fn public_source_values_win_and_private_source_values_do_not_fall_back_to_externs() {
    let library = Source {
        path: "library.clj",
        source: "(ns lib) (def label \"live\") (def ^:private secret \"private\")",
    };
    let app = Source {
        path: "consumer.clj",
        source: "(ns consumer (:require [lib :as l]))
            (defn label {:t/sig '[:-> string?]} [] l/label)
            (defn hidden {:t/sig '[:-> string?]} [] l/secret)",
    };
    let snapshot = externs("{lib/label {:value 0} lib/secret {:value \"stale\"}}");
    let report = Session::default().check_project_with_externs(&[library, app], &[snapshot]);
    let [error] = report.files[1].diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(error.code, "PNX2304");
    assert!(error.message.contains("l/secret"));
}
