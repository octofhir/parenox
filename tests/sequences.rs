use parenox::{Report, Session};

fn check(expression: &str, contract: &str) -> Report {
    let source = format!("(ns demo) (defn f {{:t/sig '[:-> {contract}]}} [] {expression})");
    Session::default().check("sequences.clj", &source)
}

#[test]
fn nonempty_seq_guards_support_running_totals_and_precise_positions() {
    let report =
        Session::default().check("sequences.clj", include_str!("../examples/sequences.clj"));
    assert!(report.ok && report.complete, "{report:#?}");
    let report = Session::default().check(
        "sequences_bad.clj",
        include_str!("../examples/sequences_bad.clj"),
    );
    assert!(!report.ok && report.complete, "{report:#?}");
    let [absence, scalar] = report.diagnostics.as_slice() else {
        panic!("{report:#?}")
    };
    assert_eq!(absence.code, "PNX2322");
    assert_eq!(absence.actual.as_deref(), Some("Nil | Number"));
    assert_eq!(scalar.expected.as_deref(), Some("Seqable or Nil"));
    assert_eq!(scalar.actual.as_deref(), Some("false"));
}

#[test]
fn seq_emptiness_preserves_nil_elements_and_index_errors() {
    for (expression, contract) in [
        ("(seq [])", "nil?"),
        ("(next nil)", "nil?"),
        ("(first nil)", "nil?"),
        ("(next [1])", "nil?"),
        ("(if (seq [nil]) (first (seq [nil])) 42)", "nil?"),
        ("(nth (seq [1 \"name\"]) 1)", "string?"),
        ("(nth (seq [1]) 3 \"fallback\")", "string?"),
        ("(count (seq [1 2]))", "number?"),
    ] {
        let report = check(expression, contract);
        assert!(report.ok && report.complete, "{expression}: {report:#?}");
    }
    for (expression, code) in [
        ("(nth (seq [1]) 1)", "PNX2801"),
        ("(next false)", "PNX2322"),
        ("(seq [] [])", "PNX2554"),
    ] {
        let report = check(expression, "number?");
        assert_eq!(report.diagnostics.len(), 1, "{expression}: {report:#?}");
        assert_eq!(
            report.diagnostics[0].code, code,
            "{expression}: {report:#?}"
        );
    }
}

#[test]
fn rest_returns_a_truthy_empty_list_and_keeps_remaining_positions() {
    for (expression, contract) in [
        ("(if (rest nil) \"truthy\" 0)", "string?"),
        ("(if (rest [1]) \"truthy\" 0)", "string?"),
        ("(seq (rest []))", "nil?"),
        ("(seq (rest [nil]))", "nil?"),
        ("(first (rest [1 nil]))", "nil?"),
        ("(next (rest [1 2]))", "nil?"),
        ("(nth (rest [1 \"tail\"]) 0)", "string?"),
        ("(nth (rest [1]) 0 \"missing\")", "string?"),
        ("(seq ())", "nil?"),
        ("(seq '())", "nil?"),
        ("(seq (#()))", "nil?"),
        ("(apply + (rest [4]))", "number?"),
        ("(first (conj (rest [4]) \"reset\"))", "string?"),
    ] {
        let report = check(expression, contract);
        assert!(report.ok && report.complete, "{expression}: {report:#?}");
    }
    let report = check("(rest false)", "number?");
    let [diagnostic] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(diagnostic.code, "PNX2322");
    assert_eq!(diagnostic.expected.as_deref(), Some("Seqable or Nil"));

    let report = check("(nth (rest nil) 0)", "number?");
    assert_eq!(report.diagnostics[0].code, "PNX2801", "{report:#?}");
}

#[test]
fn emptiness_distinguishes_nil_empty_and_present_false_values() {
    for (expression, contract) in [
        ("(empty? nil)", "#{true}"),
        ("(empty? (rest [1]))", "#{true}"),
        ("(empty? {})", "#{true}"),
        ("(empty? #{})", "#{true}"),
        ("(empty? \"\")", "#{true}"),
        ("(empty? [nil])", "#{false}"),
        ("(empty? {:x false})", "#{false}"),
        ("(not-empty \"\")", "nil?"),
        ("(not-empty \"Ada\")", "#{\"Ada\"}"),
        (
            "(not-empty [nil])",
            "(clojure.spec.alpha/coll-of nil? :kind vector?)",
        ),
        (
            "(filterv not-empty [\"\" \"Ada\"])",
            "(clojure.spec.alpha/coll-of #{\"Ada\"} :kind vector?)",
        ),
    ] {
        let report = check(expression, contract);
        assert!(report.ok && report.complete, "{expression}: {report:#?}");
    }
    for expression in ["(empty? false)", "(not-empty 0)"] {
        let report = check(expression, "boolean?");
        assert_eq!(report.diagnostics[0].code, "PNX2322", "{report:#?}");
    }
}

