use parenox::{
    ExternChangeKind as Change, ExternDiff, ExternMismatch, ExternMismatchReason as Reason,
    ExternPathStep as Step, ExternSnapshot, Source, compare_externs,
};

fn detail(report: &ExternDiff) -> &ExternMismatch {
    let [change] = report.changes.as_slice() else {
        panic!("{report:#?}")
    };
    change
        .detail
        .as_ref()
        .unwrap_or_else(|| panic!("{report:#?}"))
}

fn diff(before: &str, after: &str) -> ExternDiff {
    compare_externs(
        ExternSnapshot {
            sources: &[],
            externs: &[Source {
                path: "before.edn",
                source: before,
            }],
        },
        ExternSnapshot {
            sources: &[],
            externs: &[Source {
                path: "after.edn",
                source: after,
            }],
        },
    )
}

#[test]
fn reports_each_kind_of_known_change_and_ignores_alpha_renames() {
    let before = "{api/broaden [string? :-> (clojure.spec.alpha/nilable string?)]
                   api/removed [:-> number?]
                   api/boundary [number? :-> string?]
                   api/wrap {:macro :let-like}
                   api/same (t/all [a] [a :-> a])}";
    let after = "{api/new [:-> boolean?]
                  api/broaden [(clojure.spec.alpha/nilable string?) :-> string?]
                  api/boundary [string? :-> string?]
                  api/wrap [:-> string?]
                  api/same (t/all [renamed] [renamed :-> renamed])}";
    let report = diff(before, after);
    assert!(!report.compatible && report.complete, "{report:#?}");
    assert_eq!(
        report
            .changes
            .iter()
            .map(|change| (change.name.as_str(), change.kind, change.argument_count))
            .collect::<Vec<_>>(),
        [
            ("api/boundary", Change::Breaking, Some(1)),
            ("api/broaden", Change::Compatible, None),
            ("api/new", Change::Added, None),
            ("api/removed", Change::Removed, None),
            ("api/wrap", Change::Breaking, None),
        ]
    );
    let boundary = &report.changes[0];
    assert_eq!(
        boundary.before.as_ref().unwrap().origin.path.as_deref(),
        Some("before.edn")
    );
    assert_eq!(
        boundary.after.as_ref().unwrap().origin.path.as_deref(),
        Some("after.edn")
    );
    assert!(report.changes[2].before.is_none());
    assert!(report.changes[3].after.is_none());
}

#[test]
fn preserving_old_calls_is_directional_and_additions_do_not_break_them() {
    let before = "{api/pick [string? :-> string?]}";
    let after = "{api/pick [[string? :-> string?] [string? number? :-> string?]]
                 api/more [:-> boolean?]}";
    let report = diff(before, after);
    assert!(report.compatible && report.complete, "{report:#?}");
    let reverse = diff(after, before);
    assert!(!reverse.compatible && reverse.complete, "{reverse:#?}");
    let pick = reverse
        .changes
        .iter()
        .find(|change| change.name == "api/pick")
        .unwrap();
    assert_eq!(pick.argument_count, Some(2));
    assert_eq!(pick.kind, Change::Breaking);
    assert_eq!(pick.detail.as_ref().unwrap().reason, Reason::MissingArity);
}

#[test]
fn a_new_fixed_clause_can_break_an_old_variadic_promise() {
    let report = diff(
        "{api/pick [number? & string? :-> number?]}",
        "{api/pick [[string? :-> number?] [number? & string? :-> number?]]}",
    );
    let [change] = report.changes.as_slice() else {
        panic!("{report:#?}")
    };
    assert_eq!(change.kind, Change::Breaking);
    assert_eq!(change.argument_count, Some(1));
    assert_eq!(detail(&report).path, [Step::Argument { index: 1 }]);
}

#[test]
fn a_distinct_quantified_promise_is_not_assumed_compatible() {
    let report = diff(
        "{api/same (t/all [a] [a :-> a])}",
        "{api/same (t/all [(a :< number?)] [a :-> a])}",
    );
    assert!(!report.compatible && !report.complete, "{report:#?}");
    assert_eq!(report.changes[0].kind, Change::Unproven);
    assert_eq!(detail(&report).reason, Reason::GenericUnproven);
}

#[test]
fn return_regressions_point_to_the_exact_collection_item_and_field() {
    let before = "{api/result [:-> {:rows (clojure.spec.alpha/coll-of
                      {:data (clojure.spec.alpha/tuple number? string?)} :kind vector?) }]}";
    let after = before.replace("number? string?", "number? boolean?");
    let report = diff(before, &after);
    assert!(report.complete && !report.compatible, "{report:#?}");
    let detail = detail(&report);
    assert_eq!(
        detail.path,
        [
            Step::Result,
            Step::Field {
                key: ":rows".into()
            },
            Step::Element,
            Step::Field {
                key: ":data".into()
            },
            Step::Item { index: 1 },
        ]
    );
    assert_eq!(
        detail.reason,
        Reason::TypeMismatch {
            expected: "String".into(),
            actual: "Boolean".into()
        }
    );
    let reverse = diff(&after, before);
    assert!(!reverse.compatible && reverse.complete);
}

#[test]
fn callback_inputs_flip_the_comparison_back_to_what_the_new_library_can_send() {
    let before = "{api/run [(clojure.spec.alpha/fspec
                      :args (clojure.spec.alpha/cat :param {:item number?}) :ret nil?)
                      :-> nil?]}";
    let after = before.replace("number?", "string?");
    let report = diff(before, &after);
    let detail = detail(&report);
    assert_eq!(
        detail.path,
        [
            Step::Argument { index: 1 },
            Step::CallbackArgument { index: 1 },
            Step::Field {
                key: ":item".into()
            }
        ]
    );
    assert_eq!(
        detail.reason,
        Reason::TypeMismatch {
            expected: "Number".into(),
            actual: "String".into()
        }
    );
}

