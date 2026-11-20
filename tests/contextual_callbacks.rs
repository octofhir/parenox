use parenox::{Session, Source};

const EXAMPLE: &str = include_str!("../examples/contextual_callbacks.clj");

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
