use parenox::{Report, Session};

fn check(expression: &str, contract: &str) -> Report {
    let source = format!("(defn run {{:t/sig '[:-> {contract}]}} [] {expression})");
    Session::default().check("group_by.clj", &source)
}

#[test]
fn groups_typed_collections_and_keeps_known_bucket_positions() {
    let source = include_str!("../examples/group_by.clj");
    let report = Session::default().check("group_by.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");

    let edited = source.replace(
        r#"(s/tuple #{"first"} #{"last"})"#,
        r#"(s/tuple #{"last"} #{"first"})"#,
    );
    let report = Session::default().check("group_by.clj", &edited);
    let [error] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert!(!report.ok && report.complete, "{report:#?}");
    assert_eq!(error.code, "PNX2322");
}

#[test]
fn grouping_preserves_nil_false_and_maybe_empty_collections() {
    let source = r#"
        (ns demo (:require [clojure.spec.alpha :as s]))
        (defn markers {:t/sig '[:-> {"state" (s/tuple nil? #{false})}]} []
          (group-by (fn [_] "state") [nil false]))
        (defn typed {:t/sig '[(s/coll-of number? :kind vector?)
                             :-> (t/map {:opt {:items (s/coll-of number? :kind vector?)}})]}
          [values]
          (group-by (fn [_] :items) values))
        (defn empty-input {:t/sig '[:-> {}]} []
          (group-by 42 nil))
    "#;
    let report = Session::default().check("group_by.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");

    let edited = source.replace(":opt {:items", ":req {:items");
    let report = Session::default().check("group_by.clj", &edited);
    assert!(!report.ok && report.complete, "{report:#?}");
    assert_eq!(report.diagnostics[0].code, "PNX2322");
}

#[test]
fn callback_and_collection_effects_precede_grouping() {
    let report = check(
        r#"(try
               (group-by (fnil identity (throw (ex-info "callback" {:stage :callback})))
                         (throw (ex-info "collection" {:stage :collection})))
               (catch clojure.lang.ExceptionInfo e (:stage (ex-data e))))"#,
        "#{:callback}",
    );
    assert!(report.ok && report.complete, "{report:#?}");

    let report = check(
        r#"(try (group-by (fn [_] (throw (ex-info "item" {:stage :body}))) [1])
                (catch clojure.lang.ExceptionInfo e (:stage (ex-data e))))"#,
        "#{:body}",
    );
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn diagnostics_distinguish_bad_collections_from_unsupported_group_keys() {
    for (source, code) in [
        ("(group-by identity [false])", "PNX9001"),
        ("(group-by identity 42)", "PNX2322"),
        ("(group-by identity)", "PNX2554"),
    ] {
        let report = check(source, "map?");
        let [error] = report.diagnostics.as_slice() else {
            panic!("{source}: {report:#?}");
        };
        assert_eq!(error.code, code, "{source}: {report:#?}");
    }
}

#[test]
fn nested_group_callbacks_are_bounded() {
    let mut body = ":bucket".to_string();
    for _ in 0..8 {
        body = format!("(group-by (fn [_] (do {body} :bucket)) [1 2])");
    }
    let report = check(&body, "map?");
    assert!(!report.complete, "{report:#?}");
    assert!(
        report.diagnostics.iter().any(|error| {
            error.code == "PNX9001" && error.message.contains("group-by callback analysis limit")
        }),
        "{report:#?}"
    );
}
