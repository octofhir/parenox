use parenox::{Session, Source};

#[test]
fn apply_keeps_the_selected_target_and_its_argument_context() {
    let source = r#"(ns reports (:require [clojure.spec.alpha :as s]))
        (defn choose
          {:t/sig '[boolean? :-> (s/or :number int? :text string?)]} [numeric?]
          (apply (if numeric? #(inc %) str)
                 [(if numeric? 1 "label")]))
        (defn format-number
          {:t/sig '[(s/fspec :args (s/cat :value number?) :ret string?) :-> string?]} [render]
          (render 7))
        (defn format-text
          {:t/sig '[(s/fspec :args (s/cat :value string?) :ret string?) :-> string?]} [render]
          (render "label"))
        (defn by-type {:t/sig '[boolean? :-> string?]} [numeric?]
          (apply (if numeric? format-number format-text)
                 [(fn [message]
                    (if numeric?
                      (str (inc message))
                      (subs message 1)))]))"#;
    let mut session = Session::default();
    let path = "reports.clj";
    let baseline = session.check(path, source);
    assert!(baseline.ok && baseline.complete, "{baseline:#?}");

    let edited = source.replace("(str (inc message))", "(subs message 1)");
    let report = session.check(path, &edited);
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert!(report.complete);
    assert_eq!(finding.code, "PNX2322");
    assert_eq!(finding.actual.as_deref(), Some("Number"));
    assert_eq!(
        &edited[finding.span.start.offset..finding.span.end.offset],
        "message"
    );
    assert_eq!(
        finding.callback_contract.as_ref().unwrap().name,
        "reports/format-number"
    );
}

#[test]
fn exact_apply_arguments_keep_callback_context_and_compiler_checks() {
    let source = r#"(ns reports (:require [clojure.spec.alpha :as s]))
        (defn message
          {:t/sig '(t/all [value]
            [(s/nilable {:message-fn (s/fspec :args (s/cat :text value) :ret value)})
             value :-> value])}
          [report fallback]
          (if-let [reply report] ((:message-fn reply) fallback) fallback))
        (defn absent {:t/sig '[:-> string?]} []
          (apply message [nil "label"]))
        (defn present {:t/sig '[:-> string?]} []
          (apply message [{:message-fn (fn [value] (subs value 1))} "label"]))
        (defn prefixed {:t/sig '[:-> string?]} []
          (apply message {:message-fn (fn [value] (subs value 1))} ["label"]))
        (defn nil-spread {:t/sig '[:-> string?]} []
          (apply message {:message-fn (fn [value] (subs value 1))} "label" nil))
        (defn nil-argument {:t/sig '[:-> string?]} []
          (apply message nil "label" nil))
        (defn prefix-first {:t/sig '[:-> #{:prefix}]} []
          (try
            (apply message (throw (ex-info "prefix" {:site :prefix}))
                           [(throw (ex-info "vector" {:site :vector}))])
            (catch clojure.lang.ExceptionInfo error (:site (ex-data error)))))
        (defn skipped {:t/sig '[:-> #{:eager}]} []
          (try
            (apply message {:message-fn (fn [value] (do (inc value) "skipped"))}
                           [(throw (ex-info "eager" {:site :eager}))])
            (catch clojure.lang.ExceptionInfo error (:site (ex-data error)))))"#;
    let path = "reports.clj";
    let mut session = Session::default();
    let good = session.check(path, source);
    assert!(good.ok && good.complete, "{good:#?}");

    let broken = source
        .replace("(subs value 1)", r#"(do (inc value) "bad")"#)
        .replace(r#"(do (inc value) "skipped")"#, "missing-callback-name");
    let report = session.check(path, &broken);
    assert!(report.complete, "{report:#?}");
    let [live, prefixed, nil_spread, skipped] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    for finding in [live, prefixed, nil_spread] {
        assert_eq!(finding.code, "PNX2322");
        assert_eq!(finding.actual.as_deref(), Some("String"));
        assert_eq!(
            finding.callback_contract.as_ref().unwrap().name,
            "reports/message"
        );
        assert_eq!(
            &broken[finding.span.start.offset..finding.span.end.offset],
            "value"
        );
    }
    assert_eq!(skipped.code, "PNX2304");
    assert_eq!(
        &broken[skipped.span.start.offset..skipped.span.end.offset],
        "missing-callback-name"
    );
    let cold = Session::default().check(path, &broken);
    assert_eq!(
        serde_json::to_value(&report.diagnostics).unwrap(),
        serde_json::to_value(&cold.diagnostics).unwrap()
    );
    let restored = session.check(path, source);
    assert!(restored.ok && restored.complete, "{restored:#?}");
}

#[test]
fn apply_checks_vector_metadata_before_invoking_contextual_callbacks() {
    let source = r#"(ns reports (:require [clojure.spec.alpha :as s]))
        (s/def ::render (s/fspec :args (s/cat :text string?) :ret string?))
        (defn fixed {:t/sig '[::render string? :-> string?]} [render text] (render text))
        (defn generic
          {:t/sig '(t/all [value]
            [value (s/fspec :args (s/cat :text value) :ret value) :-> value])}
          [text render] (render text))
        (defn tracked {:t/sig '[:-> string?]} []
          (apply generic "label" ^:trace [(fn [text] (subs text 1))]))
        (defn metadata-blocks {:t/sig '[:-> #{:metadata}]} []
          (try
            (apply fixed ^{:trace (throw (ex-info "metadata" {:site :metadata}))}
              [(fn [text] (do (inc text) "skipped")) "label"])
            (catch clojure.lang.ExceptionInfo error (:site (ex-data error)))))
        (defn element-first {:t/sig '[:-> #{:element}]} []
          (try
            (apply fixed ^{:trace (throw (ex-info "metadata" {:site :metadata}))}
              [(fn [text] text) (throw (ex-info "element" {:site :element}))])
            (catch clojure.lang.ExceptionInfo error (:site (ex-data error)))))"#;
    let path = "reports.clj";
    let mut session = Session::default();
    let good = session.check(path, source);
    assert!(good.ok && good.complete, "{good:#?}");
    let broken = source
        .replace("(subs text 1)", r#"(do (inc text) "bad")"#)
        .replace(r#"(do (inc text) "skipped")"#, "missing-callback-name");
    let report = session.check(path, &broken);
    assert!(report.complete, "{report:#?}");
    let [live, skipped] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(live.code, "PNX2322");
    assert_eq!(live.actual.as_deref(), Some("String"));
    assert_eq!(
        &broken[live.span.start.offset..live.span.end.offset],
        "text"
    );
    assert_eq!(skipped.code, "PNX2304");
    assert_eq!(
        &broken[skipped.span.start.offset..skipped.span.end.offset],
        "missing-callback-name"
    );
    let cold = Session::default().check(path, &broken);
    assert_eq!(
        serde_json::to_value(&report.diagnostics).unwrap(),
        serde_json::to_value(&cold.diagnostics).unwrap()
    );
    let restored = session.check(path, source);
    assert!(restored.ok && restored.complete, "{restored:#?}");
}

#[test]
fn apply_context_follows_the_standard_list_binding() {
    let source = r#"(ns reports
          (:refer-clojure :rename {list pack})
          (:require [clojure.spec.alpha :as s]))
        (defn message
          {:t/sig '(t/all [value]
            [value (s/nilable (s/fspec :args (s/cat :text value) :ret value)) :-> value])}
          [text render]
          (if-let [callback render] (callback text) text))
        (defn absent {:t/sig '[string? :-> string?]} [text]
          (apply message text (pack nil)))
        (defn present {:t/sig '[string? :-> string?]} [text]
          (apply message text (pack (fn [value] (subs value 1)))))
        (defn no-args {:t/sig '[:-> #{:local}]} [] :local)
        (defn shadowed {:t/sig '[:-> #{:local}]} []
          (let [pack (fn [_] [])] (apply no-args (pack :ignored))))"#;
    let path = "reports.clj";
    let mut session = Session::default();
    let passing = session.check(path, source);
    assert!(passing.ok && passing.complete, "{passing:#?}");

    let broken = source.replace("(subs value 1)", r#"(do (inc value) "bad")"#);
    let report = session.check(path, &broken);
    assert!(report.complete, "{report:#?}");
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(finding.code, "PNX2322");
    assert_eq!(finding.actual.as_deref(), Some("String"));
    assert_eq!(
        &broken[finding.span.start.offset..finding.span.end.offset],
        "value"
    );
    assert_eq!(
        finding.callback_contract.as_ref().unwrap().name,
        "reports/message"
    );
    let cold = Session::default().check(path, &broken);
    assert_eq!(
        serde_json::to_value(&report.diagnostics).unwrap(),
        serde_json::to_value(&cold.diagnostics).unwrap()
    );
    let restored = session.check(path, source);
    assert!(restored.ok && restored.complete, "{restored:#?}");
}

#[test]
fn concat_of_literal_vectors_supplies_written_callback_arguments() {
    let source = r#"(ns reports
          (:refer-clojure :rename {concat join})
          (:require [clojure.spec.alpha :as s]))
        (defn render
          {:t/sig '(t/all [value]
            [(s/fspec :args (s/cat :text value) :ret value) value :-> value])}
          [callback text] (callback text))
        (defn present {:t/sig '[string? :-> string?]} [text]
          (apply render (join [(fn [value] (subs value 1))] nil [text])))
        (defn skipped {:t/sig '[:-> #{:later}]} []
          (try
            (apply render (join [(fn [value] (do (inc value) "skipped"))]
                                [(throw (ex-info "later" {:site :later}))]))
            (catch clojure.lang.ExceptionInfo error (:site (ex-data error)))))
        (defn no-args {:t/sig '[:-> #{:shadowed}]} [] :shadowed)
        (defn shadowed {:t/sig '[:-> #{:shadowed}]} []
          (let [join (fn [_ _] [])] (apply no-args (join [:one] [:two]))))"#;
    let path = "reports.clj";
    let mut session = Session::default();
    let passing = session.check(path, source);
    assert!(passing.ok && passing.complete, "{passing:#?}");

    let broken = source
        .replace("(subs value 1)", r#"(do (inc value) "bad")"#)
        .replace(r#"(do (inc value) "skipped")"#, "missing-callback-name");
    let report = session.check(path, &broken);
    assert!(report.complete, "{report:#?}");
    let [live, skipped] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(live.code, "PNX2322");
    assert_eq!(live.actual.as_deref(), Some("String"));
    assert_eq!(
        &broken[live.span.start.offset..live.span.end.offset],
        "value"
    );
    assert_eq!(skipped.code, "PNX2304");
    assert_eq!(
        &broken[skipped.span.start.offset..skipped.span.end.offset],
        "missing-callback-name"
    );
    let cold = Session::default().check(path, &broken);
    assert_eq!(
        serde_json::to_value(&report.diagnostics).unwrap(),
        serde_json::to_value(&cold.diagnostics).unwrap()
    );
    let restored = session.check(path, source);
    assert!(restored.ok && restored.complete, "{restored:#?}");
}

#[test]
fn applies_str_to_realized_bounded_string_mappings() {
    let source = r#"(ns demo (:refer-clojure :rename {map each partial prefix}))
(defn suffix {:t/sig '[:-> #{" hey  end"}]} []
  (let [args ["hey" nil "end"]]
    (apply str (each (prefix str " ") args))))
(defn plain {:t/sig '[:-> #{"xy"}]} []
  (apply str (clojure.core/map str ["x" nil "y"])))
(defn empty-input {:t/sig '[:-> #{""}]} []
  (apply str (each (prefix str "ignored") nil)))
(defn capture-first {:t/sig '[:-> #{:prefix}]} []
  (try (apply str (each (prefix str (throw (ex-info "prefix" {:stage :prefix})))
                       (throw (ex-info "source" {:stage :source}))))
       (catch clojure.lang.ExceptionInfo error (:stage (ex-data error)))))
"#;
    let report = Session::default().check("demo.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn forced_string_mapping_reports_its_written_source_and_requires_ordinary_traversal() {
    let source = r#"(ns demo (:require [clojure.spec.alpha :as s]))
(defn invalid {:t/sig '[:-> string?]} []
  (apply str (map (partial str " ") 42)))
(defn custom {:t/sig '[(s/tuple #{"x"} #{"y"}) :-> string?]} [items]
  (apply str (map (partial str " ") items)))
(defn abstract-source {:t/sig '[number? :-> string?]} [items]
  (apply str (map str items)))
(defn exact-value {:t/sig '[integer? :-> string?]} [items]
  (case items 5 (apply str (map str items)) "fallback"))
(defn possible-empty {:t/sig '[boolean? :-> string?]} [empty?]
  (let [items (if empty? nil 42)]
    (apply str (map str items))))
"#;
    let report = Session::default().check("demo.clj", source);
    let mismatches: Vec<_> = report
        .diagnostics
        .iter()
        .filter(|finding| finding.code == "PNX2322")
        .collect();
    let [mismatch] = mismatches.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(
        &source[mismatch.span.start.offset..mismatch.span.end.offset],
        "42"
    );
    assert!(
        report.diagnostics.iter().any(|finding| {
            finding.code == "PNX9001"
                && &source[finding.span.start.offset..finding.span.end.offset] == "items"
        }),
        "{report:#?}"
    );
}

#[test]
fn string_mapping_stops_on_a_proven_capture_failure() {
    let source = r#"(ns demo)
(defn ignores-value {:t/sig '[number? :-> string?]} [_] "prefix")
(defn contract-only {:t/sig '[:-> string?]} []
  (apply str (map (partial str (ignores-value :wrong)) (subs 42 0))))
(defn runtime-failure {:t/sig '[:-> string?]} []
  (apply str (map (partial str (subs false 0)) (subs :unreachable 0))))
"#;
    let report = Session::default().check("demo.clj", source);
    let findings: Vec<_> = report
        .diagnostics
        .iter()
        .filter(|finding| finding.code == "PNX2322")
        .map(|finding| &source[finding.span.start.offset..finding.span.end.offset])
        .collect();
    assert_eq!(findings, [":wrong", "42", "false"], "{report:#?}");
}

#[test]
fn spreads_nil_vectors_and_exact_seqs_across_project_namespaces() {
    let input = [
        Source {
            path: "apply.clj",
            source: include_str!("../examples/apply.clj"),
        },
        Source {
            path: "provider.clj",
            source: include_str!("../examples/arities/provider.clj"),
        },
        Source {
            path: "variadic.clj",
            source: include_str!("../examples/variadic.clj"),
        },
        Source {
            path: "apply_bad.clj",
            source: include_str!("../examples/apply_bad.clj"),
        },
    ];
    let report = Session::default().check_project(&input, None);
    assert!(report.complete, "{report:#?}");
    assert!(report.files[..3].iter().all(|file| file.ok), "{report:#?}");
    let [element, arity] = report.files[3].diagnostics.as_slice() else {
        panic!("{report:#?}")
    };
    assert_eq!(element.code, "PNX2322");
    assert_eq!(
        &input[3].source[element.span.start.offset..element.span.end.offset],
        "\"3\""
    );
    assert_eq!(element.suggestions[0].replacement, "3");
    assert_eq!(arity.code, "PNX2554");
    assert!(arity.message.contains("0 or 1"), "{arity:#?}");
}

#[test]
fn checks_each_possible_exact_shape_and_protects_projected_spans() {
    let source = r#"(ns demo)
        (defn f {:t/sig '[boolean? :-> number?]} [missing?]
          (let [args (if missing? [] ["3"])]
            (apply inc args)))"#;
    let report = Session::default().check("apply.clj", source);
    assert!(report.complete, "{report:#?}");
    let [arity, element] = report.diagnostics.as_slice() else {
        panic!("{report:#?}")
    };
    assert_eq!(arity.code, "PNX2554");
    assert_eq!(element.code, "PNX2322");
    assert!(element.suggestions.is_empty(), "{element:#?}");
    assert!(element.help.as_deref().unwrap().contains("elements"));
}

#[test]
fn diagnoses_nonseqable_inputs_and_marks_open_spread_as_incomplete() {
    let source = r#"(ns demo)
        (defn bad {:t/sig '[:-> number?]} []
          (apply + true))
        (defn open {:t/sig '[(clojure.spec.alpha/coll-of number? :kind vector?) :-> number?]} [xs]
          (apply (fn [& values] (first values)) xs))"#;
    let report = Session::default().check("apply.clj", source);
    assert!(!report.complete, "{report:#?}");
    let [nonseqable, open] = report.diagnostics.as_slice() else {
        panic!("{report:#?}")
    };
    assert_eq!(nonseqable.code, "PNX2322");
    assert_eq!(open.code, "PNX9001");
    assert!(open.message.contains("cannot infer every argument count"));
}

#[test]
fn resolves_apply_and_evaluates_the_target_before_other_inputs() {
    let source = r#"(ns demo)
        (defn f {:t/sig '[:-> #{:target}]} []
          (try
            (apply (throw (ex-info "target" {:which :target}))
                   (throw (ex-info "prefix" {:which :prefix}))
                   (throw (ex-info "spread" {:which :spread})))
            (catch clojure.lang.ExceptionInfo e (:which (ex-data e)))))
        (defn g {:t/sig '[:-> number?]} []
          (let [apply (fn [a b] (+ a b))]
            (apply 1 2)))"#;
    let report = Session::default().check("apply.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn projects_fields_in_fixed_apply_arguments_through_concat() {
    let source = r#"(ns command)
(defn run-command {:t/sig '[string? {:settings {:clean {:automatic boolean?}}} :-> boolean?]}
  [uri db] (get-in db [:settings :clean :automatic]))
(defn directly-written {:t/sig '[:-> boolean?]} []
  (apply run-command "file:///🦊" [{:settings {:clean {:automatic "false"}}}]))
(defn combined {:t/sig '[:-> boolean?]} []
  (let [db {:settings {:clean {:automatic "false"}}}]
    (apply run-command "file:///🦊" (concat nil [db]))))"#;
    let report = Session::default().check("command.clj", source);
    assert!(report.complete, "{report:#?}");
    let [literal, variable] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    let written =
        |finding: &parenox::Diagnostic| &source[finding.span.start.offset..finding.span.end.offset];
    assert_eq!(written(literal), "\"false\"");
    assert_eq!(written(variable), "db");
    assert_eq!(literal.mismatch_path, variable.mismatch_path);
    assert!(literal.message.contains("Argument 2"));
    assert!(
        variable
            .notes
            .iter()
            .any(|note| note.contains(":automatic"))
    );
}

#[test]
fn checks_concat_operands_before_spreading_and_requires_ordinary_vectors() {
    let source = r#"(ns command (:require [clojure.spec.alpha :as s]))
(defn eager {:t/sig '[:-> #{:operand}]} []
  (try
    (apply (fn [_] :callee)
      (concat [:first] (throw (ex-info "late operand" {:part :operand}))))
    (catch clojure.lang.ExceptionInfo error (:part (ex-data error)))))
(defn concatenate {:t/sig '[:-> (s/and int? #{42})]} []
  (apply inc (clojure.core/concat nil [41] [])))
(defn shadowed {:t/sig '[:-> (s/and int? #{42})]} []
  (let [concat (fn [_ _] [41])]
    (apply inc (concat "raw" nil))))
(defn structural {:t/sig '[(s/tuple string?) :-> any?]} [source]
  (apply inc (concat source)))"#;
    let report = Session::default().check("command.clj", source);
    assert!(!report.complete, "{report:#?}");
    let [gap] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(gap.code, "PNX9001");
    assert_eq!(gap.severity, "warning");
    assert_eq!(
        &source[gap.span.start.offset..gap.span.end.offset],
        "source"
    );
}

#[test]
fn forwards_typed_rest_arguments_and_checks_every_reachable_overload() {
    let source = r#"
        (ns demo (:require [clojure.spec.alpha :as s]))
        (defn sum {:t/sig '[number? & number? :-> number?]} [base & more]
          (apply + base more))
        (defn total {:t/sig '[(s/coll-of number? :kind vector?) :-> number?]} [values]
          (apply sum 0 values))
        (defn describe {:t/sig '[[number? :-> string?] [number? & number? :-> number?]]}
          ([base] (str base))
          ([base & more] (apply + base more)))
        (defn from-nonempty {:t/sig '[(s/coll-of number? :kind vector?)
                                     :-> (s/nilable (s/or :label string? :total number?))]}
          [values]
          (when-let [items (seq values)] (apply describe items)))
    "#;
    let report = Session::default().check("forward.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");

    let app = r#"(ns app (:require [clojure.spec.alpha :as s]))
        (defn total {:t/sig '[(s/coll-of string? :kind vector?) :-> number?]}
          [values] (apply db/sum 0 values))"#;
    let report = Session::default().check_project(
        &[Source {
            path: "app.clj",
            source: app,
        }],
        Some(Source {
            path: "externs.edn",
            source: "{db/sum [number? & number? :-> number?]}",
        }),
    );
    assert!(report.complete, "{report:#?}");
    let [diagnostic] = report.files[0].diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(diagnostic.code, "PNX2322");
    assert_eq!(
        diagnostic.message,
        "apply collection elements: expected Number, got String"
    );
    assert_eq!(
        &app[diagnostic.span.start.offset..diagnostic.span.end.offset],
        "values"
    );
    assert!(diagnostic.suggestions.is_empty());
}

#[test]
fn open_spread_reports_a_possible_arity_and_exact_literals_stay_exact() {
    let source = r#"(ns demo (:require [clojure.spec.alpha :as s]))
        (defn fixed {:t/sig '[number? :-> number?]} [x] x)
        (defn maybe {:t/sig '[(s/coll-of number? :kind vector?) :-> number?]} [values]
          (apply fixed values))
        (defn nonempty {:t/sig '[(s/coll-of number? :kind vector?) :-> number?]} [values]
          (if-let [items (seq values)] (apply fixed items) 0))
        (defn minus {:t/sig '[(s/coll-of number? :kind vector?) :-> number?]} [values]
          (apply - values))"#;
    let report = Session::default().check("arity.clj", source);
    assert!(report.complete, "{report:#?}");
    let counts: Vec<_> = report
        .diagnostics
        .iter()
        .map(|d| (d.code.as_str(), d.message.as_str()))
        .collect();
    assert_eq!(
        counts,
        [
            (
                "PNX2554",
                "apply may pass 0 arguments, but the function expects 1"
            ),
            (
                "PNX2554",
                "apply may pass 2 arguments, but the function expects 1"
            ),
            (
                "PNX2554",
                "apply may pass 0 arguments, but the function expects at least 1"
            ),
        ]
    );

    let source = format!(
        "(ns demo) (defn f {{:t/sig '[:-> number?]}} []
           (apply + [{} \"3\"]))",
        "1 ".repeat(40)
    );
    let report = Session::default().check("literal.clj", &source);
    let [diagnostic] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(diagnostic.code, "PNX2322");
    assert_eq!(
        &source[diagnostic.span.start.offset..diagnostic.span.end.offset],
        "\"3\""
    );
    assert_eq!(diagnostic.suggestions[0].replacement, "3");
}
