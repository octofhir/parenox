use parenox::{DocumentHover, Session};

#[test]
fn count_supplies_utf16_bounds_for_the_exact_arglist_slice() {
    let source = r#"(ns demo)
        (defn arglist-interior {:t/sig '[:-> #{"🦊"}]} []
          (let [raw-arglist "[🦊]"
                width (count raw-arglist)
                interior (subs raw-arglist 1 (dec (count raw-arglist)))]
            (do width interior)))"#;
    let mut session = Session::default();
    let report = session.check("count.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");
    for (marker, expected) in [("width interior", "4"), ("interior)))", "\"🦊\"")] {
        let offset = source.find(marker).unwrap();
        let Some(DocumentHover::Local(hover)) = session.document_hover_at("count.clj", offset)
        else {
            panic!("missing hover at {marker}");
        };
        assert_eq!(hover.inferred_type, expected);
    }
    let changed = source.replace("[🦊]", "x");
    let report = session.check("count.clj", &changed);
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(finding.code, "PNX2801");
    let range = finding.span.start.offset..finding.span.end.offset;
    assert_eq!(&changed[range], "(dec (count raw-arglist))");
}

#[test]
fn ordinary_and_prepared_counts_have_fixed_integer_results() {
    let source = r#"(ns demo (:require [clojure.spec.alpha :as s]))
        (defn accept-index {:t/sig '[integer? :-> integer?]} [index] index)
        (defn last-index {:t/sig '[string? :-> integer?]} [text]
          (accept-index (dec (count text))))
        (defn size {:t/sig '[(s/coll-of string? :kind vector?) :-> int?]} [names]
          (count names))
        (defn sizes {:t/sig '[:-> (s/coll-of int? :kind vector?)]} []
          (mapv count ["a" "🦊"]))
        (defn indexed {:t/sig '[:-> #{:selected}]} []
          (get [:a :b :selected] (count "🦊")))
        (defn shadow {:t/sig '[:-> #{"local"}]} []
          (let [count (fn [_] "local")]
            (count [])))"#;
    let report = Session::default().check("count.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn counting_an_open_sequence_can_still_throw() {
    let source = r#"(ns demo (:require [clojure.spec.alpha :as s]))
        (defn f {:t/sig '[(s/coll-of any? :kind seq?) :-> #{:normal :thrown}]} [source]
          (let [result (try (count source) :normal
                            (catch clojure.lang.ExceptionInfo _ :thrown))]
            result))"#;
    let mut session = Session::default();
    let report = session.check("count.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");
    let offset = source.rfind("result").unwrap();
    let Some(DocumentHover::Local(hover)) = session.document_hover_at("count.clj", offset) else {
        panic!("missing count catch outcomes");
    };
    assert_eq!(hover.inferred_type, ":normal | :thrown");
}
