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
