use parenox::{Report, Session};

fn check(expression: &str, expected: &str) -> Report {
    let source = format!(
        "(ns demo (:require [clojure.spec.alpha :as s])) (defn f {{:t/sig '[:-> {expected}]}} [] {expression})"
    );
    Session::default().check("remaining.clj", &source)
}

#[test]
fn sequential_rest_preserves_the_original_and_distinguishes_seq_list_and_nil() {
    for (expression, expected) in [
        (
            "(let [[head & tail :as original] [:a :b]] original)",
            "(s/coll-of keyword? :kind vector?)",
        ),
        (
            "(let [[head & tail] [:a :b]] tail)",
            "(s/coll-of keyword? :kind seq?)",
        ),
        (
            "(let [[head & tail] '(:a :b)] tail)",
            "(s/coll-of keyword? :kind list?)",
        ),
        ("(let [[head & tail] [:only]] tail)", "nil?"),
        ("(let [[head & tail] nil] tail)", "nil?"),
        ("(let [[head & [second]] [:a :b]] second)", "#{:b}"),
        (
            r#"(let [[entry & tail] {:id "row"}] (subs (val entry) 0))"#,
            "string?",
        ),
    ] {
        let report = check(expression, expected);
        assert!(report.ok && report.complete, "{expression}: {report:#?}");
    }

    let report = check(
        "(let [[head & tail] [:a :b]] tail)",
        "(s/coll-of keyword? :kind list?)",
    );
    assert!(!report.ok && report.complete, "{report:#?}");
    assert_eq!(report.diagnostics[0].code, "PNX2322");

    let report = check("(let [[head & tail extra] []] nil)", "nil?");
    assert_eq!(report.diagnostics[0].code, "PNX1001", "{report:#?}");
}

#[test]
fn separated_fields_loop_keeps_element_types_on_back_edges() {
    // Adapted from dsql.core/reduce-separated-old in the context-pg corpus.
    let good = r#"(ns field-display (:require [clojure.spec.alpha :as s]))
      (defn collect-fields
        {:t/sig '[(s/coll-of string? :kind seq?) :-> (s/coll-of string? :kind vector?)]}
        [fields]
        (if (empty? fields)
          []
          (loop [[x & xs] fields
                 acc []]
            (let [acc' (conj acc x)]
              (if (empty? xs)
                acc'
                (recur xs (conj acc' ",")))))))"#;
    let report = Session::default().check("fields.clj", good);
    assert!(report.ok && report.complete, "{report:#?}");

    let bad = good.replace("(conj acc x)", "(conj acc (inc x))");
    let report = Session::default().check("fields.clj", &bad);
    assert!(!report.ok && report.complete, "{report:#?}");
    assert!(
        report.diagnostics.iter().any(|d| {
            d.code == "PNX2322"
                && d.actual.as_deref() == Some("String")
                && &bad[d.span.start.offset..d.span.end.offset] == "x"
        }),
        "{report:#?}"
    );
}
