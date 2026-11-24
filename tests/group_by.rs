use parenox::{Report, Session};

fn check(expression: &str, contract: &str) -> Report {
    let source = format!("(defn run {{:t/sig '[:-> {contract}]}} [] {expression})");
    Session::default().check("group_by.clj", &source)
}

#[test]
fn groups_typed_collections_and_keeps_known_bucket_positions() {
    let source = include_str!("../examples/group_by.clj");
    let report = Session::default().check("group_by.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");

    let edited = source.replace(
        r#"(s/tuple #{"first"} #{"last"})"#,
        r#"(s/tuple #{"last"} #{"first"})"#,
    );
    let report = Session::default().check("group_by.clj", &edited);
    let [error] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert!(!report.ok && report.complete, "{report:#?}");
    assert_eq!(error.code, "PNX2322");
}

#[test]
fn grouping_preserves_nil_false_and_maybe_empty_collections() {
    let source = r#"
        (ns demo (:require [clojure.spec.alpha :as s]))
        (defn markers {:t/sig '[:-> {"state" (s/tuple nil? #{false})}]} []
          (group-by (fn [_] "state") [nil false]))
        (defn typed {:t/sig '[(s/coll-of number? :kind vector?)
                             :-> (t/map {:opt {:items (s/coll-of number? :kind vector?)}})]}
          [values]
          (group-by (fn [_] :items) values))
        (defn empty-input {:t/sig '[:-> {}]} []
          (group-by 42 nil))
        (defn boolean-key {:t/sig '[:-> {false (s/tuple #{false})}]} []
          (group-by identity [false]))
    "#;
    let report = Session::default().check("group_by.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");

    let edited = source.replace(":opt {:items", ":req {:items");
    let report = Session::default().check("group_by.clj", &edited);
    assert!(!report.ok && report.complete, "{report:#?}");
    assert_eq!(report.diagnostics[0].code, "PNX2322");
}

#[test]
fn callback_and_collection_effects_precede_grouping() {
    let report = check(
        r#"(try
               (group-by (fnil identity (throw (ex-info "callback" {:stage :callback})))
                         (throw (ex-info "collection" {:stage :collection})))
               (catch clojure.lang.ExceptionInfo e (:stage (ex-data e))))"#,
        "#{:callback}",
    );
    assert!(report.ok && report.complete, "{report:#?}");

    let report = check(
        r#"(try (group-by (fn [_] (throw (ex-info "item" {:stage :body}))) [1])
                (catch clojure.lang.ExceptionInfo e (:stage (ex-data e))))"#,
        "#{:body}",
    );
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn diagnostics_distinguish_bad_collections_from_unsupported_group_keys() {
    for (source, code) in [
        ("(group-by identity [[false]])", "PNX9001"),
        ("(group-by identity 42)", "PNX2322"),
        ("(group-by identity)", "PNX2554"),
    ] {
        let report = check(source, "map?");
        let [error] = report.diagnostics.as_slice() else {
            panic!("{source}: {report:#?}");
        };
        assert_eq!(error.code, code, "{source}: {report:#?}");
    }
}

#[test]
fn nested_group_callbacks_are_bounded() {
    let mut body = ":bucket".to_string();
    for _ in 0..8 {
        body = format!("(group-by (fn [_] (do {body} :bucket)) [1 2])");
    }
    let report = check(&body, "map?");
    assert!(!report.complete, "{report:#?}");
    assert!(
        report.diagnostics.iter().any(|error| {
            error.code == "PNX9001" && error.message.contains("group-by callback analysis limit")
        }),
        "{report:#?}"
    );
}

#[test]
fn reduce_uses_the_nonempty_bucket_after_lookup() {
    let source = r#"
        (ns demo (:require [clojure.spec.alpha :as s]))
        (defn exact {:t/sig '[:-> int?]} []
          (reduce (fn [a b] (+ a b)) (:bucket (group-by (fn [_] :bucket) [1 2]))))
        (defn guarded {:t/sig '[(s/coll-of int? :kind vector?) :-> number?]} [items]
          (let [{:keys [bucket]} (group-by (fn [_] :bucket) items)]
            (if bucket (reduce (fn [a b] (+ a b)) bucket) 0)))
        (defn dictionary {:t/sig '[(s/coll-of string? :kind vector?) :-> string?]} [items]
          (if-let [bucket (get (group-by identity items) "bucket")]
            (reduce (fn [a _] a) bucket)
            ""))
        (defn updated {:t/sig '[(s/coll-of int? :kind vector?) :-> map?]} [items]
          (update-vals (group-by (fn [_] :bucket) items)
            (fn [bucket] (reduce (fn [a b] (+ a b)) bucket))))
        (defn comprehension {:t/sig '[(s/coll-of int? :kind vector?) :-> t/any]} [items]
          (for [[_ bucket] (group-by (fn [_] :bucket) items)]
            (reduce (fn [a b] (+ a b)) bucket)))
    "#;
    let report = Session::default().check("group_by.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn reduce_still_checks_empty_input_when_a_bucket_is_absent() {
    let source = r#"
        (ns demo (:require [clojure.spec.alpha :as s]))
        (defn unchecked-lookup {:t/sig '[(s/coll-of int? :kind vector?) :-> number?]} [items]
          (reduce (fn [a b] (+ a b)) (get (group-by (fn [_] :bucket) items) :bucket)))
        (defn empty-default {:t/sig '[:-> number?]} []
          (reduce (fn [a b] (+ a b)) (get (group-by (fn [_] :bucket) [1]) :missing [])))
    "#;
    let report = Session::default().check("group_by.clj", source);
    assert_eq!(
        report
            .diagnostics
            .iter()
            .filter(|finding| finding.code == "PNX2554")
            .count(),
        2,
        "{report:#?}"
    );
}

#[test]
fn vals_preserves_bucket_shapes_for_a_lazy_mapping_callback() {
    let source = r#"
        (ns demo (:require [clojure.spec.alpha :as s]))
        (defn grouped {:t/sig '[:-> (s/coll-of number? :kind seq?)]} []
          (map (fn [group] (reduce (fn [a b] (+ a b)) group))
               (vals (group-by odd? [1 2 3]))))
        "#;
    let report = Session::default().check("group_by.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn group_buckets_satisfy_nonempty_vector_contracts() {
    let source = r#"
        (ns demo (:require [clojure.spec.alpha :as s]))
        (defn bucket {:t/sig '[(s/coll-of int? :kind vector? :min-count 1)
                                :-> (s/coll-of int? :kind vector? :min-count 1)]}
          [items]
          (:bucket (group-by (constantly :bucket) items)))
        (defn reduce-bucket {:t/sig '[(s/coll-of int? :kind vector? :min-count 1) :-> number?]}
          [items]
          (reduce (fn [a b] (+ a b)) (bucket items)))
        (defn varying-keys {:t/sig '[string? string? :-> #{false}]}
          [a b]
          (if-let [bucket (get (group-by identity [a b]) "bucket")]
            (empty? bucket)
            false))
    "#;
    let report = Session::default().check("group_by.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");

    let maybe_empty = source.replacen(":min-count 1", ":min-count 0", 1);
    let report = Session::default().check("group_by.clj", &maybe_empty);
    let [diagnostic] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(diagnostic.code, "PNX2322", "{report:#?}");
}

#[test]
fn nonempty_buckets_keep_vector_operations() {
    let source = r#"
        (ns demo (:require [clojure.spec.alpha :as s]))
        (defn read-bucket
          {:t/sig '[(s/coll-of int? :kind vector? :min-count 1)
                    :-> (s/tuple int? int? int? int? #{true} #{true} map?
                                 (s/coll-of int? :kind vector?) number? int?)]}
          [items]
          (let [bucket (:bucket (group-by (fn [_] :bucket) items))
                [head] bucket]
            [head (nth bucket 0) (bucket 0) (get bucket 0)
             (contains? bucket 0) (vector? bucket)
             {:entry (find bucket 0)}
             (conj bucket 0)
             (reduce-kv (fn [acc _ item] (+ acc item)) 0 bucket)
             (first (apply list bucket))]))
    "#;
    let report = Session::default().check("group_by.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");
}
