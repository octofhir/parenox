use parenox::Session;

#[test]
fn preconditions_check_destructured_parameters_and_refine_later_conditions() {
    let source = r#"(ns demo.preconditions
  (:require [clojure.spec.alpha :as s] [clojure.string :as str]))
(defn validate {:t/sig '[(s/tuple (s/nilable string?)) :-> #{:ok}]}
  [[text]]
  {:pre [(string? text) (str/trim text)]}
  :ok)
(defn validate-metadata {:t/sig '[(s/nilable string?) :-> #{:ok}]}
  ^{:pre [(string? text) (str/trim text)]} [text]
  :ok)"#;
    let report = Session::default().check("preconditions.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");

    let reversed = source.replacen(
        "[(string? text) (str/trim text)]",
        "[(str/trim text) (string? text)]",
        1,
    );
    let report = Session::default().check("preconditions.clj", &reversed);
    assert!(!report.ok && report.complete, "{report:#?}");
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(finding.code, "PNX2322");
    let span = finding.span;
    assert_eq!(&reversed[span.start.offset..span.end.offset], "text");
    assert!(finding.notes.iter().any(|note| note.contains("*assert*")));
}

#[test]
fn the_body_must_typecheck_when_assertions_are_disabled() {
    let source = r#"(ns demo.preconditions
  (:require [clojure.spec.alpha :as s] [clojure.string :as str]))
(defn trim-text {:t/sig '[(s/nilable string?) :-> string?]} [text]
  {:pre [(string? text)]}
  (str/trim text))"#;
    let report = Session::default().check("preconditions.clj", source);
    assert!(!report.ok && report.complete, "{report:#?}");
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(finding.code, "PNX2322");
    assert_eq!(finding.actual.as_deref(), Some("Nil | String"));
}

#[test]
fn a_recur_iteration_checks_its_own_preconditions() {
    let source = r#"(ns demo.preconditions (:require [clojure.string :as str]))
(defn trim-again {:t/sig '[string? :-> #{:done}]} [text]
  {:pre [(str/trim text)]}
  (if (nil? text) :done (recur nil)))"#;
    let report = Session::default().check("preconditions.clj", source);
    assert!(!report.ok && report.complete, "{report:#?}");
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(finding.code, "PNX2322");
    assert_eq!(finding.actual.as_deref(), Some("Nil"));
    assert!(
        finding
            .notes
            .iter()
            .any(|note| note.contains("later iteration"))
    );
}

#[test]
fn inline_and_private_preconditions_may_throw_an_assertion_error() {
    let source = r#"(ns demo.preconditions)
(defn- validate [enabled] {:pre [enabled]} :body)
(defn private-call {:t/sig '[:-> #{:body :caught}]} []
  (try (validate false)
       (catch Exception _ :wrong)
       (catch AssertionError _ :caught)))
(defn inline-call {:t/sig '[:-> #{:body :caught}]} []
  (try ((fn [] {:pre [nil]} :body))
       (catch Exception _ :wrong)
       (catch Error _ :caught)))
(defn truthy-call {:t/sig '[:-> #{:body}]} []
  (try ((fn [] {:pre [0 [] {} ""]} :body))
       (catch AssertionError _ :wrong)))"#;
    let report = Session::default().check("preconditions.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");

    for expected in ["#{:body}", "#{:caught}"] {
        let restricted = source.replace("#{:body :caught}", expected);
        let report = Session::default().check("preconditions.clj", &restricted);
        assert!(!report.ok && report.complete, "{report:#?}");
        assert_eq!(report.diagnostics.len(), 2, "{report:#?}");
        for finding in report.diagnostics {
            assert_eq!(finding.code, "PNX2322", "{finding:#?}");
        }
    }
}

#[test]
fn an_explicit_condition_map_overrides_metadata_but_a_lone_map_is_the_body() {
    let source = r#"(ns demo.preconditions)
(defn explicit {:t/sig '[:-> #{:ok}]}
  ^{:pre (true) :post [false]} [] {} :ok)
(defn unrelated {:t/sig '[:-> #{:ok}]}
  ^{:post [false]} [] {:documentation true} :ok)
(defn lone-map {:t/sig '[:-> map?]} [] {:pre [false] :post [false]})
(defn nil-condition {:t/sig '[:-> #{:ok}]} [] {:pre nil :post nil} :ok)
(defn false-condition {:t/sig '[:-> #{:ok}]} [] {:pre false :post false} :ok)
(defn empty-condition {:t/sig '[:-> #{:ok}]} [] {:pre []} :ok)"#;
    let report = Session::default().check("preconditions.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn nonvector_conditions_are_explicitly_unsupported() {
    for (conditions, message, value) in [
        ("{:pre (true)}", "vector of expressions", "(true)"),
        ("{:post (true)}", "postcondition", "(true)"),
    ] {
        let source = format!(
            "(ns demo.preconditions) (defn f {{:t/sig '[:-> number?]}} [] {conditions} 42)"
        );
        let report = Session::default().check("preconditions.clj", &source);
        assert!(!report.complete, "{report:#?}");
        let finding = report
            .diagnostics
            .iter()
            .find(|finding| finding.code == "PNX9001" && finding.message.contains(message))
            .unwrap_or_else(|| panic!("{report:#?}"));
        let span = finding.span;
        assert_eq!(&source[span.start.offset..span.end.offset], value);
    }
}
