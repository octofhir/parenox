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
        ("(get [:sku] 010 :missing)", "#{:missing}"),
        (r#"(get-in [{:sku "A"}] [0 :sku])"#, "string?"),
        (r#"(get-in [{:sku "A"}] [9 :sku] :missing)"#, "keyword?"),
        ("(get-in [:sku] [2147483648] :missing)", "#{:missing}"),
        (r#"(nth "😀" 1 :missing)"#, "char?"),
        (r#"(get "😀A" 2 :missing)"#, r"#{\A}"),
        (r#"(nth "éΩ" 1)"#, r"#{\Ω}"),
        (r#"(.charAt "😀A" 2)"#, r"#{\A}"),
        (r#"(nth "😀" 2 :missing)"#, "#{:missing}"),
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
fn nth_preserves_proved_vector_elements_and_fallbacks() {
    for expression in [
        r#"(mapv first (nth [[["x"] ["y"]]] 0x0))"#,
        r#"(mapv first (nth [] 1N [["x"] ["y"]]))"#,
        r#"(mapv first (nth nil 1 [["x"] ["y"]]))"#,
    ] {
        let report = check(expression, r#"(clojure.spec.alpha/tuple #{"x"} #{"y"})"#);
        assert!(report.ok && report.complete, "{expression}: {report:#?}");
    }
}

#[test]
fn vector_invocation_uses_checked_indices_and_preserves_stored_vectors() {
    for (expression, expected) in [
        ("(let [i 4294967296N] ([:hit] i))", "#{:hit}"),
        ("([:first :second] 0x1)", "#{:second}"),
        (
            r#"(let [indexed [[["x"] ["y"]]]] (mapv first (indexed 0)))"#,
            r#"(clojure.spec.alpha/tuple #{"x"} #{"y"})"#,
        ),
    ] {
        let report = check(expression, expected);
        assert!(report.ok && report.complete, "{expression}: {report:#?}");
    }
}

#[test]
fn subvec_keeps_nested_positions_and_distinguishes_an_empty_slice() {
    for (expression, expected) in [
        (
            r#"(mapv first (subvec [["drop"] ["x"] ["y"]] 1 3))"#,
            r#"(clojure.spec.alpha/tuple #{"x"} #{"y"})"#,
        ),
        ("(peek (subvec [:first :last] 1))", "#{:last}"),
        ("(empty? (subvec [:first] 1 1))", "#{true}"),
    ] {
        let report = check(expression, expected);
        assert!(report.ok && report.complete, "{expression}: {report:#?}");
    }
}

#[test]
fn subvec_marks_the_rejected_bound_and_only_its_exception() {
    for (expression, bound) in [
        ("(subvec [:one] -1 0)", "-1"),
        ("(subvec [:one] 0 (inc 1))", "(inc 1)"),
        ("(subvec [:one] 1 0)", "0"),
    ] {
        let source = format!(
            "(defn f {{:t/sig '[:-> #{{:bounds}}]}} []
          (try {expression}
            (catch IllegalArgumentException _ :wrong)
            (catch IndexOutOfBoundsException _ :bounds)))"
        );
        let report = Session::default().check("slice.clj", &source);
        assert!(report.complete, "{report:#?}");
        let [finding] = report.diagnostics.as_slice() else {
            panic!("{report:#?}");
        };
        assert_eq!(finding.code, "PNX2801");
        assert_eq!(
            finding.contract.as_ref().unwrap().name,
            "clojure.core/subvec"
        );
        assert_eq!(
            &source[finding.span.start.offset..finding.span.end.offset],
            bound
        );
    }
}

#[test]
fn keeps_unmodeled_numeric_indices_and_writes_explicit() {
    for (expression, code) in [
        ("(nth [:sku] 0.5)", "PNX9001"),
        ("(get [:sku] 9223372036854775808N)", "PNX9001"),
        (r#"(get "abc" 0.5)"#, "PNX9001"),
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

#[test]
fn vector_and_string_reads_distinguish_the_accepted_jvm_index_classes() {
    let (source, _) = include_str!("fixtures/index-key-classes.clj")
        .split_once(";; JVM-only assertions.")
        .unwrap();
    let report = Session::default().check("indexed.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");

    let overclaim = source.replace("#{:found :missing}", "#{:found}");
    let report = Session::default().check("indexed.clj", &overclaim);
    assert!(!report.ok && report.complete, "{report:#?}");
    assert_eq!(report.diagnostics.len(), 6, "{report:#?}");
    assert!(report.diagnostics.iter().all(|finding| {
        finding.code == "PNX2322" && finding.actual.as_deref().unwrap().contains(":missing")
    }));
}
