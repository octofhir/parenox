use parenox_analysis::{Report, Session, Source};

fn specs() -> &'static str {
    include_str!("../../../tests/fixtures/signed-long-types.clj")
        .split_once(";; JVM-only")
        .unwrap()
        .0
}

fn check(body: &str) -> Report {
    Session::default().check("signed_long_types.clj", &format!("{}{body}", specs()))
}

#[test]
fn fixed_arithmetic_keeps_wide_values_exact_and_can_cancel_back_into_i32() {
    let report = check(
        r#"
(defn max-long {:t/sig '[:-> (s/and int? #{9223372036854775807})]} []
  (try (+ 9223372036854775806 1)
       (catch ArithmeticException _ :unexpected)))
(defn min-long {:t/sig '[:-> (s/and int? #{-9223372036854775808})]} []
  (* -4611686018427387904 2))
(defn wide-product {:t/sig '[:-> (s/and int? #{4294967296})]} []
  (* 2147483648 2))
(defn small-difference {:t/sig '[:-> (s/and int? #{2})]} []
  (- 9223372036854775807 9223372036854775805))
(defn opposite-limits {:t/sig '[:-> (s/and int? #{2})]} []
  (+ 9223372036854775807 -9223372036854775808 3))
(defn prepared {:t/sig '[:-> (s/and int? #{2147483647})]} []
  (apply dec [2147483648]))
(defn wide-callback {:t/sig '[:-> (s/and int? #{2147483647})]} []
  (reduce - 2147483650 [1 2]))
(defn nested {:t/sig '[:-> (s/and int? #{1})]} []
  (- (inc 2147483647) 2147483647))
"#,
    );
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn equality_compares_exact_integer_values_across_boxed_classes() {
    let report = check(
        r#"
(defn max-boxes {:t/sig '[:-> #{true}]} []
  (= 9223372036854775807N (inc 9223372036854775806)))
(defn min-boxes {:t/sig '[:-> #{true}]} []
  (clojure.core/= -9223372036854775808 -9223372036854775808N))
(defn same-radix {:t/sig '[:-> #{true}]} []
  (apply = [2147483648 0x80000000 2147483648N]))
(defn same-ratio {:t/sig '[:-> #{true}]} []
  (= 2147483648 4294967296/2))
(defn adjacent {:t/sig '[:-> #{false}]} []
  (= 9223372036854775807 9223372036854775806N))
(defn opposite-signs {:t/sig '[:-> #{false}]} []
  (= 2147483648 -2147483648))
(defn typed {:t/sig '[(s/and integer? #{9223372036854775807}) :-> #{true}]} [x]
  (= 9223372036854775807 x))
"#,
    );
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn finite_integer_contracts_separate_boxed_class_from_numeric_value() {
    let body = r#"
(defn requires-fixed {:t/sig '[(s/and int? #{9223372036854775807}) :-> keyword?]}
  [_] :accepted)
(defn requires-any-int {:t/sig '[(s/and integer? #{9223372036854775807}) :-> keyword?]}
  [_] :accepted)
(defn positive {:t/sig '[:-> keyword?]} []
  (requires-fixed 9223372036854775807)
  (requires-any-int 9223372036854775807)
  (requires-any-int 9223372036854775807N))
(defn fixed-member {:t/sig '[:-> ::fixed]} [] 2147483648)
(defn any-member {:t/sig '[:-> ::any-int]} [] -9223372036854775808N)
"#;
    let report = check(body);
    assert!(report.ok && report.complete, "{report:#?}");

    for (call, name) in [
        ("(requires-fixed 9223372036854775807N)", "requires-fixed"),
        ("(requires-fixed 9223372036854775806)", "requires-fixed"),
        (
            "(requires-any-int 9223372036854775806N)",
            "requires-any-int",
        ),
        (
            "(requires-any-int 9223372036854775807M)",
            "requires-any-int",
        ),
    ] {
        let report = check(&format!(
            r#"{body}
(defn rejected {{:t/sig '[:-> keyword?]}} [] {call})
"#
        ));
        let [finding] = report.diagnostics.as_slice() else {
            panic!("{call}: {report:#?}");
        };
        assert_eq!(finding.code, "PNX2322", "{call}: {finding:#?}");
        assert_eq!(
            finding.contract.as_ref().unwrap().name,
            format!("demo.signed-long-types/{name}")
        );
    }
}

#[test]
fn signed_long_overflow_remains_possible_without_proven_compiler_mode() {
    let body = r#"
(defn increment-overflow {:t/sig '[:-> (s/or :value int? :overflow #{:overflow})]} []
  (try (inc 9223372036854775807)
       (catch ClassCastException _ :unexpected)
       (catch ArithmeticException _ :overflow)))
(defn unary-overflow {:t/sig '[:-> (s/or :value int? :overflow #{:overflow})]} []
  (try (- -9223372036854775808)
       (catch ArithmeticException _ :overflow)))
(defn intermediate-overflow {:t/sig '[:-> (s/or :value int? :overflow #{:overflow})]} []
  (try (+ 9223372036854775807 1 -1)
       (catch ArithmeticException _ :overflow)))
"#;
    let report = check(body);
    assert!(report.ok && report.complete, "{report:#?}");

    let report = check(&format!(
        r#"{body}
(defn assumes-wrap {{:t/sig '[:-> int?]}} []
  (try (dec -9223372036854775808) (catch ArithmeticException _ :overflow)))
(defn assumes-throw {{:t/sig '[:-> #{{:overflow}}]}} []
  (try (inc 9223372036854775807) (catch ArithmeticException _ :overflow)))
"#
    ));
    let [wrap, throws] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    for (finding, name) in [(wrap, "assumes-wrap"), (throws, "assumes-throw")] {
        assert_eq!(finding.code, "PNX2322", "{finding:#?}");
        assert_eq!(
            finding.contract.as_ref().unwrap().name,
            format!("demo.signed-long-types/{name}")
        );
    }
}

#[test]
fn checked_int_distinguishes_arithmetic_overflow_from_values_beyond_long() {
    let report = check(
        r#"
(defn max-int {:t/sig '[:-> (s/and int? #{2147483647})]} []
  (try (s/conform ::checked-int (- 2147483648 1))
       (catch ArithmeticException _ :unexpected)))
(defn wide-fixed {:t/sig '[:-> #{:arithmetic}]} []
  (try (s/conform ::checked-int (inc 2147483647))
       (catch ArithmeticException _ :arithmetic)
       (catch IllegalArgumentException _ :unexpected)))
(defn wide-boxed {:t/sig '[:-> #{:arithmetic}]} []
  (try (s/valid? ::checked-int -9223372036854775808N)
       (catch ArithmeticException _ :arithmetic)
       (catch IllegalArgumentException _ :unexpected)))
(defn beyond-max {:t/sig '[:-> #{:argument}]} []
  (try (s/conform ::checked-int 9223372036854775808)
       (catch ArithmeticException _ :unexpected)
       (catch IllegalArgumentException _ :argument)))
(defn below-min {:t/sig '[:-> #{:argument}]} []
  (try (s/valid? ::checked-int -9223372036854775809N)
       (catch ArithmeticException _ :unexpected)
       (catch IllegalArgumentException _ :argument)))
"#,
    );
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn wide_extern_signatures_preserve_exact_values_and_classes_without_source() {
    let provider = format!(
        r#"{}
(defn fixed {{:t/sig '[(s/and int? #{{9223372036854775807}})
                       :-> (s/and int? #{{-9223372036854775808}})]}} [_]
  -9223372036854775808)
(defn same-class-agnostic {{:t/sig '[(s/and integer? #{{2147483648}})
                                      :-> (s/and integer? #{{2147483648}})]}} [n]
  n)
"#,
        specs()
    );
    let source = Source {
        path: "provider.clj",
        source: &provider,
    };
    let exported = Session::default().export_externs(&[source], &[]);
    let edn = exported
        .edn
        .unwrap_or_else(|| panic!("{:#?}", exported.report));
    let externs = Source {
        path: "provider.edn",
        source: &edn,
    };
    let validated = Session::default().validate_externs(&[source], &[externs]);
    assert!(validated.ok && validated.complete, "{validated:#?}");
    let check = |body: &str| {
        let source = format!(
            "(ns consumer (:require [clojure.spec.alpha :as s] [demo.signed-long-types :as wire]))\n{body}"
        );
        Session::default().check_project_with_externs(
            &[Source {
                path: "consumer.clj",
                source: &source,
            }],
            &[externs],
        )
    };
    let accepted = check(
        r#"
(defn fixed {:t/sig '[:-> (s/and int? #{-9223372036854775808})]} []
  (wire/fixed (inc 9223372036854775806)))
(defn class-agnostic {:t/sig '[:-> (s/and integer? #{2147483648})]} []
  (wire/same-class-agnostic 2147483648N))
"#,
    );
    assert!(accepted.ok && accepted.complete, "{accepted:#?}");

    for call in [
        "(wire/fixed 9223372036854775806)",
        "(wire/fixed 9223372036854775807N)",
        "(wire/same-class-agnostic 2147483649N)",
    ] {
        let report = check(&format!(
            r#"(defn rejected {{:t/sig '[:-> integer?]}} [] {call})"#
        ));
        let [finding] = report.files[0].diagnostics.as_slice() else {
            panic!("{call}: {report:#?}");
        };
        assert_eq!(finding.code, "PNX2322", "{call}: {finding:#?}");
        assert_eq!(
            finding.contract.as_ref().unwrap().path.as_deref(),
            Some("provider.edn")
        );
    }
}

#[test]
fn wide_exact_values_do_not_become_i32_nth_indices() {
    let body = r#"
(defn vector-cast {:t/sig '[:-> #{:zero}]} [] ([:zero] 4294967296))
(defn nth-gap {:t/sig '[:-> any?]} [] (nth [:zero] 4294967296 :outside))
"#;
    let report = check(body);
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(finding.code, "PNX9001", "{finding:#?}");
    assert!(finding.message.contains("i32 range"), "{finding:#?}");
}
