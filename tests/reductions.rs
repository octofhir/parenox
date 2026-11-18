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

#[test]
fn open_typed_reductions_follow_accumulator_states_and_possible_empty_calls() {
    let source = r#"(ns demo (:require [clojure.spec.alpha :as s]))
        (s/def ::numbers (s/coll-of number? :kind vector?))
        (s/def ::labels (s/coll-of string? :kind vector?))
        (defn sum-values {:t/sig '[::numbers :-> number?]} [numbers]
          (reduce + 0 numbers))
        (defn labels {:t/sig '[::labels :-> ::labels]} [labels]
          (reduce conj [] labels))
        (defn default-or-sum {:t/sig '[::numbers :-> (s/or :empty string? :total number?)]}
          [numbers] (reduce (fn ([] "empty") ([a b] (+ a b))) numbers))
        (defn last-nonempty {:t/sig '[::numbers :-> string?]} [numbers]
          (if-let [items (seq numbers)] (reduce (fn [_ item] (str item)) 0 items) "empty"))"#;
    let report = Session::default().check("reduce.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn an_open_reduction_checks_a_later_unsafe_state_without_repeating_diagnostics() {
    let source = r#"(ns demo (:require [clojure.spec.alpha :as s]))
        (defn run {:t/sig '[(s/coll-of number? :kind vector?)
                             :-> (s/or :n number? :s string?)]}
          [items]
          (reduce (fn [acc _] (if (number? acc) "second" (inc acc))) 0 items))"#;
    let report = Session::default().check("reduce.clj", source);
    let [diagnostic] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(diagnostic.code, "PNX2322");
    assert_eq!(diagnostic.actual.as_deref(), Some("String"));
    assert!(
        diagnostic
            .notes
            .iter()
            .any(|note| note.contains("reduction step 2"))
    );
}

#[test]
fn expanding_reduction_states_and_nested_steps_report_analysis_limits() {
    let open = Session::default().check(
        "reduce.clj",
        "(ns demo (:require [clojure.spec.alpha :as s]))
         (defn run {:t/sig '[(s/coll-of number? :kind vector?) :-> number?]} [items]
           (reduce (fn [acc _] [acc acc]) [] items))",
    );
    assert!(!open.complete, "{open:#?}");
    assert!(open.diagnostics.iter().any(|d| {
        d.code == "PNX9001" && d.message.contains("accumulator types did not stabilize")
    }));

    let items = "1 ".repeat(31);
    let nested = check(
        &format!("(reduce (fn [_ _] (reduce + 0 [{items}])) 0 [{items}])"),
        "number?",
    );
    assert!(!nested.complete, "{nested:#?}");
    assert!(
        nested
            .diagnostics
            .iter()
            .any(|d| { d.code == "PNX9001" && d.message.contains("step analysis limit") })
    );
}
