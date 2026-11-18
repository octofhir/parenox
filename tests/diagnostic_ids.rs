use std::collections::{BTreeMap, BTreeSet};

use parenox::{Diagnostic, Session, Source};

fn ids(diagnostics: &[Diagnostic]) -> Vec<&str> {
    diagnostics
        .iter()
        .map(|diagnostic| diagnostic.id.as_deref().expect("missing diagnostic ID"))
        .collect()
}

#[test]
fn separate_argument_errors_at_one_spread_span_have_distinct_ids() {
    let source = "(ns app)
        (defn label {:t/sig '[string? string? :-> string?]} [a b] (str a b))
        (defn bad-call {:t/sig '[:-> string?]} []
          (let [args [1 2]] (apply label args)))";
    let mut session = Session::default();
    let first = session.check("app.clj", source);
    let [left, right] = first.diagnostics.as_slice() else {
        panic!("{first:#?}");
    };
    assert_eq!(left.code, "PNX2322");
    assert_eq!(left.code, right.code);
    assert_eq!(left.span, right.span);
    assert_ne!(left.id, right.id);
    assert_eq!(
        serde_json::to_value(left).unwrap()["id"].as_str(),
        left.id.as_deref()
    );

    let warm = session.check("app.clj", source);
    assert_eq!(warm.stats.reused_functions, 2);
    let first_ids = ids(&first.diagnostics);
    assert_eq!(first_ids, ids(&warm.diagnostics));
    let cold = Session::default().check("app.clj", source);
    assert_eq!(first_ids, ids(&cold.diagnostics));

    let moved = session.check("app.clj", &format!("\n{source}"));
    assert!(
        ids(&moved.diagnostics)
            .iter()
            .all(|id| !first_ids.contains(id))
    );
}

#[test]
fn file_ids_do_not_depend_on_project_input_order() {
    let one = Source {
        path: "src/clínica:uno.clj",
        source: "(ns one) (defn f {:t/sig '[:-> string?]} [] 1)",
    };
    let two = Source {
        path: "src/two.clj",
        source: "(ns two) (defn f {:t/sig '[:-> string?]} [] 1)",
    };
    let mut session = Session::default();
    let before = session.check_project(&[one, two], None);
    let after = session.check_project(&[two, one], None);
    assert_eq!(
        before.files[0].diagnostics[0].span,
        before.files[1].diagnostics[0].span
    );
    assert_eq!(
        before.files[0].diagnostics[0].code,
        before.files[1].diagnostics[0].code
    );
    assert_ne!(
        before.files[0].diagnostics[0].id,
        before.files[1].diagnostics[0].id
    );
    let by_path = |report: &parenox::ProjectReport| {
        report
            .files
            .iter()
            .map(|file| (file.path.clone(), file.diagnostics[0].id.clone()))
            .collect::<BTreeMap<_, _>>()
    };
    assert_eq!(by_path(&before), by_path(&after));
}

#[test]
fn reader_and_extern_diagnostics_have_ids_even_with_duplicate_paths() {
    let broken = Source {
        path: "broken.clj",
        source: "(",
    };
    let externs = Source {
        path: "externs.edn",
        source: "{clojure.core/inc [number? :-> number?]}",
    };
    let report = Session::default().check_project(&[broken, broken], Some(externs));
    assert_eq!(report.files[0].diagnostics[0].code, "PNX9000");
    assert!(
        report.files[1]
            .diagnostics
            .iter()
            .any(|d| d.code == "PNX9000")
    );
    assert_eq!(report.files[2].diagnostics[0].code, "PNX2790");
    let all = report
        .files
        .iter()
        .flat_map(|file| ids(&file.diagnostics))
        .collect::<Vec<_>>();
    assert_eq!(all.len(), 4, "{report:#?}");
    assert_eq!(all.iter().collect::<BTreeSet<_>>().len(), all.len());
}
