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

#[test]
fn a_changed_export_rechecks_only_roots_that_use_it_including_private_helpers() {
    let source = r#"(ns app (:require [lib :as lib :refer [lookup] :rename {lookup find-label}]))
(defn- get-label [id] (find-label id))
(defn hot {:t/sig '[number? :-> string?]} [id] (#(get-label %) id))
(defn cold {:t/sig '[number? :-> number?]} [id] (lib/other id))
(defn independent {:t/sig '[:-> string?]} [] "constant")"#;
    let provider = r#"(ns lib)
(defn lookup {:t/sig '[number? :-> string?]} [n] (str n))
(defn other {:t/sig '[number? :-> number?]} [n] n)
(def unused :initial)"#;
    let mut session = Session::default();
    let check = |session: &mut Session, provider| {
        session.check_project(
            &[
                parenox::Source {
                    path: "app.clj",
                    source,
                },
                parenox::Source {
                    path: "lib.clj",
                    source: provider,
                },
            ],
            None,
        )
    };
    assert!(check(&mut session, provider).ok);
    let unused_edit = provider.replace(":initial", ":edited");
    let unused = check(&mut session, &unused_edit);
    assert!(unused.ok && unused.complete, "{unused:#?}");
    assert_eq!(unused.files[0].stats.checked_functions, 0);
    assert_eq!(unused.files[0].stats.reused_functions, 3);

    let changed_contract = unused_edit
        .replace("[number? :-> string?]", "[number? :-> number?]")
        .replace("(str n)", "n");
    let changed = check(&mut session, &changed_contract);
    assert!(changed.complete && !changed.ok, "{changed:#?}");
    assert_eq!(changed.files[0].stats.checked_functions, 1);
    assert_eq!(changed.files[0].stats.reused_functions, 2);
    let [mismatch] = changed.files[0].diagnostics.as_slice() else {
        panic!("{changed:#?}")
    };
    assert_eq!(mismatch.code, "PNX2322");
    assert_eq!(mismatch.actual.as_deref(), Some("Number"));

    let restored = check(&mut session, &unused_edit);
    assert!(restored.ok, "{restored:#?}");
    assert_eq!(restored.files[0].stats.checked_functions, 1);
}

#[test]
fn a_newly_available_var_replaces_a_cached_unknown_result() {
    let source = r#"(ns app (:require [lib :as lib]))
(defn using {:t/sig '[:-> number?]} [] (lib/missing))
(defn independent {:t/sig '[:-> string?]} [] "constant")"#;
    let mut session = Session::default();
    let app = parenox::Source {
        path: "app.clj",
        source,
    };
    let missing = session.check_project(&[app], None);
    assert_eq!(
        missing.files[0].diagnostics[0].code, "PNX9003",
        "{missing:#?}"
    );
    let known = session.check_project(
        &[
            app,
            parenox::Source {
                path: "lib.clj",
                source: "(ns lib) (defn missing {:t/sig '[:-> number?]} [] 1)",
            },
        ],
        None,
    );
    assert!(known.ok, "{known:#?}");
    assert_eq!(known.files[0].stats.checked_functions, 1);
    assert_eq!(known.files[0].stats.reused_functions, 1);
}
