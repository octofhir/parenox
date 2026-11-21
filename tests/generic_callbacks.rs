use parenox::{DocumentHover, Report, Session, Source};

const EXAMPLE: &str = include_str!("../examples/generic_callbacks.clj");

fn check(extra: &str) -> Report {
    Session::default().check("generics.clj", &format!("{EXAMPLE}\n{extra}"))
}

#[test]
fn infers_one_type_for_lambdas_and_generic_forwarding() {
    let report = check(
        "(defn returns-nil {:t/sig '[:-> nil?]} []
           (transform (fn [value] value) nil))
         (defn named {:t/sig '[number? :-> number?]} [x] (inc x))
         (defn apply-named {:t/sig '[:-> number?]} [] (apply transform [named 1]))",
    );
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn infers_callback_results_independently_for_each_call() {
    let report = check(
        "(defn rendered {:t/sig '[number? :-> string?]} [id]
           (convert #(str (inc %)) id))
         (defn nil-result {:t/sig '[:-> nil?]} [] (convert (fn [_] nil) 1))
         (defn forward-convert
           {:t/sig '(t/all [a b] [(s/fspec :args (s/cat :arg a) :ret b) a :-> b])}
           [f x] (convert #(f %) x))
         (defn forwarded {:t/sig '[string? :-> number?]} [text]
           (forward-convert #(count %) text))",
    );
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn later_argument_failure_keeps_generic_callback_compiler_checks() {
    let source = format!(
        "{EXAMPLE}\n{}",
        r#"(defn stopped {:t/sig '[:-> t/any]} []
             (convert (fn [arg] (subs :dead-callback 0) missing-callback-name)
                      (subs :bad-argument 0)))"#
    );
    let report = Session::default().check("generics.clj", &source);
    let findings: Vec<_> = report
        .diagnostics
        .iter()
        .map(|finding| {
            (
                finding.code.as_str(),
                &source[finding.span.start.offset..finding.span.end.offset],
            )
        })
        .collect();
    assert_eq!(
        findings,
        [
            ("PNX2304", "missing-callback-name"),
            ("PNX2322", ":bad-argument"),
        ],
        "{report:#?}"
    );
}

#[test]
fn fixed_callbacks_receive_context_after_operands_can_return() {
    let source = format!(
        "{EXAMPLE}\n{}",
        r#"(defn run-fixed
             {:t/sig '[(s/fspec :args (s/cat :x string?) :ret string?) string? :-> string?]}
             [f x] (f x))
           (defn accepts-at-runtime {:t/sig '[string? :-> string?]} [_] "kept")
           (defn use-fixed {:t/sig '[:-> string?]} []
             (run-fixed ^:probe (fn [item] (str item)) (subs "kept" 0)))"#
    );
    let offset = source.find("str item").unwrap() + "str ".len();
    let mut session = Session::default();
    for (operand, inferred) in [
        ("(subs \"kept\" 0)", Some("String")),
        ("(subs nil 0)", None),
        ("(accepts-at-runtime nil)", Some("String")),
        ("(subs \"kept\" 0)", Some("String")),
    ] {
        let edited = source.replace("(subs \"kept\" 0)", operand);
        let report = session.check("generics.clj", &edited);
        if operand.contains("nil") {
            let [finding] = report.diagnostics.as_slice() else {
                panic!("{report:#?}");
            };
            assert_eq!(finding.code, "PNX2322");
            assert_eq!(
                &edited[finding.span.start.offset..finding.span.end.offset],
                "nil"
            );
        } else {
            assert!(report.ok && report.complete, "{report:#?}");
        }
        let hover = session.document_hover_at("generics.clj", offset);
        assert_eq!(
            hover.map(|hover| {
                let DocumentHover::Local(local) = hover else {
                    panic!("expected a local callback argument");
                };
                local.inferred_type
            }),
            inferred.map(str::to_owned)
        );
    }
}

#[test]
fn a_dead_contextual_call_checks_closure_syntax_without_invoking_it() {
    let source = format!(
        "{EXAMPLE}\n{}",
        r#"(defn run-fixed
             {:t/sig '[(s/fspec :args (s/cat :x string?) :ret string?) string? :-> string?]}
             [f x] (f x))
           (defn compile-only {:t/sig '[:-> string?]} []
             (run-fixed (fn self [x y] (self x y) missing-callback-name)
                        (subs :bad-argument 0)))"#
    );
    let report = Session::default().check("generics.clj", &source);
    let findings: Vec<_> = report
        .diagnostics
        .iter()
        .map(|finding| {
            (
                finding.code.as_str(),
                &source[finding.span.start.offset..finding.span.end.offset],
            )
        })
        .collect();
    assert_eq!(
        findings,
        [
            ("PNX2304", "missing-callback-name"),
            ("PNX2322", ":bad-argument"),
        ],
        "{report:#?}"
    );
}

#[test]
fn stored_callbacks_get_context_and_specialize_again_at_each_generic_call() {
    let source = format!(
        "{EXAMPLE}\n{}",
        r#"(defn both {:t/sig '[string? number? :-> (s/tuple string? number?)]} [word n]
             (let [choose (fn [value] value) callback choose]
               [(convert callback word) (convert callback n)]))
           (defn fixed {:t/sig '[(s/fspec :args (s/cat :x string?) :ret string?) :-> string?]} [f]
             (f "arg"))
           (defn use-fixed {:t/sig '[:-> string?]} []
             (let [trimmed (fn [word] (str/trim word))]
               (fixed trimmed)))"#
    );
    let mut session = Session::default();
    let report = session.check("generics.clj", &source);
    assert!(report.ok && report.complete, "{report:#?}");
    let offset = source.rfind("value] value").unwrap() + "value] ".len();
    let Some(DocumentHover::Local(value)) = session.document_hover_at("generics.clj", offset)
    else {
        panic!("expected a type at the shared callback body");
    };
    assert_eq!(value.inferred_type, "Number | String");
}

#[test]
fn a_stored_callback_reports_the_failed_operand_and_the_consuming_contract() {
    let source = format!(
        "{EXAMPLE}\n{}",
        r#"(defn bad {:t/sig '[string? :-> number?]} [word]
             (let [bump (fn [value] (inc value))]
               (convert bump word)))"#
    );
    let report = Session::default().check("generics.clj", &source);
    let [diagnostic] = report.diagnostics.as_slice() else {
        panic!("{report:#?}")
    };
    assert!(report.complete);
    assert_eq!(diagnostic.code, "PNX2322");
    assert_eq!(
        &source[diagnostic.span.start.offset..diagnostic.span.end.offset],
        "value"
    );
    assert_eq!(diagnostic.actual.as_deref(), Some("String"));
    assert_eq!(
        diagnostic.callback_contract.as_ref().unwrap().name,
        "demo.generic-callbacks/convert"
    );

    let wrong_arity = "(defn bad {:t/sig '[:-> number?]} []
                        (let [too-many (fn [a b] a)] (convert too-many 0)))";
    let report = check(wrong_arity);
    let arity = report
        .diagnostics
        .iter()
        .find(|finding| finding.code == "PNX2554")
        .unwrap();
    assert!(arity.message.contains("expects 2 arguments, got 1"));
    let start = EXAMPLE.len() + 1 + wrong_arity.rfind("too-many").unwrap();
    assert_eq!(arity.span.start.offset, start);
    assert_eq!(arity.span.end.offset, start + "too-many".len());
}

#[test]
fn uses_only_the_matching_arity_of_a_named_callback_including_exact_apply() {
    let report = check(
        "(defn render {:t/sig '[[:-> number?] [number? :-> string?]]}
           ([] 1) ([id] (str id)))
         (defn direct {:t/sig '[:-> string?]} [] (convert render 42))
         (defn spread {:t/sig '[:-> string?]} [] (apply convert [render 42]))",
    );
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn vector_elements_supply_callback_inputs_and_the_result_can_be_a_vector() {
    let report = check(
        "(defn render {:t/sig '[number? :-> string?]} [x] (str (inc x)))
         (defn literal {:t/sig '[:-> ::names]} [] (map-values #(str (inc %)) [1 2]))
         (defn spread {:t/sig '[:-> ::names]} [] (apply map-values [render [1 2]]))
         (defn branch {:t/sig '[boolean? :-> ::names]} [keep]
           (map-values #(str (inc %)) (if keep [] [1 2])))
         (defn forward-map
           {:t/sig '(t/all [x y] [(s/fspec :args (s/cat :arg x) :ret y)
                                 (s/coll-of x :kind vector?) :-> (s/coll-of y :kind vector?)])}
           [f xs] (map-values #(f %) xs))",
    );
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn vector_evidence_checks_each_possible_element_type() {
    let report = check(
        "(defn bad {:t/sig '[:-> ::names]} []
           (map-values #(str (inc %)) [1 \"two\"]))",
    );
    assert!(!report.ok, "{report:#?}");
    assert!(
        report.diagnostics.iter().any(|d| {
            d.code == "PNX2322"
                && d.message.contains("inc")
                && d.actual.as_deref() == Some("Number | String")
        }),
        "{report:#?}"
    );
}

#[test]
fn a_generic_vector_contract_does_not_promise_a_first_element() {
    let report = check(
        "(defn unsafe-head
           {:t/sig '(t/all [a] [(s/coll-of a :kind vector?) :-> a])}
           [xs] (first xs))",
    );
    let [diagnostic] = report.diagnostics.as_slice() else {
        panic!("{report:#?}")
    };
    assert_eq!(diagnostic.code, "PNX2322");
    assert_eq!(diagnostic.expected.as_deref(), Some("a"));
    assert!(diagnostic.actual.as_deref().unwrap().contains("Nil"));
}

#[test]
fn empty_literal_cannot_invent_an_element_type_but_a_direct_argument_can_supply_it() {
    let example = "(defn fill
         {:t/sig '(t/all [a] [(s/coll-of a :kind vector?) a :-> (s/coll-of a :kind vector?)])}
         [xs x] (conj xs x))
       (defn with-direct {:t/sig '[:-> ::names]} [] (fill [] \"Ada\"))";
    let good = check(example);
    assert!(good.ok && good.complete, "{good:#?}");

    let source = format!(
        "{EXAMPLE}\n{example}\n(defn without-evidence {{:t/sig '[:-> ::names]}} [] (map-values #(str %) []))"
    );
    let report = Session::default().check("generics.clj", &source);
    assert!(!report.complete, "{report:#?}");
    let gaps: Vec<_> = report
        .diagnostics
        .iter()
        .filter(|d| d.code == "PNX9010")
        .collect();
    let [gap] = gaps.as_slice() else {
        panic!("{report:#?}")
    };
    assert!(gap.message.contains("vector's elements"));
    assert_eq!(&source[gap.span.start.offset..gap.span.end.offset], "[]");
}

#[test]
fn a_seq_is_not_evidence_for_a_vector_parameter() {
    let report = check("(defn bad {:t/sig '[:-> ::names]} [] (map-values #(str %) (list 1)))");
    assert!(!report.complete, "{report:#?}");
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| { d.code == "PNX2322" && d.expected.as_deref() == Some("Vector<Unknown>") }),
        "{report:#?}"
    );
}

#[test]
fn generic_externs_can_describe_vector_transforms() {
    let app = Source {
        path: "app.clj",
        source: "(ns app (:require [vendor :as v] [clojure.spec.alpha :as s]))
          (defn labels {:t/sig '[:-> (s/coll-of string? :kind vector?)]} []
            (v/map-values #(str (inc %)) [1 2]))",
    };
    let externs = Source {
        path: "externs.edn",
        source: "{vendor/map-values
          (t/all [a b] [(clojure.spec.alpha/fspec :args (clojure.spec.alpha/cat :x a) :ret b)
                        (clojure.spec.alpha/coll-of a :kind vector?)
                        :-> (clojure.spec.alpha/coll-of b :kind vector?)])}",
    };
    let report = Session::default().check_project(&[app], Some(externs));
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn checks_named_callback_inputs_and_the_inferred_call_result() {
    let report = check(
        "(defn render {:t/sig '[number? :-> string?]} [id] (str id))
         (defn wrong-input {:t/sig '[:-> string?]} [] (convert render \"id\"))
         (defn wrong-result {:t/sig '[:-> number?]} [] (convert #(str (inc %)) 42))",
    );
    assert!(report.complete, "{report:#?}");
    let [input, output] = report.diagnostics.as_slice() else {
        panic!("{report:#?}")
    };
    assert_eq!(input.code, "PNX2322");
    assert!(input.message.contains("Argument 1"));
    assert_eq!(output.code, "PNX2322");
    assert_eq!(output.expected.as_deref(), Some("Number"));
    assert_eq!(output.actual.as_deref(), Some("String"));
}

#[test]
fn rigid_input_and_output_are_not_interchangeable_even_in_a_recursive_call() {
    let report = check(
        "(defn swapped
           {:t/sig '(t/all [a b] [(s/fspec :args (s/cat :arg a) :ret b) a :-> b])}
           [f x] (swapped (fn [_] x) (f x)))",
    );
    let [result] = report.diagnostics.as_slice() else {
        panic!("{report:#?}")
    };
    assert_eq!(result.code, "PNX2322");
    assert_eq!(result.expected.as_deref(), Some("b"));
    assert_eq!(result.actual.as_deref(), Some("a"));
}

#[test]
fn a_rejected_callback_with_a_checked_result_has_one_type_error() {
    let body = "#(inc (str %))";
    let source = format!("{EXAMPLE}\n(defn bad {{:t/sig '[:-> number?]}} [] (convert {body} 1))");
    let report = Session::default().check("generics.clj", &source);
    assert!(report.complete);
    let [error] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(error.code, "PNX2322");
    assert_eq!(error.contract.as_ref().unwrap().name, "clojure.core/inc");
    assert_eq!(
        error.callback_contract.as_ref().unwrap().name,
        "demo.generic-callbacks/convert"
    );
    assert_eq!(
        &source[error.span.start.offset..error.span.end.offset],
        "(str %)"
    );
}

#[test]
fn an_independent_coverage_gap_survives_a_callback_type_error() {
    let report = check(
        "(defn erased {:t/sig '[number? :-> t/any]} [n] n)
         (defn mixed {:t/sig '[boolean? :-> string?]} [bad-number?]
           (convert (fn [n] (if bad-number? (inc \"not a number\") (subs (erased n) 0))) 1))",
    );
    assert!(!report.complete);
    let [gap, error] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    assert_eq!(gap.code, "PNX9010");
    assert!(gap.message.contains("this callback's result"));
    assert_eq!(error.code, "PNX2322");
    assert_eq!(error.contract.as_ref().unwrap().name, "clojure.core/inc");
}

#[test]
fn conflicting_arguments_and_callback_results_are_not_joined_into_a_wider_type() {
    let source = format!(
        "{EXAMPLE}
         (defn choose-first {{:t/sig '(t/all [a] [a a :-> a])}} [a b] a)
         (defn bad-argument {{:t/sig '[:-> number?]}} [] (choose-first 1 \"text\"))
         (defn bad-result {{:t/sig '[:-> number?]}} [] (transform (fn [x] \"callback\") 1))",
    );
    let report = Session::default().check("generics.clj", &source);
    let [argument, result] = report.diagnostics.as_slice() else {
        panic!("{report:#?}");
    };
    for (diagnostic, label) in [(argument, "\"text\""), (result, "\"callback\"")] {
        assert_eq!(diagnostic.code, "PNX2322");
        assert_eq!(diagnostic.expected.as_deref(), Some("Number"));
        assert_eq!(
            &source[diagnostic.span.start.offset..diagnostic.span.end.offset],
            label
        );
    }
    assert!(report.complete);
}

#[test]
fn generic_bodies_cannot_assume_properties_of_their_parameter() {
    let report = check(
        "(defn numeric-only {:t/sig '(t/all [a] [a :-> a])} [x] (inc x))
         (defn not-identity {:t/sig '(t/all [a] [a :-> a])} [x]
           (if (string? x) 1 x))
         (defn not-always-nil {:t/sig '(t/all [a] [a :-> nil?])} [x]
           (get x :key))
         (defn set-test {:t/sig '(t/all [a] [a :-> nil?])} [x]
           (if (contains? #{x} :key) 42 nil))",
    );
    assert!(!report.ok && !report.complete, "{report:#?}");
    for (name, code) in [
        ("not-identity", "PNX2322"),
        ("not-always-nil", "PNX9004"),
        ("set-test", "PNX2322"),
    ] {
        assert!(
            report
                .diagnostics
                .iter()
                .any(|d| d.code == code && d.message.contains(name)),
            "{report:#?}"
        );
    }
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.actual.as_deref() == Some("a"))
    );
}

#[test]
fn missing_evidence_is_incomplete_and_leaves_the_call_reachable() {
    let report = check(
        "(defn no-evidence {:t/sig '[:-> number?]} []
           (transform #(inc %) (missing)))",
    );
    assert!(!report.ok && !report.complete, "{report:#?}");
    assert!(report.diagnostics.iter().any(|d| d.code == "PNX2304"));
    assert!(report.diagnostics.iter().any(|d| d.code == "PNX9010"));
}

#[test]
fn missing_evidence_does_not_hide_independent_mismatches() {
    let report = check(
        "(defn with-label
           {:t/sig '(t/all [a] [string? (s/fspec :args (s/cat :s string?) :ret string?) a :-> a])}
           [label f x] x)
         (defn bad {:t/sig '[:-> number?]} []
           (with-label 1 (fn [text] false) (missing)))",
    );
    assert!(!report.ok && !report.complete, "{report:#?}");
    assert!(report.diagnostics.iter().any(|d| d.code == "PNX9010"));
    for actual in ["Number", "false"] {
        assert!(
            report.diagnostics.iter().any(|d| {
                d.code == "PNX2322"
                    && d.expected.as_deref() == Some("String")
                    && d.actual.as_deref() == Some(actual)
            }),
            "{report:#?}"
        );
    }
}

#[test]
fn an_eager_argument_can_throw_before_the_deferred_callback_body() {
    let report = check(
        "(defn ordered {:t/sig '[:-> #{:argument}]} []
           (try
             (transform
               (fn [x] (throw (ex-info \"body\" {:stage :callback})))
               (throw (ex-info \"argument\" {:stage :argument})))
             (catch clojure.lang.ExceptionInfo e (:stage (ex-data e)))))",
    );
    assert!(report.ok && report.complete, "{report:#?}");
}

#[test]
fn generic_apply_reports_an_open_spread_as_unsupported() {
    let report = check(
        "(defn open {:t/sig '[(s/coll-of number? :kind vector?) :-> number?]} [values]
           (apply transform values))",
    );
    assert!(!report.ok && !report.complete, "{report:#?}");
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| { d.code == "PNX9001" && d.message.contains("open spread") }),
        "{report:#?}"
    );
}

#[test]
fn unimplemented_parameter_positions_are_explicitly_rejected() {
    for (signature, parameters) in [
        ("(t/all [a b] [a :-> a])", "[x]"),
        ("(t/all [a a] [a :-> a])", "[x]"),
        (
            "(t/all [a b] [(s/fspec :args (s/cat :in b) :ret b) a :-> b])",
            "[f x]",
        ),
        ("(t/all [a b c] [a b c :-> a])", "[x y z]"),
        (
            "(t/all [a b c] [(s/fspec :args (s/cat :in a) :ret b) (s/fspec :args (s/cat :in a) :ret c) a :-> {:b b :c c}])",
            "[f g x]",
        ),
        ("(t/all [a] [a (s/nilable a) :-> a])", "[x y]"),
        ("(t/all [a] [a & a :-> a])", "[x & more]"),
        ("(t/all [a] [(s/coll-of a :kind set?) :-> a])", "[x]"),
        (
            "(t/all [a] [a :-> (s/coll-of (s/coll-of a :kind vector?) :kind vector?)])",
            "[x]",
        ),
    ] {
        let report = check(&format!(
            "(defn unsupported {{:t/sig '{signature}}} {parameters} x)"
        ));
        assert_eq!(report.diagnostics.len(), 1, "{report:#?}");
        assert_eq!(report.diagnostics[0].code, "PNX9001");
    }
}

#[test]
fn editing_an_extern_generic_contract_rechecks_the_contextual_body() {
    let app = Source {
        path: "app.clj",
        source: "(ns app (:require [vendor :as v]))
          (defn label {:t/sig '[:-> string?]} []
            (v/transform #(clojure.string/trim %) \"text\"))
          (defn independent {:t/sig '[:-> number?]} [] 1)",
    };
    let externs = Source {
        path: "externs.edn",
        source: "{vendor/transform
          (t/all [a b] [(clojure.spec.alpha/fspec :args (clojure.spec.alpha/cat :x a) :ret b)
                        a :-> b])}",
    };
    let mut session = Session::default();
    assert!(session.check_project(&[app], Some(externs)).ok);
    let edited = externs.source.replace(":x a", ":x number?");
    let changed = Source {
        source: &edited,
        ..externs
    };
    let result = session.check_project(&[app], Some(changed));
    assert_eq!(result.files[0].stats.checked_functions, 1);
    assert_eq!(result.files[0].stats.reused_functions, 1);
    assert!(
        result.files[0]
            .diagnostics
            .iter()
            .any(|d| { d.code == "PNX2322" && d.actual.as_deref() == Some("Number") })
    );
}
