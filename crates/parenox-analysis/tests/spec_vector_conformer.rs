use parenox_analysis::{Report, Session, Source};

const SOURCE: &str = r#"
(ns demo.commands (:require [clojure.spec.alpha :as s]))
(s/def :server-capabilities/code-action-provider
  (s/conformer #(when (vector? %) {:code-action-kinds %})))
(s/def :server-capabilities/execute-command-provider
  (s/conformer (clojure.core/fn [commands]
                 (clojure.core/when (clojure.core/vector? commands) {:commands commands}))))
(s/def ::capabilities (s/keys :opt-un [:server-capabilities/execute-command-provider]))
"#;

fn check(body: &str) -> Report {
    Session::default().check("demo/commands.clj", &format!("{SOURCE}{body}"))
}

#[test]
fn conditional_wrappers_preserve_vectors_and_return_valid_nil_for_other_values() {
    for input in ["nil", "false", "::s/invalid", r#""command""#, "'(false)"] {
        let report = check(&format!(
            r#"
(defn absent {{:t/sig '[:-> nil?]}} []
  (try (s/conform :server-capabilities/code-action-provider {input})
       (catch Throwable _ :unexpected)))
(defn valid {{:t/sig '[:-> #{{true}}]}} []
  (try (s/valid? :server-capabilities/execute-command-provider {input})
       (catch Throwable _ :unexpected)))
"#,
        ));
        assert!(report.ok && report.complete, "{input}: {report:#?}");
    }

    let report = check(
        r#"
(defn action {:t/sig '[:-> {:code-action-kinds (s/tuple #{::s/invalid} #{false})}]} []
  (s/conform :server-capabilities/code-action-provider [::s/invalid false]))
(defn retain-entry {:t/sig '[:-> #{:item}]} []
  (key (:commands (s/conform :server-capabilities/execute-command-provider
                             (clojure.lang.MapEntry/create :item false)))))
(defn nested {:t/sig '[:-> {:execute-command-provider nil?}]} []
  (s/conform ::capabilities {:execute-command-provider false}))
"#,
    );
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn open_input_keeps_both_branches_and_shares_the_vector_predicate() {
    let body = r#"
(defn optional-output
  {:t/sig '[t/any :-> (s/nilable {:commands (s/coll-of t/any :kind vector?)})]}
  [input]
  (s/conform :server-capabilities/execute-command-provider input))
(defn valid {:t/sig '[t/any :-> #{true}]} [input]
  (s/valid? :server-capabilities/execute-command-provider input))
(defn preserve-shape {:t/sig '[(s/coll-of keyword? :kind vector?) :-> {:commands (s/coll-of keyword? :kind vector?)}]}
  [input]
  (s/conform :server-capabilities/execute-command-provider input))
(defn branch
  {:t/sig '[t/any :-> (s/nilable (s/coll-of t/any :kind vector?))]}
  [input]
  (when (vector? input) input))
(defn direct {:t/sig '[:-> (s/tuple #{true} #{true} #{false})]} []
  [(vector? (clojure.lang.MapEntry/create :item true))
   (apply vector? [[]])
   (vector? '(false))])
"#;
    let report = check(body);
    assert!(report.ok && report.complete, "{report:#?}");
    let report = check(&format!(
        r#"{body}
(defn denies-vector-branch {{:t/sig '[number? :-> nil?]}} [input]
  (s/conform :server-capabilities/execute-command-provider input))
"#,
    ));
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(finding.code, "PNX2322");
    assert!(finding.actual.as_deref().unwrap().contains("Vector<Any>"));
}

#[test]
fn shadowed_predicates_are_left_unchecked() {
    for source in [
        r#"(ns demo.shadow (:refer-clojure :exclude [vector?]) (:require [clojure.spec.alpha :as s]))
(defn vector? [_] true)
(s/def ::wrapper (s/conformer #(when (vector? %) {:commands %})))"#,
        r#"(ns demo.shadow (:require [clojure.spec.alpha :as s]))
(s/def ::wrapper (s/conformer (fn [vector?] (when (vector? vector?) {:commands vector?}))))"#,
    ] {
        let source = format!(
            "{source}\n(defn result {{:t/sig '[:-> nil?]}} [] (s/conform ::wrapper false))"
        );
        let report = Session::default().check("demo/shadow.clj", &source);
        assert!(!report.complete, "{report:#?}");
        assert!(
            report.diagnostics.iter().any(|finding| finding.code == "PNX9001" && finding.message.contains("s/conformer")),
            "{report:#?}"
        );
    }
}

#[test]
fn changing_the_conversion_rechecks_a_consumer_with_the_same_input_domain() {
    let mut session = Session::default();
    let consumer = Source {
        path: "consumer.clj",
        source: r#"
(ns consumer (:require [clojure.spec.alpha :as s] [demo.commands]))
(defn absent {:t/sig '[:-> nil?]} []
  (s/conform :server-capabilities/code-action-provider false))"#,
    };
    let mut check = |source| {
        session
            .check_project(
                &[
                    consumer,
                    Source {
                        path: "demo/commands.clj",
                        source,
                    },
                ],
                None,
            )
            .files
            .remove(0)
    };
    let initial = check(SOURCE);
    assert!(initial.ok && initial.complete, "{initial:#?}");
    let warm = check(SOURCE);
    assert_eq!(warm.stats.reused_functions, 1, "{warm:#?}");
    let updated = SOURCE.replace(
        "#(when (vector? %) {:code-action-kinds %})",
        "(fn [input] {:code-action-kinds input})",
    );
    let changed = check(&updated);
    let [finding] = changed.diagnostics.as_slice() else {
        panic!("{changed:#?}");
    };
    assert_eq!(finding.code, "PNX2322");
    assert!(finding.message.contains(":code-action-kinds"));
}
