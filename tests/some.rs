use parenox::{DocumentHover, Session};

#[test]
fn some_distinguishes_an_inequality_predicate_from_the_matching_value() {
    let source = r#"(ns demo (:require [clojure.spec.alpha :as s]))
        (defn finds-flag {:t/sig '[:-> #{true}]} []
          (some #(not= :allowed (:type %)) [{:type :allowed} {:type :blocked}]))
        (defn no-match {:t/sig '[:-> nil?]} []
          (some #(not= :allowed (:type %)) [{:type :allowed}]))
        (defn finds-type
          {:t/sig '[(s/coll-of {:type #{:allowed :blocked}} :kind vector?)
                     :-> (s/nilable #{:blocked})]}
          [layers]
          (some (fn [layer] (when (not= :allowed (:type layer)) (:type layer))) layers))
        (defn wrong-type
          {:t/sig '[(s/coll-of {:type keyword?} :kind vector?) :-> keyword?]}
          [layers]
          (let [unsupported (some #(not= :allowed (:type %)) layers)]
            (:type unsupported)))"#;
    let report = Session::default().check("search.clj", source);
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert!(report.complete, "{report:#?}");
    assert_eq!(finding.code, "PNX2322");
    assert_eq!(finding.actual.as_deref(), Some("Nil"));
    assert_eq!(finding.contract.as_ref().unwrap().name, "demo/wrong-type");
}

#[test]
fn inequality_preserves_variadic_and_unary_behavior() {
    let source = r#"
        (defn unary {:t/sig '[:-> #{false}]} [] (not= :value))
        (defn same {:t/sig '[:-> #{false}]} [] (apply not= [1 1N 1]))
        (defn different {:t/sig '[:-> #{true}]} [] (apply not= [1 1 2]))
        (defn zero-args {:t/sig '[:-> any?]} [] (not=))"#;
    let report = Session::default().check("search.clj", source);
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert!(report.complete, "{report:#?}");
    assert_eq!(finding.code, "PNX2554");
    assert_eq!(
        &source[finding.span.start.offset..finding.span.end.offset],
        "(not=)"
    );
}

#[test]
fn some_returns_the_first_truthy_value_without_visiting_later_items() {
    let source = r#"(ns demo)
        (defn finds-empty-string {:t/sig '[:-> #{""}]} []
          (some identity [nil false "" :later]))
        (defn stops-before-a-mismatch {:t/sig '[:-> string?]} []
          (some (fn [x] (subs x 0)) ["first" 42]))
        (defn no-match {:t/sig '[:-> nil?]} []
          (some identity '(nil false)))
        (defn no-invocation {:t/sig '[:-> nil?]} []
          (some nil []))
        (defn returns-reduced-itself {:t/sig '[:-> #{:kept}]} []
          (if (reduced? (some (fn [x] (reduced x)) [false])) :kept :lost))"#;
    let report = Session::default().check("search.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn searches_preserve_literal_vectors_in_map_entries() {
    for entry_count in [1, 36] {
        let entries = (0..entry_count)
            .map(|i| format!(":bucket{i} [1 2]"))
            .collect::<Vec<_>>()
            .join(" ");
        let source = format!(
            "(defn first-total {{:t/sig '[:-> number?]}} []
               (some (fn [[_ bucket]] (reduce (fn [a b] (+ a b)) bucket))
                     {{{entries}}}))
             (defn all-totals {{:t/sig '[:-> #{{true}}]}} []
               (every? (fn [[_ bucket]] (= 3 (reduce (fn [a b] (+ a b)) bucket)))
                       {{{entries}}}))"
        );
        let report = Session::default().check("search.clj", &source);
        assert!(
            report.ok && report.complete,
            "{entry_count} entries: {report:#?}"
        );
    }
}

#[test]
fn some_checks_a_selected_callback_with_its_elements_and_captures() {
    let source = r#"(ns demo (:require [clojure.spec.alpha :as s]))
        (defn find-text
          {:t/sig '[(s/nilable string?) (s/coll-of string? :kind vector?) :-> (s/nilable string?)]}
          [prefix names]
          (some (if (some? prefix)
                  (fn [name] (subs name (count prefix)))
                  (fn [name] nil))
                names))
        (defn first-text
          {:t/sig '[(s/coll-of string? :kind vector?) :-> (s/nilable string?)]}
          [names]
          (let [saved (fn [name] (subs name 0))]
            (some saved names)))"#;
    let mut session = Session::default();
    let report = session.check("search.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");
    for marker in ["name (count prefix)", "prefix)))", "name 0"] {
        let offset = source.find(marker).unwrap();
        let Some(DocumentHover::Local(hover)) = session.document_hover_at("search.clj", offset)
        else {
            panic!("missing hover at {marker}");
        };
        assert_eq!(hover.inferred_type, "String");
    }
}

#[test]
fn some_reports_the_bad_element_of_a_literal_at_its_source() {
    let source = r#"(ns demo)
        (defn text {:t/sig '[string? :-> string?]} [word] word)
        (defn bad {:t/sig '[:-> string?]} []
          (some text [42 "later"]))"#;
    let report = Session::default().check("search.clj", source);
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(finding.code, "PNX2322");
    let range = finding.span.start.offset..finding.span.end.offset;
    assert_eq!(&source[range], "42");
    assert_eq!(finding.contract.as_ref().unwrap().name, "demo/text");
}

#[test]
fn some_checks_compiler_errors_in_uninvoked_callbacks() {
    let source = r#"(ns demo)
        (defn f {:t/sig '[:-> nil?]} []
          (some (if false
                  (fn [x] missing)
                  (fn [x] (inc "uninvoked")))
                []))"#;
    let report = Session::default().check("search.clj", source);
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(finding.code, "PNX2304");
    let range = finding.span.start.offset..finding.span.end.offset;
    assert_eq!(&source[range], "missing");
}

#[test]
fn some_evaluates_selection_then_collection_then_callback() {
    let source = r#"(ns demo)
        (defn test-before-source {:t/sig '[:-> #{:condition}]} []
          (try
            (some (if (throw (ex-info "condition" {:reason :condition}))
                    (fn [x] x)
                    (fn [x] (inc x)))
                  (throw (ex-info "source" {:reason :source})))
            (catch clojure.lang.ExceptionInfo e (:reason (ex-data e)))))
        (defn source-before-callback {:t/sig '[:-> #{:source}]} []
          (try
            (some (fn [x] (throw (ex-info "callback" {:reason :callback})))
                  (throw (ex-info "source" {:reason :source})))
            (catch clojure.lang.ExceptionInfo e (:reason (ex-data e)))))
        (defn ignore-late-throw {:t/sig '[:-> #{:found}]} []
          (try
            (some (fn [x] (if x :found (throw (ex-info "late" {})))) [true false])
            (catch clojure.lang.ExceptionInfo e :late)))"#;
    let report = Session::default().check("search.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn incomplete_search_keeps_compiler_errors_and_withholds_partial_hover() {
    let block = |count| format!("(some (fn [_] nil) [{}])", "false ".repeat(count));
    let source = format!(
        "(ns demo)
         (defn f {{:t/sig '[:-> any?]}} []
           {}{}
           (some (fn [item] (when (number? item) item)) [\"prefix\" 42])
           (some (fn [item] unbound) [42]))",
        block(32).repeat(3),
        block(31),
    );
    let mut session = Session::default();
    let report = session.check("search.clj", &source);
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.code == "PNX9001" && d.message.contains("step analysis limit")),
        "{report:#?}"
    );
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.code == "PNX2304" && d.message.ends_with("unbound")),
        "{report:#?}"
    );
    let offset = source.find("item) item").unwrap();
    assert!(session.document_hover_at("search.clj", offset).is_none());
}

#[test]
fn unreachable_search_does_not_consume_live_callback_visits() {
    let block = format!("(some (fn [_] nil) [{}])", "false ".repeat(32));
    let source = format!(
        "(ns demo)
         (defn f {{:t/sig '[:-> string?]}} []
           (when false {})
           (some (fn [item] (subs item 0)) [\"reachable\"]))",
        block.repeat(5),
    );
    let mut session = Session::default();
    let report = session.check("search.clj", &source);
    assert!(report.ok && report.complete, "{report:#?}");
    let offset = source.find("item 0").unwrap();
    let Some(DocumentHover::Local(hover)) = session.document_hover_at("search.clj", offset) else {
        panic!("missing hover after unreachable searches");
    };
    assert_eq!(hover.inferred_type, "\"reachable\"");
}

#[test]
fn some_with_an_unchecked_source_does_not_report_only_the_known_items() {
    let source = r#"(ns demo)
        (defn f [choose opaque]
          (let [predicate (fn [item] (subs item 0))]
            (some predicate (if choose ["known"] opaque))))"#;
    let mut session = Session::default();
    let report = session.check("search.clj", source);
    assert!(!report.complete, "{report:#?}");
    let offset = source.find("item 0").unwrap();
    assert!(session.document_hover_at("search.clj", offset).is_none());
}
