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
fn take_preserves_prefix_types_and_caps_sequence_length() {
    for (expression, result) in [
        ("(= 10 (count (take 10 (range))))", "#{true}"),
        (
            "(= 1000000000 (count (take 1000000000 (range))))",
            "#{true}",
        ),
        ("(= 1 (count (take 10 [:one])))", "#{true}"),
        ("(apply inc (take 1 (range 4 20)))", "int?"),
        ("(nth (take 2 [false \"middle\" :tail]) 1)", "#{\"middle\"}"),
        (
            "(first (first (take 1 [[\"nested\"] nil])))",
            "#{\"nested\"}",
        ),
        ("(empty? (take -1 false))", "#{true}"),
        (
            "(try (take 0 (throw (ex-info \"source\" {}))) (catch Exception e :source))",
            "#{:source}",
        ),
    ] {
        let report = check(expression, result);
        assert!(report.ok && report.complete, "{expression}: {report:#?}");
    }
    let report = check("(take 2 false)", "any?");
    assert!(report.complete, "{report:#?}");
    assert_eq!(report.diagnostics[0].code, "PNX2322");
    let report = check("(take 1)", "any?");
    assert!(!report.complete, "{report:#?}");
    assert_eq!(report.diagnostics[0].code, "PNX9001");
}

#[test]
fn drop_preserves_tail_types_and_consumes_sequence_bounds() {
    for (expression, result) in [
        ("(= 33 (count (drop 67 (range 100))))", "#{true}"),
        (
            "(apply inc (take 1 (drop 2 [1 2 8 7])))",
            "(s/and int? #{9})",
        ),
        (
            "(first (first (drop 1 [false [\"nested\"]])))",
            "#{\"nested\"}",
        ),
        ("(first (drop -1 [:first \"next\"]))", "#{:first}"),
        ("(empty? (drop 3 [1 2]))", "#{true}"),
        ("(empty? (drop 1000000000 (range 100)))", "#{true}"),
        ("(= 10 (count (take 10 (drop 1000 (range)))))", "#{true}"),
    ] {
        let report = check(expression, result);
        assert!(report.ok && report.complete, "{expression}: {report:#?}");
    }

    let report = check("(drop 0 false)", "any?");
    assert!(report.complete, "{report:#?}");
    assert_eq!(report.diagnostics[0].code, "PNX2322");
    let report = check(
        "(try (drop :bad [1]) (catch ClassCastException _ :caught))",
        "#{:caught}",
    );
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert!(report.complete, "{report:#?}");
    assert_eq!(finding.code, "PNX2322");
    let report = check("(drop 1)", "any?");
    assert!(!report.complete, "{report:#?}");
    assert_eq!(report.diagnostics[0].code, "PNX9001");
}

#[test]
fn split_at_preserves_both_sequence_slices() {
    for (expression, result) in [
        (
            "(let [[prefix tail] (split-at 2 [false \"middle\" :tail])]
               [(nth prefix 1) (first tail)])",
            "(s/tuple #{\"middle\"} #{:tail})",
        ),
        (
            "(let [[prefix tail] (split-at 90 (range 100))]
               (and (= 90 (count prefix)) (= 10 (count tail))))",
            "#{true}",
        ),
        (
            "(first (first (first (split-at 1 [[\"nested\"] :tail]))))",
            "#{\"nested\"}",
        ),
        (
            "(let [[prefix tail] (split-at -1 [8])]
               (when (empty? prefix) (apply inc tail)))",
            "(s/and int? #{9})",
        ),
    ] {
        let report = check(expression, result);
        assert!(report.ok && report.complete, "{expression}: {report:#?}");
    }
    let report = check(
        "(try (split-at :bad [1]) (catch ClassCastException _ :caught))",
        "#{:caught}",
    );
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert!(report.complete, "{report:#?}");
    assert_eq!(finding.code, "PNX2322");
}

