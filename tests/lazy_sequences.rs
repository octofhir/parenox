use parenox::{Report, Session, Source};

fn check(body: &str, result: &str) -> Report {
    let source = format!(
        "(ns demo (:require [clojure.spec.alpha :as s]))
         (defn run {{:t/sig '[:-> {result}]}} [] {body})"
    );
    Session::default().check("demo.clj", &source)
}

#[test]
fn range_accepts_zero_through_three_arguments_and_preserves_bounded_lengths() {
    for (expression, result) in [
        ("(range)", "(s/coll-of integer? :kind seq?)"),
        ("(= 4 (count (range 4)))", "#{true}"),
        ("(= 3 (count (range 2 5)))", "#{true}"),
        ("(= 5 (count (range 9 0 -2)))", "#{true}"),
        ("(seq (range 0 9 -2))", "nil?"),
        ("(seq (range 2 2 0))", "nil?"),
        ("(= 2 (first (range 2 6 0)))", "#{true}"),
        ("(range 0 1 0.25)", "(s/coll-of number? :kind seq?)"),
    ] {
        let report = check(expression, result);
        assert!(report.ok && report.complete, "{expression}: {report:#?}");
    }

    for (expression, code) in [
        ("(range 0 1 2 3)", "PNX2554"),
        ("(range 0 10 \"step\")", "PNX2322"),
    ] {
        let report = check(expression, "t/any");
        assert!(
            report
                .diagnostics
                .iter()
                .any(|finding| finding.code == code),
            "{report:#?}"
        );
    }
}

#[test]
fn large_integer_ranges_keep_exact_lengths() {
    for expression in [
        "(= 33 (count (range 33)))",
        "(= 34 (count (range 100 0 -3)))",
        "(= 33 (count (rest (range 100 0 -3))))",
        "(= 0 (count (range 5 1 2)))",
        "(= 1000000000 (count (range -1000000000 1000000000 2)))",
        "(= 2 (count (range 2147483647 -2147483648 -2147483648)))",
    ] {
        let report = check(expression, "#{true}");
        assert!(report.ok && report.complete, "{expression}: {report:#?}");
    }
    let report = check("(nth (range 100) 100 \"missing\")", "#{\"missing\"}");
    assert!(report.ok && report.complete, "{report:#?}");

    let report = check("(apply inc (range 100))", "any?");
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert!(report.complete, "{report:#?}");
    assert_eq!(finding.code, "PNX2554");
    assert!(finding.message.contains("100 arguments"), "{report:#?}");
}

#[test]
fn lazy_map_accepts_one_callback_argument_per_source() {
    for (expression, result) in [
        (
            "(map (fn [x y] (+ x y)) [1] [2])",
            "(s/coll-of number? :kind seq?)",
        ),
        ("(map inc (range 0 6 2))", "(s/coll-of number? :kind seq?)"),
        (
            "(map (fn [x y] x) [] [2])",
            "(s/coll-of number? :kind seq?)",
        ),
        ("(map (fn [x y] x) nil)", "(s/coll-of number? :kind seq?)"),
        (
            "(try (map (fn [_] (throw (ex-info \"late\" {}))) [1]) (catch Exception e :failed))",
            "(s/coll-of number? :kind seq?)",
        ),
        (
            "(try (map identity (throw (ex-info \"now\" {}))) (catch Exception e :failed))",
            "#{:failed}",
        ),
    ] {
        let report = check(expression, result);
        assert!(report.ok && report.complete, "{expression}: {report:#?}");
    }
}

#[test]
fn lazy_map_preserves_the_shortest_source_bounds() {
    for (expression, result) in [
        ("(= 100 (count (map inc (range 100))))", "#{true}"),
        ("(= 2 (count (map + (range 100) [10 20])))", "#{true}"),
        (
            "(map + (range) [10 20])",
            "(s/coll-of number? :kind seq? :min-count 1 :max-count 2)",
        ),
        ("(apply inc (map inc [10]))", "int?"),
        ("(empty? (map + (range) []))", "#{true}"),
    ] {
        let report = check(expression, result);
        assert!(report.ok && report.complete, "{expression}: {report:#?}");
    }
    let source = r#"
        (ns demo (:require [clojure.spec.alpha :as s]))
        (defn mapped
          {:t/sig '[(s/coll-of int? :kind vector? :min-count 2 :max-count 4)
                    :-> (s/coll-of number? :kind seq? :min-count 2 :max-count 3)]}
          [items] (map + items (range 3)))"#;
    let report = Session::default().check("demo.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");

    let edited = source.replacen(":min-count 2", ":min-count 0", 1);
    let report = Session::default().check("demo.clj", &edited);
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(finding.code, "PNX2322");
}

#[test]
fn unsupported_map_traversal_is_distinguished_from_a_nonseqable_source() {
    let array = check("(map identity (byte-array 4))", "t/any");
    assert!(!array.complete, "{array:#?}");
    assert!(
        array
            .diagnostics
            .iter()
            .all(|finding| finding.severity == "warning"),
        "{array:#?}"
    );
    assert!(
        array
            .diagnostics
            .iter()
            .any(|finding| finding.code == "PNX9001"),
        "{array:#?}"
    );

    let number = check("(map identity 4)", "t/any");
    assert!(
        number
            .diagnostics
            .iter()
            .any(|finding| finding.code == "PNX2322"),
        "{number:#?}"
    );
}

#[test]
fn mapping_checks_source_arity_even_without_a_callback_contract() {
    let report = Session::default().check_project(
        &[
            Source {
                path: "loader.clj",
                source: "(ns loader) (defn process [state parameter] state)",
            },
            Source {
                path: "demo.clj",
                source: r#"
                    (ns demo (:require [loader :as load]))
                    (defn lazy-call {:t/sig '[:-> t/any]} []
                      (->> [1 2] (map load/process)))
                    (defn transduced {:t/sig '[:-> t/any]} []
                      (into [] (map load/process) [1]))
                    (defn unapplied {:t/sig '[:-> t/any]} []
                      (map load/process))
                "#,
            },
        ],
        None,
    );
    let report = report
        .files
        .iter()
        .find(|file| file.path == "demo.clj")
        .unwrap();
    let arity_errors: Vec<_> = report
        .diagnostics
        .iter()
        .filter(|finding| finding.code == "PNX2554")
        .collect();
    assert_eq!(arity_errors.len(), 2, "{report:#?}");
    assert!(
        arity_errors.iter().all(|finding| finding
            .message
            .contains("load/process expects 2 arguments, got 1")),
        "{report:#?}"
    );
}
