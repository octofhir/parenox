use parenox::{Session, Source};

const EXAMPLE: &str = include_str!("../examples/showcase.clj");
const BROKEN: &str = include_str!("../examples/showcase_bad.clj");

fn demo_sources(broken: &str) -> [Source<'_>; 2] {
    [
        Source {
            path: "examples/showcase.clj",
            source: EXAMPLE,
        },
        Source {
            path: "examples/showcase_bad.clj",
            source: broken,
        },
    ]
}

#[test]
fn showcase_checks_completely_without_suppressions() {
    let report = Session::default().check("showcase.clj", EXAMPLE);
    assert!(
        report.ok && report.complete && report.suppressed.is_empty(),
        "{report:#?}"
    );
}

#[test]
fn broken_showcase_reports_actionable_errors_with_cross_file_contracts() {
    let report = Session::default().check_project(&demo_sources(BROKEN), None);
    assert!(report.files[0].ok && report.files[0].complete);
    let [missing, guard, nilable, callback, score, reason] = report.files[1].diagnostics.as_slice()
    else {
        panic!("{report:#?}");
    };
    assert_eq!(missing.code, "PNX2802");
    assert!(missing.message.contains(":note"));
    assert_eq!(guard.code, "PNX9008");
    assert_eq!(nilable.actual.as_deref(), Some("Nil | String"));
    assert_eq!(
        nilable.contract.as_ref().unwrap().name,
        "demo.showcase/clean-label"
    );
    let [suggestion] = nilable.suggestions.as_slice() else {
        panic!("{nilable:#?}")
    };
    assert_eq!(
        &BROKEN[suggestion.span.start.offset..suggestion.span.end.offset],
        "title"
    );
    assert_eq!(suggestion.applicability, "requires-review");
    assert_eq!(
        callback.contract.as_ref().unwrap().name,
        "demo.showcase/zip-results"
    );
    assert_eq!(
        callback.contract.as_ref().unwrap().path.as_deref(),
        Some("examples/showcase.clj")
    );
    assert_eq!(
        &BROKEN[callback.span.start.offset..callback.span.end.offset],
        "batch/event-row"
    );
    assert!(score.actual.as_deref().unwrap().contains(":score: String"));
    assert!(reason.actual.as_deref().unwrap().contains(":title: Number"));
}

#[test]
fn repairing_the_showcase_clears_all_six_errors_in_a_warm_session() {
    let repaired = BROKEN
        .replace(
            ":meter {:text \"Meter reading\" :score (* (:value event) weight)}))",
            ":meter {:text \"Meter reading\" :score (* (:value event) weight)}\n    :note {:text (batch/clean-label (:value event)) :score 0}))",
        )
        .replace("\n  enabled)", "\n  (and enabled (string? title)))")
        .replace("batch/event-row weights events", "batch/event-row events weights")
        .replace(":score (str score)", ":score score")
        .replace(":reason 422", ":reason \"Tenant is required\"");
    let mut session = Session::default();
    let initial = session.check_project(&demo_sources(BROKEN), None);
    assert_eq!(initial.files[1].diagnostics.len(), 6);
    let fixed = session.check_project(&demo_sources(&repaired), None);
    assert!(fixed.ok && fixed.complete, "{fixed:#?}");
    assert_eq!(fixed.files[0].stats.checked_functions, 0);
    assert_eq!(
        fixed.files[0].stats.reused_functions,
        initial.files[0].stats.checked_functions
    );
}

#[test]
fn showcase_exposes_regressions_in_dispatch_guard_and_response_contract() {
    for (original, edited, code, context) in [
        (
            ":note {:text (clean-label (:value event)) :score 0}",
            "",
            "PNX2802",
            ":note",
        ),
        (
            "(and enabled (string? title))",
            "enabled",
            "PNX9008",
            "Guard body",
        ),
        (
            ":score (score-total entries)",
            ":score (str (score-total entries))",
            "PNX2322",
            "summarize-batch",
        ),
        (
            "{:reason \"Tenant is required\"}",
            "{:reason 422}",
            "PNX2322",
            "summarize-batch",
        ),
    ] {
        let source = EXAMPLE.replace(original, edited);
        assert_ne!(source, EXAMPLE);
        let report = Session::default().check("showcase.clj", &source);
        assert!(
            report
                .diagnostics
                .iter()
                .any(|d| d.code == code && d.message.contains(context)),
            "{report:#?}"
        );
    }
}
