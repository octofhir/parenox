use parenox::Session;

const TYPES: &str = "
    (ns config (:require [clojure.spec.alpha :as s]))
    (s/def :defaults/label number?)
    (s/def :override/label (s/nilable string?))
    (s/def ::defaults (s/keys :req-un [:defaults/label]))
    (s/def ::override (s/keys :opt-un [:override/label]))
";

#[test]
fn optional_overwrite_preserves_required_presence_and_both_value_types() {
    let source = format!(
        "{TYPES}
         (defn present {{:t/sig '[::defaults ::override :-> #{{true}}]}} [left right]
           (contains? (merge left right) :label))
         (defn read-label {{:t/sig '[::defaults ::override :-> (s/or :old number? :new (s/nilable string?))]}}
           [left right]
           (get (merge left right) :label false))"
    );
    let report = Session::default().check("config.clj", &source);
    assert!(report.ok && report.complete, "{:#?}", report.diagnostics);
    let invalid = source.replace("(s/or :old number? :new (s/nilable string?))", "number?");
    let report = Session::default().check("config.clj", &invalid);
    assert_eq!(report.diagnostics.len(), 1);
    let diagnostic = &report.diagnostics[0];
    assert_eq!(diagnostic.code, "PNX2322");
    assert_eq!(diagnostic.actual.as_deref(), Some("Nil | Number | String"));
}

#[test]
fn rightmost_value_wins_and_nil_is_not_an_overwrite_when_it_is_an_operand() {
    let report = Session::default().check(
        "config.clj",
        "(defn empty-result {:t/sig '[:-> nil?]} [] (merge nil (merge)))
         (defn nilable {:t/sig '[(clojure.spec.alpha/nilable map?) :-> (clojure.spec.alpha/nilable map?)]}
           [x] (merge nil x))
         (defn read-label {:t/sig '[:-> nil?]} []
           (get (apply merge [{:label 42} nil {:label \"old\"} {:label nil}]) :label \"fallback\"))
         (defn sibling {:t/sig '[:-> number?]} []
           (get (merge {\"other\" 42 :label 0} {:label \"new\"}) \"other\"))",
    );
    assert!(report.ok && report.complete, "{:#?}", report.diagnostics);
}

#[test]
fn dictionary_tail_may_overwrite_a_fixed_value() {
    let report = Session::default().check(
        "config.clj",
        "(ns config (:require [clojure.spec.alpha :as s]))
         (defn merged {:t/sig '[(s/map-of keyword? string?) :-> number?]} [right]
           (:label (merge {:label 42} right)))",
    );
    assert_eq!(report.diagnostics.len(), 1, "{:#?}", report.diagnostics);
    assert_eq!(
        report.diagnostics[0].actual.as_deref(),
        Some("Number | String")
    );
}

#[test]
fn undeclared_open_fields_are_not_assumed_to_preserve_left_values() {
    let report = Session::default().check(
        "config.clj",
        "(defn merged {:t/sig '[map? :-> number?]} [right]
           (:label (merge {:label 42} right)))",
    );
    assert!(!report.complete);
    assert_eq!(report.diagnostics.len(), 1, "{:#?}", report.diagnostics);
    assert_eq!(report.diagnostics[0].code, "PNX9003");
}

#[test]
fn non_map_operands_are_outside_the_supported_contract() {
    let source = "(defn invalid {:t/sig '[:-> map?]} [] (merge {} 42))";
    let report = Session::default().check("config.clj", source);
    assert_eq!(report.diagnostics.len(), 1, "{:#?}", report.diagnostics);
    let diagnostic = &report.diagnostics[0];
    assert!(!report.complete);
    assert_eq!(diagnostic.code, "PNX9001");
    assert_eq!(
        &source[diagnostic.span.start.offset..diagnostic.span.end.offset],
        "42"
    );
}

#[test]
fn open_apply_does_not_assume_an_override_is_present() {
    let report = Session::default().check(
        "config.clj",
        "(ns config (:require [clojure.spec.alpha :as s]))
         (defn merged {:t/sig '[(s/coll-of (s/map-of keyword? string?) :kind vector?) :-> string?]} [more]
           (:label (apply merge {:label 42} more)))",
    );
    assert!(!report.complete);
    assert_eq!(report.diagnostics.len(), 1, "{report:#?}");
    assert_eq!(report.diagnostics[0].code, "PNX9001");
}
