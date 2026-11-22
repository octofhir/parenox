use parenox::Session;

#[test]
fn primitive_function_hints_follow_jvm_signature_rules() {
    for (source, tag, expected) in [
        (
            "(defn rejected {:t/sig '[number? :-> number?]} [^int x] x)",
            "int",
            "Only long and double",
        ),
        (
            r#"(defn rejected {:t/sig '[number? :-> number?]} [^"byte" x] x)"#,
            r#""byte""#,
            "Only long and double",
        ),
        (
            "(defn rejected ^double [a b c d e] a)",
            "double",
            "4 or fewer",
        ),
        (
            "(defn rejected ^long [& values] 0)",
            "long",
            "cannot be variadic",
        ),
        (
            "(defn rejected [& ^bytes values] values)",
            "bytes",
            "cannot have type hint",
        ),
    ] {
        let report = Session::default().check("hints.clj", source);
        let [finding] = report.diagnostics.as_slice() else {
            panic!("{source}: {report:#?}");
        };
        assert_eq!(finding.code, "PNX1001");
        assert!(finding.message.contains(expected), "{report:#?}");
        assert_eq!(
            &source[finding.span.start.offset..finding.span.end.offset],
            tag
        );
    }

    let valid = r#"
(ns hints.valid (:require [clojure.spec.alpha :as s]))
(defn native {:t/sig '[int? :-> int?]} ^"long" [^long n] n)
(defn boxed {:t/sig '[number? :-> number?]} [^Integer n] n)
(defn array-argument {:t/sig '[bytes? :-> int?]} [^bytes value] (alength value))
(defn destructured {:t/sig '[(s/tuple number?) :-> number?]} [^int [n]] n)
"#;
    let report = Session::default().check("hints.clj", valid);
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn lexical_hint_conflicts_with_a_primitive_initializer_but_allows_boxed_values() {
    for init in [
        "1",
        "1/1",
        "1.0",
        "##NaN",
        "(long 1)",
        "(int 1)",
        "(char 65)",
    ] {
        let source =
            format!("(defn rejected {{:t/sig '[:-> t/any]}} [] (let [^bytes value {init}] value))");
        let report = Session::default().check("hints.clj", &source);
        let [finding] = report.diagnostics.as_slice() else {
            panic!("{init}: {report:#?}");
        };
        assert_eq!(finding.code, "PNX1001");
        assert!(finding.message.contains("primitive initializer"));
        assert_eq!(
            &source[finding.span.start.offset..finding.span.end.offset],
            "bytes"
        );
    }

    let valid = r#"
(defn boxed-call {:t/sig '[:-> number?]} []
  (let [^bytes value (identity 1)] value))
(defn boxed-number {:t/sig '[:-> number?]} []
  (let [^bytes value 1N] value))
(defn boxed-decimal {:t/sig '[:-> number?]} []
  (let [^bytes value 1M] value))
(defn shadowed-conversion {:t/sig '[:-> number?]} []
  (let [long (fn [n] n) ^bytes value (long 1)] value))
"#;
    let report = Session::default().check("hints.clj", valid);
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn native_arguments_change_the_type_seen_by_the_function_body() {
    let valid = r#"
(defn entered-long {:t/sig '[number? :-> int?]} [^long value] value)
(defn entered-double {:t/sig '[int? :-> false?]} [^double value]
  (if (int? value) (inc "unreachable") false))
(defn contextual {:t/sig '[:-> int?]} []
  ((fn [^long value] value) 1.5))
"#;
    let report = Session::default().check("hints.clj", valid);
    assert!(report.ok && report.complete, "{report:#?}");

    for (source, code, span, message) in [
        (
            "(defn result {:t/sig '[int? :-> int?]} [^double value] value)",
            "PNX2322",
            "value",
            "Return value",
        ),
        (
            "(defn char-input {:t/sig '[char? :-> number?]} [^long value] value)",
            "PNX2322",
            "long",
            "JVM long parameter",
        ),
        (
            r#"(defn wrong-operand {:t/sig '[:-> number?]} [] ((fn [^long value] value) "text"))"#,
            "PNX2322",
            r#""text""#,
            "JVM long parameter",
        ),
        (
            "(defn open-input {:t/sig '[t/any :-> number?]} [^double value] value)",
            "PNX9004",
            "double",
            "JVM double parameter",
        ),
    ] {
        let report = Session::default().check("hints.clj", source);
        let [finding] = report.diagnostics.as_slice() else {
            panic!("{source}: {report:#?}");
        };
        assert_eq!(finding.code, code);
        assert!(finding.message.contains(message), "{report:#?}");
        assert_eq!(
            &source[finding.span.start.offset..finding.span.end.offset],
            span
        );
    }
    let source =
        r#"(defn open-input {:t/sig '[t/any :-> number?]} [^double value] (+ value "bad"))"#;
    let report = Session::default().check("hints.clj", source);
    let location: Vec<_> = report
        .diagnostics
        .iter()
        .map(|finding| {
            (
                finding.code.as_str(),
                &source[finding.span.start.offset..finding.span.end.offset],
            )
        })
        .collect();
    assert_eq!(location, [("PNX9004", "double"), ("PNX2322", r#""bad""#)]);
}

#[test]
fn native_recur_preserves_slot_types_and_checks_primitive_operands() {
    let valid = r#"
(defn countdown {:t/sig '[int? :-> number?]} [^double value]
  (if (= value 0.0) value (let [zero 0.0] (recur zero))))
(defn narrow {:t/sig '[int? :-> int?]} [^long value]
  (if (= value 0) value (recur 0)))
(defn boxed-shadow {:t/sig '[int? :-> number?]} [^double value]
  (let [value (identity value) ^String boxed value] boxed))
"#;
    let report = Session::default().check("hints.clj", valid);
    assert!(report.ok && report.complete, "{report:#?}");

    for (source, tag, reason) in [
        (
            "(defn bad {:t/sig '[number? :-> number?]} [^double n] (if false (recur 0) n))",
            "0",
            "recur to a JVM double parameter",
        ),
        (
            "(defn bad {:t/sig '[int? :-> int?]} [^long n] (if false (recur (identity 0)) n))",
            "(identity 0)",
            "cannot take a boxed value",
        ),
        (
            "(defn bad {:t/sig '[number? :-> number?]} [^double n] (let [^bytes value n] value))",
            "bytes",
            "primitive initializer",
        ),
        (
            "(defn bad {:t/sig '[:-> number?]} [] (let [^bytes n (do 1)] n))",
            "bytes",
            "primitive initializer",
        ),
    ] {
        let report = Session::default().check("hints.clj", source);
        let [finding] = report.diagnostics.as_slice() else {
            panic!("{source}: {report:#?}");
        };
        assert_eq!(finding.code, "PNX1001");
        assert!(finding.message.contains(reason), "{report:#?}");
        assert_eq!(
            &source[finding.span.start.offset..finding.span.end.offset],
            tag
        );
    }
}

#[test]
fn native_returns_distinguish_unboxed_mismatch_from_boxed_conversion() {
    let valid = r#"
(defn boxed {:t/sig '[int? :-> number?]} ^double [value] (identity value))
(defn widen-if {:t/sig '[boolean? :-> number?]} ^double [flag] (if flag 1 2.0))
(defn wrapped-long {:t/sig '[:-> int?]} ^long [] (identity 9223372036854775808N))
(defn native-counter {:t/sig '[int? :-> int?]} ^long [^long n]
  (if (= n 0) n (recur 0)))
"#;
    let report = Session::default().check("hints.clj", valid);
    assert!(report.ok && report.complete, "{report:#?}");

    for (source, code, span, message) in [
        (
            "(defn result {:t/sig '[int? :-> int?]} ^double [value] (identity value))",
            "PNX2322",
            "(identity value)",
            "Return value",
        ),
        (
            "(defn invalid {:t/sig '[int? :-> number?]} ^double [^long value] value)",
            "PNX1001",
            "value",
            "cannot use a primitive long",
        ),
        (
            "(defn invalid {:t/sig '[boolean? :-> number?]} ^double [flag] (if flag 1 2))",
            "PNX1001",
            "(if flag 1 2)",
            "cannot use a primitive long",
        ),
        (
            "(defn invalid {:t/sig '[:-> number?]} ^double [] '1)",
            "PNX1001",
            "'1",
            "cannot use a primitive long",
        ),
        (
            "(defn unchecked {:t/sig '[:-> number?]} ^double [] (+ 1 2))",
            "PNX9001",
            "(+ 1 2)",
            "compiler type for this return expression is unchecked",
        ),
    ] {
        let report = Session::default().check("hints.clj", source);
        let [finding] = report.diagnostics.as_slice() else {
            panic!("{source}: {report:#?}");
        };
        assert_eq!(finding.code, code);
        assert!(finding.message.contains(message), "{report:#?}");
        assert_eq!(
            &source[finding.span.start.offset..finding.span.end.offset],
            span
        );
    }
}