#[test]
fn field_failures_explain_missing_optional_and_extra_keys() {
    for (before, after, reason) in [
        ("{:id number?}", "{}", Reason::MissingField),
        (
            "{:id number?}",
            "(t/map {:opt {:id number?}})",
            Reason::OptionalField,
        ),
        ("{:id number?}", "map?", Reason::UnexpectedExtraEntries),
        (
            "(t/map {:req {:id number?} :extra :open})",
            "map?",
            Reason::UndeclaredField,
        ),
        ("{}", "{:id number?}", Reason::UnexpectedField),
    ] {
        let report = diff(
            &format!("{{api/result [:-> {before}]}}"),
            &format!("{{api/result [:-> {after}]}}"),
        );
        assert!(report.complete && !report.compatible, "{report:#?}");
        let detail = detail(&report);
        let mut path = vec![Step::Result];
        if reason != Reason::UnexpectedExtraEntries {
            path.push(Step::Field { key: ":id".into() });
        }
        assert_eq!(detail.path, path);
        assert_eq!(detail.reason, reason);
    }
}

#[test]
fn a_variadic_break_names_the_first_rejected_rest_argument() {
    let report = diff(
        "{api/items [string? & number? :-> nil?]}",
        "{api/items [string? & boolean? :-> nil?]}",
    );
    assert_eq!(report.changes[0].argument_count, Some(2));
    assert_eq!(detail(&report).path, [Step::Argument { index: 2 }]);
}

#[test]
fn dictionary_changes_distinguish_key_domain_from_value_type() {
    let before = "{api/labels [:-> (clojure.spec.alpha/map-of keyword? string?)]}";
    for (old, new, path) in [
        ("keyword?", "string?", Step::MapKey),
        ("string?", "boolean?", Step::MapValue),
    ] {
        let report = diff(before, &before.replace(old, new));
        assert!(!report.compatible && report.complete, "{report:#?}");
        assert_eq!(detail(&report).path, [Step::Result, path]);
    }
}

#[test]
fn reports_the_rejected_input_union_member_but_not_an_arbitrary_expected_branch() {
    let before = "{api/pick [(clojure.spec.alpha/nilable string?) :-> nil?]}";
    let after = "{api/pick [string? :-> nil?]}";
    let report = diff(before, after);
    assert_eq!(
        detail(&report).path,
        [Step::Argument { index: 1 }, Step::UnionMember]
    );
    assert_eq!(
        detail(&report).reason,
        Reason::TypeMismatch {
            expected: "String".into(),
            actual: "Nil".into()
        }
    );
    let report = diff(
        "{api/pick [:-> (clojure.spec.alpha/nilable string?)]}",
        "{api/pick [:-> boolean?]}",
    );
    assert_eq!(detail(&report).path, [Step::Result]);
    assert_eq!(
        detail(&report).reason,
        Reason::TypeMismatch {
            expected: "Nil | String".into(),
            actual: "Boolean".into()
        }
    );
}

#[test]
fn specs_resolve_separately_even_with_the_same_path_without_checking_bodies() {
    let old_spec = Source {
        path: "schema.clj",
        source: "(ns api (:require [clojure.spec.alpha :as s])) (s/def ::field number?) (defn example [] (unresolved))",
    };
    let new_spec = Source {
        source: "(ns api (:require [clojure.spec.alpha :as s])) (s/def ::field string?) (defn example [] (unsupported))",
        ..old_spec
    };
    let declaration = Source {
        path: "api.edn",
        source: "{vendor/pick [:api/field :-> boolean?]}",
    };
    let report = compare_externs(
        ExternSnapshot {
            sources: &[old_spec],
            externs: &[declaration],
        },
        ExternSnapshot {
            sources: &[new_spec],
            externs: &[declaration],
        },
    );
    assert!(report.before.ok && report.after.ok, "{report:#?}");
    assert_eq!(
        report.before.stats.checked_functions + report.after.stats.checked_functions,
        0
    );
    let [change] = report.changes.as_slice() else {
        panic!("{report:#?}")
    };
    assert_eq!(change.kind, Change::Breaking);
    assert!(
        change
            .before
            .as_ref()
            .unwrap()
            .declaration
            .contains("Number")
    );
    assert!(
        change
            .after
            .as_ref()
            .unwrap()
            .declaration
            .contains("String")
    );
}

#[test]
fn invalid_or_conflicting_snapshots_do_not_turn_parse_errors_into_api_removals() {
    let baseline = Source {
        path: "before.edn",
        source: "{api/once [:-> number?]}",
    };
    let malformed = diff(baseline.source, "{api/once [");
    assert!(!malformed.complete && !malformed.compatible && malformed.changes.is_empty());
    assert_eq!(malformed.after.files[0].diagnostics[0].code, "PNX9000");

    let duplicate = Source {
        path: "duplicate.edn",
        ..baseline
    };
    let conflicting = compare_externs(
        ExternSnapshot {
            sources: &[],
            externs: &[baseline],
        },
        ExternSnapshot {
            sources: &[],
            externs: &[baseline, duplicate],
        },
    );
    assert!(!conflicting.complete && !conflicting.compatible && conflicting.changes.is_empty());
    assert!(
        conflicting
            .after
            .files
            .iter()
            .all(|f| f.diagnostics.iter().any(|d| d.code == "PNX2790"))
    );
}
