use parenox::{Report, Session};

fn check(expression: &str, contract: &str) -> Report {
    let source = format!("(defn run {{:t/sig '[:-> {contract}]}} [] {expression})");
    Session::default().check("map_builders.clj", &source)
}

#[test]
fn small_literal_maps_retain_values_at_proven_symbol_keys() {
    let expression = r#"(let [key :amount unit "currency"]
      {key 7 unit :euro :present true})"#;
    let contract = r#"{:amount number? "currency" keyword? :present boolean?}"#;
    let report = check(expression, contract);
    assert!(report.ok && report.complete, "{report:#?}");

    let array_map =
        "(let [key :amount] (get {key :seven :a 1 :b 2 :c 3 :d 4 :e 5 :f 6 :g 7 :h 8} :amount))";
    let report = check(array_map, "#{:seven}");
    assert!(report.ok && report.complete, "{report:#?}");

    let hash_map = "(let [key :last] {:amount 7 :a 1 :b 2 :c 3 :d 4 :e 5 :f 6 :g 7 key 8})";
    let report = check(hash_map, "t/any");
    assert!(!report.complete, "{report:#?}");
    assert_eq!(report.diagnostics[0].code, "PNX9001");
}

#[test]
fn hash_map_values_do_not_inherit_source_order_reachability() {
    let entries = r#"{:key0 (throw nil) :key1 1 :key2 2 :key3 (inc "hash-order")
                     :key4 4 :key5 5 :key6 6 :key7 7 TAIL 8}"#;
    let report = check(&entries.replace("TAIL", ":key8"), "string?");
    assert!(report.ok && report.complete, "{report:#?}");

    let hash_map = entries.replace("TAIL", r#""key8""#);
    let source = format!("(defn run {{:t/sig '[:-> string?]}} [] {hash_map})");
    let report = Session::default().check("map_builders.clj", &source);
    let [error] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(error.code, "PNX2322");
    assert_eq!(
        &source[error.span.start.offset..error.span.end.offset],
        r#""hash-order""#
    );
}

