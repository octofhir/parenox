use parenox::{DocumentHover, Session};

#[test]
fn a_named_local_fn_checks_each_reached_part_of_a_recursive_lookup() {
    let source = r#"(ns demo (:require [clojure.spec.alpha :as s]))
        (defn update-existing-in
          {:t/sig '[{:outer {:label string?}}
                    (s/tuple #{:outer} #{:label})
                    (s/fspec :args (s/cat :value string?) :ret string?)
                    nil?
                    :-> {:outer {:label string?}}]}
          [m ks f args]
          (let [up (fn up [m ks f args]
                     (let [[k & ks] ks]
                       (if-let [kv (find m k)]
                         (if ks
                           (assoc m k (up (val kv) ks f args))
                           (assoc m k (apply f (val kv) args)))
                         m)))]
            (up m ks f args)))"#;
    let mut session = Session::default();
    let report = session.check("named.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");
    let call = source.find("(val kv) ks f args").unwrap() + "(val kv) ".len();
    let Some(DocumentHover::Local(path)) = session.document_hover_at("named.clj", call) else {
        panic!("missing type on the recursive call's remaining path");
    };
    assert_eq!(path.inferred_type, "SeqTuple<:label>");

    let changed = source.replace("(apply f (val kv) args)", "(apply f m args)");
    let report = session.check("named.clj", &changed);
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(finding.code, "PNX2322");
    assert_eq!(finding.actual.as_deref(), Some("{:label: String}"));
    let range = finding.span.start.offset..finding.span.end.offset;
    assert_eq!(&changed[range], "m");
}

#[test]
fn a_named_fn_can_alias_itself_and_can_be_shadowed_by_its_parameter() {
    let source = r#"(ns demo)
        (defn alias {:t/sig '[:-> #{:self}]} []
          (let [step (fn [_] :outer)]
            ((fn step
               ([value] (let [again step] (again value :self)))
               ([value result] result))
             0)))
        (defn shadow {:t/sig '[:-> #{:parameter}]} []
          ((fn step [step] (step {:key :parameter})) :key))"#;
    let report = Session::default().check("named.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn unfinished_recursion_does_not_publish_hover_from_a_prefix_of_the_calls() {
    let source = r#"(ns demo)
        (defn explore {:t/sig '[number? :-> t/any]} [n]
          ((fn again [waiting]
             (if (= waiting 0)
               waiting
               (again (dec waiting))))
           n))"#;
    let mut session = Session::default();
    let report = session.check("named.clj", source);
    assert!(!report.complete, "{report:#?}");
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.code == "PNX9001" && d.message.contains("Local fn nesting limit")),
        "{report:#?}"
    );
    let offset = source.find("waiting 0").unwrap();
    assert!(session.document_hover_at("named.clj", offset).is_none());
}

#[test]
fn uninvoked_named_functions_keep_compiler_diagnostics() {
    let source = r#"(ns demo (:require [clojure.spec.alpha :as s]))
        (defn empty-result {:t/sig '[:-> (s/coll-of number? :kind vector?)]} []
          (mapv (fn step [x] (if x (step false) missing))
                []))"#;
    let report = Session::default().check("named.clj", source);
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(finding.code, "PNX2304");
    let range = finding.span.start.offset..finding.span.end.offset;
    assert_eq!(&source[range], "missing");
}
