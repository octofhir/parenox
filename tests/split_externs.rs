use parenox::{Session, Source};

const APP: Source = Source {
    path: "app.clj",
    source: "(ns app (:require [vendor :as v]))
      (defn label {:t/sig '[:-> string?]} [] (v/label 7))
      (defn count-items {:t/sig '[:-> number?]} [] (v/count-items \"abc\"))",
};
const LABEL: Source = Source {
    path: "label.edn",
    source: "{vendor/label [number? :-> string?]}",
};
const COUNT: Source = Source {
    path: "count.edn",
    source: "{vendor/count-items [string? :-> number?]}",
};

#[test]
fn same_namespace_sidecars_keep_contract_origins_and_only_recheck_affected_callers() {
    let mut session = Session::default();
    let initial = session.check_project_with_externs(&[APP], &[LABEL, COUNT]);
    assert!(initial.ok && initial.complete, "{initial:#?}");
    let changed = Source {
        source: "{vendor/label [string? :-> string?]}",
        ..LABEL
    };
    let updated = session.check_project_with_externs(&[APP], &[changed, COUNT]);
    assert_eq!(updated.stats.parsed_files, 1);
    assert_eq!(updated.stats.checked_functions, 1);
    assert_eq!(updated.stats.reused_functions, 1);
    let [diagnostic] = updated.files[0].diagnostics.as_slice() else {
        panic!("{updated:#?}")
    };
    assert_eq!(diagnostic.code, "PNX2322");
    assert_eq!(
        diagnostic.contract.as_ref().unwrap().path.as_deref(),
        Some(LABEL.path)
    );

    let reordered = session.check_project_with_externs(&[APP], &[COUNT, changed]);
    assert_eq!(reordered.stats.checked_functions, 0);
    assert_eq!(reordered.stats.reused_functions, 2);
    assert_eq!(reordered.files[0].diagnostics[0].id, diagnostic.id);
    let fresh = Session::default().check_project_with_externs(&[APP], &[COUNT, changed]);
    assert_eq!(
        serde_json::to_value(&reordered.files[0].diagnostics).unwrap(),
        serde_json::to_value(&fresh.files[0].diagnostics).unwrap()
    );
}

#[test]
fn duplicate_names_are_reported_in_both_files_without_installing_either_declaration() {
    let collision = Source {
        path: "duplicates.edn",
        source: "{vendor/label {:macro :let-like}}",
    };
    let mut session = Session::default();
    let report = session.check_project_with_externs(&[APP], &[LABEL, COUNT, collision]);
    assert!(!report.ok, "{report:#?}");
    let [unresolved] = report.files[0].diagnostics.as_slice() else {
        panic!("{report:#?}")
    };
    assert_eq!(unresolved.code, "PNX9003");
    assert!(unresolved.message.contains("v/label"), "{unresolved:?}");
    for (file, other) in [(LABEL, collision), (collision, LABEL)] {
        let report = report.files.iter().find(|r| r.path == file.path).unwrap();
        let [diagnostic] = report.diagnostics.as_slice() else {
            panic!("{report:#?}")
        };
        assert_eq!(diagnostic.code, "PNX2790");
        assert_eq!(
            &file.source[diagnostic.span.start.offset..diagnostic.span.end.offset],
            "vendor/label"
        );
        assert_eq!(
            diagnostic.contract.as_ref().unwrap().path.as_deref(),
            Some(file.path)
        );
        assert!(
            diagnostic
                .notes
                .iter()
                .any(|note| note.contains(other.path))
        );
    }

    let swapped = session.check_project_with_externs(&[APP], &[collision, LABEL, COUNT]);
    assert_eq!(swapped.stats.reused_functions, 2);
    for file in &report.files {
        let other = swapped
            .files
            .iter()
            .find(|other| other.path == file.path)
            .unwrap();
        assert_eq!(
            serde_json::to_value(&file.diagnostics).unwrap(),
            serde_json::to_value(&other.diagnostics).unwrap()
        );
    }

    let fixed = session.check_project_with_externs(&[APP], &[LABEL, COUNT]);
    assert!(fixed.ok && fixed.complete, "{fixed:#?}");
    assert_eq!(fixed.stats.checked_functions, 1);
    assert_eq!(fixed.stats.reused_functions, 1);
}

#[test]
fn a_malformed_sidecar_cannot_leave_stale_declarations_or_hide_its_sibling() {
    let mut session = Session::default();
    assert!(
        session
            .check_project_with_externs(&[APP], &[LABEL, COUNT])
            .ok
    );
    let broken = Source {
        source: "{vendor/label [",
        ..LABEL
    };
    let report = session.check_project_with_externs(&[APP], &[broken, COUNT]);
    assert_eq!(report.stats.checked_functions, 1);
    assert_eq!(report.stats.reused_functions, 1);
    assert_eq!(report.files[1].diagnostics[0].code, "PNX9000");
    assert!(report.files[2].ok);
    assert_eq!(report.files[0].diagnostics.len(), 1);
    assert!(
        report.files[0].diagnostics[0].message.contains("v/label"),
        "{report:#?}"
    );
}
