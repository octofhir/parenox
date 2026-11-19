use parenox::Session;

const SOURCE: &str = include_str!("../examples/jvm_strings.cljc");

#[test]
fn substring_uses_utf16_bounds_and_marks_the_invalid_index() {
    let source = r#"(ns captures)
        (defn suffix {:t/sig '[:-> #{"a"}]} [] (subs "😀a" 2))
        (defn prefix {:t/sig '[:-> #{"😀"}]} [] (subs "😀a" 0 2))
        (defn half {:t/sig '[:-> string?]} [] (subs "😀a" 0 1))"#;
    let mut session = Session::default();
    let report = session.check("members.cljc", source);
    assert!(report.ok && report.complete, "{report:#?}");

    let invalid = r#"(ns captures (:refer-clojure :rename {subs piece}))
        (defn outside [] (piece "😀" 3))
        (defn reversed [text] (piece text 4 2))
        (defn wrong [] (piece "abc" "0"))"#;
    let report = session.check("members.cljc", invalid);
    let errors: Vec<_> = report
        .diagnostics
        .iter()
        .filter(|finding| finding.severity == "error")
        .collect();
    assert_eq!(errors.len(), 3, "{report:#?}");
    for (finding, (code, target)) in
        errors
            .into_iter()
            .zip([("PNX2801", "3"), ("PNX2801", "2"), ("PNX2322", "\"0\"")])
    {
        assert_eq!(finding.code, code);
        assert_eq!(
            &invalid[finding.span.start.offset..finding.span.end.offset],
            target
        );
    }
}

#[test]
fn checks_jvm_utf16_length_and_characters_after_nil_guards() {
    let mut session = Session::default();
    let report = session.check("jvm_strings.cljc", SOURCE);
    assert!(report.ok && report.complete, "{report:#?}");
    let editable = SOURCE.replace("(when name", "(do");
    let report = session.check("jvm_strings.cljc", &editable);
    let [error] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(error.actual.as_deref(), Some("Nil | String"));
    assert_eq!(
        error.contract.as_ref().unwrap().name,
        "java.lang.String/charAt"
    );
    assert_eq!(
        &editable[error.span.start.offset..error.span.end.offset],
        "name"
    );
}

#[test]
fn checks_member_arguments_and_marks_other_receivers_incomplete() {
    let source = r#"(defn indexed {:t/sig '[string? :-> char?]} [text] (.charAt text "0"))
         (defn unknown {:t/sig '[map? :-> number?]} [data] (.length data))"#;
    let report = Session::default().check("members.clj", source);
    assert_eq!(report.diagnostics.len(), 2, "{report:#?}");
    assert_eq!(report.diagnostics[0].expected.as_deref(), Some("Number"));
    assert_eq!(report.diagnostics[0].severity, "error");
    assert_eq!(report.diagnostics[1].severity, "warning");
    assert!(!report.complete);
}
