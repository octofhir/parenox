use parenox::{DocumentHover, Report, Session};

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
    let [element, state, entry] = report.diagnostics.as_slice() else {
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
    assert_eq!(entry.code, "PNX2322");
    assert_eq!(
        &source[entry.span.start.offset..entry.span.end.offset],
        "(val entry)"
    );
    assert_eq!(entry.actual.as_deref(), Some("String"));
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
fn conditional_reducers_keep_the_selected_function_and_its_capture_types() {
    let source = r#"(ns demo (:require [clojure.spec.alpha :as s]))
        (defn run {:t/sig '[boolean? :-> #{:left :right}]} [left?]
          (reduce (if left?
                    (fn [acc _] (case acc :start :left :left :left :mixed))
                    (fn [acc _] (case acc :start :right :right :right :mixed)))
                  :start [1 2]))
        (defn label {:t/sig '[(s/nilable string?) :-> string?]} [label]
          (reduce (if (some? label)
                    (fn [_ index] (subs label index))
                    (fn [acc _] (str acc label)))
                  "" [0]))"#;
    let mut session = Session::default();
    let report = session.check("conditional.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");
    for (marker, ty) in [("label index", "String"), ("label)))", "Nil")] {
        let offset = source.find(marker).unwrap();
        let Some(DocumentHover::Local(value)) =
            session.document_hover_at("conditional.clj", offset)
        else {
            panic!("expected a local type for {marker}");
        };
        assert_eq!(value.inferred_type, ty);
    }
}

#[test]
fn conditional_reducers_preserve_eager_argument_order_and_compile_inactive_bodies() {
    let report = check(
        r#"(try
               (reduce (if (throw (ex-info "test" {:stage :test}))
                         (fn [_ _] (throw (ex-info "body" {:stage :body})))
                         (fn [a _] a))
                       (throw (ex-info "init" {:stage :init}))
                       (throw (ex-info "source" {:stage :source})))
               (catch clojure.lang.ExceptionInfo e (:stage (ex-data e))))"#,
        "#{:test}",
    );
    assert!(report.ok && report.complete, "{report:#?}");
    let report = check(
        "(reduce (if false (fn [a b] (+ a b)) (fn [a _] a)) \"safe\" [nil])",
        "string?",
    );
    assert!(report.ok && report.complete, "{report:#?}");
    let report = check(
        "(reduce (if nil (fn [a _] (missing-left a)) (fn [a _] (missing-right a))) 0 [])",
        "number?",
    );
    let [left, right] = report.diagnostics.as_slice() else {
        panic!("{report:#?}")
    };
    assert_eq!(left.code, "PNX2304");
    assert!(left.message.contains("missing-left"));
    assert_eq!(right.code, "PNX2304");
    assert!(right.message.contains("missing-right"));
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

#[test]
fn reduced_results_stop_the_fold_and_only_unwrap_one_layer() {
    for (expression, contract) in [
        (
            r#"(reduce (fn [n item] (if (string? item) (reduced n) (+ n item)))
                       0 [2 3 "stop" false])"#,
            "number?",
        ),
        (
            r#"(unreduced (reduce (fn [_ x] (reduced (reduced x))) 0 ["first" 1]))"#,
            "#{\"first\"}",
        ),
        ("(reduced? (reduce + (reduced 1) []))", "#{true}"),
        ("(reduced? (reduce + [(reduced 1)]))", "#{true}"),
        ("(reduced? (reduce (fn [] (reduced 1)) []))", "#{true}"),
        ("(reduced? (reduce (fn [] (reduced 1)) nil))", "#{true}"),
        (
            "(unreduced (ensure-reduced (reduced \"only\")))",
            "#{\"only\"}",
        ),
    ] {
        let report = check(expression, contract);
        assert!(report.ok && report.complete, "{expression}: {report:#?}");
    }
}

#[test]
fn mixed_reduced_and_continuing_steps_join_terminal_values_without_reusing_wrappers() {
    let source = r#"(ns demo (:require [clojure.spec.alpha :as s]))
        (s/def ::values (s/coll-of (s/or :n number? :s string?) :kind vector?))
        (defn run {:t/sig '[::values :-> (s/or :n number? :s string?)]} [values]
          (reduce (fn [n item]
                    (if (string? item) (reduced item) (+ n item)))
                  0 values))"#;
    let report = Session::default().check("reduce.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn direct_reductions_use_map_entries_set_elements_and_utf16_units() {
    for (expression, expected) in [
        ("(reduce 42 :seed {})", "#{:seed}"),
        ("(reduce 42 :seed #{})", "#{:seed}"),
        ("(reduce 42 :seed \"\")", "#{:seed}"),
        ("(reduce 42 \"a\")", "char?"),
        ("(val (reduce 42 {:rate 5}))", "number?"),
        (
            "(reduce (fn [state entry] (+ state (val entry))) 0 {:tax 1 :price 5})",
            "number?",
        ),
        (
            "(reduce (fn [state item] (if (char? item) :char false)) :start \"😀\")",
            "#{:char}",
        ),
        ("(reduce + 0 #{1 2})", "number?"),
    ] {
        let report = check(expression, expected);
        assert!(report.ok && report.complete, "{expression}: {report:#?}");
    }
}
