use parenox::Session;

const SOURCE: &str = include_str!("../examples/jvm_strings.cljc");

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
