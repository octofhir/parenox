use parenox::{Report, Session};

fn check(body: &str) -> Report {
    let source = format!("(ns demo (:require [clojure.spec.alpha :as s]))\n{body}");
    Session::default().check("ordered.clj", &source)
}

#[test]
fn reads_and_updates_paths_with_known_vector_positions() {
    let mut session = Session::default();
    let report = session.check(
        "ordered_paths.clj",
        include_str!("../examples/ordered_paths.clj"),
    );
    assert!(report.ok && report.complete, "{report:#?}");

    let report = check(
        "(defn f {:t/sig '[boolean? :-> number?]} [east?]
           (let [path (if east? [:east :retail] [:west :wholesale])]
             (get-in {:east {:retail 1} :west {:wholesale 2}} path)))",
    );
    assert!(report.ok && report.complete, "{report:#?}");

    let report = check(
        r#"(defn f {:t/sig '[:-> string?]} []
             (let [path []] (get-in "catalog" path 0)))"#,
    );
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn distinguishes_ordered_contracts_from_homogeneous_vectors() {
    let source = include_str!("../examples/ordered_paths_bad.clj");
    let report = Session::default().check("ordered_paths_bad.clj", source);
    assert!(!report.ok && report.complete, "{report:#?}");
    let [order, missing] = &report.diagnostics[..] else {
        panic!("{report:#?}");
    };
    assert_eq!(order.expected.as_deref(), Some("Tuple<String, Keyword>"));
    assert_eq!(order.actual.as_deref(), Some("Tuple<:retail, String>"));
    assert_eq!(missing.actual.as_deref(), Some("Nil"));

    for (spec, expression) in [
        (
            "(s/coll-of keyword? :kind vector?)",
            "(get-in {:tier 1} x 0)",
        ),
        ("(s/tuple keyword? number?)", "(get-in {:tier 1} x 0)"),
    ] {
        let report = check(&format!(
            "(defn f {{:t/sig '[{spec} :-> number?]}} [x] {expression})"
        ));
        assert!(!report.ok && !report.complete, "{report:#?}");
        assert_eq!(report.diagnostics.len(), 1, "{report:#?}");
        assert_eq!(report.diagnostics[0].code, "PNX9001");
    }
}

#[test]
fn binds_literal_and_unknown_length_vectors_with_clojure_nil_semantics() {
    let report = check(
        "(defn first-price {:t/sig '[(s/coll-of number? :kind vector?) :-> number?]}
           [[first-price]] (or first-price 0))
         (defn entry {:t/sig '[:-> (s/coll-of (s/tuple keyword? number?) :kind vector?)]}
           [] (mapv identity {:retail 140}))
         (defn unpack {:t/sig '[:-> number?]} []
           (let [[[k price] :as values] [[:retail 140]]]
             (+ price (count values))))",
    );
    assert!(report.ok && report.complete, "{report:#?}");

    let report = check(
        "(defn f {:t/sig '[(s/coll-of number? :kind vector?) :-> number?]}
           [[n]] (inc n))",
    );
    assert!(!report.ok && report.complete, "{report:#?}");
    assert_eq!(
        report.diagnostics[0].actual.as_deref(),
        Some("Nil | Number")
    );
}
