use parenox::Session;

#[test]
fn integer_contracts_accept_canonicalized_jvm_integer_literals() {
    for expression in [
        "42",
        "42N",
        "0x2a",
        "052",
        "2r101010",
        "84/2",
        "2147483648",
        "-9223372036854775808N",
        "99999999999999999999999999999999999999999N",
    ] {
        let source = format!("(defn f {{:t/sig '[:-> integer?]}} [] {expression})");
        let report = Session::default().check("numbers.clj", &source);
        assert!(report.ok && report.complete, "{expression}: {report:#?}");
    }
}

#[test]
fn an_unchecked_number_is_not_proof_of_a_clojure_integer() {
    let source = "(defn f {:t/sig '[number? :-> integer?]} [x] x)";
    let report = Session::default().check("numbers.clj", source);
    assert!(!report.complete, "{report:#?}");
    assert_eq!(report.diagnostics.len(), 1, "{report:#?}");
    assert_eq!(report.diagnostics[0].code, "PNX9004");
}

#[test]
fn integer_contracts_reject_other_numeric_classes_even_for_whole_values() {
    for expression in ["42.0", "42M", "85/2", "42e0", "##NaN"] {
        let source = format!("(defn f {{:t/sig '[:-> integer?]}} [] {expression})");
        let report = Session::default().check("numbers.clj", &source);
        assert!(!report.ok && report.complete, "{expression}: {report:#?}");
        let findings = &report.diagnostics;
        assert_eq!(findings.len(), 1, "{expression}: {report:#?}");
        assert_eq!(findings[0].code, "PNX2322");
        assert_eq!(findings[0].expected.as_deref(), Some("Integer"));
    }
}
