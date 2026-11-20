use parenox::Session;

#[test]
fn postconditions_bind_the_exact_result_and_keep_the_argument_in_the_body() {
    let source = r#"(ns demo.postconditions (:require [clojure.string :as str]))
(defn echo {:t/sig '[string? :-> #{:result}]} [%]
  {:pre [(str/trim %)]
   :post [(if (= % :result) true (inc "unexpected result"))]}
  (str/trim %)
  :result)"#;
    let report = Session::default().check("postconditions.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn assertions_refine_later_postconditions_without_strengthening_the_return_contract() {
    let source = r#"(ns demo.postconditions
  (:require [clojure.spec.alpha :as s] [clojure.string :as str]))
(defn refine {:t/sig '[(s/nilable string?) :-> (s/nilable string?)]}
  ^{:post [(string? %) (str/trim %)]} [value]
  value)
(defn use-precondition {:t/sig '[(s/nilable string?) :-> #{:ok}]} [value]
  {:pre [(string? value)]
   :post [(str/trim value)]}
  :ok)
(defn use-returned-local {:t/sig '[(s/nilable string?) :-> (s/nilable string?)]} [result]
  {:pre [(string? result)]
   :post [(str/trim %)]}
  result)"#;
    let report = Session::default().check("postconditions.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");

    for (changed, offending) in [
        (
            source.replace("[(string? %) (str/trim %)]", "[(str/trim %) (string? %)]"),
            "%",
        ),
        (
            source.replacen(":-> (s/nilable string?)", ":-> string?", 1),
            "value",
        ),
        (source.replace(":pre [(string? value)]", ":pre []"), "value"),
        (source.replace(":pre [(string? result)]", ":pre []"), "%"),
    ] {
        let report = Session::default().check("postconditions.clj", &changed);
        assert!(!report.ok && report.complete, "{report:#?}");
        let [finding] = report.diagnostics.as_slice() else {
            panic!("{report:#?}");
        };
        assert_eq!(finding.code, "PNX2322");
        let span = finding.span;
        assert_eq!(&changed[span.start.offset..span.end.offset], offending);
    }
}

#[test]
fn private_and_inline_postconditions_can_throw_an_assertion_error() {
    let source = r#"(ns demo.postconditions)
(defn- only-nil [value] {:post [(nil? %)]} value)
(defn private-call {:t/sig '[:-> #{:body :caught}]} []
  (try (only-nil :body)
       (catch Exception _ :wrong)
       (catch AssertionError _ :caught)))
(defn inline-call {:t/sig '[:-> #{:body :caught}]} []
  (try ((fn [] {:post [false]} :body))
       (catch Exception _ :wrong)
       (catch Error _ :caught)))"#;
    let report = Session::default().check("postconditions.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");

    for expected in ["#{:body}", "#{:caught}"] {
        let restricted = source.replace("#{:body :caught}", expected);
        let report = Session::default().check("postconditions.clj", &restricted);
        assert!(!report.ok && report.complete, "{report:#?}");
        assert_eq!(report.diagnostics.len(), 2, "{report:#?}");
        for finding in report.diagnostics {
            assert_eq!(finding.code, "PNX2322", "{finding:#?}");
        }
    }
}

#[test]
fn postconditions_after_a_precondition_failure_or_body_throw_have_no_effects() {
    let source = r#"(ns demo.postconditions (:require [clojure.string :as str]))
(defn pre-fails {:t/sig '[:-> #{:body :pre-error}]} []
  (try ((fn [] {:pre [false] :post [(str/trim 42)]} :body))
       (catch Exception _ :wrong)
       (catch AssertionError _ :pre-error)))
(defn body-throws {:t/sig '[:-> #{:body-error}]} []
  (try ((fn [] {:post [false (str/trim 42)]}
          (throw (ex-info "body" {}))))
       (catch ExceptionInfo _ :body-error)
       (catch AssertionError _ :wrong)))"#;
    let report = Session::default().check("postconditions.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn an_empty_post_vector_removes_the_function_tail_position() {
    let source = r#"(ns demo.postconditions)
(defn function-recur {:t/sig '[boolean? :-> #{:done}]} [again]
  {:post false}
  (if again (recur false) :done))
(defn nested-loop {:t/sig '[:-> #{:done}]} []
  {:post []}
  (loop [again true] (if again (recur false) :done)))"#;
    let report = Session::default().check("postconditions.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");
    let wrapped = source.replace("{:post false}", "{:post []}");
    let report = Session::default().check("postconditions.clj", &wrapped);
    assert!(!report.ok, "{report:#?}");
    let finding = report
        .diagnostics
        .iter()
        .find(|finding| finding.code == "PNX1001" && finding.message.contains("tail position"))
        .unwrap_or_else(|| panic!("{report:#?}"));
    let span = finding.span;
    assert_eq!(
        &wrapped[span.start.offset..span.end.offset],
        "(recur false)"
    );
}

#[test]
fn only_active_postcondition_vectors_are_checked() {
    let source = r#"(ns demo.postconditions)
(defn explicit {:t/sig '[:-> #{:ok}]} ^{:post (true)} [] {} :ok)
(defn lone-map {:t/sig '[:-> map?]} ^{:post [(= (:post %) [false])]} [] {:post [false]})
(defn omitted {:t/sig '[:-> #{:ok}]} [] {:post nil} :ok)"#;
    let report = Session::default().check("postconditions.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");
    let active = source.replace("[] {} :ok", "[] :ok");
    let report = Session::default().check("postconditions.clj", &active);
    assert!(!report.complete, "{report:#?}");
    let finding = report
        .diagnostics
        .iter()
        .find(|finding| finding.code == "PNX9001" && finding.message.contains("postcondition"))
        .unwrap_or_else(|| panic!("{report:#?}"));
    let span = finding.span;
    assert_eq!(&active[span.start.offset..span.end.offset], "(true)");
}
