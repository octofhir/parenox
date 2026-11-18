use parenox::Session;

#[test]
fn found_entries_keep_nil_and_false_through_presence_guards() {
    let entries = include_str!("../examples/entries.clj");
    let report = Session::default().check("entries.clj", entries);
    assert!(report.ok && report.complete, "{report:#?}");

    let source = r#"
        (defn nil-entry {:t/sig '[:-> nil?]} []
          (if-let [[found-key value :as entry] (find {:name nil} :name)]
            (if (= found-key (key entry)) value "different key")
            "missing"))
        (defn false-entry {:t/sig '[:-> #{false}]} []
          (val (apply find [{:enabled false} :enabled])))
        (defn missing-entry {:t/sig '[map? :-> nil?]} [row]
          (find (dissoc row :hidden) :hidden))
        (defn nil-target {:t/sig '[:-> nil?]} []
          (find nil 0.5))
    "#;
    let report = Session::default().check("presence.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn finite_key_alternatives_keep_key_and_value_types_paired() {
    let source = r#"
        (ns demo (:require [clojure.spec.alpha :as s]))
        (defn choose {:t/sig '[#{:count :name}
                              :-> (s/or :count (s/tuple #{:count} number?)
                                        :name (s/tuple #{:name} string?))]}
          [selector]
          (find {:count 7 :name "Ada"} selector))
        (defn label {:t/sig '[(s/map-of string? (s/nilable string?)) string? :-> (s/nilable string?)]}
          [labels selector]
          (if-let [entry (find labels selector)] (val entry) "missing"))
    "#;
    let report = Session::default().check("choices.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");

    let edited = source.replace(
        r#"(if-let [entry (find labels selector)] (val entry) "missing")"#,
        "(val (find labels selector))",
    );
    let report = Session::default().check("choices.clj", &edited);
    let [error] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert!(!report.ok && report.complete, "{report:#?}");
    assert_eq!(error.code, "PNX2322");
    assert_eq!(error.expected.as_deref(), Some("MapEntry"));
}

#[test]
fn indexed_inputs_return_entries_with_the_requested_index() {
    let source = r#"
        (ns demo (:require [clojure.spec.alpha :as s]))
        (defn fixed {:t/sig '[:-> number?]} [] (key (find [nil false] 1)))
        (defn nested {:t/sig '[:-> #{false}]} []
          (val (find (find {:status false} :status) 1)))
        (defn absent {:t/sig '[:-> nil?]} []
          (or (find [nil false] -1) (find [nil false] 2) (find [nil false] :unknown)))
        (defn typed {:t/sig '[(s/coll-of string? :kind vector?) :-> (s/nilable string?)]}
          [items]
          (when-let [entry (find items 1)] (val entry)))
    "#;
    let report = Session::default().check("indexed.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn diagnostics_distinguish_invalid_targets_and_unmodeled_keys() {
    let source = r#"
        (defn invalid {:t/sig '[:-> nil?]} [] (find #{:name} :name))
        (defn wide-index {:t/sig '[:-> nil?]} [] (find [:name] 4294967296))
        (defn undeclared {:t/sig '[map? :-> nil?]} [row] (find row :name))
    "#;
    let report = Session::default().check("diagnostics.clj", source);
    let [target, index, undeclared] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert!(!report.ok && !report.complete, "{report:#?}");
    assert_eq!(target.contract.as_ref().unwrap().name, "clojure.core/find");
    for (diagnostic, code, syntax) in [
        (target, "PNX2322", "#{:name}"),
        (index, "PNX9001", "4294967296"),
        (undeclared, "PNX9003", ":name"),
    ] {
        assert_eq!(diagnostic.code, code);
        let span = diagnostic.span;
        assert_eq!(&source[span.start.offset..span.end.offset], syntax);
    }
}

#[test]
fn both_arguments_are_evaluated_even_when_the_target_is_nil() {
    let source = r#"
        (defn f {:t/sig '[:-> #{:key}]} []
          (try
            (find nil (throw (ex-info "key failed" {:stage :key})))
            (catch clojure.lang.ExceptionInfo e (:stage (ex-data e)))))
    "#;
    let report = Session::default().check("order.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");
}
