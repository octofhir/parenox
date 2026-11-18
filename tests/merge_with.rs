use parenox::{Report, Session};

fn check(expression: &str, contract: &str) -> Report {
    let source = format!("(defn run {{:t/sig '[:-> {contract}]}} [] {expression})");
    Session::default().check("merge_with.clj", &source)
}

#[test]
fn combines_present_values_from_left_to_right_and_skips_disjoint_keys() {
    let source = include_str!("../examples/merge_with.clj");
    let report = Session::default().check("merge_with.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");

    for (form, contract) in [
        ("(merge-with 42 nil nil)", "nil?"),
        ("(merge-with 42)", "nil?"),
        (
            "(merge-with 42 {:a nil} {:b false})",
            "{:a nil? :b #{false}}",
        ),
        (
            r#"(merge-with (fn [a b] [a b]) {:label nil} {:label false} {:label "end"})"#,
            r#"{:label (clojure.spec.alpha/tuple
                        (clojure.spec.alpha/tuple nil? #{false}) #{"end"})}"#,
        ),
        ("(merge-with reduced nil {})", "{}"),
    ] {
        let report = check(form, contract);
        assert!(report.ok && report.complete, "{form}: {report:#?}");
    }
}

#[test]
fn optional_conflicts_keep_both_one_sided_outcomes_and_required_presence() {
    let source = r#"
        (ns demo (:require [clojure.spec.alpha :as s]))
        (defn merge-one {:t/sig '[(t/map {:opt {:key string?}})
                                 :-> {:key (s/or :untouched number? :combined #{false})}]}
          [right]
          (merge-with (fn [_ _] false) {:key 42} right))
        (defn merge-two
          {:t/sig '[(t/map {:opt {:key number?}}) (t/map {:opt {:key string?}})
                     :-> (t/map {:opt {:key (s/or :left number? :right string? :both #{false})}})]}
          [left right]
          (merge-with (fn [_ _] false) left right))
    "#;
    let report = Session::default().check("optional.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");

    let edited = source.replace("(s/or :untouched number? :combined #{false})", "#{false}");
    let report = Session::default().check("optional.clj", &edited);
    let [error] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert!(!report.ok && report.complete, "{report:#?}");
    assert_eq!(error.code, "PNX2322");
}

#[test]
fn disjoint_dictionary_domains_and_masked_keys_do_not_invoke_the_callback() {
    let source = r#"
        (ns demo (:require [clojure.spec.alpha :as s]))
        (defn disjoint {:t/sig '[(s/map-of keyword? number?) (s/map-of string? string?) :-> map?]}
          [a b]
          (merge-with 42 a b))
        (defn masked {:t/sig '[(s/map-of keyword? number?) (s/map-of #{:hidden} string?) :-> map?]}
          [a b]
          (merge-with 42 (dissoc a :hidden) b))
        (defn empty-with-open {:t/sig '[map? :-> map?]} [row]
          (merge-with 42 {} row {}))
    "#;
    let report = Session::default().check("disjoint.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn throwing_combiners_end_only_paths_with_a_conflict() {
    let source = r#"
        (defn required {:t/sig '[:-> #{:caught}]} []
          (try
            (merge-with (fn [_ _] (throw (ex-info "merge" {}))) {:value nil} {:value false})
            (catch clojure.lang.ExceptionInfo _ :caught)))
        (defn optional {:t/sig '[(t/map {:opt {:value string?}}) :-> {:value number?}]}
          [data]
          (try
            (merge-with (fn [_ _] (throw (ex-info "merge" {}))) {:value 1} data)
            (catch clojure.lang.ExceptionInfo _ {:value 42})))
    "#;
    let report = Session::default().check("effects.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");

    let report = check(
        r#"(try
               (merge-with (fn [_ _] (throw (ex-info "merge" {:stage :callback})))
                           {:value 1} {:value 2}
                           (throw (ex-info "operand" {:stage :last})))
               (catch clojure.lang.ExceptionInfo e (:stage (ex-data e))))"#,
        "#{:last}",
    );
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn mismatch_points_to_the_conflicting_value_and_bounds_open_overlap_and_work() {
    use std::fmt::Write;

    let source = r#"(defn run {:t/sig '[:-> map?]} [] (merge-with + {:count 1} {:count "2"}))"#;
    let report = Session::default().check("literal.clj", source);
    let [error] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(error.code, "PNX2322");
    assert_eq!(
        &source[error.span.start.offset..error.span.end.offset],
        r#""2""#
    );
    assert!(error.notes.iter().any(|note| note.contains(":count")));

    let report = Session::default().check(
        "open.clj",
        "(defn run {:t/sig '[map? :-> map?]} [row] (merge-with + row {:count 1}))",
    );
    assert!(!report.complete, "{report:#?}");
    assert_eq!(report.diagnostics[0].code, "PNX9001");
    assert!(report.diagnostics[0].message.contains("possible overlap"));

    let mut map = String::from("{");
    for id in 0..129 {
        write!(map, ":k{id} {id} ").unwrap();
    }
    map.push('}');
    let report = check(&format!("(merge-with + {map} {map})"), "map?");
    let [error] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert!(!report.complete, "{report:#?}");
    assert_eq!(error.code, "PNX9001");
    assert!(error.message.contains("callback analysis limit"));
}