#[test]
fn concat_preserves_order_and_adds_source_lengths() {
    for (expression, result) in [
        ("(empty? (concat))", "#{true}"),
        (
            "(let [items (concat nil [false] [\"middle\"] '(tail))]
               [(first items) (nth items 1) (nth items 2)])",
            "(s/tuple #{false} #{\"middle\"} #{tail})",
        ),
        (
            "(first (first (concat [[\"nested\"]] nil)))",
            "#{\"nested\"}",
        ),
        (
            "(let [args (concat nil [8])] (apply inc args))",
            "(s/and int? #{9})",
        ),
        ("(= 101 (count (concat (range 100) [1])))", "#{true}"),
        (
            "(= 37 (count (concat \"123456789012345678901234567890123456\" [1])))",
            "#{true}",
        ),
        (
            "(try (concat [1] (throw (ex-info \"operand\" {}))) (catch Exception e :operand))",
            "#{:operand}",
        ),
    ] {
        let report = check(expression, result);
        assert!(report.ok && report.complete, "{expression}: {report:#?}");
    }
    let report = check("(concat [1] false)", "any?");
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert!(report.complete, "{report:#?}");
    assert_eq!(finding.code, "PNX2322");
    let report = check("(concat (byte-array 4))", "any?");
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert!(!report.complete, "{report:#?}");
    assert_eq!(finding.code, "PNX9001");
}

#[test]
fn interpose_tracks_inserted_separators_and_length() {
    for (expression, result) in [
        ("(empty? (interpose :separator []))", "#{true}"),
        ("(apply inc (interpose :unused [8]))", "(s/and int? #{9})"),
        (
            "(let [items (interpose false [:a :b])]
               [(first items) (nth items 1) (nth items 2)])",
            "(s/tuple #{:a} #{false} #{:b})",
        ),
        (
            "(first (first (rest (interpose [\"separator\"] [1 2]))))",
            "#{\"separator\"}",
        ),
        ("(= 199 (count (interpose nil (range 100))))", "#{true}"),
        (
            "(= 1999999999 (count (interpose nil (range 1000000000))))",
            "#{true}",
        ),
        (
            "(try (interpose (throw (ex-info \"separator\" {:site :separator}))
                             (throw (ex-info \"source\" {:site :source})))
                  (catch clojure.lang.ExceptionInfo e (:site (ex-data e))))",
            "#{:separator}",
        ),
    ] {
        let report = check(expression, result);
        assert!(report.ok && report.complete, "{expression}: {report:#?}");
    }
    let report = Session::default().check(
        "demo.clj",
        r#"
        (ns demo (:require [clojure.spec.alpha :as s]))
        (defn at-most-one
          {:t/sig '[(s/coll-of int? :kind vector? :max-count 1)
                    :-> (s/coll-of int? :kind seq? :max-count 1)]}
          [items] (interpose :unused items))"#,
    );
    assert!(report.ok && report.complete, "{report:#?}");
    let report = check("(interpose :separator)", "any?");
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert!(!report.complete, "{report:#?}");
    assert_eq!(finding.code, "PNX9001");
}

#[test]
fn interleave_preserves_round_order_and_stops_at_the_shortest_source() {
    for (expression, result) in [
        ("(empty? (interleave))", "#{true}"),
        ("(apply inc (interleave [8]))", "(s/and int? #{9})"),
        (
            "(let [items (interleave [:a :ignored] [false] [\"third\"])]
               [(first items) (nth items 1) (nth items 2)])",
            "(s/tuple #{:a} #{false} #{\"third\"})",
        ),
        (
            "(first (first (interleave [[\"nested\"]] [false])))",
            "#{\"nested\"}",
        ),
        (
            "(= 200 (count (interleave (range 100) (range 150))))",
            "#{true}",
        ),
        (
            "(= 2000000000 (count (interleave (range 1000000000) (range 1100000000))))",
            "#{true}",
        ),
        ("(empty? (interleave [] [1] false))", "#{true}"),
    ] {
        let report = check(expression, result);
        assert!(report.ok && report.complete, "{expression}: {report:#?}");
    }
    let report = Session::default().check(
        "demo.clj",
        r#"
        (ns demo (:require [clojure.spec.alpha :as s]))
        (defn bounded
          {:t/sig '[(s/coll-of int? :kind vector? :min-count 2 :max-count 4)
                    :-> (s/coll-of int? :kind seq? :min-count 4 :max-count 6)]}
          [items] (interleave items (range 3)))"#,
    );
    assert!(report.ok && report.complete, "{report:#?}");
    let report = check("(interleave [] false)", "any?");
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert!(report.complete, "{report:#?}");
    assert_eq!(finding.code, "PNX2322");
}