#[test]
fn emptiness_guards_refine_sequence_state_and_open_empty_shapes() {
    let report = Session::default().check(
        "sequences.clj",
        r#"(ns demo (:require [clojure.spec.alpha :as s]))
           (defn sum-values
             {:t/sig '[(s/coll-of number? :kind vector?) :-> number?]}
             [values]
             (loop [items (seq values) total 0]
               (if-not (empty? items)
                 (recur (next items) (+ total (first items)))
                 total)))
           (defn empty-indexed
             {:t/sig '[(s/coll-of number? :kind vector?) :-> nil?]}
             [values]
             (when (empty? values) (nth values 0 nil)))
           (defn fallback
             {:t/sig '[(s/keys) :-> string?]} [configuration]
             (if (empty? configuration)
               (:unexpected configuration "missing")
               "configured"))"#,
    );
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn tuple_spec_positions_do_not_describe_a_vector_sequence() {
    let source = r#"(ns demo (:require [clojure.spec.alpha :as s]))
(defn indexed {:t/sig '[(s/tuple #{"x"}) :-> #{"x"}]} [[head]] head)
(defn read-head {:t/sig '[(s/tuple #{"x"}) :-> any?]} [input]
  (subs (first input) 2))
(defn choose {:t/sig '[(s/tuple #{"x"}) :-> any?]} [input]
  (let [choice (if (seq input) :present :empty)] choice))
(defn counted-empty {:t/sig '[(s/coll-of string? :kind vector?) :-> any?]} [input]
  (when (empty? input) (first input)))
(defn ordinary {:t/sig '[boolean? :-> (s/nilable #{"x"})]} [condition]
  (let [optional (if condition ["x"] nil)
        items (if condition ["x"] [])]
    (when (seq items) (first items))
    (first optional)))"#;
    let mut session = Session::default();
    let report = session.check("vector_sources.clj", source);
    assert!(!report.complete, "{report:#?}");
    assert_eq!(report.diagnostics.len(), 3, "{report:#?}");
    for finding in &report.diagnostics {
        assert_eq!(finding.code, "PNX9001");
        assert_eq!(finding.severity, "warning");
        assert_eq!(
            &source[finding.span.start.offset..finding.span.end.offset],
            "input"
        );
    }
    let offset = source.find("choice))").unwrap();
    let Some(parenox::DocumentHover::Local(hover)) =
        session.document_hover_at("vector_sources.clj", offset)
    else {
        panic!("missing hover on the branches of a structural sequence");
    };
    assert_eq!(hover.inferred_type, ":empty | :present");
}

#[test]
fn ordinary_vector_origins_survive_sequence_reads_and_callbacks() {
    for expression in [
        r#"(first (first (seq [["x"]])))"#,
        r#"(first (first (rest [nil ["x"]])))"#,
        r#"(let [outer [["x"]]] (apply first outer))"#,
        r#"(reduce (fn [_ item] (first item)) nil [["x"]])"#,
        r#"(some first [["x"]])"#,
        r#"(first (or nil (if-let [value "x"] [value] [])))"#,
        r#"(first (case :hit :hit ["x"] ["wrong"]))"#,
    ] {
        let report = check(expression, r#"#{"x"}"#);
        assert!(report.ok && report.complete, "{expression}: {report:#?}");
    }
}

#[test]
fn seq_operations_handle_entry_types_sets_and_jvm_string_positions() {
    for (expression, contract) in [
        ("(seq {})", "nil?"),
        ("(next {:only 1})", "nil?"),
        ("(val (first {:only \"Ada\"}))", "#{\"Ada\"}"),
        ("(key (first (next {:a 1 :b 2})))", "keyword?"),
        ("(nth (seq {:a 1 :b 2}) 2 \"missing\")", "#{\"missing\"}"),
        ("(seq #{})", "nil?"),
        (
            "(first (seq #{:a :b}))",
            "(clojure.spec.alpha/nilable keyword?)",
        ),
        ("(first \"Ada\")", "#{\\A}"),
        ("(next \"\")", "nil?"),
        ("(nth (seq \"🚀\") 1)", "char?"),
        ("(nth (rest \"🚀x\") 1)", "#{\\x}"),
    ] {
        let report = check(expression, contract);
        assert!(report.ok && report.complete, "{expression}: {report:#?}");
    }
    let source = r#"(ns demo (:require [clojure.spec.alpha :as s]))
        (defn f {:t/sig '[(s/keys) :-> string?]} [source]
          (str (first source)))"#;
    let report = Session::default().check("sequences.clj", source);
    assert!(!report.complete, "{report:#?}");
    assert_eq!(report.diagnostics[0].code, "PNX9001");
    assert!(report.diagnostics[0].message.contains("open map"));
}