#[test]
fn ordered_literals_check_computed_keys_before_values() {
    let source = r#"(ns demo)
(defn key-fails {:t/sig '[:-> #{:key}]} []
  (try {(throw (ex-info "key" {:stage :key})) (inc "unreached")}
       (catch clojure.lang.ExceptionInfo e (:stage (ex-data e)))))
(defn value-fails {:t/sig '[:-> #{:value}]} []
  (try {:first (throw (ex-info "value" {:stage :value})) (inc "unreached") nil}
       (catch clojure.lang.ExceptionInfo e (:stage (ex-data e)))))
(defn exact-key {:t/sig '[:-> #{:stored}]} []
  (req! {(identity :key) :stored} :key))"#;
    let report = Session::default().check("map_order.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");

    let source = r#"(defn run {:t/sig '[:-> map?]} []
      {(subs 42 0) (inc "unreached")})"#;
    let report = Session::default().check("map_order.clj", source);
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(finding.code, "PNX2322");
    assert_eq!(
        &source[finding.span.start.offset..finding.span.end.offset],
        "42"
    );
}

#[test]
fn runtime_key_collisions_follow_map_value_evaluation() {
    let source = r#"
(defn collision {:t/sig '[:-> #{:duplicate}]} []
  (let [key :amount]
    (try
      (do {key 1 :amount 2}
          (inc "unreached"))
      (catch IllegalArgumentException _ :duplicate))))
(defn stopped {:t/sig '[:-> #{:early}]} []
  (let [key :amount]
    (try
      {key (throw (ex-info "early" {})) :amount (inc false)}
      (catch clojure.lang.ExceptionInfo _ :early))))
"#;
    let report = Session::default().check("map_builders.clj", source);
    let [error] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(error.code, "PNX2790");
    assert_eq!(error.span.start.offset, source.find(":amount 2").unwrap());
    assert_eq!(
        &source[error.span.start.offset..error.span.end.offset],
        ":amount"
    );
}

#[test]
fn builds_and_renames_a_typed_map_and_rejects_definite_key_collisions() {
    let source = include_str!("../examples/map_builders.clj");
    let report = Session::default().check("map_builders.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");

    let edited = source.replace(r#":unit "unit""#, r#":unit "amount""#);
    let report = Session::default().check("map_builders.clj", &edited);
    let [error] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert!(!report.ok && report.complete, "{report:#?}");
    assert_eq!(error.code, "PNX2790");
    assert!(error.message.contains(":amount") && error.message.contains(":unit"));
    assert!(error.message.contains(r#""amount""#));
}

#[test]
fn update_keys_preserves_optional_presence_and_checks_possible_collisions() {
    let source = include_str!("../examples/map_builders.clj");
    let edited = source.replace(
        r#":opt {"pending" boolean? "done" boolean?}"#,
        r#":req {"pending" boolean? "done" boolean?}"#,
    );
    let report = Session::default().check("map_builders.clj", &edited);
    let [error] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert!(!report.ok && report.complete, "{report:#?}");
    assert_eq!(error.code, "PNX2322");

    let edited = source.replace(r#":done "done""#, r#":done "pending""#);
    let report = Session::default().check("map_builders.clj", &edited);
    let [error] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert!(!report.complete, "{report:#?}");
    assert_eq!(error.code, "PNX9001");
    assert!(error.message.contains("may map both"));
}

#[test]
fn update_keys_keeps_values_and_treats_multiple_possible_destinations_as_optional() {
    let source = r#"
        (defn rename {:t/sig '[boolean? :-> (t/map {:opt {:a nil? :b nil?}})]}
          [switch?]
          (update-keys {:old nil} (fn [_] (if switch? :a :b))))
        (defn absent {:t/sig '[:-> {}]} []
          (update-keys nil (fn [] 42)))
        (defn removed {:t/sig '[:-> {:new #{false}}]} []
          (update-keys (dissoc {:old false :hidden 7} :hidden) (fn [_] :new)))
    "#;
    let report = Session::default().check("map_builders.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");

    let edited = source.replace(":opt {:a nil? :b nil?}", ":req {:a nil? :b nil?}");
    let report = Session::default().check("map_builders.clj", &edited);
    assert_eq!(report.diagnostics[0].code, "PNX2322", "{report:#?}");
}

#[test]
fn zipmap_stops_at_the_shorter_input_and_keeps_the_last_value_for_duplicates() {
    for (body, expected) in [
        (
            "(zipmap [:a :b :a :unused] [1 nil false])",
            "{:a #{false} :b nil?}",
        ),
        (r#"(zipmap '("plan") [7 :unused])"#, r#"{"plan" number?}"#),
        (
            r#"(zipmap [false \x 7 nil false] [:before 2 :count "text" nil])"#,
            r#"{false nil? \x number? 7 keyword? nil string?}"#,
        ),
        (
            r#"(apply zipmap [[:key] ["value"]])"#,
            r#"{:key #{"value"}}"#,
        ),
        ("(zipmap [:extra] [])", "{}"),
        ("(zipmap nil nil)", "{}"),
    ] {
        let report = check(body, expected);
        assert!(report.ok && report.complete, "{body}: {report:#?}");
    }
}

#[test]
fn zipmap_keeps_prior_values_when_a_later_duplicate_may_be_absent() {
    let source = r#"
        (ns demo (:require [clojure.spec.alpha :as s]))
        (defn value {:t/sig '[(s/coll-of #{:slot} :kind vector?) :-> {:slot #{"early" false}}]}
          [keys]
          (zipmap (cons :slot keys) ["early" false]))
        (defn fixed-keys {:t/sig '[(s/coll-of number? :kind vector?)
                                   :-> (t/map {:opt {:a number? :b number?}})]}
          [values]
          (zipmap [:a :b] values))
    "#;
    let report = Session::default().check("mixed.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");

    let edited = source.replace(r#"#{"early" false}"#, "#{false}");
    let report = Session::default().check("mixed.clj", &edited);
    let [error] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(error.code, "PNX2322");
    assert!(error.actual.as_deref().unwrap().contains("String"));
}

#[test]
fn diagnostics_distinguish_unsupported_keys_from_invalid_sequence_arguments() {
    let source = r#"
        (ns demo (:require [clojure.spec.alpha :as s]))
        (defn unknown-keys {:t/sig '[(s/map-of keyword? string?) :-> map?]} [row]
          (update-keys row identity))
        (defn compound-key {:t/sig '[:-> map?]} [] (zipmap [[1]] [:value]))
        (defn invalid-seq {:t/sig '[:-> map?]} [] (zipmap nil 42))
    "#;
    let report = Session::default().check("map_builders.clj", source);
    let [unknown, compound, invalid] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    for (error, code, token) in [
        (unknown, "PNX9001", "row"),
        (compound, "PNX9001", "[[1]]"),
        (invalid, "PNX2322", "42"),
    ] {
        assert_eq!(error.code, code);
        let span = error.span;
        assert_eq!(&source[span.start.offset..span.end.offset], token);
    }
}

#[test]
fn update_keys_bounds_expansion_of_finite_key_domains() {
    let keys = (0..129)
        .map(|id| format!(":key{id}"))
        .collect::<Vec<_>>()
        .join(" ");
    let source = format!(
        r#"
        (ns demo (:require [clojure.spec.alpha :as s]))
        (defn rename {{:t/sig '[keyword? :-> #{{{keys}}}]}} [_] :key0)
        (defn too-many-inputs {{:t/sig '[(s/map-of #{{{keys}}} number?) :-> map?]}}
          [row]
          (update-keys row identity))
        (defn too-many-outputs {{:t/sig '[:-> map?]}} []
          (update-keys {{:name 1}} rename))
    "#
    );
    let report = Session::default().check("map_builders.clj", &source);
    let [input, result] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert!(!report.complete, "{report:#?}");
    assert_eq!(input.code, "PNX9001");
    assert_eq!(result.code, "PNX9001");
    assert!(input.message.contains("too many possible input keys"));
    assert!(result.message.contains("too many possible result keys"));
}

#[test]
fn zipmap_reports_when_the_first_equivalent_key_has_an_unmodeled_class() {
    let source = r#"
        (defn zipped {:t/sig '[:-> map?]} []
          (zipmap [1N 1] [:old :new]))
    "#;
    let report = Session::default().check("key_class.clj", source);
    let [gap] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert!(!report.complete);
    assert_eq!(gap.code, "PNX9001");
    assert_eq!(
        &source[gap.span.start.offset..gap.span.end.offset],
        "[1N 1]"
    );
}
