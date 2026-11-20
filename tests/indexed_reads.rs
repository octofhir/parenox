use parenox::{Report, Session};

fn check(expression: &str, result: &str) -> Report {
    Session::default().check(
        "indexed.clj",
        &format!("(defn f {{:t/sig '[:-> {result}]}} [] {expression})"),
    )
}

#[test]
fn checks_saved_indices_and_diagnoses_out_of_bounds_at_the_index() {
    let mut session = Session::default();
    let report = session.check("indices.clj", include_str!("../examples/indices.clj"));
    assert!(report.ok && report.complete, "{report:#?}");

    let source = include_str!("../examples/indices_bad.clj");
    let report = session.check("indices_bad.clj", source);
    assert!(!report.ok && report.complete, "{report:#?}");
    let [bounds, nil] = &report.diagnostics[..] else {
        panic!("{report:#?}");
    };
    assert_eq!(bounds.code, "PNX2801");
    assert_eq!(
        &source[bounds.span.start.offset..bounds.span.end.offset],
        "2"
    );
    assert!(bounds.message.contains("length 2"), "{bounds:#?}");
    assert_eq!(nil.actual.as_deref(), Some("Nil"));
}

#[test]
fn read_fallbacks_distinguish_missing_from_present_nil() {
    for (expression, result) in [
        ("(nth nil 8)", "nil?"),
        ("(nth [:zero :one] 1N)", "#{:one}"),
        ("(nth [:zero :one] 0x1)", "#{:one}"),
        ("(nth [:zero :one] 2N :missing)", "#{:missing}"),
        (r#"(nth [:sku] -1 "missing")"#, "string?"),
        (r#"(get [:sku nil] 1 "missing")"#, "nil?"),
        (r#"(get [:sku] 1 "missing")"#, "string?"),
        (r#"(get-in [{:sku "A"}] [0 :sku])"#, "string?"),
        (r#"(get-in [{:sku "A"}] [9 :sku] :missing)"#, "keyword?"),
        (r#"(nth "😀" 1 :missing)"#, "char?"),
        (r#"(let [[_ character] "ab"] character)"#, "char?"),
        ("(contains? [:sku nil] 1)", "#{true}"),
        ("(contains? [:sku] -1)", "#{false}"),
        ("(req! [:sku nil] 1)", "nil?"),
        ("(key (nth (mapv identity {:sku 1}) 0))", "keyword?"),
    ] {
        let report = check(expression, result);
        assert!(report.ok && report.complete, "{expression}: {report:#?}");
    }
}

#[test]
fn keeps_unmodeled_numeric_indices_and_writes_explicit() {
    for (expression, code) in [
        ("(nth [:sku] 0.5)", "PNX9001"),
        ("(get [:sku] 010)", "PNX9001"),
        ("(get-in [:sku] [2147483648])", "PNX9001"),
        ("(nth [:sku] :not-an-index)", "PNX2322"),
        ("(nth #{} 0)", "PNX2322"),
        ("(req! [:sku] 1)", "PNX2741"),
        ("(assoc-in [:sku] [0] :new)", "PNX9001"),
    ] {
        let report = check(expression, "keyword?");
        assert_eq!(report.diagnostics.len(), 1, "{expression}: {report:#?}");
        assert_eq!(
            report.diagnostics[0].code, code,
            "{expression}: {report:#?}"
        );
    }
}
