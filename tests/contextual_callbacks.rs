use parenox::{DocumentHover, Session, Source};

const EXAMPLE: &str = include_str!("../examples/contextual_callbacks.clj");
const CALLBACK_MAP: &str = r#"
    (s/def ::run ::text-transform)
    (s/def ::options (s/keys :req-un [::run]))
    (defn transform-options {:t/sig '[::options string? :-> string?]} [options text]
      ((:run options) text))"#;

#[test]
fn expected_signature_types_fn_parameters_and_anonymous_arguments() {
    let report = Session::default().check("callbacks.clj", EXAMPLE);
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn conditional_callback_has_argument_context_and_branch_narrowing() {
    let source = format!(
        r#"{EXAMPLE}
        (defn selected {{:t/sig '[(s/nilable string?) :-> string?]}} [suffix]
          (transform (if (some? suffix)
                       (fn [text] (str (subs suffix 1) text))
                       #(subs % 1))
                     "label"))"#
    );
    let path = "conditional_callback.clj";
    let mut session = Session::default();
    let passing = session.check(path, &source);
    assert!(passing.ok && passing.complete, "{passing:#?}");
    let capture = source.rfind("(subs suffix 1)").unwrap() + "(subs ".len();
    let Some(DocumentHover::Local(hover)) = session.document_hover_at(path, capture) else {
        panic!("Expected a narrowed type for the captured suffix");
    };
    assert_eq!(hover.inferred_type, "String");
    let broken = source.replace(
        "(str (subs suffix 1) text)",
        "(do (inc text) (subs suffix 1))",
    );
    let report = session.check(path, &broken);
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert!(report.complete);
    assert_eq!(finding.code, "PNX2322");
    assert_eq!(finding.actual.as_deref(), Some("String"));
    assert_eq!(
        &broken[finding.span.start.offset..finding.span.end.offset],
        "text"
    );
    assert_eq!(
        finding.callback_contract.as_ref().unwrap().name,
        "demo.contextual-callbacks/transform"
    );
    let Some(DocumentHover::Local(hover)) =
        session.document_hover_at(path, finding.span.start.offset)
    else {
        panic!("Expected a type for the callback operand");
    };
    assert_eq!(hover.inferred_type, "String");
    let cold = Session::default().check(path, &broken);
    assert_eq!(
        serde_json::to_value(&report.diagnostics).unwrap(),
        serde_json::to_value(&cold.diagnostics).unwrap()
    );
    let restored = session.check(path, &source);
    assert!(restored.ok && restored.complete, "{restored:#?}");
}

#[test]
fn conditional_callback_keeps_eager_failures_and_compiler_checks() {
    let source = format!(
        r#"{EXAMPLE}
        (defn blocked {{:t/sig '[boolean? :-> #{{:argument}}]}} [first?]
          (try
            (transform (if first?
                         (fn [text] (inc text))
                         (fn [text] text))
                       (throw (ex-info "argument" {{:stage :argument}})))
            (catch clojure.lang.ExceptionInfo e (:stage (ex-data e)))))
        (defn metadata-fails {{:t/sig '[boolean? :-> (s/or :result string? :failure #{{:metadata}})]}} [first?]
          (try
            (transform (if first?
                         ^{{:trace (throw (ex-info "metadata" {{:stage :metadata}}))}}
                         (fn [text] (inc text))
                         (fn [text] text))
                       "label")
            (catch clojure.lang.ExceptionInfo _ :metadata)))"#
    );
    let mut session = Session::default();
    let passing = session.check("conditional_callback.clj", &source);
    assert!(passing.ok && passing.complete, "{passing:#?}");
    let broken = source.replacen("(inc text)", "(inc unresolved)", 1);
    let report = session.check("conditional_callback.clj", &broken);
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(finding.code, "PNX2304");
    assert_eq!(
        &broken[finding.span.start.offset..finding.span.end.offset],
        "unresolved"
    );
}

#[test]
fn conditional_callbacks_keep_ordinary_branches() {
    let source = format!(
        r#"{EXAMPLE}
        (s/def ::maybe-transform (s/nilable ::text-transform))
        (defn render {{:t/sig '[::maybe-transform :-> string?]}} [callback]
          (if-let [f callback] (f "label") "fallback"))
        (defn trim-label {{:t/sig '[string? :-> string?]}} [text] (subs text 1))
        (defn nil-fallback {{:t/sig '[(s/nilable string?) :-> string?]}} [suffix]
          (render (if (some? suffix)
                    (fn [text] (str (subs suffix 1) text))
                    nil)))
        (defn known-fallback {{:t/sig '[boolean? :-> string?]}} [trim?]
          (let [fallback #(subs % 1)]
            (transform (if trim? trim-label fallback) "label")))
        (defn caught {{:t/sig '[:-> #{{:branch}}]}} []
          (try
            (transform (if true
                         (throw (ex-info "branch" {{:stage :branch}}))
                         (fn [text] (inc text)))
                       (throw (ex-info "operand" {{:stage :operand}})))
            (catch clojure.lang.ExceptionInfo e (:stage (ex-data e)))))"#
    );
    let path = "mixed_callback.clj";
    let mut session = Session::default();
    let passing = session.check(path, &source);
    assert!(passing.ok && passing.complete, "{passing:#?}");
    let broken = source.replace(
        "(str (subs suffix 1) text)",
        "(do (inc text) (subs suffix 1))",
    );
    let report = session.check(path, &broken);
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert!(report.complete);
    assert_eq!(finding.code, "PNX2322");
    assert_eq!(finding.actual.as_deref(), Some("String"));
    assert_eq!(
        &broken[finding.span.start.offset..finding.span.end.offset],
        "text"
    );
    assert_eq!(
        finding.callback_contract.as_ref().unwrap().name,
        "demo.contextual-callbacks/render"
    );
    let cold = Session::default().check(path, &broken);
    assert_eq!(
        serde_json::to_value(&report.diagnostics).unwrap(),
        serde_json::to_value(&cold.diagnostics).unwrap()
    );
    let restored = session.check(path, &source);
    assert!(restored.ok && restored.complete, "{restored:#?}");
    let wrong_fallback =
        source.replace("(if trim? trim-label fallback)", "(if trim? nil fallback)");
    let report = session.check(path, &wrong_fallback);
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert!(report.complete);
    assert_eq!(finding.code, "PNX2322");
    assert!(finding.actual.as_deref().unwrap().contains("Nil"));
    assert_eq!(
        &wrong_fallback[finding.span.start.offset..finding.span.end.offset],
        "(if trim? nil fallback)"
    );
}

#[test]
fn conditional_callback_fields_can_have_an_implicit_nil_branch() {
    let source = r#"(ns reports (:refer-clojure :rename {when when-ready})
                       (:require [clojure.spec.alpha :as s]))
        (s/def ::message-fn (s/nilable (s/fspec :args (s/cat :text string?) :ret string?)))
        (defn render {:t/sig '[{:message-fn ::message-fn} :-> string?]} [report]
          (if-let [f (:message-fn report)] (f "label") "fallback"))
        (defn raw {:t/sig '[(s/nilable string?) :-> string?]} [prefix]
          (render {:message-fn (if (some? prefix) (fn [text] (str (subs prefix 1) text)))}))
        (defn renamed {:t/sig '[(s/nilable string?) :-> string?]} [prefix]
          (render {:message-fn (when-ready (some? prefix) (fn [label] (str (subs prefix 1) label)))}))
        (defn negative {:t/sig '[(s/nilable string?) :-> string?]} [prefix]
          (render {:message-fn (when-not (nil? prefix) #(str (subs prefix 1) %))}))
        (defn inverted {:t/sig '[(s/nilable string?) :-> string?]} [prefix]
          (render {:message-fn (if-not (nil? prefix) #(str (subs prefix 1) %))}))"#;
    let path = "reports.clj";
    let mut session = Session::default();
    let passing = session.check(path, source);
    assert!(passing.ok && passing.complete, "{passing:#?}");

    let edited = source.replace(
        "(str (subs prefix 1) label)",
        "(do (inc label) (subs prefix 1))",
    );
    let warm = session.check(path, &edited);
    let [finding] = warm.diagnostics.as_slice() else {
        panic!("{warm:#?}");
    };
    assert!(warm.complete);
    assert_eq!(finding.code, "PNX2322");
    assert_eq!(finding.actual.as_deref(), Some("String"));
    assert_eq!(
        &edited[finding.span.start.offset..finding.span.end.offset],
        "label"
    );
    assert_eq!(
        finding.callback_contract.as_ref().unwrap().name,
        "reports/render"
    );
    let cold = Session::default().check(path, &edited);
    assert_eq!(
        serde_json::to_value(&warm.diagnostics).unwrap(),
        serde_json::to_value(&cold.diagnostics).unwrap()
    );
    let restored = session.check(path, source);
    assert!(restored.ok && restored.complete, "{restored:#?}");

    let shadowed = format!(
        r#"{source}
        (defn shadowed {{:t/sig '[:-> string?]}} []
          (let [when-ready (fn [_ _] "different operation")]
            (render {{:message-fn (when-ready true (fn [text] (inc text)))}})))"#
    );
    let report = session.check(path, &shadowed);
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.code == "PNX9001" && d.message.contains("fn is currently supported")),
        "{report:#?}"
    );
    assert!(
        report
            .diagnostics
            .iter()
            .all(|d| d.callback_contract.is_none()),
        "{report:#?}"
    );
}

#[test]
fn binding_conditional_callbacks_keep_their_local_scope_and_nil_rules() {
    let source = r#"(ns reports (:require [clojure.spec.alpha :as s]))
        (s/def ::message-fn (s/nilable (s/fspec :args (s/cat :text string?) :ret string?)))
        (defn render {:t/sig '[{:message-fn ::message-fn} :-> string?]} [report]
          (if-let [f (:message-fn report)] (f "label") "fallback"))
        (defn choose {:t/sig '[{:prefix (s/nilable string?)} :-> string?]} [view]
          (render {:message-fn (when-let [prefix (:prefix view)]
                                (fn [text] (str (subs prefix 1) text)))}))
        (defn choose-else {:t/sig '[{:prefix (s/nilable string?)} :-> string?]} [view]
          (render {:message-fn (if-let [prefix (:prefix view)]
                                (fn [text] (str prefix text))
                                (fn [text] (subs text 1)))}))
        (defn falsey {:t/sig '[:-> string?]} []
          (render {:message-fn (when-let [flag false] (fn [text] (do (inc text) text)))}))
        (defn non-nil {:t/sig '[:-> string?]} []
          (render {:message-fn (when-some [flag false] (fn [text] (str flag text)))}))
        (defn choose-false {:t/sig '[:-> string?]} []
          (render {:message-fn (if-some [flag false] #(str flag %) nil)}))
        (defn from-first {:t/sig '[:-> string?]} []
          (render {:message-fn (when-first [prefix ["root"]] #(str (subs prefix 1) %))}))"#;
    let path = "reports.clj";
    let mut session = Session::default();
    let passing = session.check(path, source);
    assert!(passing.ok && passing.complete, "{passing:#?}");

    let edited = source.replace("(str flag text)", "(do (inc text) (str flag text))");
    let warm = session.check(path, &edited);
    let [finding] = warm.diagnostics.as_slice() else {
        panic!("{warm:#?}");
    };
    assert!(warm.complete);
    assert_eq!(finding.code, "PNX2322");
    assert_eq!(finding.actual.as_deref(), Some("String"));
    assert_eq!(
        &edited[finding.span.start.offset..finding.span.end.offset],
        "text"
    );
    assert_eq!(
        finding.callback_contract.as_ref().unwrap().name,
        "reports/render"
    );
    let cold = Session::default().check(path, &edited);
    assert_eq!(
        serde_json::to_value(&warm.diagnostics).unwrap(),
        serde_json::to_value(&cold.diagnostics).unwrap()
    );
    let restored = session.check(path, source);
    assert!(restored.ok && restored.complete, "{restored:#?}");

    let out_of_scope = source.replace("(fn [text] (subs text 1))", "(fn [text] (subs prefix 1))");
    let report = session.check(path, &out_of_scope);
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(finding.code, "PNX2304");
    assert_eq!(
        &out_of_scope[finding.span.start.offset..finding.span.end.offset],
        "prefix"
    );
}

#[test]
fn binding_conditional_checks_a_shadowed_constructor_before_later_operands() {
    let source = r#"(ns reports (:require [clojure.spec.alpha :as s]))
        (s/def ::callback (s/nilable (s/fspec :args (s/cat :text string?) :ret string?)))
        (defn render {:t/sig '[::callback string? :-> string?]} [callback text]
          (if-let [f callback] (f text) text))
        (defn replacement {:t/sig '[(s/coll-of t/any :kind vector?) number? :-> string?]}
          [_ n] (str (inc n)))
        (defn eager {:t/sig '[:-> #{:later}]} []
          (try
            (render (when-let [fn replacement] (fn [] "wrong-type"))
                    (throw (ex-info "later" {})))
            (catch clojure.lang.ExceptionInfo _ :later)))"#;
    let report = Session::default().check("reports.clj", source);
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert!(report.complete);
    assert_eq!(finding.code, "PNX2322");
    assert_eq!(finding.actual.as_deref(), Some("String"));
    assert_eq!(
        &source[finding.span.start.offset..finding.span.end.offset],
        "\"wrong-type\""
    );
    assert!(finding.callback_contract.is_none());
}

#[test]
fn a_throwing_callback_binding_prevents_evaluation_of_later_operands() {
    let source = r#"(ns reports (:require [clojure.spec.alpha :as s]))
        (s/def ::message-fn (s/nilable (s/fspec :args (s/cat :text string?) :ret string?)))
        (defn render {:t/sig '[{:message-fn ::message-fn} number? :-> string?]} [report _]
          (if-let [callback (:message-fn report)] (callback "label") "fallback"))
        (defn apply-report {:t/sig '[:-> string?]} []
          (try
            (render {:message-fn (when-let [{:keys [prefix]
                                            :or {prefix (throw (ex-info "binding" {}))}} {}]
                                   (fn [text] (str prefix text)))}
                    (inc "later"))
            (catch clojure.lang.ExceptionInfo _ "fallback")))"#;
    let mut session = Session::default();
    let report = session.check("reports.clj", source);
    assert!(report.ok && report.complete, "{report:#?}");

    let edited = source.replace(r#"(throw (ex-info "binding" {}))"#, r#""prefix""#);
    let report = session.check("reports.clj", &edited);
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert!(report.complete);
    assert_eq!(finding.code, "PNX2322");
    assert_eq!(
        &edited[finding.span.start.offset..finding.span.end.offset],
        "\"later\""
    );

    let unbound = source.replace("(inc \"later\")", "(inc unresolved)");
    let report = session.check("reports.clj", &unbound);
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(finding.code, "PNX2304");
    assert_eq!(
        &unbound[finding.span.start.offset..finding.span.end.offset],
        "unresolved"
    );
}

#[test]
fn contextual_callbacks_can_end_a_sequence_of_branch_forms() {
    let source = r#"(ns reports (:require [clojure.spec.alpha :as s]))
        (s/def ::message-fn (s/nilable (s/fspec :args (s/cat :text string?) :ret string?)))
        (defn render {:t/sig '[{:message-fn ::message-fn} number? :-> string?]} [report _]
          (if-let [callback (:message-fn report)] (callback "label") "fallback"))
        (defn select {:t/sig '[(s/nilable string?) :-> string?]} [optional]
          (render {:message-fn (when-let [prefix optional]
                                 (subs prefix 1)
                                 (fn [message] (str prefix (subs message 1))))}
                  (inc 0)))
        (defn select-if-ready {:t/sig '[(s/nilable string?) :-> string?]} [prefix]
          (render {:message-fn (when-not (nil? prefix)
                                 (do (subs prefix 1) #(str prefix %)))}
                  0))"#;
    let mut session = Session::default();
    let path = "reports.clj";
    let report = session.check(path, source);
    assert!(report.ok && report.complete, "{report:#?}");

    let edited = source.replacen("(subs message 1)", "(inc message)", 1);
    let report = session.check(path, &edited);
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert!(report.complete);
    assert_eq!(finding.code, "PNX2322");
    assert_eq!(finding.actual.as_deref(), Some("String"));
    assert_eq!(
        &edited[finding.span.start.offset..finding.span.end.offset],
        "message"
    );

    let throwing = source
        .replace("[prefix optional]", "[prefix \"root\"]")
        .replacen("(subs prefix 1)", "(throw (ex-info \"entered\" {}))", 1)
        .replace("(inc 0)", "(inc \"later\")");
    let report = session.check(path, &throwing);
    assert!(report.ok && report.complete, "{report:#?}");

    let unresolved = throwing.replace("(subs message 1)", "(subs missing 1)");
    let report = session.check(path, &unresolved);
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(finding.code, "PNX2304");
    assert_eq!(
        &unresolved[finding.span.start.offset..finding.span.end.offset],
        "missing"
    );
}

#[test]
fn do_sequences_return_contextual_callbacks_after_their_eager_forms() {
    let source = r#"(ns reports (:require [clojure.spec.alpha :as s]))
        (s/def ::message-fn (s/fspec :args (s/cat :text string?) :ret string?))
        (defn render {:t/sig '[::message-fn number? :-> string?]} [callback _]
          (callback "label"))
        (defn produce {:t/sig '[string? :-> string?]} [prefix]
          (let [do (fn [_] "local")]
            (render (do (subs prefix 1)
                        (do (inc 1)
                            (fn [message] (str prefix (subs message 1)))))
                    (inc 2))))"#;
    let mut session = Session::default();
    let path = "reports.clj";
    let report = session.check(path, source);
    assert!(report.ok && report.complete, "{report:#?}");

    let edited = source.replace("(subs message 1)", "(inc message)");
    let report = session.check(path, &edited);
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert!(report.complete);
    assert_eq!(finding.code, "PNX2322");
    assert_eq!(
        &edited[finding.span.start.offset..finding.span.end.offset],
        "message"
    );

    let throwing = source
        .replace("(subs prefix 1)", "(throw (ex-info \"entered\" {}))")
        .replace("(inc 1)", "(inc \"inner\")")
        .replace("(inc 2)", "(inc \"later\")");
    let report = session.check(path, &throwing);
    assert!(report.ok && report.complete, "{report:#?}");

    let unresolved = throwing.replace("(inc \"inner\")", "(inc unresolved)");
    let report = session.check(path, &unresolved);
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(finding.code, "PNX2304");
    assert_eq!(
        &unresolved[finding.span.start.offset..finding.span.end.offset],
        "unresolved"
    );
}

#[test]
fn let_tail_callbacks_capture_evaluated_bindings() {
    let source = r#"(ns reports (:refer-clojure :rename {let bind})
        (:require [clojure.spec.alpha :as s]))
        (s/def ::message-fn (s/fspec :args (s/cat :text string?) :ret string?))
        (defn render {:t/sig '[::message-fn number? :-> string?]} [callback _]
          (callback "label"))
        (defn prepare {:t/sig '[:-> string?]} []
          (render (bind [{:keys [prefix] :or {prefix "root"}} {}
                         suffix (subs prefix 1)]
                    (count suffix)
                    (fn [message] (str suffix (subs message 1))))
                  (inc 2)))
        (defn primitive {:t/sig '[string? :-> string?]} [prefix]
          (render (let* [suffix (subs prefix 1)] #(str suffix %)) 0))"#;
    let mut session = Session::default();
    let path = "reports.clj";
    let report = session.check(path, source);
    assert!(report.ok && report.complete, "{report:#?}");

    let edited = source.replace("(subs message 1)", "(inc message)");
    let report = session.check(path, &edited);
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert!(report.complete);
    assert_eq!(finding.code, "PNX2322");
    assert_eq!(
        &edited[finding.span.start.offset..finding.span.end.offset],
        "message"
    );

    let throwing = source
        .replace("prefix \"root\"", "prefix (throw (ex-info \"default\" {}))")
        .replace("}} {}\n", "}} (inc \"initializer\")\n")
        .replace("(inc 2)", "(inc \"later\")");
    let report = session.check(path, &throwing);
    assert!(report.ok && report.complete, "{report:#?}");

    let unresolved = throwing.replace("(count suffix)", "(count missing)");
    let report = session.check(path, &unresolved);
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(finding.code, "PNX2304");
    assert_eq!(
        &unresolved[finding.span.start.offset..finding.span.end.offset],
        "missing"
    );

    let broken_helper = source
        .replacen(
            "suffix (subs prefix 1)]",
            "suffix (subs prefix 1) helper (fn [] (inc missing))]",
            1,
        )
        .replace(
            "(str suffix (subs message 1))",
            "(str suffix (subs message 1) (helper))",
        );
    let report = session.check(path, &broken_helper);
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(finding.code, "PNX2304");
    assert_eq!(
        &broken_helper[finding.span.start.offset..finding.span.end.offset],
        "missing"
    );
}

#[test]
fn returning_a_let_bound_callback_keeps_its_definition_scope() {
    let source = r#"(ns reports (:require [clojure.spec.alpha :as s]))
        (s/def ::message-fn (s/fspec :args (s/cat :text string?) :ret string?))
        (defn render {:t/sig '[::message-fn :-> string?]} [callback]
          (callback "label"))
        (defn select {:t/sig '[string? :-> string?]} [prefix]
          (render (let [suffix (subs prefix 1)
                        callback (fn [message] (str (subs suffix 1) (subs message 1)))
                        forwarded callback
                        suffix 7]
                    (do (inc suffix) forwarded))))"#;
    let mut session = Session::default();
    let path = "reports.clj";
    let report = session.check(path, source);
    assert!(report.ok && report.complete, "{report:#?}");

    for (body, code, operand) in [
        ("(inc message)", "PNX2322", "message"),
        ("(subs missing 1)", "PNX2304", "missing"),
    ] {
        let edited = source.replace("(subs message 1)", body);
        let report = session.check(path, &edited);
        let [finding] = report.diagnostics.as_slice() else {
            panic!("{report:#?}");
        };
        assert_eq!(finding.code, code);
        assert_eq!(
            &edited[finding.span.start.offset..finding.span.end.offset],
            operand
        );
    }

    let shadowed = source.replace("forwarded callback", "forwarded callback forwarded 7");
    let report = session.check(path, &shadowed);
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert!(report.complete);
    assert_eq!(finding.code, "PNX2322");
    assert!(shadowed[finding.span.start.offset..finding.span.end.offset].starts_with("(let ["));
    assert!(finding.callback_contract.is_none());
}

#[test]
fn nilable_callback_contracts_contextualize_written_closures() {
    let source = r#"(ns reports (:require [clojure.spec.alpha :as s]))
        (s/def ::render (s/nilable (s/fspec :args (s/cat :text string?) :ret string?)))
        (defn message {:t/sig '[::render :-> string?]} [render]
          (if-let [callback render] (callback "label") "fallback"))
        (defn from-map {:t/sig '[{:message-fn ::render} :-> string?]} [report]
          (message (:message-fn report)))
        (defn forward {:t/sig '(t/all [value] [value ::render :-> value])} [value render]
          (message render)
          value)
        (defn without-closure {:t/sig '[:-> string?]} []
          (str (message nil) (from-map {:message-fn nil})))
        (defn direct {:t/sig '[:-> string?]} []
          (message (fn [text] (subs text 1))))
        (defn mapped {:t/sig '[:-> string?]} []
          (from-map {:message-fn (fn [text] (subs text 1))}))
        (defn generic {:t/sig '[:-> number?]} []
          (forward 42 (fn [text] (subs text 1))))"#;
    let path = "reports.clj";
    let mut session = Session::default();
    let passing = session.check(path, source);
    assert!(passing.ok && passing.complete, "{passing:#?}");

    let broken = source.replace("(subs text 1)", r#"(do (inc text) "bad")"#);
    let report = session.check(path, &broken);
    assert!(report.complete, "{report:#?}");
    assert_eq!(report.diagnostics.len(), 3, "{report:#?}");
    for (finding, contract) in
        report
            .diagnostics
            .iter()
            .zip(["reports/message", "reports/from-map", "reports/forward"])
    {
        assert_eq!(finding.code, "PNX2322");
        assert_eq!(finding.actual.as_deref(), Some("String"));
        assert_eq!(finding.callback_contract.as_ref().unwrap().name, contract);
        assert_eq!(
            &broken[finding.span.start.offset..finding.span.end.offset],
            "text"
        );
        let Some(DocumentHover::Local(hover)) =
            session.document_hover_at(path, finding.span.start.offset)
        else {
            panic!("Expected a local type at the callback operand");
        };
        assert_eq!(hover.inferred_type, "String");
    }
    let cold = Session::default().check(path, &broken);
    assert_eq!(
        serde_json::to_value(&report.diagnostics).unwrap(),
        serde_json::to_value(&cold.diagnostics).unwrap()
    );
    let restored = session.check(path, source);
    assert!(restored.ok && restored.complete, "{restored:#?}");
}

#[test]
fn nilable_maps_keep_context_for_their_callback_fields() {
    let source = r#"(ns reports (:require [clojure.spec.alpha :as s]))
        (s/def ::message-fn (s/fspec :args (s/cat :text string?) :ret string?))
        (s/def ::report (s/nilable (s/keys :req-un [::message-fn])))
        (defn message {:t/sig '[::report :-> string?]} [report]
          (if-let [reply report] ((:message-fn reply) "label") "fallback"))
        (defn forward {:t/sig '(t/all [value] [value {:report ::report} :-> value])}
          [value options] (message (:report options)) value)
        (defn absent {:t/sig '[:-> number?]} []
          (message nil)
          (forward 42 {:report nil}))
        (defn direct {:t/sig '[:-> string?]} []
          (message {:message-fn (fn [text] (subs text 1))}))
        (defn nested {:t/sig '[:-> number?]} []
          (forward 42 {:report {:message-fn (fn [text] (subs text 1))}}))"#;
    let path = "reports.clj";
    let mut session = Session::default();
    let passing = session.check(path, source);
    assert!(passing.ok && passing.complete, "{passing:#?}");
    let broken = source.replace("(subs text 1)", r#"(do (inc text) "bad")"#);
    let report = session.check(path, &broken);
    assert!(report.complete, "{report:#?}");
    let [direct, nested] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    for (finding, name) in [(direct, "reports/message"), (nested, "reports/forward")] {
        assert_eq!(finding.code, "PNX2322");
        assert_eq!(finding.actual.as_deref(), Some("String"));
        assert_eq!(finding.callback_contract.as_ref().unwrap().name, name);
        assert_eq!(
            &broken[finding.span.start.offset..finding.span.end.offset],
            "text"
        );
    }
    let cold = Session::default().check(path, &broken);
    assert_eq!(
        serde_json::to_value(&report.diagnostics).unwrap(),
        serde_json::to_value(&cold.diagnostics).unwrap()
    );
    let restored = session.check(path, source);
    assert!(restored.ok && restored.complete, "{restored:#?}");
}

#[test]
fn conditional_callbacks_keep_context_inside_nested_map_fields() {
    let source = format!(
        r#"{EXAMPLE}
        (s/def ::message-fn ::text-transform)
        (s/def ::report (s/keys :req-un [::message-fn]))
        (defn render {{:t/sig '[{{:report ::report}} :-> string?]}} [options]
          ((:message-fn (:report options)) "label"))
        (defn choose {{:t/sig '[(s/nilable string?) :-> string?]}} [prefix]
          (render {{:report {{(if true :message-fn :other)
                            (if (some? prefix)
                              (fn [text] (str (subs prefix 1) text))
                              #(subs % 1))}}}}))"#
    );
    let path = "conditional_fields.clj";
    let mut session = Session::default();
    let passing = session.check(path, &source);
    assert!(passing.ok && passing.complete, "{passing:#?}");
    let edited = source.replace(
        "(str (subs prefix 1) text)",
        "(do (inc text) (subs prefix 1))",
    );
    let report = session.check(path, &edited);
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert!(report.complete);
    assert_eq!(finding.code, "PNX2322");
    assert_eq!(finding.actual.as_deref(), Some("String"));
    assert_eq!(
        &edited[finding.span.start.offset..finding.span.end.offset],
        "text"
    );
    assert_eq!(
        finding.callback_contract.as_ref().unwrap().name,
        "demo.contextual-callbacks/render"
    );
    let cold = Session::default().check(path, &edited);
    assert_eq!(
        serde_json::to_value(&report.diagnostics).unwrap(),
        serde_json::to_value(&cold.diagnostics).unwrap()
    );
    let reverted = session.check(path, &source);
    assert!(reverted.ok && reverted.complete, "{reverted:#?}");
}

#[test]
fn callback_argument_use_and_return_errors_point_inside_the_body() {
    let source = format!(
        "{EXAMPLE}
         (defn wrong-argument {{:t/sig '[:-> string?]}} []
           (transform (fn [text] (do (inc text) \"ok\")) \"label\"))
         (defn wrong-result {{:t/sig '[:-> string?]}} []
           (transform (fn [text] 42) \"label\"))"
    );
    let report = Session::default().check("callbacks.clj", &source);
    assert!(!report.ok && report.complete, "{report:#?}");
    let [argument, result] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(argument.code, "PNX2322");
    assert_eq!(argument.actual.as_deref(), Some("String"));
    assert_eq!(
        &source[argument.span.start.offset..argument.span.end.offset],
        "text"
    );
    assert_eq!(result.code, "PNX2322");
    assert_eq!(result.actual.as_deref(), Some("Number"));
    assert_eq!(result.expected.as_deref(), Some("String"));
    assert_eq!(
        &source[result.span.start.offset..result.span.end.offset],
        "42"
    );
    assert_eq!(
        result.contract.as_ref().unwrap().name,
        "demo.contextual-callbacks/transform"
    );
}

#[test]
fn contextual_types_check_arity_and_named_function_variance() {
    let source = format!(
        "{EXAMPLE}
         (defn broad {{:t/sig '[(s/nilable string?) :-> #{{\"ok\"}}]}} [x] \"ok\")
         (defn narrow {{:t/sig '[#{{\"literal\"}} :-> string?]}} [x] x)
         (defn ok-use {{:t/sig '[:-> string?]}} [] (transform broad \"label\"))
         (defn bad-use {{:t/sig '[:-> string?]}} [] (transform narrow \"label\"))
         (defn bad-arity {{:t/sig '[:-> string?]}} []
           (transform (fn [x y] \"bad\") \"label\"))"
    );
    let report = Session::default().check("callbacks.clj", &source);
    assert!(!report.ok && report.complete, "{report:#?}");
    let [types, arity] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(types.code, "PNX2322");
    assert_eq!(
        &source[types.span.start.offset..types.span.end.offset],
        "narrow"
    );
    assert_eq!(arity.code, "PNX2554");
    assert_eq!(
        &source[arity.span.start.offset..arity.span.end.offset],
        "(fn [x y] \"bad\")"
    );
    assert_eq!(
        arity.contract.as_ref().unwrap().name,
        "demo.contextual-callbacks/transform"
    );
}

#[test]
fn the_higher_order_function_must_call_its_parameter_with_the_declared_types() {
    let source = format!(
        "{EXAMPLE}
         (defn bad-call {{:t/sig '[::text-transform :-> string?]}} [f] (f 42))"
    );
    let report = Session::default().check("callbacks.clj", &source);
    assert_eq!(report.diagnostics.len(), 1, "{report:#?}");
    let diagnostic = &report.diagnostics[0];
    assert_eq!(diagnostic.code, "PNX2322");
    assert_eq!(diagnostic.expected.as_deref(), Some("String"));
    assert_eq!(
        &source[diagnostic.span.start.offset..diagnostic.span.end.offset],
        "42"
    );
}

#[test]
fn an_unsupported_fspec_option_is_not_silently_dropped() {
    let source = EXAMPLE.replace(
        "(s/fspec :args (s/cat :text string?) :ret string?)",
        "(s/fspec :args (s/cat :text string?) :ret string? :fn identity)",
    );
    let report = Session::default().check("callbacks.clj", &source);
    assert!(!report.ok && !report.complete, "{report:#?}");
    assert!(report.diagnostics.iter().any(|d| {
        d.code == "PNX9001"
            && d.message
                .contains("Only :args and :ret are supported in s/fspec")
    }));
}

#[test]
fn checking_a_callback_does_not_execute_it_during_argument_evaluation() {
    let source = format!(
        "{EXAMPLE}
         (defn catch-priority {{:t/sig '[:-> #{{:argument}}]}} []
           (try
             (transform
               (fn [x] (throw (ex-info \"deferred\" {{:stage :callback}})))
               (throw (ex-info \"eager\" {{:stage :argument}})))
             (catch clojure.lang.ExceptionInfo e (:stage (ex-data e)))))"
    );
    let report = Session::default().check("callbacks.clj", &source);
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn contextual_map_fields_type_callbacks_at_their_written_uses() {
    let source = format!(
        "{EXAMPLE}{CALLBACK_MAP}
         (defn exact {{:t/sig '[:-> string?]}} []
           (transform-options {{:run #(str % \"!\")}} \"label\"))
         (defn invalid {{:t/sig '[:-> string?]}} []
           (transform-options {{:run (fn [text] (do (inc text) \"ok\"))}} \"label\"))"
    );
    let mut session = Session::default();
    let report = session.check("callback_map.clj", &source);
    assert!(report.complete, "{report:#?}");
    let [error] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(error.code, "PNX2322");
    assert_eq!(error.actual.as_deref(), Some("String"));
    assert_eq!(
        error.callback_contract.as_ref().unwrap().name,
        "demo.contextual-callbacks/transform-options"
    );
    assert_eq!(
        &source[error.span.start.offset..error.span.end.offset],
        "text"
    );
    let Some(DocumentHover::Local(local)) =
        session.document_hover_at("callback_map.clj", error.span.start.offset)
    else {
        panic!("Expected a type at the callback operand");
    };
    assert_eq!(local.inferred_type, "String");
}

#[test]
fn exact_computed_map_keys_select_callback_contracts() {
    let source = format!(
        r#"{EXAMPLE}{CALLBACK_MAP}
         (defn with-call-key {{:t/sig '[:-> string?]}} []
           (transform-options {{(identity :run) (fn [text] (subs text 1))}} "input"))
         (defn with-local-key {{:t/sig '[:-> string?]}} []
           (let [key :run]
             (transform-options {{key (fn [text] (subs text 1))}} "input")))"#
    );
    let path = "computed_callback.clj";
    let mut session = Session::default();
    let report = session.check(path, &source);
    assert!(report.ok && report.complete, "{report:#?}");

    let broken = source.replacen("(subs text 1)", r#"(do (inc text) "ok")"#, 1);
    let report = session.check(path, &broken);
    let [finding] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(finding.code, "PNX2322");
    assert_eq!(finding.actual.as_deref(), Some("String"));
    assert_eq!(
        &broken[finding.span.start.offset..finding.span.end.offset],
        "text"
    );
    assert_eq!(
        finding.callback_contract.as_ref().unwrap().name,
        "demo.contextual-callbacks/transform-options"
    );
    let cold = Session::default().check(path, &broken);
    assert_eq!(
        serde_json::to_value(&report.diagnostics).unwrap(),
        serde_json::to_value(&cold.diagnostics).unwrap()
    );
    let rolled_back = session.check(path, &source);
    assert!(rolled_back.ok && rolled_back.complete, "{rolled_back:#?}");
}

#[test]
fn later_runtime_failure_keeps_map_callback_body_in_compiler_checks() {
    let source = format!(
        "{EXAMPLE}{CALLBACK_MAP}
         (defn caught {{:t/sig '[:-> #{{:argument}}]}} []
           (try
             (transform-options {{:run (fn [text] (inc text))}}
                                (throw (ex-info \"eager\" {{:stage :argument}})))
             (catch clojure.lang.ExceptionInfo e (:stage (ex-data e)))))"
    );
    let mut session = Session::default();
    let report = session.check("callback_map.clj", &source);
    assert!(report.ok && report.complete, "{report:#?}");
    let unbound = source.replace("(inc text)", "(inc unresolved)");
    let report = session.check("callback_map.clj", &unbound);
    let [error] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(error.code, "PNX2304");
    assert_eq!(
        &unbound[error.span.start.offset..error.span.end.offset],
        "unresolved"
    );
}

#[test]
fn maps_with_multiple_callbacks_preserve_each_fields_context() {
    let source = format!(
        r#"{EXAMPLE}
        (s/def ::run ::text-transform)
        (s/def ::audit (s/fspec :args (s/cat :amount int?) :ret string?))
        (s/def ::label string?)
        (s/def ::options (s/keys :req-un [::run ::audit ::label]))
        (defn transform-options {{:t/sig '[::options :-> string?]}} [options]
          ((:run options) ((:audit options) 1)))
        (defn valid {{:t/sig '[:-> string?]}} []
          (transform-options ^:tracked {{:label "ordinary"
                                         :run #(str/upper-case %)
                                         :audit #(str (inc %))}}))
        (defn invalid {{:t/sig '[:-> string?]}} []
          (transform-options ^:tracked {{:run (fn [text] (do (inc text) "ok"))
                                         :label "ordinary"
                                         :audit (fn [amount] (do (subs amount 1) "ok"))}}))"#
    );
    let mut session = Session::default();
    let report = session.check("options.clj", &source);
    assert!(report.complete, "{report:#?}");
    let findings: Vec<_> = report
        .diagnostics
        .iter()
        .map(|error| {
            let span = error.span;
            assert_eq!(
                error.callback_contract.as_ref().unwrap().name,
                "demo.contextual-callbacks/transform-options"
            );
            (
                error.code.as_str(),
                &source[span.start.offset..span.end.offset],
            )
        })
        .collect();
    assert_eq!(findings, [("PNX2322", "text"), ("PNX2322", "amount")]);
}

#[test]
fn map_fields_and_map_metadata_are_eager_but_callback_bodies_are_not() {
    let source = format!(
        r#"{EXAMPLE}{CALLBACK_MAP}
        (defn stops-in-field {{:t/sig '[:-> #{{:field}}]}} []
          (try
            (transform-options
              ^{{:marker (throw (ex-info "meta" {{:stage :meta}}))}}
              {{:run (fn [text] (inc text))
                :tag (throw (ex-info "field" {{:stage :field}}))}}
              (throw (ex-info "argument" {{:stage :argument}})))
            (catch clojure.lang.ExceptionInfo e (:stage (ex-data e)))))
        (defn stops-in-metadata {{:t/sig '[:-> #{{:meta}}]}} []
          (try
            (transform-options
              ^{{:marker (throw (ex-info "meta" {{:stage :meta}}))}}
              {{:tag :ordinary :run #(inc %)}}
              (throw (ex-info "argument" {{:stage :argument}})))
            (catch clojure.lang.ExceptionInfo e (:stage (ex-data e)))))
        (defn ordinary-map {{:t/sig '[:-> #{{:first}}]}} []
          (try
            ^{{:marker (throw (ex-info "meta" {{:stage :meta}}))}}
            {{:first (throw (ex-info "first" {{:stage :first}}))
              :second (throw (ex-info "second" {{:stage :second}}))}}
            (catch clojure.lang.ExceptionInfo e (:stage (ex-data e)))))"#
    );
    let mut session = Session::default();
    let report = session.check("options.clj", &source);
    assert!(report.ok && report.complete, "{report:#?}");
    let broken = source.replace("(inc text)", "(inc unresolved)");
    let report = session.check("options.clj", &broken);
    let [error] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(error.code, "PNX2304");
    assert_eq!(
        &broken[error.span.start.offset..error.span.end.offset],
        "unresolved"
    );
}

#[test]
fn contract_edits_refresh_multi_field_callback_types_and_rollback() {
    let provider = r#"
        (ns api (:require [clojure.spec.alpha :as s]))
        (s/def ::run (s/fspec :args (s/cat :value string?) :ret string?))
        (s/def ::tag keyword?)
        (s/def ::options (s/keys :req-un [::run ::tag]))
        (defn use-options {:t/sig '[::options :-> string?]} [options]
          ((:run options) "label"))"#;
    let client = Source {
        path: "client.clj",
        source: r#"(ns client (:require [api :as api]))
         (defn run {:t/sig '[:-> string?]} []
           (api/use-options {:tag :example :run (fn [value] (do (inc value) "ok"))}))"#,
    };
    let check = |session: &mut Session, source| {
        session.check_project(
            &[
                client,
                Source {
                    path: "api.clj",
                    source,
                },
            ],
            None,
        )
    };
    let mut session = Session::default();
    let before = check(&mut session, provider);
    let edited = provider
        .replace(":value string?", ":value int?")
        .replace("\"label\"", "1");
    let changed = check(&mut session, &edited);
    assert!(changed.ok && changed.complete, "{changed:#?}");
    let after = check(&mut session, provider);
    let diagnostics = |report: &parenox::ProjectReport| {
        serde_json::to_value(
            report
                .files
                .iter()
                .flat_map(|file| &file.diagnostics)
                .collect::<Vec<_>>(),
        )
        .unwrap()
    };
    assert!(!before.ok && before.complete, "{before:#?}");
    assert_eq!(diagnostics(&before), diagnostics(&after));
    assert_eq!(
        diagnostics(&after),
        diagnostics(&check(&mut Session::default(), provider))
    );
}

#[test]
fn externs_can_supply_context_and_edits_recheck_only_consumers() {
    let app = Source {
        path: "app.clj",
        source: "(ns app (:require [vendor :as v]))
           (defn invoke {:t/sig '[:-> string?]} []
             (v/transform #(clojure.string/upper-case %)))
           (defn independent {:t/sig '[:-> number?]} [] 1)",
    };
    let externs = Source {
        path: "externs.edn",
        source: "{vendor/transform
          [(clojure.spec.alpha/fspec :args (clojure.spec.alpha/cat :x string?) :ret string?)
           :-> string?]}",
    };
    let mut session = Session::default();
    let first = session.check_project(&[app], Some(externs));
    assert!(first.ok && first.complete, "{first:#?}");
    let bad = externs
        .source
        .replace(":x string?", ":x (clojure.spec.alpha/nilable string?)");
    let edited = Source {
        source: &bad,
        ..externs
    };
    let after = session.check_project(&[app], Some(edited));
    assert_eq!(after.files[0].stats.checked_functions, 1);
    assert_eq!(after.files[0].stats.reused_functions, 1);
    assert_eq!(after.files[0].diagnostics.len(), 1, "{after:#?}");
    assert_eq!(
        after.files[0].diagnostics[0].actual.as_deref(),
        Some("Nil | String")
    );
    let cold = Session::default().check_project(&[app], Some(edited));
    assert_eq!(
        serde_json::to_value(&after.files[0].diagnostics).unwrap(),
        serde_json::to_value(&cold.files[0].diagnostics).unwrap()
    );
}
