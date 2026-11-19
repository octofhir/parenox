use parenox::{Report, Session, Source};

fn check(expression: &str, expected: &str) -> Report {
    Session::default().check(
        "list.clj",
        &format!("(ns demo) (defn f {{:t/sig '[:-> {expected}]}} [] {expression})"),
    )
}

#[test]
fn lists_keep_order_and_accumulate_across_recur_states() {
    let good = include_str!("../examples/list_accumulators.clj");
    let report = Session::default().check("list_accumulators.clj", good);
    assert!(report.ok && report.complete, "{report:#?}");

    let bad = include_str!("../examples/list_accumulators_bad.clj");
    let report = Session::default().check("list_accumulators_bad.clj", bad);
    let [diagnostic] = &report.diagnostics[..] else {
        panic!("{report:#?}");
    };
    assert_eq!(diagnostic.code, "PNX2322");
    assert_eq!(
        &bad[diagnostic.span.start.offset..diagnostic.span.end.offset],
        "42"
    );
}

#[test]
fn list_builders_preserve_nil_elements_and_take_seqable_tails() {
    for (expression, result) in [
        ("(if (list) :truthy :unexpected)", "#{:truthy}"),
        ("(first (cons nil '(:next)))", "nil?"),
        ("(first (next (cons :head nil)))", "nil?"),
        (
            "(if (contains? (first (next (cons :head {:key :value}))) 1) :yes :no)",
            "#{:yes}",
        ),
        ("(first (next (apply cons [:x [:y]])))", "#{:y}"),
        (
            r#"(try (list (throw (ex-info "first" {:stage :first})) (throw (ex-info "second" {:stage :second}))) (catch ExceptionInfo e (:stage (ex-data e))))"#,
            "#{:first}",
        ),
    ] {
        let report = check(expression, result);
        assert!(report.ok && report.complete, "{expression}: {report:#?}");
    }
}

#[test]
fn spreading_an_open_vector_into_list_does_not_invent_a_length() {
    let source = r#"(ns demo (:require [clojure.spec.alpha :as s]))
      (defn f {:t/sig '[(s/coll-of string? :kind vector?) :-> string?]} [names]
        (first (apply list names)))"#;
    let report = Session::default().check("spread.clj", source);
    let [diagnostic] = &report.diagnostics[..] else {
        panic!("{report:#?}");
    };
    assert_eq!(diagnostic.actual.as_deref(), Some("Nil | String"));
}
#[test]
fn list_contracts_distinguish_persistent_lists_from_other_sequences() {
    let list = "(clojure.spec.alpha/coll-of string? :kind list?)";
    for value in [
        "()",
        "'(\"first\" \"second\")",
        "(list \"first\")",
        "(conj nil \"first\")",
        "(cons \"first\" nil)",
        "(next (list \"first\" \"second\"))",
        "(rest nil)",
        "(rest [\"first\"])",
        "(rest (list \"first\"))",
        "(apply list [\"first\" \"second\"])",
    ] {
        let report = check(value, list);
        assert!(report.ok && report.complete, "{value}: {report:#?}");
    }
    for value in [
        "[\"first\"]",
        "(seq [\"first\"])",
        "(cons \"first\" ())",
        "(cons \"first\" (list \"second\"))",
        "(list 1)",
        "nil",
    ] {
        let report = check(value, list);
        let [diagnostic] = report.diagnostics.as_slice() else {
            panic!("{value}: {report:#?}");
        };
        assert_eq!(diagnostic.code, "PNX2322", "{value}: {report:#?}");
    }
}

#[test]
fn list_contracts_round_trip_through_externs_and_keep_sequence_reads() {
    let library = Source {
        path: "labels.clj",
        source: r#"(ns labels (:require [clojure.spec.alpha :as s]))
            (s/def ::labels (s/coll-of string? :kind list?))
            (defn leading {:t/sig '[::labels :-> (s/nilable string?)]} [labels]
              (first labels))
            (defn as-seq {:t/sig '[::labels :-> (s/coll-of string? :kind seq?)]} [labels]
              labels)"#,
    };
    let exported = Session::default().export_externs(&[library], &[]);
    let Some(edn) = exported.edn else {
        panic!("{:#?}", exported.report);
    };
    assert!(edn.contains(":kind clojure.core/list?"), "{edn}");
}
