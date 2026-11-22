use parenox::{DocumentHover, Report, Session};

fn check(body: &str) -> Report {
    let source = format!("(ns demo (:require [clojure.spec.alpha :as s]))\n{body}");
    Session::default().check("vectors.clj", &source)
}

#[test]
fn checks_vector_pipelines_and_narrows_elements_with_core_predicates() {
    let report = Session::default().check("vectors.clj", include_str!("../examples/vectors.clj"));
    assert!(report.ok && report.complete, "{report:#?}");

    for (expr, element) in [
        (r#"(filterv string? [nil "Ada" 1])"#, r#"#{ "Ada" }"#),
        (r#"(filterv nil? [nil "Ada"])"#, "nil?"),
        (r#"(filterv identity [false nil ""])"#, "string?"),
        ("(filterv some? [false nil])", "boolean?"),
        ("(mapv identity #{:active})", "keyword?"),
        (r#"(mapv identity "Ada")"#, "char?"),
        ("(mapv inc [] [1])", "string?"),
        (r#"(mapv inc (next (seq ["skipped" 1 2])))"#, "number?"),
        (
            r#"(filterv string? (seq [nil "Ada" false]))"#,
            r#"#{ "Ada" }"#,
        ),
        ("(mapv inc (next (seq [1])))", "string?"),
        ("(filterv (fn [x] false) [1])", "string?"),
    ] {
        let report = check(&format!(
            "(defn f {{:t/sig '[:-> (s/coll-of {element} :kind vector?)]}} [] {expr})"
        ));
        assert!(report.ok && report.complete, "{expr}: {report:#?}");
    }
}

#[test]
fn scoped_callback_expressions_use_the_actual_invocation_types() {
    let body = r#"(ns labels (:require [clojure.spec.alpha :as s]))
        (defn labels {:t/sig '[string? :-> (s/coll-of string? :kind vector?)]} [title]
          (mapv (let [prefix (subs title 1)]
                  (fn [message] (str prefix (subs message 1))))
                ["label"]))
        (defn immediate {:t/sig '[string? :-> string?]} [prefix]
          ((do (count prefix)
               (let [prefix 7
                     callback #(str (inc prefix) (subs % 1))]
                 callback))
           prefix))"#;
    let mut session = Session::default();
    let report = session.check("labels.clj", body);
    assert!(report.ok && report.complete, "{report:#?}");

    let edited = body.replace("(subs message 1)", "(inc message)");
    let report = session.check("labels.clj", &edited);
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert!(report.complete);
    assert_eq!(finding.code, "PNX2322");
    assert_eq!(
        &edited[finding.span.start.offset..finding.span.end.offset],
        "message"
    );
}

#[test]
fn vector_operations_keep_one_callback_choice_for_the_whole_collection() {
    let source = r#"(ns vectors (:require [clojure.spec.alpha :as s]))
        (defn direct
          {:t/sig '[boolean? :-> (s/or :numbers (s/tuple int? int?)
                                     :strings (s/tuple string? string?))]} [numeric?]
          (mapv (if numeric? #(inc %) #(subs % 1))
                (if numeric? [1 2] ["cat" "dog"])))
        (defn transduced
          {:t/sig '[boolean? :-> (s/or :numbers (s/tuple int? int?)
                                     :strings (s/tuple string? string?))]} [numeric?]
          (into [] (map (if numeric? #(inc %) #(subs % 1)))
                (if numeric? [1 2] ["cat" "dog"])))
        (defn named
          {:t/sig '[boolean? :-> (s/or :numbers (s/tuple int? int?)
                                     :strings (s/tuple string? string?))]} [numeric?]
          (mapv (if numeric? inc str) [1 2]))
        (defn mixed
          {:t/sig '[boolean? :-> (s/or :numbers (s/tuple int? int?)
                                     :strings (s/tuple string? string?))]} [numeric?]
          (mapv (if numeric? #(inc %) str) [1 2]))
        (defn filtered
          {:t/sig '[boolean? :-> (s/or :number (s/tuple int?) :string (s/tuple string?))]}
          [numeric?]
          (filterv (if numeric? number? string?) [nil 1 "cat"]))"#;
    let mut session = Session::default();
    let path = "vectors.clj";
    let report = session.check(path, source);
    assert!(report.ok && report.complete, "{report:#?}");

    let edited = source.replacen("#(subs % 1)", "#(inc %)", 1);
    let report = session.check(path, &edited);
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert!(report.complete);
    assert_eq!(finding.code, "PNX2322");
    assert_eq!(
        &edited[finding.span.start.offset..finding.span.end.offset],
        "%"
    );
}

#[test]
fn an_ordinary_callback_choice_is_evaluated_before_the_collection() {
    let report = check(
        r#"(defn choose {:t/sig '[boolean? :-> #{:callback :collection}]} [stop?]
             (try
               (mapv (if stop?
                       (throw (ex-info "callback" {:stage :callback}))
                       inc)
                     (throw (ex-info "collection" {:stage :collection})))
               (catch clojure.lang.ExceptionInfo error (:stage (ex-data error)))))"#,
    );
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn optional_callbacks_follow_their_branch_and_evaluate_the_prefix_first() {
    let source = r#"(ns vectors (:require [clojure.spec.alpha :as s]))
        (defn enabled
          {:t/sig '[boolean? :-> (s/coll-of int? :kind vector?)]} [ready?]
          (mapv (when ready? (inc 0) #(inc %))
                (if ready? [1] [])))
        (defn enabled-unless
          {:t/sig '[boolean? :-> (s/coll-of int? :kind vector?)]} [skip?]
          (mapv (when-not skip? (inc 0) inc)
                (if skip? [] [1])))
        (defn omitted
          {:t/sig '[boolean? :-> (s/coll-of int? :kind vector?)]} [ready?]
          (mapv (if ready? inc) (if ready? [1] [])))
        (defn order {:t/sig '[boolean? :-> #{:callback :input}]} [ready?]
          (try
            (mapv (when ready?
                    (throw (ex-info "callback" {:site :callback}))
                    (fn [element] element))
                  (throw (ex-info "input" {:site :input})))
            (catch clojure.lang.ExceptionInfo error (:site (ex-data error)))))"#;
    let path = "vectors.clj";
    let mut session = Session::default();
    let report = session.check(path, source);
    assert!(report.ok && report.complete, "{report:#?}");

    let edited = source.replace("(fn [element] element)", "(fn [element] missing)");
    let report = session.check(path, &edited);
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(finding.code, "PNX2304");
    assert_eq!(
        &edited[finding.span.start.offset..finding.span.end.offset],
        "missing"
    );
}

#[test]
fn into_checks_transducers_with_the_actual_vector_positions() {
    let report = check(
        r#"
(defn namespace-usages
  {:t/sig '[:-> (s/tuple {:to #{example.source} :name #{read-config}})]} []
  (let [namespace 'example.source
        uri "file:///example.clj"
        db {:analysis {"file:///example.clj"
                       {:var-usages [{:to 'example.source :name 'read-config}
                                     {:to 'example.dest :name 'some-key}]}}}]
    (into [] (filter #(= namespace (:to %))) (get-in db [:analysis uri :var-usages]))))
(defn names {:t/sig '[:-> (s/tuple #{"read"} #{"write"})]} []
  (into [] (map (fn [element] (:name element))) [{:name "read"} {:name "write"}]))
(defn map-falsey {:t/sig '[:-> (s/tuple nil? #{false})]} []
  (into [] (map identity) [nil false]))
(defn keep-false {:t/sig '[:-> (s/tuple #{false})]} []
  (into [] (filter some?) [nil false]))
(defn no-input {:t/sig '[:-> #{:empty}]} []
  (when (empty? (into [] (map nil) nil)) :empty))
(defn constructor-before-source {:t/sig '[:-> #{:callback}]} []
  (try
    (into [] (map (throw (ex-info "callback" {:part :callback})))
             (throw (ex-info "source" {:part :source})))
    (catch clojure.lang.ExceptionInfo error (:part (ex-data error)))))
"#,
    );
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn into_checks_reached_callback_uses_and_leaves_custom_traversal_open() {
    let body = r#"(ns lens (:require [clojure.spec.alpha :as s]))
(defn range-for {:t/sig '[{:row int?} :-> int?]} [position] (:row position))
(defn direct {:t/sig '[:-> any?]} []
  (into [] (map range-for) [{:row "wrong"}]))
(defn structural {:t/sig '[(s/tuple {:row string?}) :-> any?]} [source]
  (into [] (map range-for) source))"#;
    let report = Session::default().check("lens.clj", body);
    let [error, gap] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(error.code, "PNX2322");
    assert_eq!(gap.code, "PNX9001");
    assert_eq!(gap.severity, "warning");
    let at = |diagnostic: &parenox::Diagnostic| {
        &body[diagnostic.span.start.offset..diagnostic.span.end.offset]
    };
    assert_eq!(at(error), "\"wrong\"");
    assert_eq!(at(gap), "source");
}

#[test]
fn into_keep_retains_false_and_narrows_optional_results() {
    let report = check(
        r#"
(defn kept {:t/sig '[:-> (s/tuple #{false} #{:reachable})]} []
  (into [] (keep :generator) [{:generator false} {} {:generator :reachable}]))
(defn optional {:t/sig '[boolean? :-> (s/coll-of #{false} :kind vector?)]} [include?]
  (into [] (keep #(when include? %)) [false]))
(defn keeps-reduced {:t/sig '[:-> #{true}]} []
  (reduced? (first (into [] (keep reduced) [:first]))))
(defn empty-source {:t/sig '[:-> #{:empty}]} []
  (when (empty? (into [] (keep (fn [item] (inc "unused"))) nil)) :empty))
"#,
    );
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn repeated_conj_and_peek_keep_the_actual_reduction_positions() {
    let report = check(
        r#"
(defn next-row
  {:t/sig '[:-> (s/tuple (s/and int? #{1}) (s/and int? #{1}) (s/and int? #{2}))]} []
  (let [previous [0 1 2]
        current \b]
    (reduce
      (fn [row [diagonal above other]]
        (let [update-val (if (= other current) diagonal (inc (min diagonal above (peek row))))]
          (conj row update-val)))
      [(inc (first previous))]
      [[0 1 \a] [1 2 \c]])))
(defn nested {:t/sig '[:-> (s/tuple #{"x"} #{"y"})]} []
  (mapv first (conj [["x"]] ["y"])))
(defn backed {:t/sig '[:-> #{"leaf"}]} []
  (try (peek (peek (conj [] ["leaf"])))
       (catch Throwable _ :unexpected)))
(defn empty-vector {:t/sig '[:-> nil?]} [] (peek (conj)))
(defn nil-stack {:t/sig '[:-> nil?]} [] (peek nil))
"#,
    );
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn a_structural_tuple_does_not_prove_peek_or_conj_behavior() {
    let report = check(
        r#"
(defn direct {:t/sig '[(s/tuple #{:first}) :-> any?]} [source]
  (peek source))
(defn appended {:t/sig '[(s/tuple #{:first}) :-> any?]} [source]
  (peek (conj source :last)))
"#,
    );
    assert!(!report.complete, "{report:#?}");
    let [direct, appended] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    for finding in [direct, appended] {
        assert_eq!(finding.code, "PNX9001");
        assert_eq!(finding.severity, "warning");
        assert!(finding.message.contains("peek"));
    }
}

#[test]
fn assoc_updates_ordinary_vector_positions_without_losing_nested_facts() {
    let report = check(
        r#"
(defn replace-child {:t/sig '[:-> (s/tuple #{:current} #{:next})]} []
  (let [keyset {:child {:order 1}}
        i (:order (keyset :child))
        children [[:current] [:old]]
        c [:next]]
    (mapv first (assoc children i c))))
(defn append-after-wrap {:t/sig '[:-> (s/tuple #{:first} #{:second})]} []
  (assoc [:old] 4294967296 :first 4294967297N :second))
(defn growing {:t/sig '[:-> (s/tuple #{:first} #{:second})]} []
  (assoc [] 0 :old 1 :second 0 :first))
(defn checks-all-operands-first {:t/sig '[:-> #{:operand}]} []
  (try (assoc [] -1 :bad 0 (throw (ex-info "operand" {:which :operand})))
       (catch clojure.lang.ExceptionInfo error (:which (ex-data error)))))
"#,
    );
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn vector_assoc_distinguishes_index_failure_and_runtime_key_class() {
    let source = r#"(ns stack (:require [clojure.spec.alpha :as s]))
(defn bounds {:t/sig '[:-> #{:bounds}]} []
  (try (assoc [:old] 2 :new)
       (catch IllegalArgumentException _ :wrong)
       (catch IndexOutOfBoundsException _ :bounds)))
(defn invalid-key {:t/sig '[:-> #{:invalid}]} []
  (try (assoc [:old] "0" :new)
       (catch IndexOutOfBoundsException _ :wrong)
       (catch IllegalArgumentException _ :invalid)))
(defn fixed-value {:t/sig '[(s/and int? #{0}) :-> #{:ok :invalid}]} [index]
  (let [outcome (try (do (assoc [:old] index :new) :ok)
                     (catch IllegalArgumentException _ :invalid))]
    outcome))"#;
    let mut session = Session::default();
    let report = session.check("stack.clj", source);
    assert!(report.complete, "{report:#?}");
    let [bounds, key] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    for (finding, (code, fragment)) in [bounds, key]
        .into_iter()
        .zip([("PNX2801", "2"), ("PNX2322", "\"0\"")])
    {
        assert_eq!(finding.code, code);
        assert_eq!(
            finding.contract.as_ref().unwrap().name,
            "clojure.core/assoc"
        );
        assert_eq!(
            &source[finding.span.start.offset..finding.span.end.offset],
            fragment
        );
    }
    let offset = source.rfind("outcome").unwrap();
    let Some(DocumentHover::Local(hover)) = session.document_hover_at("stack.clj", offset) else {
        panic!("missing assoc outcome hover");
    };
    assert_eq!(hover.inferred_type, ":invalid | :ok");
}

#[test]
fn two_argument_into_preserves_both_halves_of_a_vector_edit() {
    let report = check(
        r#"
(defn cut-middle {:t/sig '[:-> (s/tuple #{:first} #{:last})]} []
  (let [i 1]
    (letfn [(cut [v] (into (subvec v 0 i) (subvec v (inc i))))]
      (mapv first (cut [[:first] [:remove] [:last]])))))
(defn empty-source {:t/sig '[:-> #{"original"}]} []
  (peek (into ["original"] nil)))
(defn target-first {:t/sig '[:-> #{:target}]} []
  (try (into (throw (ex-info "target" {:part :target}))
             (throw (ex-info "source" {:part :source})))
       (catch clojure.lang.ExceptionInfo error (:part (ex-data error)))))
(defn source-before-append {:t/sig '[:-> #{:source}]} []
  (try (into [:target] (throw (ex-info "source" {:part :source})))
       (catch clojure.lang.ExceptionInfo error (:part (ex-data error)))))
"#,
    );
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn two_argument_into_requires_proof_for_both_vector_operations() {
    let source = r#"(ns stack (:require [clojure.spec.alpha :as s]))
(defn target {:t/sig '[(s/tuple #{:kept}) :-> any?]} [previous]
  (into previous [:new]))
(defn source {:t/sig '[(s/tuple #{:new}) :-> any?]} [items]
  (into [:kept] items))"#;
    let report = Session::default().check("stack.clj", source);
    assert!(!report.complete, "{report:#?}");
    let [target, input] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    for (finding, expected) in [target, input].into_iter().zip(["previous", "items"]) {
        assert_eq!(finding.code, "PNX9001");
        assert_eq!(finding.severity, "warning");
        assert_eq!(
            &source[finding.span.start.offset..finding.span.end.offset],
            expected
        );
    }
}

#[test]
fn unchecked_into_visits_withhold_only_callback_hovers() {
    let source = r#"(ns traversal (:require [clojure.spec.alpha :as s]))
(defn inline {:t/sig '[(s/tuple keyword?) :-> any?]} [custom]
  (mapv (fn [source] (into [] (map (fn [item] item)) source))
        [[:known] custom]))
(defn saved {:t/sig '[(s/tuple keyword?) :-> any?]} [custom]
  (let [mark :outside
        mapper (fn [item] item)]
    (mapv (fn [source] (into [] (map (fnil mapper :default)) source))
          [[:known] custom])
    mark))"#;
    let mut session = Session::default();
    let report = session.check("traversal.clj", source);
    assert!(!report.complete, "{report:#?}");
    for (offset, _) in source.match_indices("item)") {
        assert!(session.document_hover_at("traversal.clj", offset).is_none());
    }
    let offset = source.rfind("mark").unwrap();
    let Some(DocumentHover::Local(hover)) = session.document_hover_at("traversal.clj", offset)
    else {
        panic!("missing hover outside the callback");
    };
    assert_eq!(hover.inferred_type, ":outside");
}

#[test]
fn points_at_elements_and_callback_bodies_without_suggesting_collection_replacements() {
    let source = include_str!("../examples/vectors_bad.clj");
    let report = Session::default().check("vectors_bad.clj", source);
    assert!(!report.ok && report.complete, "{report:#?}");
    let [direct, inline] = &report.diagnostics[..] else {
        panic!("{report:#?}");
    };
    let fragment = |d: &parenox::Diagnostic| &source[d.span.start.offset..d.span.end.offset];
    assert_eq!(fragment(direct), r#""2""#);
    assert_eq!(direct.actual.as_deref(), Some("String"));
    let [suggestion] = direct.suggestions.as_slice() else {
        panic!("{direct:#?}");
    };
    assert_eq!(suggestion.span, direct.span);
    assert_eq!(suggestion.replacement, "2");
    assert_eq!(suggestion.applicability, "requires-review");
    assert_eq!(fragment(inline), "name");
    assert_eq!(inline.expected.as_deref(), Some("Non-nil value"));
    assert_eq!(inline.suggestions[0].replacement, r#"(or name "")"#);

    let report = check(
        r#"(defn indirect {:t/sig '[:-> any?]} []
             (let [values ["2"]] (mapv inc values)))"#,
    );
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert!(finding.suggestions.is_empty());
    assert!(finding.help.as_deref().unwrap().contains("elements"));
}

#[test]
fn empty_inputs_skip_invocation_but_still_check_callback_expressions() {
    for (expr, code) in [
        (r#"(mapv (fnil inc (inc "bad")) [])"#, "PNX2322"),
        ("(mapv (fn [x y] missing) [])", "PNX2304"),
        ("(mapv (do 0 (let [] (fn [x] missing))) [])", "PNX2304"),
        ("(mapv inc [1] [2])", "PNX2554"),
        ("(mapv inc 42)", "PNX2322"),
        ("(mapv inc {:x 1})", "PNX2322"),
        (r#"(mapv inc (seq [1 "wrong"]))"#, "PNX2322"),
        ("(filterv number? [1] [2])", "PNX2554"),
    ] {
        let report = check(&format!(
            "(defn f {{:t/sig '[:-> (s/coll-of number? :kind vector?)]}} [] {expr})"
        ));
        assert_eq!(report.diagnostics.len(), 1, "{expr}: {report:#?}");
        assert_eq!(report.diagnostics[0].code, code, "{expr}: {report:#?}");
    }
}

#[test]
fn some_predicate_preserves_false_and_respects_shadowing() {
    for body in [
        r#"(defn f {:t/sig '[:-> (s/coll-of string? :kind vector?)]}
             [] (filterv some? [nil false "Ada"]))"#,
        r#"(defn- some? [value] true)
           (defn f {:t/sig '[:-> (s/coll-of string? :kind vector?)]}
             [] (filterv some? [nil "Ada"]))"#,
    ] {
        let report = check(body);
        assert!(report.complete && !report.ok, "{report:#?}");
        assert_eq!(report.diagnostics.len(), 1, "{report:#?}");
        assert_eq!(report.diagnostics[0].code, "PNX2322", "{report:#?}");
    }
}

#[test]
fn closed_map_entries_preserve_key_value_and_absence_semantics() {
    let report = Session::default().check("entries.clj", include_str!("../examples/entries.clj"));
    assert!(report.ok && report.complete, "{report:#?}");

    for (args, params, expr, element) in [
        (
            "boolean?",
            "[include?]",
            r#"(mapv val (if include? {:x "Ada"} {}))"#,
            "string?",
        ),
        ("", "[]", "(mapv val {:x nil})", "nil?"),
        ("", "[]", "(mapv inc {})", "string?"),
        (
            "",
            "[]",
            "(mapv (fn [[_ _ missing]] missing) {:x 12})",
            "nil?",
        ),
        (
            "",
            "[]",
            "(mapv (fn [[_ _ :as entry]] (count entry)) {:x 12})",
            "number?",
        ),
        (
            "",
            "[]",
            "(mapv (fn [[_ {:keys [n]}]] (inc n)) {:x {:n 12}})",
            "number?",
        ),
        (
            "",
            "[]",
            "(mapv identity {:x 12})",
            "(s/coll-of (s/or :k keyword? :v number?) :kind vector?)",
        ),
    ] {
        let report = check(&format!(
            "(defn f {{:t/sig '[{args} :-> (s/coll-of {element} :kind vector?)]}} {params} {expr})"
        ));
        assert!(report.ok && report.complete, "{expr}: {report:#?}");
    }
}

#[test]
fn map_entry_diagnostics_distinguish_vectors_and_open_maps() {
    let source = include_str!("../examples/entries_bad.clj");
    let report = Session::default().check("entries_bad.clj", source);
    assert!(report.complete && !report.ok, "{report:#?}");
    let [element, vector] = &report.diagnostics[..] else {
        panic!("{report:#?}");
    };
    assert_eq!(element.code, "PNX2322");
    assert_eq!(
        &source[element.span.start.offset..element.span.end.offset],
        "total"
    );
    assert_eq!(element.expected.as_deref(), Some("Number"));
    assert_eq!(element.actual.as_deref(), Some("String"));
    assert!(element.suggestions.is_empty(), "{element:#?}");
    assert_eq!(vector.code, "PNX2322");
    assert_eq!(vector.expected.as_deref(), Some("MapEntry"));
    assert_eq!(vector.actual.as_deref(), Some("Vector<Number | :admitted>"));

    let report = check(
        "(s/def :sample/n number?)
         (defn f {:t/sig '[(s/keys :req-un [:sample/n]) :-> (s/coll-of number? :kind vector?)]}
           [counts] (mapv val counts))",
    );
    assert!(!report.complete && !report.ok, "{report:#?}");
    assert_eq!(report.diagnostics.len(), 1, "{report:#?}");
    assert_eq!(report.diagnostics[0].code, "PNX9001");
    assert!(
        report.diagnostics[0].message.contains("open map"),
        "{report:#?}"
    );
}

#[test]
fn vector_calls_evaluate_the_callback_expression_then_their_collection_arguments() {
    for (label, expression) in [
        (
            "callback",
            r#"(mapv (fnil identity (throw (ex-info "callback" {:stage :callback})))
                     (throw (ex-info "collection" {:stage :collection})))"#,
        ),
        (
            "callback",
            r#"(mapv (let [prefix (throw (ex-info "callback" {:stage :callback}))]
                       (fn [message] (str prefix message)))
                     (throw (ex-info "collection" {:stage :collection})))"#,
        ),
        (
            "first",
            r#"(mapv + (throw (ex-info "first" {:stage :first}))
                       (throw (ex-info "second" {:stage :second})))"#,
        ),
        (
            "collection",
            r#"(filterv #(some? %) (throw (ex-info "collection" {:stage :collection})))"#,
        ),
    ] {
        let report = check(&format!(
            "(defn f {{:t/sig '[:-> #{{:{label}}}]}} []
               (try {expression}
                 (catch clojure.lang.ExceptionInfo e (:stage (ex-data e)))))"
        ));
        assert!(report.ok && report.complete, "{expression}: {report:#?}");
    }

    let report = check(
        "(defn f {:t/sig '[:-> nil?]} []
           (mapv (fn [x] (dummy x)) (throw (ex-info \"stop\" {}))))",
    );
    assert_eq!(report.diagnostics.len(), 1, "{report:#?}");
    assert_eq!(report.diagnostics[0].code, "PNX2304");
}

#[test]
fn guaranteed_callbacks_can_terminate_vector_pipelines() {
    for operation in ["mapv", "filterv"] {
        let body = format!(
            r#"(ns demo)
              (defn f {{:t/sig '[:-> #{{:row}}]}} []
                (try
                  ({operation} (fn [_] (throw (ex-info "failed" {{:stage :row}}))) [:only])
                  (catch ExceptionInfo e (:stage (ex-data e)))))"#
        );
        let report = Session::default().check("vectors.clj", &body);
        assert!(report.ok && report.complete, "{operation}: {report:#?}");
    }
}

#[test]
fn empty_collections_still_return_and_open_collections_can_be_empty() {
    let source = r#"(ns demo (:require [clojure.spec.alpha :as s]))
      (defn available {:t/sig '[(s/coll-of number? :kind vector?) :-> (s/coll-of number? :kind vector?)]}
        [xs] (filterv (fn [_] (throw (ex-info "rejected" {}))) xs))
      (defn no-exception {:t/sig '[:-> (s/coll-of number? :kind vector?)]} []
        (try (mapv identity [1 2])
             (catch ExceptionInfo _ :unexpected)))
      (defn shortest-empty {:t/sig '[:-> (s/coll-of number? :kind vector?)]} []
        (try (mapv (fn [_ _] (throw (ex-info "unused" {}))) [1 2] [])
             (catch ExceptionInfo _ :unexpected)))"#;
    let report = Session::default().check("vectors.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");
}
