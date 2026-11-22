use parenox::Session;

#[test]
fn maps_and_keywords_are_functions_in_direct_calls_and_callbacks() {
    let report =
        Session::default().check("callables.clj", include_str!("../examples/callables.clj"));
    assert!(report.ok && report.complete, "{report:#?}");

    for (body, ret) in [
        ("(:label #{:other})", "nil?"),
        ("(:label :not-a-map \"safe\")", "string?"),
        ("({:label false} :label true)", "#{false}"),
        ("(apply :label [{:label \"ready\"}])", "string?"),
        ("(apply {:label \"ready\"} [:label])", "string?"),
        (
            "(let [f (if flag {:label \"ready\"} {})] (f :label \"fallback\"))",
            "string?",
        ),
    ] {
        let source = format!("(ns demo) (defn f {{:t/sig '[boolean? :-> {ret}]}} [flag] {body})");
        let report = Session::default().check("callables.clj", &source);
        assert!(report.ok && report.complete, "{body}: {report:#?}");
    }
}

#[test]
fn symbols_invoke_lookup_with_their_own_key_and_default() {
    for (body, expected) in [
        (
            r#"(let [lookup 'account/id]
                 (lookup {'account/id "label"} "missing"))"#,
            r#"#{"label"}"#,
        ),
        ("('local {'local nil} :missing)", "nil?"),
        ("('account/id {} :missing)", "#{:missing}"),
        ("(apply 'account/id [{'account/id :ready}])", "#{:ready}"),
        (
            "(first (mapv 'account/id [{'account/id :ready}]))",
            "#{:ready}",
        ),
        ("(first (mapv 'field [{}] [:missing]))", "#{:missing}"),
        ("(key {})", "nil?"),
    ] {
        let source = format!("(defn lookup {{:t/sig '[symbol? :-> {expected}]}} [key] {body})");
        let report = Session::default().check("callables.clj", &source);
        assert!(report.ok && report.complete, "{body}: {report:#?}");
    }
}

#[test]
fn a_symbol_call_rejects_wrong_arity_at_the_invocation() {
    for invocation in ["('field)", "('field {} :missing :extra)"] {
        let source = format!(
            r#"(defn lookup {{:t/sig '[:-> #{{:arity}}]}} []
                 (try {invocation}
                   (catch ClassCastException _ (inc "unreachable"))
                   (catch clojure.lang.ArityException _ :arity)))"#
        );
        let report = Session::default().check("callables.clj", &source);
        let [finding] = report.diagnostics.as_slice() else {
            panic!("{invocation}: {report:#?}");
        };
        assert!(report.complete, "{report:#?}");
        assert_eq!(finding.code, "PNX2554");
        assert_eq!(
            &source[finding.span.start.offset..finding.span.end.offset],
            invocation
        );
    }
}

#[test]
fn set_lookup_returns_the_stored_value_and_respects_custom_comparators() {
    for (body, contract) in [
        ("(#{:present} :missing)", "nil?"),
        ("(let [items #{:present}] (items :missing))", "nil?"),
        ("(apply #{:present} [:missing])", "nil?"),
        ("(get #{:present} :missing :fallback)", "#{:fallback}"),
        ("(int? (#{1N} 1))", "#{false}"),
        ("(int? (get #{1N} 1))", "#{false}"),
        ("(contains? #{:present} :missing)", "#{false}"),
        ("(get #{nil} nil :fallback)", "nil?"),
        ("(get #{false} false :fallback)", "#{false}"),
        ("(get #{:a :b} :b :fallback)", "#{:b}"),
        ("(contains? #{nil false} nil)", "#{true}"),
        ("(int? (#{1} 1N))", "#{true}"),
        ("(int? (get (conj #{1} 1N) 1N))", "#{true}"),
        ("(int? (get (conj #{1N} 1) 1))", "#{false}"),
        ("(int? (get (conj #{} 1N 1) 1))", "#{false}"),
        ("(get (conj #{:other} nil) nil :miss)", "nil?"),
        ("(get (conj #{:other} false) false :miss)", "#{false}"),
        ("(contains? (conj #{:initial} :added) :added)", "#{true}"),
        ("(contains? (conj #{:initial} :added) :missing)", "#{false}"),
        (
            "(try (req! #{nil false} nil) (catch IllegalArgumentException _ :wrong))",
            "nil?",
        ),
    ] {
        let source = format!("(defn lookup {{:t/sig '[:-> {contract}]}} [] {body})");
        let report = Session::default().check("callables.clj", &source);
        assert!(report.ok && report.complete, "{body}: {report:#?}");
    }

    let source = r#"(ns demo (:require [clojure.spec.alpha :as s]))
      (defn lookup {:t/sig '[(s/coll-of #{"UPPER"} :kind set?) :-> (s/nilable #{"UPPER"})]}
        [items] (items "upper"))"#;
    let report = Session::default().check("callables.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");

    let comparison = r#"(ns demo (:require [clojure.spec.alpha :as s]))
      (defn lookup {:t/sig '[(s/coll-of #{"UPPER"} :kind set?) :-> #{false}]}
        [items] (contains? items "upper"))"#;
    let report = Session::default().check("callables.clj", comparison);
    let [error] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(error.code, "PNX2322");
    assert_eq!(error.actual.as_deref(), Some("Boolean"));
}

#[test]
fn conjoining_an_unknown_member_does_not_preserve_proof_of_absence() {
    let source = r#"
(defn lookup {:t/sig '[string? :-> #{false}]} [item]
  (contains? (conj #{"old"} item) "new"))
"#;
    let report = Session::default().check("callables.clj", source);
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert!(!report.ok && report.complete);
    assert_eq!(finding.code, "PNX2322");
    assert_eq!(finding.actual.as_deref(), Some("Boolean"));
}

#[test]
fn exact_set_observations_respect_emptiness_count_and_unordered_members() {
    let source = r#"
(ns reader.exact-set-observations)
(defn first-member {:t/sig '[:-> #{:head}]} [] (first (set [:head :head])))
(defn exact-count {:t/sig '[:-> #{true}]} [] (= 1 (count (set [:head :head]))))
(defn nonempty {:t/sig '[:-> #{false}]} [] (empty? (conj #{} :head)))
(defn retains-set {:t/sig '[:-> #{true}]} [] (contains? (not-empty (set [:head])) :head))
(defn now-empty {:t/sig '[:-> nil?]} [] (not-empty (disj #{1} 1N)))
(defn nonempty-seq {:t/sig '[:-> #{:present}]} []
  (if (seq (set [:head])) :present :empty))
(defn last-next {:t/sig '[:-> nil?]} []
  (next (set [:head :head])))
(defn empty-rest {:t/sig '[:-> #{false}]} []
  (contains? (set (rest (set [:head]))) :head))
"#;
    let report = Session::default().check("set-observations.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");

    let uncertain = r#"
(defn maybe {:t/sig '[keyword? :-> #{:head}]} [key]
  (first (disj #{:head} key)))
"#;
    let report = Session::default().check("set-observations.clj", uncertain);
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert!(!report.ok && report.complete);
    assert_eq!(finding.code, "PNX2322");
    assert_eq!(finding.actual.as_deref(), Some("Nil | :head"));
}

#[test]
fn set_builds_membership_proofs_from_ordinarily_constructed_values() {
    let source = r#"
(ns reader.set-construction (:require [clojure.spec.alpha :as s]))
(defn distinct-values {:t/sig '[:-> (s/tuple #{true} #{false} nil? #{false})]} []
  (let [items (set [nil :item false :item])]
    [(contains? items :item) (contains? items :missing)
     (get items nil :missing) (get items false :missing)]))
(defn list-and-empty {:t/sig '[:-> (s/tuple #{true} #{false})]} []
  [(contains? (set '(:x :x :y)) :y) (contains? (set nil) :x)])
(defn existing {:t/sig '[:-> #{false}]} []
  (int? (get (set #{1N}) 1)))
(def members #{:known})
(defn source-set {:t/sig '[:-> #{false}]} []
  (contains? (set members) :missing))
(defn derived-list {:t/sig '[:-> #{true}]} []
  (contains? (set (rest '(:removed :tail))) :tail))
"#;
    let report = Session::default().check("set-construction.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn set_does_not_confuse_equivalent_members_with_different_stored_objects() {
    let source = r#"
(ns demo.set-storage (:require [clojure.spec.alpha :as s]))
(defn mixed {:t/sig '[:-> any?]} [] (set [1 1N]))
(defn opaque {:t/sig '[(s/coll-of keyword? :kind set?) :-> any?]} [items]
  (set items))
"#;
    let report = Session::default().check("set-storage.clj", source);
    assert!(!report.ok && !report.complete);
    let findings: Vec<_> = report
        .diagnostics
        .iter()
        .map(|finding| {
            (
                finding.code.as_str(),
                &source[finding.span.start.offset..finding.span.end.offset],
            )
        })
        .collect();
    assert_eq!(findings, [("PNX9001", "[1 1N]"), ("PNX9001", "items")]);
}

#[test]
fn disj_removes_equivalent_members_and_keeps_falsey_and_identity_rules() {
    let source = r#"
(ns reader.set-removal (:require [clojure.spec.alpha :as s]))
(defn removes-alias {:t/sig '[:-> (s/tuple #{false} #{:keep} #{false} nil?)]} []
  (let [items (disj #{1 :keep false nil} 1N nil)]
    [(contains? items 1) (items :keep) (get items false :wrong) (get items nil)]))
(defn identity-and-nil {:t/sig '[:-> (s/tuple #{:same} nil? nil? #{:keep})]} []
  [(disj :same) (disj nil :a) (disj false :a) (get (apply disj [#{1 :keep} 1N]) :keep)])
(defn uncertain {:t/sig '[string? :-> boolean?]} [input]
  (contains? (disj #{"old" "new"} input) "new"))
"#;
    let report = Session::default().check("set-removal.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");

    let uncertain = source.replace("'[string? :-> boolean?]", "'[string? :-> #{true}]");
    let report = Session::default().check("set-removal.clj", &uncertain);
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert!(report.complete && !report.ok);
    assert_eq!(finding.code, "PNX2322");
    assert_eq!(finding.actual.as_deref(), Some("Boolean"));
}

#[test]
fn an_open_apply_spread_does_not_prove_set_removal() {
    let source = r#"
(ns demo.set-apply (:require [clojure.spec.alpha :as s]))
(defn open {:t/sig '[(s/coll-of #{:item} :kind vector?) :-> boolean?]} [keys]
  (contains? (apply disj #{:item} keys) :item))
"#;
    let report = Session::default().check("set-spread.clj", source);
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert!(!report.ok && !report.complete);
    assert_eq!(finding.code, "PNX9001");
    assert_eq!(
        &source[finding.span.start.offset..finding.span.end.offset],
        "keys"
    );
}

#[test]
fn disj_reports_a_proven_non_set_at_the_target_operand() {
    let source = r#"
(defn caught {:t/sig '[:-> #{:class-cast}]} []
  (try (disj :wrong :remove)
       (catch ClassCastException _ :class-cast)))
"#;
    let report = Session::default().check("invalid-set.clj", source);
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert!(report.complete && !report.ok);
    assert_eq!(finding.code, "PNX2322");
    assert_eq!(
        &source[finding.span.start.offset..finding.span.end.offset],
        ":wrong"
    );
}

#[test]
fn finite_set_queries_check_all_key_alternatives() {
    let source = r#"
(defn present {:t/sig '[#{:a :b} :-> #{true}]} [key]
  (contains? #{:a :b} key))
(defn absent {:t/sig '[#{:c :d} :-> #{false}]} [key]
  (contains? #{:a :b} key))
(defn invoke {:t/sig '[#{:a :b} :-> #{:a :b}]} [key]
  (#{:a :b} key))
(defn required {:t/sig '[#{:a :b} :-> #{:a :b}]} [key]
  (try (req! #{:a :b} key)
       (catch IllegalArgumentException _ :wrong)))
(defn mixed {:t/sig '[#{:a :c} :-> #{:a :missing}]} [key]
  (try (req! #{:a :b} key)
       (catch IllegalArgumentException _ :missing)))
"#;
    let report = Session::default().check("callables.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");

    let source = "(defn f {:t/sig '[#{:a :c} :-> #{false}]} [key] (contains? #{:a :b} key))";
    let report = Session::default().check("callables.clj", source);
    let [error] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(error.code, "PNX2322");
    assert_eq!(error.actual.as_deref(), Some("Boolean"));
}

#[test]
fn callable_collections_report_arity_and_keep_undeclared_field_diagnostics() {
    let report = Session::default().check(
        "callables_bad.clj",
        include_str!("../examples/callables_bad.clj"),
    );
    let [map, keyword, bounds] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(map.code, "PNX2554");
    assert!(map.message.contains("Map invocation"));
    assert_eq!(keyword.code, "PNX2554");
    assert!(keyword.message.contains("Keyword invocation"));
    assert_eq!(bounds.code, "PNX2801");

    let source = "(ns demo (:require [clojure.spec.alpha :as s]))
       (defn labels {:t/sig '[(s/keys) :-> string?]} [shape]
         ((identity :label) shape))";
    let report = Session::default().check("callables.clj", source);
    assert!(!report.complete, "{report:#?}");
    assert_eq!(report.diagnostics[0].code, "PNX9003");
}

#[test]
fn vector_invocation_uses_jvm_integer_keys_and_strict_bounds() {
    for (expression, contract) in [
        (r#"([1 "chosen"] 1)"#, "string?"),
        (r#"(let [index 1] ([1 "chosen"] index))"#, "string?"),
        (r#"(apply [1 "chosen"] [1])"#, "string?"),
        (r#"([1 "chosen"] 4294967297)"#, "string?"),
        (r#"([1 "chosen"] 01)"#, "string?"),
        (
            r#"(mapv (fn [entry] (entry 1)) {:a "value"})"#,
            "(clojure.spec.alpha/coll-of string? :kind vector?)",
        ),
    ] {
        let source = format!("(ns demo) (defn run {{:t/sig '[:-> {contract}]}} [] {expression})");
        let report = Session::default().check("index.clj", &source);
        assert!(report.ok && report.complete, "{expression}: {report:#?}");
    }
    for (expression, code) in [
        ("([1] 0 2)", "PNX2554"),
        ("([1] 1)", "PNX2801"),
        ("([1] -1)", "PNX2801"),
        ("([1] 0.0)", "PNX2322"),
        ("([1] :missing)", "PNX2322"),
    ] {
        let source = format!("(ns demo) (defn run {{:t/sig '[:-> number?]}} [] {expression})");
        let report = Session::default().check("index.clj", &source);
        let [diagnostic] = report.diagnostics.as_slice() else {
            panic!("{expression}: {report:#?}");
        };
        assert_eq!(diagnostic.code, code);
        if code == "PNX2801" {
            assert!(
                !diagnostic
                    .help
                    .as_deref()
                    .unwrap()
                    .contains("third argument")
            );
        }
    }
    let report = Session::default().check(
        "index.clj",
        "(ns demo) (defn run {:t/sig '[number? :-> number?]} [i] ([1] i))",
    );
    assert!(!report.complete, "{report:#?}");
    assert_eq!(report.diagnostics[0].code, "PNX9001");
}
