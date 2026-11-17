use parenox::Session;

#[test]
fn an_edit_reuses_shifted_clean_functions_and_refreshes_moved_diagnostics() {
    let source = r#"(ns demo)
(defn- plus-one [x] (inc x))
(defn good {:t/sig '[:-> number?]} [] (plus-one 1))
(defn bad {:t/sig '[:-> number?]} [] (plus-one "invalid"))
(defn untouched {:t/sig '[:-> number?]} [] 7)
"#;
    let mut session = Session::default();
    let first = session.check("moved.clj", source);
    assert_eq!(first.stats.checked_functions, 3);
    assert_eq!(first.diagnostics.len(), 1);
    assert_eq!(first.diagnostics[0].span.start.line, 2);
    assert_eq!(
        first.diagnostics[0].notes[0],
        "While checking call to demo/plus-one at 4:38"
    );

    let moved = format!(";; Added in an editor\n{source}");
    let shifted = session.check("moved.clj", &moved);
    assert_eq!(shifted.stats.checked_functions, 1);
    assert_eq!(shifted.stats.reused_functions, 2);
    assert_ne!(first.source_hash, shifted.source_hash);
    assert_eq!(shifted.diagnostics.len(), 1);
    let diagnostic = &shifted.diagnostics[0];
    assert_eq!(diagnostic.span.start.line, 3);
    assert_eq!(
        diagnostic.notes[0],
        "While checking call to demo/plus-one at 5:38"
    );
    assert_eq!(
        &moved[diagnostic.span.start.offset..diagnostic.span.end.offset],
        "x"
    );

    let warm = session.check("moved.clj", &moved);
    assert_eq!(warm.stats.reused_functions, 3);
    assert_eq!(warm.source_hash, shifted.source_hash);

    let helper_edit = moved.replace("(inc x)", "(identity x)");
    let changed = session.check("moved.clj", &helper_edit);
    assert_eq!(changed.stats.checked_functions, 2);
    assert_eq!(changed.stats.reused_functions, 1);
    assert_eq!(changed.diagnostics.len(), 1);
    assert_eq!(changed.diagnostics[0].span.start.line, 5);
    assert!(
        changed.diagnostics[0]
            .message
            .contains("Return value of demo/bad")
    );
}