#[test]
fn repeat_models_finite_and_infinite_sequences() {
    for (expression, result) in [
        ("(empty? (repeat -1 :unused))", "#{true}"),
        ("(= 1000000000 (count (repeat 1000000000 nil)))", "#{true}"),
        ("(first (first (repeat 2 [\"nested\"])))", "#{\"nested\"}"),
        ("(apply inc (repeat 1 8))", "(s/and int? #{9})"),
        ("(= 3 (count (take 3 (repeat false))))", "#{true}"),
        (
            "(= 2 (count (interleave (repeat :separator) [1])))",
            "#{true}",
        ),
        (
            "(try (repeat 9223372036854775808N :unused)
                  (catch IllegalArgumentException _ :caught))",
            "#{:caught}",
        ),
    ] {
        let report = check(expression, result);
        assert!(report.ok && report.complete, "{expression}: {report:#?}");
    }
    let report = check(
        "(try (repeat nil :unused) (catch NullPointerException _ :caught))",
        "#{:caught}",
    );
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert!(report.complete, "{report:#?}");
    assert_eq!(finding.code, "PNX2322");
}

#[test]
fn cycle_distinguishes_empty_inputs_and_checks_eager_sequence_conversion() {
    for (expression, result) in [
        ("(empty? (cycle nil))", "#{true}"),
        ("(= 100 (count (take 100 (cycle [:a :b]))))", "#{true}"),
        (
            "(take 4 (cycle [:a :b]))",
            "(s/coll-of #{:a :b} :kind seq? :min-count 4 :max-count 4)",
        ),
    ] {
        let report = check(expression, result);
        assert!(report.ok && report.complete, "{expression}: {report:#?}");
    }
    let report = check(
        "(try (cycle false) (catch IllegalArgumentException _ :caught))",
        "#{:caught}",
    );
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert!(report.complete, "{report:#?}");
    assert_eq!(finding.code, "PNX2322");
}

#[test]
fn repeatedly_checks_nullary_callbacks_and_preserves_length() {
    for (expression, result) in [
        ("(first (repeatedly (fn [] :tick)))", "#{:tick}"),
        (
            "(= 1000000000 (count (repeatedly 1000000000 (constantly nil))))",
            "#{true}",
        ),
        ("(apply + (repeatedly 2 (fn [] 1)))", "(s/and int? #{2})"),
        ("(empty? (repeatedly 0 inc))", "#{true}"),
        (
            "(= 3 (count (take 3 (repeatedly (constantly false)))))",
            "#{true}",
        ),
        (
            "(try (repeatedly (throw (ex-info \"count\" {:site :count}))
                              (throw (ex-info \"callback\" {:site :callback})))
                  (catch clojure.lang.ExceptionInfo e (:site (ex-data e))))",
            "#{:count}",
        ),
        (
            "(try (repeatedly 1 (fn [] (throw (ex-info \"deferred\" {}))))
                  (catch Exception _ :eager))",
            "(s/coll-of number? :kind seq?)",
        ),
    ] {
        let report = check(expression, result);
        assert!(report.ok && report.complete, "{expression}: {report:#?}");
    }
    let report = check("(repeatedly 1 identity)", "any?");
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert!(report.complete, "{report:#?}");
    assert_eq!(finding.code, "PNX2554");
    let report = check("(repeatedly :bad inc)", "any?");
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert!(report.complete, "{report:#?}");
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
