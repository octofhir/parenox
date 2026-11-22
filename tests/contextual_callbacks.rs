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
