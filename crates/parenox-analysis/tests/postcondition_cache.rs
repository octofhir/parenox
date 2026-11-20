use parenox_analysis::{Session, Source};

#[test]
fn an_extern_edit_rechecks_functions_that_use_it_only_in_postconditions() {
    let caller = Source {
        path: "caller.clj",
        source: r#"
(ns demo.postcondition (:require [vendor :as v]))
(defn guarded {:t/sig '[:-> #{:ready}]} []
  {:post [(v/accepted? %)]}
  :ready)
(defn wrapped {:t/sig '[:-> #{:ready}]} []
  ((fn ^{:post [(v/accepted? %)]} [] :ready)))
(defn independent {:t/sig '[:-> #{:other}]} [] :other)
"#,
    };
    let matching = Source {
        path: "vendor.edn",
        source: "{vendor/accepted? [keyword? :-> boolean?]}",
    };
    let mismatching = Source {
        source: "{vendor/accepted? [number? :-> boolean?]}",
        ..matching
    };

    let mut session = Session::default();
    let first = session.check_project_with_externs(&[caller], &[matching]);
    assert!(first.ok && first.complete, "{first:#?}");
    let warm = session.check_project_with_externs(&[caller], &[matching]);
    assert_eq!(warm.files[0].stats.reused_functions, 3, "{warm:#?}");

    let changed = session.check_project_with_externs(&[caller], &[mismatching]);
    let report = &changed.files[0];
    assert!(!report.ok && report.complete, "{changed:#?}");
    assert_eq!(report.stats.checked_functions, 2, "{changed:#?}");
    assert_eq!(report.stats.reused_functions, 1, "{changed:#?}");
    let expected_offsets: Vec<_> = caller
        .source
        .match_indices("(v/accepted? %)")
        .map(|(offset, _)| offset + "(v/accepted? ".len())
        .collect();
    assert_eq!(
        report.diagnostics.len(),
        expected_offsets.len(),
        "{changed:#?}"
    );
    for (finding, offset) in report.diagnostics.iter().zip(expected_offsets) {
        assert_eq!(finding.code, "PNX2322");
        assert_eq!(finding.contract.as_ref().unwrap().name, "vendor/accepted?");
        assert_eq!(finding.span.start.offset, offset);
        assert_eq!(&caller.source[offset..finding.span.end.offset], "%");
    }

    let cold = Session::default().check_project_with_externs(&[caller], &[mismatching]);
    assert_eq!(
        serde_json::to_value(&report.diagnostics).unwrap(),
        serde_json::to_value(&cold.files[0].diagnostics).unwrap()
    );
    let restored = session.check_project_with_externs(&[caller], &[matching]);
    assert!(restored.ok && restored.complete, "{restored:#?}");
}

#[test]
fn the_implicit_postcondition_result_shadows_only_the_percent_parameter() {
    let path = "navigation.clj";
    let source = r#"
(ns demo.postcondition-navigation)
(defn guarded {:t/sig '[keyword? string? :-> string?]} [% value]
  {:post [(some? %) (some? value)
          (let [% :shadow] (some? %))]}
  (str % value))
"#;
    let mut session = Session::default();
    let report = session.check(path, source);
    assert!(report.ok && report.complete, "{report:#?}");

    let percent_binding = source.find("[% value]").unwrap() + 1;
    let value_binding = percent_binding + "% ".len();
    let implicit_use = source.find("(some? %)").unwrap() + "(some? ".len();
    let value_condition = source.find("(some? value)").unwrap() + "(some? ".len();
    let nested_binding = source.find("[% :shadow]").unwrap() + 1;
    let nested_use = source.rfind("(some? %)").unwrap() + "(some? ".len();
    let percent_body = source.find("(str % value)").unwrap() + "(str ".len();
    let value_body = percent_body + "% ".len();

    assert!(session.definition(path, implicit_use).is_none());
    assert!(session.references(path, implicit_use, true).is_empty());
    assert_binding(
        &session,
        path,
        percent_body,
        percent_binding,
        &[percent_binding, percent_body],
    );
    assert_binding(
        &session,
        path,
        value_condition,
        value_binding,
        &[value_binding, value_condition, value_body],
    );
    assert_binding(
        &session,
        path,
        nested_use,
        nested_binding,
        &[nested_binding, nested_use],
    );
}

#[test]
fn a_leading_map_in_fn_star_uses_the_real_percent_parameter() {
    let path = "fn-star.clj";
    let source = r#"
(ns demo.fn-star)
(defn raw []
  (fn* [%] {:post [(some? %)]} %))
"#;
    let mut session = Session::default();
    session.check(path, source);

    let binding = source.find("[%]").unwrap() + 1;
    let map_use = source.find("(some? %)").unwrap() + "(some? ".len();
    let body_use = source.rfind('%').unwrap();
    assert_binding(
        &session,
        path,
        map_use,
        binding,
        &[binding, map_use, body_use],
    );
}

fn assert_binding(session: &Session, path: &str, usage: usize, binding: usize, expected: &[usize]) {
    let definition = session.definition(path, usage).unwrap();
    assert_eq!(definition.path, path);
    assert_eq!(definition.span.start.offset, binding);

    let references = session.references(path, usage, true);
    assert!(references.iter().all(|reference| reference.path == path));
    let offsets: Vec<_> = references
        .iter()
        .map(|reference| reference.span.start.offset)
        .collect();
    assert_eq!(offsets, expected);
}
