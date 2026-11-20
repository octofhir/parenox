use parenox::Session;

#[test]
fn maps_and_proven_scalars_have_exact_predicate_results() {
    let source = r#"(ns demo.map-predicate (:require [clojure.spec.alpha :as s]))
(defn known {:t/sig '[:-> (s/tuple #{true} #{false} #{false} #{false})]} []
  [(map? {:value 42}) (map? nil) (map? 42) (map? "text")])
(defn applied {:t/sig '[:-> #{true}]} [] (apply map? [{}]))
(defn callback {:t/sig '[:-> (s/coll-of #{true} :kind vector?)]} []
  (mapv clojure.core/map? [{} {:value 42}]))
(defn cannot-throw {:t/sig '[:-> #{true}]} []
  (try (map? {}) (catch Throwable _ :unexpected)))"#;
    let report = Session::default().check("map-predicate.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn map_branches_narrow_open_inputs_and_keep_existing_fields() {
    let source = r#"(ns demo.map-predicate (:require [clojure.spec.alpha :as s]))
(defn choose {:t/sig '[any? :-> map?]} [value]
  (if (map? value) value {}))
(defn preserve {:t/sig '[(s/nilable {:id #{:known}}) :-> #{:known :missing}]} [value]
  (if (map? value) (:id value) :missing))
(defn nested {:t/sig '[{:value any?} :-> map?]} [owner]
  (if (map? (:value owner)) (:value owner) {}))"#;
    let report = Session::default().check("map-predicate.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn broad_numeric_classes_can_also_implement_persistent_map() {
    let source = r#"(ns demo.map-predicate)
(defn number-map {:t/sig '[number? :-> #{:numeric-map :other-number}]} [value]
  (if (map? value) :numeric-map :other-number))
(defn integer-map {:t/sig '[integer? :-> #{:integer-map :other-integer}]} [value]
  (if (map? value) :integer-map :other-integer))"#;
    let report = Session::default().check("map-predicate.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");

    let restricted = source
        .replace("#{:numeric-map :other-number}", "#{:other-number}")
        .replace("#{:integer-map :other-integer}", "#{:other-integer}");
    let report = Session::default().check("map-predicate.clj", &restricted);
    assert_eq!(report.diagnostics.len(), 2, "{report:#?}");
    assert!(
        report
            .diagnostics
            .iter()
            .all(|finding| finding.code == "PNX2322")
    );
}

#[test]
fn a_source_predicate_with_the_same_name_does_not_narrow() {
    let source = r#"(ns demo.map-predicate (:refer-clojure :exclude [map?]))
(defn map? {:t/sig '[any? :-> #{true}]} [_] true)
(defn untouched {:t/sig '[string? :-> string?]} [value]
  (if (map? value) value :unexpected))"#;
    let report = Session::default().check("map-predicate.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");
}
