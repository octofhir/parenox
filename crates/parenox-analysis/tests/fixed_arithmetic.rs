use parenox_analysis::{Report, Session};

fn check(body: &str) -> Report {
    let source =
        format!("(ns demo.fixed-arithmetic (:require [clojure.spec.alpha :as s]))\n{body}");
    Session::default().check("fixed_arithmetic.clj", &source)
}

#[test]
fn fixed_literals_fold_to_exact_values_including_zero_and_unary_identities() {
    let body = r#"
(defn zero {:t/sig '[:-> (s/and int? #{0})]} [] (+))
(defn one {:t/sig '[:-> (s/and int? #{1})]} [] (*))
(defn unary-add {:t/sig '[:-> (s/and int? #{42})]} [] (+ 42))
(defn unary-multiply {:t/sig '[:-> (s/and int? #{42})]} [] (* 42))
(defn negate {:t/sig '[:-> (s/and int? #{-42})]} [] (- 42))
(defn sum {:t/sig '[:-> (s/and int? #{42})]} [] (+ 16r20 8 2))
(defn subtract {:t/sig '[:-> (s/and int? #{42})]} [] (clojure.core/- 50 3 5))
(defn multiply {:t/sig '[:-> (s/and int? #{42})]} [] (* 2 3 7))
(defn increment {:t/sig '[:-> (s/and int? #{42})]} [] (inc 41))
(defn decrement {:t/sig '[:-> (s/and int? #{42})]} [] (dec 43))
(defn composed {:t/sig '[:-> (s/and int? #{42})]} [] (* 2 (+ 19 (dec 3))))
"#;
    let report = check(body);
    assert!(report.ok && report.complete, "{report:#?}");

    let report = check(&format!(
        r#"{body}
(defn wrong {{:t/sig '[:-> (s/and int? #{{41}})]}} [] (+ 20 22))
"#
    ));
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(finding.code, "PNX2322", "{finding:#?}");
    assert_eq!(finding.actual.as_deref(), Some("42"), "{finding:#?}");
}

#[test]
fn exact_arithmetic_is_shared_by_apply_and_known_callbacks() {
    let report = check(
        r#"
(defn applied {:t/sig '[:-> (s/and int? #{42})]} []
  (apply + 20 (list 11 11)))
(defn applied-negation {:t/sig '[:-> (s/and int? #{-42})]} []
  (apply clojure.core/- (seq [42])))
(defn applied-identity {:t/sig '[:-> (s/and int? #{1})]} [] (apply * nil))
(defn mapped {:t/sig '[:-> (s/coll-of (s/and int? #{42}) :kind vector?)]} []
  (mapv inc [41]))
(defn reduced {:t/sig '[:-> (s/and int? #{42})]} []
  (reduce - 63 [10 11]))
"#,
    );
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn loop_counters_keep_short_exact_runs_and_widen_unbounded_growth() {
    let report = check(
        r#"
(defn six-steps {:t/sig '[:-> (s/and int? #{6})]} []
  (loop [i 0] (if (= i 6) i (recur (inc i)))))
(defn count-to {:t/sig '[number? :-> int?]} [limit]
  (loop [i 0] (if (= i limit) i (recur (inc i)))))
"#,
    );
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn proven_fixed_operations_only_raise_possible_arithmetic_overflow() {
    let body = r#"
(defn exact {:t/sig '[:-> (s/and int? #{42})]} []
  (try (+ 40 2)
       (catch Throwable _ :unreachable)))
(defn wide-but-valid {:t/sig '[:-> int?]} []
  (try (inc 2147483647)
       (catch ArithmeticException _ :unreachable)))
(defn unary-identity {:t/sig '[int? :-> int?]} [n]
  (try (+ (* n)) (catch ArithmeticException _ :unreachable)))
(defn broad {:t/sig '[int? int? :-> (s/or :fixed int? :overflow #{:overflow})]} [left right]
  (try (+ left right)
       (catch ClassCastException _ :unreachable)
       (catch ArithmeticException _ :overflow)))
(defn broad-apply {:t/sig '[int? :-> (s/or :fixed int? :overflow #{:overflow})]} [n]
  (try (apply dec [n])
       (catch ClassCastException _ :unreachable)
       (catch ArithmeticException _ :overflow)))
(defn intermediate-overflow {:t/sig '[:-> (s/or :fixed int? :overflow #{:overflow})]} []
  (try (* 2147483647 2147483647 4 0)
       (catch ArithmeticException _ :overflow)))
"#;
    let report = check(body);
    assert!(report.ok && report.complete, "{report:#?}");

    let report = check(&format!(
        r#"{body}
(defn denies-possible-overflow {{:t/sig '[int? :-> int?]}} [n]
  (try (inc n) (catch ArithmeticException _ :overflow)))
(defn denies-intermediate-overflow {{:t/sig '[:-> int?]}} []
  (try (* 2147483647 2147483647 4 0) (catch ArithmeticException _ :overflow)))
"#
    ));
    let [broad, exact] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    for (finding, name) in [
        (broad, "denies-possible-overflow"),
        (exact, "denies-intermediate-overflow"),
    ] {
        assert_eq!(finding.code, "PNX2322", "{finding:#?}");
        assert_eq!(
            finding.contract.as_ref().unwrap().name,
            format!("demo.fixed-arithmetic/{name}")
        );
    }
}

#[test]
fn a_known_long_outside_the_int_range_definitely_throws_on_checked_conformance() {
    let report = check(
        r#"
(s/def ::checked-int (s/and integer? (s/conformer int)))
(defn in-range {:t/sig '[:-> (s/and int? #{42})]} []
  (try (s/conform ::checked-int (+ 40 2))
       (catch ArithmeticException _ :unreachable)))
(defn positive-wide {:t/sig '[:-> #{:outside-int-range}]} []
  (try (s/conform ::checked-int (+ 2147483647 1))
       (catch IllegalArgumentException _ :unreachable)
       (catch ArithmeticException _ :outside-int-range)))
(defn negative-wide {:t/sig '[:-> #{:outside-int-range}]} []
  (try (s/conform ::checked-int (apply dec [-2147483648]))
       (catch ArithmeticException _ :outside-int-range)))
"#,
    );
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn mixed_numbers_keep_numeric_results_and_local_functions_shadow_the_builtins() {
    let source = r#"
(ns demo.shadowed-arithmetic
  (:refer-clojure :exclude [+])
  (:require [clojure.spec.alpha :as s]))
(defn + {:t/sig '[int? int? :-> #{:shadowed}]} [_ _] :shadowed)
(defn shadowed {:t/sig '[:-> #{:shadowed}]} [] (+ 20 22))
(defn prepared-shadowed {:t/sig '[:-> #{:shadowed}]} [] (apply + [20 22]))
(defn qualified {:t/sig '[:-> (s/and int? #{42})]} [] (clojure.core/+ 20 22))
(defn bigint {:t/sig '[:-> number?]} [] (clojure.core/+ 1N 2))
(defn fraction {:t/sig '[:-> number?]} [] (- 3 1/2))
(defn float-apply {:t/sig '[:-> number?]} [] (apply * [1.5 2]))
"#;
    let report = Session::default().check("shadowed_arithmetic.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");

    let wrong = format!(
        r#"{source}
(defn bigint-as-fixed {{:t/sig '[:-> (s/and int? #{{3}})]}} [] (clojure.core/+ 1N 2))
(defn ratio-as-fixed {{:t/sig '[:-> (s/and int? #{{2}})]}} [] (- 3 1/2))
"#
    );
    let report = Session::default().check("shadowed_arithmetic.clj", &wrong);
    assert!(!report.ok || !report.complete, "{report:#?}");
    assert_eq!(report.diagnostics.len(), 2, "{report:#?}");
    for finding in &report.diagnostics {
        assert_eq!(finding.code, "PNX9004", "{finding:#?}");
        assert!(
            finding.message.contains("cannot verify Number"),
            "{finding:#?}"
        );
    }
}

#[test]
fn failing_inputs_and_malformed_calls_do_not_use_a_folded_result() {
    let report = check(
        r#"
(defn eager-direct {:t/sig '[:-> #{:operand}]} []
  (try (+ 40 (throw (ex-info "operand" {:stage :operand})))
       (catch ArithmeticException _ :unreachable)
       (catch ExceptionInfo error (:stage (ex-data error)))))
(defn eager-apply {:t/sig '[:-> #{:operand}]} []
  (try (apply + [40 (throw (ex-info "operand" {:stage :operand}))])
       (catch ArithmeticException _ :unreachable)
       (catch ExceptionInfo error (:stage (ex-data error)))))
"#,
    );
    assert!(report.ok && report.complete, "{report:#?}");

    for expression in ["(-)", "(inc 1 2)", "(dec)", "(apply inc [])"] {
        let report = check(&format!(
            r#"(defn bad {{:t/sig '[:-> any?]}} [] {expression})"#
        ));
        let [finding] = report.diagnostics.as_slice() else {
            panic!("{expression}: {report:#?}");
        };
        assert_eq!(finding.code, "PNX2554", "{expression}: {finding:#?}");
    }
}
