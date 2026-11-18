use parenox::{Report, Session};

fn check(expression: &str, contract: &str) -> Report {
    let source = format!("(defn run {{:t/sig '[:-> {contract}]}} [] {expression})");
    Session::default().check("reduce_kv.clj", &source)
}

#[test]
fn reducer_keeps_heterogeneous_keys_paired_with_their_value_types() {
    let source = include_str!("../examples/reduce_kv.clj");
    let report = Session::default().check("reduce_kv.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");

    let edited = source.replace("(count value)", "value");
    let report = Session::default().check("reduce_kv.clj", &edited);
    let [error] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert!(!report.ok && report.complete, "{report:#?}");
    assert_eq!(error.code, "PNX2322");
    assert_eq!(error.actual.as_deref(), Some("String"));
    assert!(
        error
            .notes
            .iter()
            .any(|note| note.contains("reduction step"))
    );
}

#[test]
fn vectors_supply_ordered_ordinals_and_reduced_stops_the_fold() {
    for (body, contract) in [
        (
            r#"(reduce-kv (fn [result index item] (conj result [index item])) [] ["first" false])"#,
            "(clojure.spec.alpha/tuple (clojure.spec.alpha/tuple number? #{\"first\"})
                                      (clojure.spec.alpha/tuple number? #{false}))",
        ),
        (
            r#"(reduce-kv (fn [total index item] (if (= index 0) (reduced item) (inc item)))
                          0 ["stop" false])"#,
            r#"#{"stop"}"#,
        ),
        (
            "(reduce-kv (fn [_ k _] (reduced k)) nil {:only nil})",
            "#{:only}",
        ),
        (
            "(reduced? (reduce-kv (fn [_ _ _] (reduced (reduced 1))) nil [42]))",
            "#{true}",
        ),
    ] {
        let report = check(body, contract);
        assert!(report.ok && report.complete, "{body}: {report:#?}");
    }
}

#[test]
fn empty_inputs_return_initial_value_with_jvm_reducer_casts() {
    for body in [
        "(reduce-kv 7 :initial nil)",
        "(reduce-kv nil :initial [])",
        "(reduce-kv 'ignored :initial {})",
        "(reduce-kv inc :initial {})",
    ] {
        let report = check(body, "#{:initial}");
        assert!(report.ok && report.complete, "{body}: {report:#?}");
    }
    for (body, code) in [
        ("(reduce-kv 7 :initial {})", "PNX2322"),
        ("(reduce-kv 7 :initial [])", "PNX2322"),
        ("(reduce-kv inc :initial [1])", "PNX2554"),
        ("(reduce-kv inc [])", "PNX2554"),
        ("(reduce-kv inc :initial 42)", "PNX2322"),
    ] {
        let report = check(body, "#{:initial}");
        let [error] = report.diagnostics.as_slice() else {
            panic!("{body}: {report:#?}");
        };
        assert!(!report.ok && report.complete, "{body}: {report:#?}");
        assert_eq!(error.code, code, "{body}: {report:#?}");
    }
}

#[test]
fn eager_argument_effects_reach_the_handler_before_reduction() {
    let report = check(
        r#"(try
               (reduce-kv (fnil + (throw (ex-info "reducer" {:stage :reducer})))
                          (throw (ex-info "initial" {:stage :initial}))
                          (throw (ex-info "input" {:stage :input})))
               (catch clojure.lang.ExceptionInfo e (:stage (ex-data e))))"#,
        "#{:reducer}",
    );
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn open_maps_and_unbounded_growth_report_analysis_gaps() {
    let report = Session::default().check(
        "open.clj",
        "(defn run {:t/sig '[map? :-> number?]} [row]
           (reduce-kv (fn [acc _ _] acc) 0 row))",
    );
    assert!(!report.complete);
    assert_eq!(report.diagnostics[0].code, "PNX9001");
    assert!(report.diagnostics[0].message.contains("open map"));

    let inputs = "1 ".repeat(16);
    let report = check(
        &format!("(reduce-kv (fn [acc _ _] [acc acc]) nil [{inputs}])"),
        "(clojure.spec.alpha/coll-of nil? :kind vector?)",
    );
    assert!(!report.complete, "{report:#?}");
    assert_eq!(report.diagnostics.len(), 1, "{report:#?}");
    assert_eq!(report.diagnostics[0].code, "PNX9001");
    assert!(report.diagnostics[0].message.contains("accumulator shape"));
}
