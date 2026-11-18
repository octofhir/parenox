use parenox::{Report, Session};

fn check(expression: &str, ret: &str) -> Report {
    let source = format!("(ns demo) (defn run {{:t/sig '[:-> {ret}]}} [] {expression})");
    Session::default().check("reduce.clj", &source)
}

#[test]
fn finite_reductions_use_empty_and_singleton_arities_and_preserve_order() {
    let source = include_str!("../examples/reductions.clj");
    let report = Session::default().check("reduce.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");
    for (body, ret) in [
        (r#"(reduce (fn [] "zero") nil)"#, "string?"),
        (r#"(reduce (fn [x y] (+ x y)) "default" [])"#, "string?"),
        (r#"(reduce nil "default" [])"#, "string?"),
        (r#"(reduce 7 "default" ())"#, "string?"),
        (r#"(reduce (fn [x y] (+ x y)) ["only"])"#, "string?"),
        (
            r#"(reduce (fn [x y] (first [y x])) (seq ["first" 1 "last"]))"#,
            "#{\"last\"}",
        ),
        (r#"(reduce - [6 2 1])"#, "number?"),
    ] {
        let report = check(body, ret);
        assert!(report.ok && report.complete, "{body}: {report:#?}");
    }
    let report = check("(reduce 7 0 [])", "number?");
    assert_eq!(report.diagnostics[0].code, "PNX2322", "{report:#?}");
    let report = check("(reduce inc nil)", "number?");
    assert_eq!(report.diagnostics[0].code, "PNX2554", "{report:#?}");
    let report = check("(reduce (fn [x y] (unresolved x)) 0 [])", "number?");
    assert_eq!(report.diagnostics[0].code, "PNX2304", "{report:#?}");
}

#[test]
fn reduction_errors_point_to_real_elements_and_explain_the_accumulator_step() {
    let source = include_str!("../examples/reductions_bad.clj");
    let report = Session::default().check("reduce_bad.clj", source);
    let [element, state] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(element.code, "PNX2322");
    assert_eq!(
        &source[element.span.start.offset..element.span.end.offset],
        "\"2\""
    );
    assert_eq!(element.suggestions[0].replacement, "2");
    assert_eq!(state.code, "PNX2322");
    assert!(
        state
            .notes
            .iter()
            .any(|note| note.contains("reduction step 2"))
    );
}

#[test]
fn reduction_arguments_follow_jvm_evaluation_order() {
    let report = check(
        r#"(try
               (reduce (fnil + (throw (ex-info "target" {:stage :target})))
                       (throw (ex-info "initial" {:stage :initial}))
                       (throw (ex-info "collection" {:stage :collection})))
               (catch clojure.lang.ExceptionInfo e (:stage (ex-data e))))"#,
        "#{:target}",
    );
    assert!(report.ok && report.complete, "{report:#?}");
    let report = check("(reduce + 0 false)", "number?");
    assert_eq!(report.diagnostics[0].code, "PNX2322", "{report:#?}");
}
