use parenox::{Report, Session};

const UNCOVERED: &str = r#"(ns coverage.uncovered)
(defn- unchecked [] (inc "bad"))
(defn checked {:t/sig '[:-> number?]} []
  BODY)"#;

const GRAPH: &str = r#"(ns coverage.graph)
(defn- leaf [n] (inc n))
(defn- wrapper [n] (leaf n))
(defn entry {:t/sig '[:-> number?]} []
  BODY)
(defn independent {:t/sig '[:-> number?]} [] 1)"#;

const DIRECT: &str = "(wrapper 41)";
const DEAD_AND_LIVE: &str = "(if false (wrapper 41) 0) (wrapper 41)";

fn assert_uncovered(report: &Report, source: &str, names: &[(&str, &str)]) {
    assert!(!report.complete, "{report:#?}");
    assert!(!report.ok, "{report:#?}");
    assert_eq!(report.diagnostics.len(), names.len(), "{report:#?}");
    for (local, qualified) in names {
        let gap = report
            .diagnostics
            .iter()
            .find(|diagnostic| {
                diagnostic.code == "PNX9002" && diagnostic.message.contains(qualified)
            })
            .unwrap_or_else(|| panic!("no coverage finding for {qualified}: {report:#?}"));
        let span = gap.span;
        assert!(
            source[span.start.offset..span.end.offset].starts_with(&format!("(defn- {local}")),
            "{gap:#?}"
        );
    }
}

fn assert_uncovered_root(body: &str) {
    let source = UNCOVERED.replace("BODY", body);
    let report = Session::default().check("uncovered.clj", &source);
    assert_uncovered(
        &report,
        &source,
        &[("unchecked", "coverage.uncovered/unchecked")],
    );
}

#[test]
fn lexically_shadowed_private_references_do_not_cover_the_body() {
    assert_uncovered_root("(let [unchecked 42] unchecked)");
}

#[test]
fn a_private_call_in_a_dead_branch_does_not_cover_the_body() {
    assert_uncovered_root("(if true 42 (unchecked))");
}

#[test]
fn a_private_call_with_a_throwing_argument_does_not_cover_the_body() {
    let source = r#"(ns coverage.arguments)
(defn- skipped [x] (inc x))
(defn checked {:t/sig '[:-> #{:caught}]} []
  (try (skipped (throw (ex-info "argument" {})))
       (catch ExceptionInfo _ :caught)))"#;
    let report = Session::default().check("arguments.clj", source);
    assert_uncovered(
        &report,
        source,
        &[("skipped", "coverage.arguments/skipped")],
    );
}

#[test]
fn an_unreferenced_private_body_has_one_declaration_gap() {
    assert_uncovered_root("42");
}

#[test]
fn checking_a_dead_private_call_still_resolves_symbols() {
    let source = UNCOVERED
        .replace("(inc \"bad\")", "(missing-function)")
        .replace("BODY", "(if true 42 (unchecked))");
    let report = Session::default().check("uncovered.clj", &source);
    let codes: Vec<_> = report
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code.as_str())
        .collect();
    assert!(!report.ok && !report.complete, "{report:#?}");
    assert_eq!(codes, ["PNX9002", "PNX2304"], "{report:#?}");
}

#[test]
fn executable_calls_cover_transitive_private_helpers_even_after_a_dead_call() {
    for body in [DIRECT, DEAD_AND_LIVE] {
        let source = GRAPH.replace("BODY", body);
        let report = Session::default().check("graph.clj", &source);
        assert!(report.ok && report.complete, "{body}: {report:#?}");
        assert_eq!(report.stats.checked_functions, 2, "{report:#?}");
    }
}

#[test]
fn warm_checks_follow_private_specialization_evidence_after_edits() {
    let mut session = Session::default();
    let path = "graph.clj";
    let direct = GRAPH.replace("BODY", DIRECT);
    let named = [
        ("leaf", "coverage.graph/leaf"),
        ("wrapper", "coverage.graph/wrapper"),
    ];

    let checked = session.check(path, &direct);
    assert!(checked.ok && checked.complete, "{checked:#?}");
    assert_eq!(checked.stats.checked_functions, 2);
    let warm = session.check(path, &direct);
    assert!(warm.ok && warm.complete, "{warm:#?}");
    assert_eq!(warm.stats.checked_functions, 0);
    assert_eq!(warm.stats.reused_functions, 2);

    for body in ["(let [wrapper 42] wrapper)", "(if true 42 (wrapper 41))"] {
        let changed = GRAPH.replace("BODY", body);
        let checked = session.check(path, &changed);
        assert_uncovered(&checked, &changed, &named);
        assert_eq!(checked.stats.checked_functions, 1);
        assert_eq!(checked.stats.reused_functions, 1);
        let warm = session.check(path, &changed);
        assert_uncovered(&warm, &changed, &named);
        assert_eq!(warm.stats.checked_functions, 0);
        assert_eq!(warm.stats.reused_functions, 2);

        let restored = GRAPH.replace("BODY", DEAD_AND_LIVE);
        let checked = session.check(path, &restored);
        assert!(checked.ok && checked.complete, "{checked:#?}");
        assert_eq!(checked.stats.checked_functions, 1);
        assert_eq!(checked.stats.reused_functions, 1);
        let warm = session.check(path, &restored);
        assert!(warm.ok && warm.complete, "{warm:#?}");
        assert_eq!(warm.stats.checked_functions, 0);
        assert_eq!(warm.stats.reused_functions, 2);
    }
}
