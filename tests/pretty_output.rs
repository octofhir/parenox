use parenox::{Session, output};

#[test]
fn pretty_groups_repeated_findings_and_keeps_each_call_context() {
    let source = r#"(ns demo)
(defn- increment [value] (inc value))
(defn first-call {:t/sig '[:-> t/any]} [] (increment :wrong))
(defn second-call {:t/sig '[:-> t/any]} [] (increment :wrong))"#;
    let report = Session::default().check("demo.clj", source);
    assert_eq!(report.diagnostics.len(), 2, "{report:#?}");
    assert_ne!(report.diagnostics[0].id, report.diagnostics[1].id);
    let rendered = output::pretty(&report, source, false);
    assert_eq!(rendered.matches("error[PNX2322]").count(), 1, "{rendered}");
    for finding in &report.diagnostics {
        let [context] = finding.notes.as_slice() else {
            panic!("{finding:#?}")
        };
        assert!(rendered.contains(context), "{rendered}");
    }
    assert!(rendered.contains("due to 1 previous error"), "{rendered}");
    assert_eq!(output::plain(&report).matches("PNX2322").count(), 2);
}

#[test]
fn nil_suggestion_rechecks_successfully_without_weakening_the_signature() {
    let source = "(ns demo (:require [clojure.spec.alpha :as s] [clojure.string :as str]))\n\
                  (defn f {:t/sig '[(s/nilable string?) :-> string?]} [x] (str/upper-case x))";
    let mut session = Session::default();
    let report = session.check("demo.clj", source);
    assert_eq!(report.diagnostics.len(), 1);
    let suggestion = &report.diagnostics[0].suggestions[0];
    let mut fixed = source.to_string();
    fixed.replace_range(
        suggestion.span.start.offset..suggestion.span.end.offset,
        &suggestion.replacement,
    );
    assert!(fixed.contains(":t/sig '[(s/nilable string?) :-> string?]"));
    assert!(session.check("demo.clj", &fixed).ok);
}

#[test]
fn numeric_string_suggests_number_literal_but_arbitrary_text_does_not() {
    for (literal, expected) in [
        ("42", Some("42")),
        ("oops", None),
        ("NaN", None),
        ("1N", None),
    ] {
        let source = format!("(defn f {{:t/sig '[:-> number?]}} [] {literal:?})");
        let report = Session::default().check("demo.clj", &source);
        let suggestion = report.diagnostics[0].suggestions.first();
        assert_eq!(suggestion.map(|s| s.replacement.as_str()), expected);
        if let Some(suggestion) = suggestion {
            let mut fixed = source.clone();
            fixed.replace_range(
                suggestion.span.start.offset..suggestion.span.end.offset,
                &suggestion.replacement,
            );
            assert!(Session::default().check("demo.clj", &fixed).ok);
        }
    }
}

#[test]
fn unicode_tabs_and_multiline_ranges_render_without_losing_source() {
    let source =
        "(defn имя {:t/sig '[:-> number?]} []\n\t(if true\n\t  \"ошибка\"\n\t  \"другая\"))";
    let report = Session::default().check("unicode.clj", source);
    assert!(!report.ok);
    let rendered = output::pretty(&report, source, false);
    assert!(rendered.contains("unicode.clj:2:"), "{rendered}");
    assert!(rendered.contains("ошибка"), "{rendered}");
    assert!(rendered.contains("другая"), "{rendered}");
    assert!(!rendered.contains('\u{1b}'));
}

#[test]
fn malformed_reader_input_still_gets_a_source_snippet() {
    for source in ["(defn broken [x]", "'", "#_", "(\n\n"] {
        let report = Session::default().check("broken.clj", source);
        let rendered = output::pretty(&report, source, false);
        assert!(rendered.contains("error[PNX9000]"), "{rendered}");
        assert!(rendered.contains("--> broken.clj:"), "{rendered}");
        assert!(rendered.contains("check is incomplete"), "{rendered}");
    }
}

#[test]
fn cached_suggestions_keep_the_correct_source_range_after_a_move() {
    let source = "(defn f {:t/sig '[:-> number?]} [] \"42\")";
    let mut session = Session::default();
    let before = session.check("demo.clj", source);
    let same = session.check("demo.clj", source);
    assert_eq!(same.stats.reused_functions, 1);
    assert_eq!(same.diagnostics[0].suggestions[0].replacement, "42");
    let moved = format!("\n{source}");
    let after = session.check("demo.clj", &moved);
    assert_eq!(
        after.diagnostics[0].suggestions[0].span.start.line,
        before.diagnostics[0].suggestions[0].span.start.line + 1
    );
    assert!(output::pretty(&after, &moved, false).contains("demo.clj:2:"));
}
